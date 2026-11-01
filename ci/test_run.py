"""The installed pre-push hook runs the hosted lint commands and rejects failures."""

import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

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
