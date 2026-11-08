"""Sub-job selection, accepted-policy union and guards against silently doing no work."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parent.parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'ci' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


selector = load('select-checks')
runner = load('run-selected')


class WorkPlan(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.packages = {'soliddraft-core': set(), 'soliddraft-app': {'soliddraft-core'},
                         'soliddraft-cli': {'soliddraft-app'}, 'soliddraft-web': {'soliddraft-app'}}
        self.git('init', '-q')
        for name in self.packages:
            self.write(f'crates/{name}/Cargo.toml', f'[package]\nname = "{name}"\n')
            self.write(f'crates/{name}/src/lib.rs', '// source\n')
        self.test_path = 'crates/soliddraft-app/tests/isolated.rs'
        self.write(self.test_path, '#[test] fn answer() {}\n')
        self.write('ci/scenarios/bracket.json', '{}')
        self.write('ci/scenarios/fillet.json', '{}')
        self.base = self.commit()

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.root), *args], stderr=subprocess.DEVNULL).decode().strip()

    def write(self, path, text):
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)

    def commit(self):
        self.git('add', '.')
        self.git('-c', 'user.name=CI Test', '-c', 'user.email=ci@example.invalid',
                 'commit', '-qm', 'fixture')
        return self.git('rev-parse', 'HEAD')

    def plan(self, changes, event='pull_request'):
        return selector.work_plan(self.root, self.base, 'HEAD', changes, event, self.packages)

    def test_source_builds_affected_packages_and_transitive_consumers(self):
        plan = self.plan([['M', 'crates/soliddraft-app/src/lib.rs']])
        self.assertEqual(plan, {'packages': ['soliddraft-app', 'soliddraft-cli', 'soliddraft-web'],
                                'test_targets': None, 'scenarios': None})
        self.assertEqual(self.plan([['M', 'crates/soliddraft-core/src/lib.rs']])['packages'],
                         sorted(self.packages))

    def test_test_only_builds_one_independent_existing_binary(self):
        self.assertEqual(self.plan([['M', self.test_path]]), {
            'packages': ['soliddraft-app'], 'test_targets': ['isolated'], 'scenarios': None})

    def test_test_import_on_either_side_keeps_all_consumer_binaries(self):
        importer = 'crates/soliddraft-cli/tests/consumer.rs'
        self.write(importer, '#[path = "../../soliddraft-app/tests/isolated.rs"] mod shared;')
        self.base = self.commit()
        self.write(importer, '// removed import')
        self.commit()
        self.assertIsNone(self.plan([['M', self.test_path], ['M', importer]])['test_targets'])
        # An import in an unchanged file also rules out narrowing to that target.
        self.assertIsNone(self.plan([['M', self.test_path]])['test_targets'])

    def test_new_removed_nested_and_renamed_tests_keep_package_coverage(self):
        for change in (['A', self.test_path], ['D', self.test_path],
                       ['R100', self.test_path, 'crates/soliddraft-core/tests/moved.rs'],
                       ['M', 'crates/soliddraft-app/tests/common/mod.rs']):
            with self.subTest(change=change):
                self.assertIsNone(self.plan([change])['test_targets'])

    def test_custom_target_manifests_keep_package_coverage(self):
        for extra in ('autotests = false\n', '\n[[test]]\nname = "custom"\n'):
            self.write('crates/soliddraft-app/Cargo.toml', '[package]\nname = "soliddraft-app"\n' + extra)
            self.commit()
            self.assertIsNone(self.plan([['M', self.test_path]])['test_targets'])

    def test_scenario_only_diff_selects_each_changed_file_and_its_jobs(self):
        changes = [['M', 'ci/scenarios/bracket.json'], ['A', 'ci/scenarios/fillet.json'], ['M', 'docs/ci.md']]
        plan = self.plan(changes)
        self.assertEqual(plan['scenarios'], ['ci/scenarios/bracket.json', 'ci/scenarios/fillet.json'])
        sources = lambda path: [selector.content(ROOT, path)] * 2
        self.assertEqual(set(selector.select(changes, 'pull_request', '', {}, sources)),
                         {'lint', 'application-loop', 'test-linux'})

    def mixed_fixture_changes(self):
        scenarios = ('p5-flanged-housing', 'sphere-revolve')
        rust_paths = ('crates/soliddraft-cli/src/main.rs',
                      'crates/soliddraft-cli/tests/independent_cli.rs')
        for name in scenarios:
            self.write(f'ci/scenarios/{name}.json', '{}')
        for path in rust_paths:
            self.write(path, '// independent CLI edit\n')
        self.write('crates/soliddraft-app/tests/scenario_reader.rs',
                   '\n'.join(f'const _: &str = include_str!("../../../ci/scenarios/{name}.json");'
                             for name in scenarios))
        self.base = self.commit()
        return ((['M', f'ci/scenarios/{name}.json'], [status, path])
                for name in scenarios for path in rust_paths for status in ('M', 'A', 'D'))

    def test_adding_rust_to_scenario_diff_cannot_drop_fixture_test_coverage(self):
        for scenario, rust in self.mixed_fixture_changes():
            with self.subTest(scenario=scenario, rust=rust):
                scenario_only = self.plan([scenario])
                self.assertEqual(runner.cargo_args(scenario_only),
                                 ['--workspace', '--all-features', '--locked'])
                mixed = self.plan([scenario, rust])
                self.assertEqual(mixed, selector.FULL_WORK)
                self.assertEqual(runner.cargo_args(mixed), runner.cargo_args(scenario_only))

    def test_accepted_work_plans_preserve_mixed_scenario_fixture_coverage(self):
        # Both policies support sub-job plans; the older-policy full fallback cannot mask this test.
        accepted = (ROOT / 'ci/select-checks.py').read_text()
        self.assertIn('def work_plan(', accepted)
        for scenario, rust in self.mixed_fixture_changes():
            with self.subTest(scenario=scenario, rust=rust):
                mixed = selector.accepted_work(self.root, self.base, 'HEAD', [scenario, rust],
                                               'pull_request', self.packages, accepted)
                self.assertEqual(mixed, selector.FULL_WORK)
                self.assertEqual(runner.cargo_args(mixed), ['--workspace', '--all-features', '--locked'])

    def test_source_helpers_policy_and_unknown_changes_cannot_drop_scenarios(self):
        scenario = ['M', 'ci/scenarios/bracket.json']
        for path in ('crates/soliddraft-app/src/lib.rs', 'ci/run-scenarios.py', 'ci/run-selected.py',
                     'ci/known-over-budget.json', 'ci/scenario-inventory.json', 'recipes/bracket.json',
                     '.github/workflows/hosted.yml', 'unknown'):
            with self.subTest(path=path):
                self.assertIsNone(self.plan([scenario, ['M', path]])['scenarios'])
        self.assertEqual(self.plan([['D', 'ci/scenarios/deleted.json']]), selector.FULL_WORK)

    def test_qualification_events_and_unsafe_paths_are_full(self):
        changes = [['M', self.test_path]]
        for event in ('schedule', 'push', 'workflow_dispatch', 'merge_group'):
            self.assertEqual(self.plan(changes, event), selector.FULL_WORK)
        for changes in (None, [], [['M', 'Cargo.lock']], [['M', 'crates/soliddraft-app/Cargo.toml']],
                        [['M', 'crates/soliddraft-app/build.rs']], [['M', 'rust-toolchain.toml']]):
            self.assertEqual(self.plan(changes), selector.FULL_WORK)

    def test_base_policy_without_plans_and_self_edits_keep_full_work(self):
        old = 'def select(*args): return {}\n'
        changes = [['M', self.test_path]]
        self.assertEqual(selector.accepted_work(self.root, self.base, 'HEAD', changes,
                                                'pull_request', self.packages, old), selector.FULL_WORK)
        current = (ROOT / 'ci/select-checks.py').read_text()
        self.assertEqual(selector.accepted_work(self.root, self.base, 'HEAD', changes,
                                                'pull_request', self.packages, current), self.plan(changes))
        self.assertEqual(self.plan([['M', 'ci/select-checks.py']]), selector.FULL_WORK)

    def test_base_and_candidate_work_are_united(self):
        candidate = {'packages': ['soliddraft-app'], 'test_targets': ['one'],
                     'scenarios': ['ci/scenarios/bracket.json']}
        accepted = {'packages': ['soliddraft-app'], 'test_targets': ['two'],
                    'scenarios': ['ci/scenarios/fillet.json']}
        result = selector.union_work(candidate, accepted)
        self.assertEqual(result['test_targets'], ['one', 'two'])
        self.assertEqual(result['scenarios'], ['ci/scenarios/bracket.json', 'ci/scenarios/fillet.json'])
        self.assertEqual(selector.union_work(candidate, selector.FULL_WORK), selector.FULL_WORK)
        accepted['packages'] = ['soliddraft-cli']
        self.assertIsNone(selector.union_work(candidate, accepted)['test_targets'])


class Runner(unittest.TestCase):
    def test_archive_and_doctest_arguments_preserve_full_features_and_lock(self):
        plan = {'packages': ['soliddraft-app'], 'test_targets': ['isolated'], 'scenarios': None}
        self.assertEqual(runner.cargo_args(plan), ['-p', 'soliddraft-app', '--test', 'isolated',
                                                 '--all-features', '--locked'])
        self.assertEqual(runner.cargo_args(plan, docs=True), ['-p', 'soliddraft-app', '--all-features', '--locked'])
        self.assertEqual(runner.cargo_args(selector.FULL_WORK), ['--workspace', '--all-features', '--locked'])

    def test_missing_empty_and_malformed_selections_fail_before_running_anything(self):
        with tempfile.TemporaryDirectory() as temporary:
            selection = pathlib.Path(temporary) / 'selection.json'
            with self.assertRaises(FileNotFoundError):
                runner.read_plan(selection)
            bad = [{}, {'work': {}}, {'work': None}]
            for key in runner.KEYS:
                for value in ([], 'all', [1], ['--workspace'], ['../../outside']):
                    bad.append({'work': dict(selector.FULL_WORK, **{key: value})})
            bad.append({'work': dict(selector.FULL_WORK, test_targets=['one'])})
            for data in bad:
                with self.subTest(data=data), mock.patch.object(runner.subprocess, 'run') as invoke:
                    selection.write_text(json.dumps(data))
                    with self.assertRaises((ValueError, KeyError)):
                        runner.run(argparse.Namespace(selection=selection, command='test-doc'))
                    invoke.assert_not_called()

    def test_scenario_failure_does_not_skip_remaining_selected_scenarios(self):
        paths = ['ci/scenarios/bracket.json', 'ci/scenarios/fillet.json']
        with tempfile.TemporaryDirectory() as temporary:
            selection = pathlib.Path(temporary) / 'selection.json'
            selection.write_text(json.dumps({'work': dict(selector.FULL_WORK, scenarios=paths)}))
            args = argparse.Namespace(command='scenarios', selection=selection, out=pathlib.Path(temporary))
            with mock.patch.object(runner.subprocess, 'run', side_effect=[
                    subprocess.CompletedProcess([], 7), subprocess.CompletedProcess([], 0)]) as invoke, \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(runner.run(args), 1)
                self.assertEqual([call.args[0][2] for call in invoke.call_args_list], paths)
        with self.assertRaises(ValueError):
            runner.scenario_paths(dict(selector.FULL_WORK, scenarios=['ci/scenarios/missing.json']), ROOT)

    def test_full_scenario_plan_runs_the_complete_inventory(self):
        expected = sorted(e['file'] for e in json.loads((ROOT / 'ci/scenario-inventory.json').read_text())['scenarios'])
        self.assertEqual(runner.scenario_paths(selector.FULL_WORK, ROOT), expected)

    def test_build_and_doc_failures_propagate(self):
        for command in ('test-build', 'test-doc'):
            with mock.patch.object(runner.subprocess, 'run', return_value=subprocess.CompletedProcess([], 7)) as invoke, \
                    contextlib.redirect_stdout(io.StringIO()):
                args = argparse.Namespace(command=command, selection=None, archive='archive.tar.zst')
                self.assertEqual(runner.run(args), 7)
                if command == 'test-build':
                    self.assertIn('--workspace', invoke.call_args.args[0])
                else:
                    self.assertEqual(invoke.call_args.args[0][0:2], ['cargo', 'metadata'])

    def test_archive_cannot_succeed_with_no_tests(self):
        args = argparse.Namespace(command='test-build', selection=None, archive='archive.tar.zst')
        for count in (0, -1, '1', None, True):
            with self.subTest(count=count), mock.patch.object(runner.subprocess, 'run',
                    return_value=subprocess.CompletedProcess([], 0, json.dumps({'test-count': count}))) as invoke, \
                    contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(ValueError):
                    runner.run(args)
                self.assertEqual(invoke.call_count, 1)
        with mock.patch.object(runner.subprocess, 'run', side_effect=[
                subprocess.CompletedProcess([], 0, '{"test-count": 1}'),
                subprocess.CompletedProcess([], 0)]) as invoke, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(runner.run(args), 0)
            self.assertEqual(invoke.call_args.args[0][1:3], ['nextest', 'archive'])


class DoctestFixtures(unittest.TestCase):
    """Real runner/Cargo checks on tiny dependency-free fixtures, one library at a time."""

    def fixture(self, members, selected, library_doc='assert_eq!(2 + 2, 4);', library_doctest=True):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = pathlib.Path(temporary.name)
        (root / 'ci').mkdir()
        shutil.copy2(ROOT / 'ci/run-selected.py', root / 'ci/run-selected.py')
        (root / 'Cargo.toml').write_text('[workspace]\nresolver = "2"\nmembers = ' + json.dumps(members) + '\n')
        for kind in members:
            package = root / kind
            (package / 'src').mkdir(parents=True)
            (package / 'Cargo.toml').write_text(
                f'[package]\nname = "soliddraft-doc-{kind}"\nversion = "0.1.0"\nedition = "2021"\n'
                + ('[lib]\ndoctest = false\n' if kind == 'lib' and not library_doctest else ''))
            source = ('/// ```\n/// ' + library_doc + '\n/// ```\npub fn answer() {}\n'
                      if kind == 'lib' else 'fn main() {}\n')
            (package / 'src' / ('lib.rs' if kind == 'lib' else 'main.rs')).write_text(source)
        env = dict(os.environ, CARGO_BUILD_JOBS='1', CARGO_NET_OFFLINE='true',
                   CARGO_TARGET_DIR=str(root / 'target'))
        subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=root, env=env,
                       capture_output=True, text=True, check=True, timeout=30)
        plan = dict(selector.FULL_WORK, packages=None if selected is None else [
            f'soliddraft-doc-{kind}' for kind in selected])
        selection = root / 'selection.json'
        selection.write_text(json.dumps({'work': plan}))
        return subprocess.run([sys.executable, str(root / 'ci/run-selected.py'), 'test-doc',
                               '--selection', str(selection)], cwd=root, env=env,
                              capture_output=True, text=True, timeout=60)

    def test_binary_only_workspace_has_no_doctest_roots(self):
        for selected in (['bin'], None):
            with self.subTest(selected=selected):
                result = self.fixture(['bin'], selected)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('No selected library doctest targets', result.stdout)

    def test_unselected_library_does_not_make_a_selected_binary_doc_eligible(self):
        result = self.fixture(['lib', 'bin'], ['bin'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('No selected library doctest targets', result.stdout)
        self.assertNotIn('Doc-tests', result.stdout + result.stderr)

    def test_library_only_and_mixed_selections_run_library_doctests(self):
        for members, selected in ((['lib'], ['lib']), (['lib', 'bin'], ['lib', 'bin']),
                                  (['lib', 'bin'], None)):
            with self.subTest(members=members, selected=selected):
                result = self.fixture(members, selected)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('Doc-tests', result.stdout + result.stderr)
                self.assertIn('1 passed', result.stdout + result.stderr)

    def test_library_doctest_failure_is_not_swallowed(self):
        result = self.fixture(['lib', 'bin'], ['lib', 'bin'], 'assert_eq!(2 + 2, 5);')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('doctest failed', result.stdout + result.stderr)

    def test_unknown_selected_package_is_not_treated_as_no_doctests(self):
        result = self.fixture(['lib', 'bin'], ['missing'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unknown selected workspace packages', result.stdout + result.stderr)
        self.assertNotIn('No selected library doctest targets', result.stdout)

    def test_library_doctest_settings_are_left_to_cargo(self):
        result = self.fixture(['lib'], ['lib'], library_doctest=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('Selected library doctest packages', result.stdout)
        self.assertIn('"cargo", "test"', result.stdout)
        self.assertNotIn('No selected library doctest targets', result.stdout)


if __name__ == '__main__':
    unittest.main()
