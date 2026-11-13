"""Disk guard boundaries and sampled per-job usage without running a build."""
import importlib.util
import json
import sys
import os
import pathlib
import subprocess
import tempfile
import textwrap
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('blade_disk', ROOT / 'ci/blade-disk.py')
disk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(disk)


class DiskBudget(unittest.TestCase):
    def test_preflight_fails_below_fifteen_gib_before_installing_anything(self):
        text = (ROOT / '.github/workflows/hosted.yml').read_text()
        scripts = [textwrap.dedent(block.split('        run: |\n', 1)[1].split('      - ', 1)[0])
                   for block in text.split('      - name: Prepare Blade Ubuntu container\n')[1:]]
        self.assertEqual(len(set(scripts)), 1)
        guard = scripts[0].split('export DEBIAN_FRONTEND', 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            marker = root / 'container'
            marker.touch()
            df = root / 'df'
            df.write_text('#!/bin/sh\nprintf "Filesystem 1024-blocks Used Available Capacity Mounted\\nfixture 999 1 %s 1%% /\\n" "$AVAILABLE_KIB"\n')
            df.chmod(0o755)
            for available, expected in [('15728639', 1), ('15728640', 0), ('20000000', 0), ('bad', 1)]:
                result = subprocess.run(['bash', '-e', '-c', guard.replace('/.dockerenv', str(marker))],
                                        env=dict(os.environ, PATH=f'{root}:{os.environ["PATH"]}',
                                                 GITHUB_WORKSPACE=str(root), LINUX_ARCHIVE_WORKSPACE=str(root),
                                                 AVAILABLE_KIB=available),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, expected, result.stderr)
                if expected:
                    self.assertIn('requires at least 15 GiB free', result.stdout)

    def test_peak_counts_aliases_once_and_never_decreases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            workspace, temporary = root / 'workspace', root / 'temporary'
            workspace.mkdir()
            temporary.mkdir()
            alias = root / 'canonical'
            alias.symlink_to(workspace, target_is_directory=True)
            (workspace / 'target').write_bytes(b'x' * 50000)
            state = {}
            with mock.patch.object(disk, 'layer_bytes', return_value=10000):
                disk.sample(state, workspace, temporary, alias)
                first = dict(state)
                (workspace / 'target').unlink()
                disk.sample(state, workspace, temporary, alias)
            self.assertEqual(state['peak_bytes'], first['peak_bytes'])
            self.assertEqual(state['peak_bytes'], state['peak_bound_bytes'] + 10000)
            self.assertEqual(state['samples'], 2)
            self.assertLess(disk.disk_bytes([workspace, workspace]), 50000)

    def test_rust_homes_stay_inside_the_ignored_build_directory(self):
        text = (ROOT / '.github/workflows/hosted.yml').read_text()
        self.assertIn('echo "CARGO_HOME=$GITHUB_WORKSPACE/target/.ci-cargo"', text)
        self.assertIn('echo "RUSTUP_HOME=$GITHUB_WORKSPACE/target/.ci-rustup"', text)
        self.assertIn('echo "CARGO_TARGET_DIR=$GITHUB_WORKSPACE/target"', text)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            (root / '.gitignore').write_text((ROOT / '.gitignore').read_text())
            for name in ('target/.ci-cargo/marker', 'target/.ci-rustup/marker', '.ci-cargo/old-marker'):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.touch()
            listed = subprocess.check_output(['git', '-C', str(root), 'ls-files', '--others',
                                              '--exclude-standard'], text=True)
            self.assertIn('.ci-cargo/old-marker', listed)
            self.assertNotIn('target/', listed)

    @unittest.skipUnless(sys.platform == 'linux', 'Blade monitor uses Linux process identities')
    def test_monitor_stops_only_its_saved_process_and_writes_peak_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            workspace, temporary = root / 'workspace', root / 'temporary'
            workspace.mkdir()
            temporary.mkdir()
            summary = root / 'summary'
            env = dict(os.environ, GITHUB_WORKSPACE=str(workspace), RUNNER_TEMP=str(temporary),
                       LINUX_ARCHIVE_WORKSPACE=str(workspace), GITHUB_JOB='fixture',
                       GITHUB_STEP_SUMMARY=str(summary))
            helper = str(ROOT / 'ci/blade-disk.py')
            try:
                subprocess.run([sys.executable, helper, 'start'], env=env, check=True,
                               capture_output=True, text=True, timeout=40)
                subprocess.run([sys.executable, helper, 'stop'], env=env, check=True,
                               capture_output=True, text=True, timeout=50)
                state = json.loads((temporary / 'blade-disk.json').read_text())
                self.assertTrue(state['stopped'])
                self.assertGreater(state['samples'], 0)
                self.assertIn('Sampled peak:', summary.read_text())
                # A stale saved PID must never terminate a different process.
                control = temporary / 'blade-disk-control.json'
                control.write_text(json.dumps({'pid': os.getpid(), 'token': 'unrelated-process-token'}))
                with mock.patch.dict(os.environ, env), mock.patch.object(disk.os, 'kill') as kill:
                    disk.stop(temporary / 'blade-disk.json', control)
                    kill.assert_not_called()
            finally:
                subprocess.run([sys.executable, helper, 'stop'], env=env, capture_output=True,
                               text=True, timeout=50)

    def test_sampling_tolerates_deleted_temporary_files_but_not_access_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            vanished = subprocess.CompletedProcess([], 1, '42 workspace\n',
                                                   'du: cannot access temporary: No such file or directory\n')
            with mock.patch.object(disk.subprocess, 'run', return_value=vanished):
                self.assertEqual(disk.disk_bytes([root]), 42)
            denied = subprocess.CompletedProcess([], 1, '42 workspace\n', 'du: Permission denied\n')
            with mock.patch.object(disk.subprocess, 'run', return_value=denied):
                with self.assertRaisesRegex(RuntimeError, 'Permission denied'):
                    disk.disk_bytes([root])
