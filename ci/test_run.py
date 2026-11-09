"""The installed pre-push hook runs the hosted lint commands and rejects failures."""

import hashlib
import io
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import urllib.error
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent.parent


class LintHook(unittest.TestCase):
    def test_hook_runs_from_a_subdirectory_and_propagates_failures(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            subprocess.run(['git', 'init', '-q', root], check=True)
            for name in ('ci/run', 'ci/bootstrap', 'ci/hooks/pre-push'):
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, target)
            subprocess.run([root / 'ci/bootstrap'], cwd=root, check=True, capture_output=True)
            setting = subprocess.check_output(
                ['git', 'config', '--local', '--get', 'core.hooksPath'], cwd=root, text=True)
            self.assertEqual(setting.strip(), 'ci/hooks')
            binaries = root / 'bin'
            binaries.mkdir()
            for tool in ('cargo', 'python3'):
                path = binaries / tool
                path.write_text('#!/bin/sh\n'
                                f'echo "{tool} $*" >> "$COMMAND_LOG"\n'
                                '[ "$*" != "$FAIL_COMMAND" ] || exit 7\n')
                path.chmod(0o755)
            commands = ['cargo fmt --all --check',
                        'cargo clippy --workspace --all-targets --all-features -- -D warnings',
                        "python3 -m unittest discover -s ci -p test_*.py"]
            log = root / 'commands'
            for failure, expected in (('', commands), ('fmt --all --check', commands[:1]),
                                      (commands[1].removeprefix('cargo '), commands[:2])):
                with self.subTest(failure=failure):
                    log.unlink(missing_ok=True)
                    env = dict(os.environ, PATH=f'{binaries}:{os.environ["PATH"]}',
                               COMMAND_LOG=str(log), FAIL_COMMAND=failure)
                    result = subprocess.run([root / 'ci/hooks/pre-push'], cwd=root / 'ci', env=env)
                    self.assertEqual(result.returncode, 7 if failure else 0)
                    self.assertEqual(log.read_text().splitlines(), expected)


def pinned_payload(binary):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode='w:gz') as archive:
        entry = tarfile.TarInfo('cargo-nextest')
        entry.size = len(binary)
        archive.addfile(entry, io.BytesIO(binary))
    return stream.getvalue()


def bootstrap_script():
    return (ROOT / 'ci/install-tool.py').read_text()


def http_error(code):
    return urllib.error.HTTPError('https://example.invalid', code, 'error', {}, None)


class PinnedTools(unittest.TestCase):
    def test_raw_packaging_helper_is_verified_and_installed_without_extraction(self):
        payload = b'#!/bin/sh\necho helper\n'
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / 'repo'
            (root / 'ci').mkdir(parents=True)
            prefix = pathlib.Path(directory) / 'tools'
            asset = {'target': 'x86_64', 'format': 'raw',
                     'url': 'https://api.github.com/repos/example/tools/releases/assets/1',
                     'sha256': hashlib.sha256(payload).hexdigest()}
            (root / 'ci/tools').write_text(json.dumps({'helper': {'version': 'asset-1',
                'binary': 'helper.AppImage', 'platforms': {'Linux-x86_64': asset}}}))
            with mock.patch.object(sys, 'argv', ['bootstrap', str(root), str(prefix), 'helper']), \
                 mock.patch('platform.system', return_value='Linux'), \
                 mock.patch('platform.machine', return_value='x86_64'), \
                 mock.patch('urllib.request.urlopen', return_value=io.BytesIO(payload)) as download:
                exec(compile(bootstrap_script(), 'bootstrap', 'exec'), {'__name__': '__main__'})
                self.assertEqual(download.call_args.args[0].full_url, asset['url'])
                self.assertEqual((prefix / 'bin/helper.AppImage').read_bytes(), payload)
                self.assertTrue(os.access(prefix / 'bin/helper.AppImage', os.X_OK))

    def test_verified_binary_survives_outage_and_corruption_triggers_repair(self):
        binary = b'#!/bin/sh\necho pinned-nextest\n'
        payload = pinned_payload(binary)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / 'repo'
            (root / 'ci').mkdir(parents=True)
            prefix = pathlib.Path(directory) / 'tools'
            asset = {'target': 'x86_64-unknown-linux-gnu',
                     'sha256': hashlib.sha256(payload).hexdigest()}
            (root / 'ci/tools').write_text(json.dumps({'cargo-nextest': {
                'version': '0.9.148', 'platforms': {'Linux-x86_64': asset}}}))
            with mock.patch.object(sys, 'argv', ['bootstrap', str(root), str(prefix)]), \
                 mock.patch('platform.system', return_value='Linux'), \
                 mock.patch('platform.machine', return_value='x86_64'):
                with mock.patch('urllib.request.urlopen', return_value=io.BytesIO(payload)) as download:
                    exec(compile(bootstrap_script(), 'bootstrap', 'exec'), {'__name__': '__main__'})
                    download.assert_called_once()
                with mock.patch('urllib.request.urlopen', side_effect=AssertionError('network is down')) as download:
                    exec(compile(bootstrap_script(), 'bootstrap', 'exec'), {'__name__': '__main__'})
                    download.assert_not_called()
                (prefix / 'bin/cargo-nextest').write_bytes(b'corrupt')
                with mock.patch('urllib.request.urlopen', return_value=io.BytesIO(payload)) as download:
                    exec(compile(bootstrap_script(), 'bootstrap', 'exec'), {'__name__': '__main__'})
                    download.assert_called_once()
                self.assertEqual((prefix / 'bin/cargo-nextest').read_bytes(), binary)

    def test_installer_checks_hash_and_writes_only_the_given_prefix(self):
        script = bootstrap_script()
        binary = b'#!/bin/sh\necho pinned-nextest\n'
        payload = pinned_payload(binary)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / 'repo'
            (root / 'ci').mkdir(parents=True)
            prefix = pathlib.Path(directory) / 'tools'
            asset = {'target': 'x86_64-unknown-linux-gnu',
                     'sha256': hashlib.sha256(payload).hexdigest()}
            tools = {'cargo-nextest': {'version': '0.9.148', 'platforms': {'Linux-x86_64': asset}}}
            for valid in (True, False):
                if not valid:
                    asset['sha256'] = '0' * 64
                (root / 'ci/tools').write_text(json.dumps(tools))
                with mock.patch.object(sys, 'argv', ['bootstrap', str(root), str(prefix)]), \
                     mock.patch('platform.system', return_value='Linux'), \
                     mock.patch('platform.machine', return_value='x86_64'), \
                     mock.patch('urllib.request.urlopen', return_value=io.BytesIO(payload)):
                    if valid:
                        exec(compile(script, 'bootstrap', 'exec'), {'__name__': '__main__'})
                    else:
                        with self.assertRaisesRegex(SystemExit, 'checksum mismatch'):
                            exec(compile(script, 'bootstrap', 'exec'), {'__name__': '__main__'})
                self.assertEqual((prefix / 'bin/cargo-nextest').read_bytes(), binary)
            self.assertEqual({str(p.relative_to(prefix)) for p in prefix.rglob('*')},
                             {'bin', 'bin/cargo-nextest', '.cargo-nextest.json'})

    def run_with_downloads(self, downloads):
        """Runs the installer against `downloads`, one per urlopen call, and returns the
        number of calls and the backoff delays it slept."""
        binary = b'#!/bin/sh\necho pinned-nextest\n'
        payload = pinned_payload(binary)
        calls = []

        def urlopen(url, timeout):
            outcome = downloads[len(calls)]
            calls.append(url)
            if isinstance(outcome, BaseException):
                raise outcome
            return io.BytesIO(payload)

        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / 'repo'
            (root / 'ci').mkdir(parents=True)
            prefix = pathlib.Path(directory) / 'tools'
            asset = {'target': 'x86_64-unknown-linux-gnu',
                     'sha256': hashlib.sha256(payload).hexdigest()}
            (root / 'ci/tools').write_text(json.dumps(
                {'cargo-nextest': {'version': '0.9.148', 'platforms': {'Linux-x86_64': asset}}}))
            with mock.patch.object(sys, 'argv', ['bootstrap', str(root), str(prefix)]), \
                 mock.patch('platform.system', return_value='Linux'), \
                 mock.patch('platform.machine', return_value='x86_64'), \
                 mock.patch('urllib.request.urlopen', side_effect=urlopen), \
                 mock.patch('time.sleep') as sleep, \
                 mock.patch('sys.stderr', io.StringIO()):
                try:
                    exec(compile(bootstrap_script(), 'bootstrap', 'exec'), {'__name__': '__main__'})
                    installed = (prefix / 'bin/cargo-nextest').read_bytes() == binary
                except BaseException as error:
                    installed = error
            return len(calls), [c.args[0] for c in sleep.call_args_list], installed

    def test_installer_retries_a_transient_forge_failure(self):
        """CI shards failed on a 503 and on a read timeout from the release download."""
        calls, delays, installed = self.run_with_downloads(
            [http_error(503), TimeoutError('The read operation timed out'),
             ConnectionResetError(), http_error(429), 'ok'])
        self.assertIs(installed, True)
        self.assertEqual((calls, delays), (5, [5, 10, 20, 40]))

    def test_installer_does_not_retry_a_missing_release(self):
        calls, delays, installed = self.run_with_downloads([http_error(404), 'ok'])
        self.assertIsInstance(installed, SystemExit)
        self.assertIn('HTTP 404', str(installed))
        self.assertEqual((calls, delays), (1, []))

    def test_installer_gives_up_after_its_last_attempt(self):
        calls, delays, installed = self.run_with_downloads([http_error(502)] * 5 + ['ok'])
        self.assertIsInstance(installed, SystemExit)
        self.assertIn('failed after 5 attempts (HTTP 502)', str(installed))
        self.assertEqual((calls, len(delays)), (5, 4))
