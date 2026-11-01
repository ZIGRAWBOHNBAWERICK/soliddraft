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


class PinnedTools(unittest.TestCase):
    def test_installer_checks_hash_and_writes_only_the_given_prefix(self):
        bootstrap = (ROOT / 'ci/bootstrap').read_text()
        script = bootstrap.split("<<'PYTHON'\n", 1)[1].rsplit('\nPYTHON', 1)[0]
        binary = b'#!/bin/sh\necho pinned-nextest\n'
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode='w:gz') as archive:
            entry = tarfile.TarInfo('cargo-nextest')
            entry.size = len(binary)
            archive.addfile(entry, io.BytesIO(binary))
        payload = stream.getvalue()
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
                             {'bin', 'bin/cargo-nextest'})
