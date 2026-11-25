"""The Cargo.lock merge driver: Cargo's format, a clean merge Cargo accepts, and real conflicts."""
import importlib.util
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
DRIVER = ROOT / 'ci/cargo-lock-merge.py'
spec = importlib.util.spec_from_file_location('cargo_lock_merge', DRIVER)
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


def crate(root, name, version='0.1.0', dependencies='', dev_dependencies=''):
    directory = root / 'crates' / name
    (directory / 'src').mkdir(parents=True, exist_ok=True)
    (directory / 'src/lib.rs').write_text('')
    (directory / 'Cargo.toml').write_text(
        f'[package]\nname = "{name}"\nversion = "{version}"\nedition = "2021"\n\n'
        f'[dependencies]\n{dependencies}\n\n\n[dev-dependencies]\n{dev_dependencies}')


def path(name):
    return f'{name} = {{ path = "../{name}" }}\n'


class CargoLockMerge(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = pathlib.Path(directory.name)

    def run_in(self, *args, check=True):
        return subprocess.run(args, cwd=self.root, capture_output=True, text=True, check=check)

    def commit(self, message):
        self.run_in('cargo', 'generate-lockfile', '--offline')
        self.run_in('git', 'add', '-A')
        self.run_in('git', 'commit', '-q', '-m', message)

    def repository(self, driver=True, unused=False):
        """A workspace whose two branches each add a crate and a dependency of app on it."""
        self.run_in('git', 'init', '-q', '-b', 'main')
        self.run_in('git', 'config', 'user.email', 'ci@example.invalid')
        self.run_in('git', 'config', 'user.name', 'CI')
        self.run_in('git', 'config', 'commit.gpgsign', 'false')
        self.run_in('git', 'config', 'core.hooksPath', '/dev/null')
        self.run_in('git', 'config', 'rerere.enabled', 'false')
        if driver:
            (self.root / 'ci').mkdir()
            shutil.copyfile(DRIVER, self.root / 'ci/cargo-lock-merge.py')
            self.run_in('git', 'config', 'merge.cargo-lock.driver', 'python3 ci/cargo-lock-merge.py %O %A %B %L')
        (self.root / '.gitattributes').write_text((ROOT / '.gitattributes').read_text())
        (self.root / 'Cargo.toml').write_text('[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n')
        crate(self.root, 'core')
        if unused:
            crate(self.root, 'unused')
        crate(self.root, 'app', dependencies=path('core'))
        self.commit('base')
        self.run_in('git', 'checkout', '-q', '-b', 'side')
        crate(self.root, 'beta')
        crate(self.root, 'app', dependencies=path('core'), dev_dependencies=path('beta'))
        self.commit('side adds beta')
        self.run_in('git', 'checkout', '-q', 'main')
        crate(self.root, 'alpha')
        crate(self.root, 'app', dependencies=path('alpha') + path('core'))
        self.commit('main adds alpha')

    def test_the_repository_lockfile_is_written_back_byte_for_byte(self):
        text = (ROOT / 'Cargo.lock').read_text()
        self.assertEqual(driver.merge(text, text, text), text)

    def test_the_repository_names_the_driver_for_cargo_lock(self):
        result = subprocess.run(['git', 'check-attr', 'merge', '--', 'Cargo.lock'], cwd=ROOT,
                                capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout, 'Cargo.lock: merge: cargo-lock\n')

    def test_without_the_driver_the_lockfile_conflicts(self):
        self.repository(driver=False)
        merged = self.run_in('git', 'merge', '--no-edit', 'side', check=False)
        self.assertNotEqual(merged.returncode, 0)
        self.assertIn('CONFLICT (content): Merge conflict in Cargo.lock', merged.stdout)

    def test_a_merge_takes_both_sides_and_cargo_accepts_it_as_locked(self):
        self.repository()
        merged = self.run_in('git', 'merge', '--no-edit', 'side', check=False)
        self.assertEqual(merged.returncode, 0, merged.stdout + merged.stderr)
        lock = (self.root / 'Cargo.lock').read_text()
        self.run_in('cargo', 'metadata', '--locked', '--offline', '--format-version', '1')
        self.run_in('cargo', 'generate-lockfile', '--offline')
        self.assertEqual((self.root / 'Cargo.lock').read_text(), lock)
        self.assertIn('dependencies = [\n "alpha",\n "beta",\n "core",\n]', lock)

    def test_a_rebase_takes_both_sides(self):
        self.repository()
        self.run_in('git', 'checkout', '-q', 'side')
        rebased = self.run_in('git', 'rebase', 'main', check=False)
        self.assertEqual(rebased.returncode, 0, rebased.stdout + rebased.stderr)
        self.run_in('cargo', 'metadata', '--locked', '--offline', '--format-version', '1')

    def test_two_versions_of_one_crate_still_conflict(self):
        self.repository()
        self.run_in('git', 'checkout', '-q', 'side')
        crate(self.root, 'core', version='0.2.0')
        self.commit('side moves core to 0.2.0')
        self.run_in('git', 'checkout', '-q', 'main')
        crate(self.root, 'core', version='0.3.0')
        self.commit('main moves core to 0.3.0')
        merged = self.run_in('git', 'merge', '--no-edit', 'side', check=False)
        self.assertNotEqual(merged.returncode, 0)
        self.assertIn('both sides changed version', merged.stderr)
        self.assertIn('<<<<<<<', (self.root / 'Cargo.lock').read_text())

    def test_a_dependency_on_a_crate_the_other_side_removed_conflicts(self):
        self.repository(unused=True)
        self.run_in('git', 'checkout', '-q', 'side')
        crate(self.root, 'app', dependencies=path('core'), dev_dependencies=path('beta') + path('unused'))
        self.commit('side depends on unused')
        self.run_in('git', 'checkout', '-q', 'main')
        self.run_in('git', 'rm', '-q', '-r', 'crates/unused')
        self.commit('main removes unused')
        merged = self.run_in('git', 'merge', '--no-edit', 'side', check=False)
        self.assertNotEqual(merged.returncode, 0)
        self.assertIn('which the other side removed', merged.stderr)
