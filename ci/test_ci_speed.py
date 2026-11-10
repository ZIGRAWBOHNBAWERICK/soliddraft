"""Runner trust, duration balancing and complete partition coverage."""
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest import mock

from balance import filterset, inventory, matcher, partition, run_shard

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('select_runner', ROOT / 'ci/select-runner.py')
selection = importlib.util.module_from_spec(spec)
spec.loader.exec_module(selection)


class CompilerCache(unittest.TestCase):
    def test_server_start_failure_leaves_the_compiler_unwrapped(self):
        action = (ROOT / '.github/actions/compile-cache/action.yml').read_text()
        script = textwrap.dedent(action.split('      run: |\n', 1)[1])
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            binary = root / 'sccache'
            binary.write_text('#!/bin/sh\n[ "$1" = --start-server ] || exit 9\n'
                              'exit "$CACHE_START_EXIT"\n')
            binary.chmod(0o755)
            output = root / 'environment'
            for code in (0, 2):
                with self.subTest(startup_exit=code):
                    output.write_text('')
                    env = dict(os.environ, PATH=f'{root}:{os.environ["PATH"]}',
                               GITHUB_ENV=str(output), RUNNER_TEMP=str(root), GITHUB_JOB='probe',
                               CACHE_VERSION='fixture', CACHE_START_EXIT=str(code))
                    result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], env=env,
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    values = dict(line.split('=', 1) for line in output.read_text().splitlines())
                    if code:
                        self.assertEqual(values, {})
                        self.assertIn('compiling without it', result.stdout)
                    else:
                        self.assertEqual(values['RUSTC_WRAPPER'], 'sccache')
                        self.assertEqual(values['SCCACHE_SERVER_UDS'], str(root / 'sccache-probe.sock'))
                        self.assertEqual(values['SCCACHE_IGNORE_SERVER_IO_ERROR'], '1')

    def test_optional_statistics_cannot_fail_required_checks(self):
        workflow = (ROOT / '.github/workflows/hosted.yml').read_text()
        blocks = workflow.split('      - name: Compiler cache statistics\n')[1:]
        self.assertTrue(blocks)
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            binary = root / 'sccache'
            binary.write_text('#!/bin/sh\nexit 2\n')
            binary.chmod(0o755)
            for block in blocks:
                script = textwrap.dedent(block.split('        run: |\n', 1)[1].split('\n\n', 1)[0])
                result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script],
                                        env=dict(os.environ, PATH=f'{root}:{os.environ["PATH"]}',
                                                 SCCACHE_SERVER_UDS=str(root / 'probe.sock')),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('::warning::Compiler cache statistics unavailable.', result.stdout)
                self.assertIn('::warning::Compiler cache server unavailable.', result.stdout)

    def test_cache_protocol_is_exported_with_its_credentials(self):
        action = (ROOT / '.github/actions/compile-cache/action.yml').read_text()
        self.assertNotIn('SCCACHE_BASEDIRS', action)
        script = action.split('        script: |\n', 1)[1].split('    - shell:', 1)[0]
        exports = dict(re.findall(r"core.exportVariable\('([^']+)', process.env.([A-Z_0-9]+) ", script))
        required = ('ACTIONS_RESULTS_URL', 'ACTIONS_RUNTIME_TOKEN', 'ACTIONS_CACHE_SERVICE_V2')
        self.assertEqual(exports, {name: name for name in required})


class RunnerHealth(unittest.TestCase):
    def select(self, **overrides):
        values = dict(event={}, actor='BohnBawerick', starter='BohnBawerick',
                      repository='ZIGRAWBOHNBAWERICK/soliddraft', boat_enabled='true',
                      boat_at='1000', bohn_at='1000', now=1001)
        values.update(overrides)
        return selection.runners(**values)

    def test_priority_and_nixos_capability(self):
        self.assertEqual(self.select()['plain'], ['boat-large'])
        fallback = self.select(boat_at='0')
        self.assertEqual(fallback['plain'], ['self-hosted', 'bohn-03'])
        self.assertEqual(fallback['plain_jobs'], '1')
        self.assertEqual(fallback['archive'], ['ubuntu-latest'])
        self.assertEqual(fallback['application'], ['ubuntu-latest'])
        self.assertEqual(self.select(boat_at='0', bohn_at='0')['plain'], ['ubuntu-latest'])
        self.assertEqual(self.select(boat_enabled='false')['plain'], ['self-hosted', 'bohn-03'])

    def test_health_expires_and_future_or_malformed_values_fail_closed(self):
        for stamp in (None, '', 'no', '1001', '0', '100.0'):
            self.assertFalse(selection.fresh(stamp, 1000), stamp)
        self.assertTrue(selection.fresh('100', 999))
        self.assertFalse(selection.fresh('100', 1000))

    def test_forks_and_outside_contributors_always_use_github(self):
        repo = 'ZIGRAWBOHNBAWERICK/soliddraft'
        trusted_pr = {'pull_request': {'user': {'login': 'BohnBawerick'},
                                      'head': {'repo': {'full_name': repo}}}}
        self.assertEqual(self.select(event=trusted_pr)['plain'], ['boat-large'])
        untrusted = [dict(actor='dependabot[bot]'), dict(starter='outsider'),
                     dict(repository='fork/soliddraft'),
                     dict(event={'pull_request': {'user': {'login': 'outsider'},
                                                 'head': {'repo': {'full_name': repo}}}}),
                     dict(event={'pull_request': {'user': {'login': 'BohnBawerick'},
                                                 'head': {'repo': {'full_name': 'fork/soliddraft'}}}}),
                     dict(event={'pull_request': {'head': {'repo': None}}})]
        for values in untrusted:
            with self.subTest(values=values):
                runners = self.select(**values)
                for key in ('plain', 'archive', 'application'):
                    self.assertEqual(runners[key], ['ubuntu-latest'])


class Balancing(unittest.TestCase):
    def test_balances_long_tests_and_includes_unknown_tests_once(self):
        items = ['slow', 'medium', 'fast', 'new', 'renamed']
        shards, loads = partition(items, {'slow': 10, 'medium': 6, 'fast': 4}, 2)
        self.assertEqual(sorted(sum(shards, [])), sorted(items))
        self.assertLessEqual(max(loads) - min(loads), 2)
        self.assertEqual(partition(list(reversed(items)), {'slow': 10, 'medium': 6, 'fast': 4}, 2),
                         (shards, loads))
        self.assertEqual(partition(['a'], {}, 4)[0], [['a'], [], [], []])
        with self.assertRaises(ValueError):
            partition(['a', 'a'], {}, 2)

    def test_platform_reads_durations_from_the_requested_workspace(self):
        from balance import read_timings
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            (root / 'ci').mkdir()
            (root / 'ci/test-durations.json').write_text('{"seconds": {"linux": 1}}')
            (root / 'ci/test-durations-macos.json').write_text('{"seconds": {"macos": 2}}')
            for platform, expected in (('darwin', {'macos': 2}), ('linux', {'linux': 1})):
                with mock.patch('balance.sys.platform', platform):
                    self.assertEqual(read_timings(root), expected)

    def test_duration_record_uses_median_and_excludes_ignored_cases(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            paths = []
            for index, seconds in enumerate((100, 1, 2)):
                path = root / f'{index}.xml'
                path.write_text(f'<testsuites><testsuite><testcase classname="binary" name="live" time="{seconds}"/>'
                                '<testcase classname="binary" name="ignored" time="99"><skipped/></testcase>'
                                '</testsuite></testsuites>')
                paths.append(str(path))
            output = root / 'durations.json'
            subprocess.run([sys.executable, str(ROOT / 'ci/balance.py'), 'record', '--source',
                            'https://github.com/example/run', '--output', str(output), *paths], check=True)
            self.assertEqual(json.loads(output.read_text())['seconds'], {'binary\tlive': 2.0})

    def test_live_inventory_preserves_filters_and_ignored_tests(self):
        listing = {'rust-suites': {'crate::test': {'testcases': {
            'run': {'ignored': False, 'filter-match': {'status': 'matches'}},
            'ignored': {'ignored': True, 'filter-match': {'status': 'matches'}},
            'filtered': {'ignored': False, 'filter-match': {'status': 'mismatch'}}}}}}
        self.assertEqual(inventory(listing), ['crate::test\trun'])

    def test_filtersets_escape_exact_names_and_group_by_binary(self):
        self.assertEqual(matcher('a,b)\\\n'), '=a\\,b\\)\\\\\\n')
        self.assertEqual(filterset([]), 'none()')
        text = filterset(['a\tx', 'a\ty', 'b\tx'])
        self.assertEqual(text.count('binary_id(=a)'), 1)
        self.assertIn('test(=x) | test(=y)', text)

    def test_all_ignored_archive_keeps_no_tests_pass_behavior(self):
        listing = {'rust-suites': {'crate::test': {'testcases': {
            'ignored': {'ignored': True, 'filter-match': {'status': 'matches'}}}}}}
        with mock.patch('balance.subprocess.run', side_effect=[
                mock.Mock(stdout=json.dumps(listing)), mock.Mock(returncode=0)]) as invoke:
            self.assertEqual(run_shard('ignored.tar.zst', '1/1'), 0)
        command = invoke.call_args.args[0]
        self.assertEqual(command[command.index('--filter-expr') + 1], 'none()')
        self.assertEqual(command[command.index('--no-tests') + 1], 'pass')

    def test_invalid_shard_cannot_invoke_nextest(self):
        with mock.patch('balance.subprocess.run') as invoke:
            for shard in ('0/4', '5/4', '1/0'):
                with self.assertRaises(ValueError):
                    run_shard('archive', shard)
            invoke.assert_not_called()

    def test_pinned_duration_data_has_valid_positive_weights(self):
        for name in ('test-durations.json', 'test-durations-macos.json', 'scenario-durations.json'):
            data = json.loads((ROOT / 'ci' / name).read_text())
            self.assertTrue(data['seconds'])
            self.assertIn('https://github.com/', data['source'])
            self.assertTrue(all(isinstance(v, (int, float)) and v >= 0 for v in data['seconds'].values()))
            items = list(data['seconds'])
            shards, loads = partition(items, data['seconds'], 4)
            self.assertEqual(sorted(sum(shards, [])), sorted(items))
            self.assertLess(max(loads) - min(loads), max(data['seconds'].values()) + 1)

class BinaryArchives(unittest.TestCase):
    def test_binaries_stay_together_with_new_cases_and_empty_workers(self):
        from balance import binary_partition
        items = ['a\tslow', 'a\tnew', 'b\tother', 'c\trenamed']
        shards, loads = binary_partition(items, {'a\tslow': 10, 'b\tother': 6}, 4)
        self.assertEqual(sorted(sum(shards, [])), sorted(items))
        self.assertEqual(next(s for s in shards if 'a\tslow' in s), ['a\tnew', 'a\tslow'])
        self.assertIn([], shards)
        self.assertEqual(binary_partition(items[::-1], {'a\tslow': 10, 'b\tother': 6}, 4), (shards, loads))
        for weight in (-1, float('inf'), float('nan')):
            with self.assertRaises(ValueError):
                binary_partition(items, {'a\tslow': weight}, 4)

    def test_filtered_archive_must_contain_the_globally_selected_cases(self):
        def listing(names):
            return {'rust-suites': {'a': {'testcases': {
                name: {'ignored': False, 'filter-match': {'status': 'matches'}}
                for name in names}}}}
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'listing.json'
            path.write_text(json.dumps(listing(['present', 'missing'])))
            with mock.patch('balance.subprocess.run', return_value=mock.Mock(stdout=json.dumps(listing(['present'])))) as invoke:
                with self.assertRaisesRegex(ValueError, 'missing selected tests'):
                    run_shard('archive', '1/1', full_inventory=path)
                self.assertEqual(invoke.call_count, 1)


if __name__ == '__main__':
    unittest.main()
