"""Reuse, progress and retry behavior for the Rust toolchain installer."""
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
INSTALLER = ROOT / "ci/install-rust-toolchain.py"


class RustToolchainInstall(unittest.TestCase):
    def test_workspace_cargo_home_supplies_rustup_with_or_without_bootstrap(self):
        for bootstrap in (False, True):
            with self.subTest(bootstrap=bootstrap), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                cargo_home = root / "target" / ".ci-cargo"
                rustup = cargo_home / "bin" / "rustup"
                github_path = root / "github-path"
                rustup_script = """#!%s
import os, pathlib, shutil, sys
assert os.environ['RUSTUP_HOME'].endswith('/target/.ci-rustup')
args = sys.argv[1:]
if args[:1] == ['--default-toolchain']:
    assert args == ['--default-toolchain', 'none', '--no-modify-path', '-y']
    rustup = pathlib.Path(os.environ['CARGO_HOME']) / 'bin' / 'rustup'
    rustup.parent.mkdir(parents=True)
    shutil.copy2(__file__, rustup)
    sys.exit(0)
with pathlib.Path(os.environ['CALLS']).open('a') as calls:
    calls.write(' '.join(args) + '\\n')
if args[:2] == ['toolchain', 'list'] and not %r:
    print('stable-x86_64-unknown-linux-gnu (default)')
if args[:2] == ['component', 'list']:
    print('clippy-x86_64-unknown-linux-gnu (installed)')
""" % (sys.executable, bootstrap)
                if bootstrap:
                    (root / "installer").write_text(rustup_script)
                    curl = root / "curl"
                    curl.write_text("""#!%s
import pathlib, sys
assert sys.argv[-1].startswith('https://static.rust-lang.org/rustup/dist/')
path = pathlib.Path(sys.argv[sys.argv.index('--output') + 1])
path.write_bytes(pathlib.Path(__file__).with_name('installer').read_bytes())
""" % sys.executable)
                    curl.chmod(0o755)
                else:
                    rustup.parent.mkdir(parents=True)
                    rustup.write_text(rustup_script)
                    rustup.chmod(0o755)

                calls = root / "calls"
                result = subprocess.run(
                    [sys.executable, str(INSTALLER), "stable", "clippy", "", "Lint toolchain setup"],
                    env=dict(os.environ, PATH=str(root), CARGO_HOME=str(cargo_home),
                             RUSTUP_HOME=str(root / "target" / ".ci-rustup"),
                             GITHUB_PATH=str(github_path), CALLS=str(calls)),
                    capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(github_path.read_text().splitlines(), [str(rustup.parent)])
                self.assertEqual("toolchain install" in calls.read_text(), bootstrap)
                self.assertIn("default stable", calls.read_text())

    def test_progressing_install_can_outlast_the_timeout(self):
        for progress in ("output", "download"):
            with self.subTest(progress=progress), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                fake_rustup = root / "rustup"
                fake_rustup.write_text("""#!/usr/bin/env python3
import os, pathlib, sys, time
if sys.argv[1:3] == ['toolchain', 'install']:
    downloads = pathlib.Path(os.environ['RUSTUP_HOME']) / 'downloads'
    downloads.mkdir(parents=True)
    for _ in range(12):
        if %r == 'output':
            sys.stderr.write('downloading component\\r')
            sys.stderr.flush()
        else:
            with (downloads / 'component.partial').open('ab') as download:
                download.write(b'data')
        time.sleep(0.15)
    print('install completed')
""" % progress)
                fake_rustup.chmod(0o755)
                result = subprocess.run(
                    [sys.executable, str(INSTALLER), "stable", "", "", "Toolchain setup"],
                    env=dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                             RUSTUP_HOME=str(root / "rustup-home"),
                             RUSTUP_INSTALL_TIMEOUT_SECONDS="1", RUSTUP_INSTALL_ATTEMPTS="1"),
                    capture_output=True, text=True, timeout=8,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("install completed", result.stdout)

    def test_stalled_install_times_out_retries_and_reports_the_step(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            marker = root / "attempts"
            fake_rustup = root / "rustup"
            fake_rustup.write_text("""#!/usr/bin/env python3
import os, pathlib, sys, time
args = sys.argv[1:]
if args[:2] == ['toolchain', 'list']:
    sys.exit(0)
if args[:2] == ['toolchain', 'install']:
    marker = pathlib.Path(%r)
    marker.write_text(marker.read_text() + 'x' if marker.exists() else 'x')
    downloads = pathlib.Path(os.environ['RUSTUP_HOME']) / 'downloads'
    downloads.mkdir(parents=True, exist_ok=True)
    (downloads / 'component.partial').write_bytes(b'data')
    time.sleep(10)
sys.exit(0)
""" % str(marker))
            fake_rustup.chmod(0o755)
            result = subprocess.run(
                [sys.executable, str(INSTALLER), "stable", "rustfmt,clippy", "",
                 "Lint toolchain setup"],
                env=dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                         RUSTUP_HOME=str(root / "rustup-home"),
                         RUSTUP_INSTALL_TIMEOUT_SECONDS="1", RUSTUP_INSTALL_ATTEMPTS="2"),
                capture_output=True, text=True, timeout=8,
            )
            attempts = marker.read_text()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(attempts, "xx")
        self.assertIn("attempt 1/2", result.stdout)
        self.assertIn("attempt 2/2", result.stdout)
        self.assertEqual(result.stdout.count("timed out after 1s without progress"), 2)
        self.assertIn("Lint toolchain setup: Rust toolchain install failed after 2 attempts", result.stderr)

    def test_installed_toolchain_is_reused_without_installing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            calls = root / "calls"
            fake_rustup = root / "rustup"
            fake_rustup.write_text("""#!/usr/bin/env python3
import pathlib, sys
args = sys.argv[1:]
with pathlib.Path(%r).open('a') as output: output.write(' '.join(args) + '\\n')
if args[:2] == ['toolchain', 'list']:
    print('stable-x86_64-unknown-linux-gnu (default)')
elif args[:2] == ['component', 'list']:
    print('rustfmt-x86_64-unknown-linux-gnu (installed)')
elif args[:2] == ['target', 'list']:
    pass
""" % str(calls))
            fake_rustup.chmod(0o755)
            result = subprocess.run(
                [sys.executable, str(INSTALLER), "stable", "rustfmt", "",
                 "Lint toolchain setup"],
                env=dict(os.environ, PATH=f"{root}:{os.environ['PATH']}"),
                capture_output=True, text=True, timeout=5,
            )
            rustup_calls = calls.read_text()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("reusing installed Rust toolchain stable", result.stdout)
        self.assertNotIn("toolchain install", rustup_calls)
        self.assertIn("default stable", rustup_calls)


if __name__ == "__main__":
    unittest.main()
