"""Independent R2 fixtures, scenario inventory and fail-closed workflow wiring."""
import hashlib
import importlib.util
import json
import pathlib
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
POLICY = 'ci/check-consumers.json'
FROZEN_BASE = ROOT / 'ci/routing-replays/select-checks.py.9e81eb4'
# The selector accepted on main before shared viewport files stopped selecting the desktop legs.
ROUND3_BASE = ROOT / 'ci/routing-replays/select-checks.py.23e47cd'
# Round 3 expectations, written independently of the selector: app edits reach app, CLI and web.
APP = {'application-loop', 'interactive-linux', 'lint', 'step-opencascade', 'test-linux', 'web'}
DESKTOP = {'interactive-macos', 'test-macos'}
WINDOWS = {'test-windows', 'package-windows'}
VIEWPORT = 'crates/soliddraft-app/src/viewport/'
CAMERA = VIEWPORT + 'camera.rs'
F1 = ('wgpu::Backend::Gl | wgpu::Backend::Noop => Tier::Basic,',
      'wgpu::Backend::Dx12 | wgpu::Backend::Metal | wgpu::Backend::Gl | wgpu::Backend::Noop => Tier::Basic,')
F2 = {'cfg': '\n#[cfg(target_os = "windows")]\ncompile_error!("narrow3 windows fault");\n',
      'cfg-attr': '\n#[cfg_attr(target_os = "macos", broken_attribute)]\nstruct Fault;\n',
      'backend': '\nfn injected() { let _ = egui_wgpu::wgpu::Backend::Metal; }\n'}


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'ci' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


selector = load('select-checks')
gate = load('check-gate')


def read(name):
    return json.loads((ROOT / 'ci' / name).read_text())


def everything():
    fixture = read('routing-cases.json')
    return set(fixture['units'] + fixture['package_units'])


def qualified():
    # Every unit and package that any event selects while Windows is deferred.
    return everything() - WINDOWS



def normal_selection(changes, event, ref, packages, sources, windows=False):
    """The frozen fixtures cover normal routing when a diff also contains non-CI files."""
    result = selector.route(changes, event, ref, packages, sources)
    return result if windows else {unit: why for unit, why in result.items() if unit not in WINDOWS}


def jobs(text):
    starts = list(re.finditer(r'^  ([\w-]+):\n', text[text.index('jobs:\n') + 6:], re.M))
    body = text[text.index('jobs:\n') + 6:]
    return {m[1]: body[m.end():starts[i + 1].start() if i + 1 < len(starts) else len(body)]
            for i, m in enumerate(starts)}


class Inventory(unittest.TestCase):
    def test_known_gaps_render_release_notes(self):
        spec = importlib.util.spec_from_file_location('release', ROOT / 'packaging/release.py')
        release = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(release)
        gaps = read('known-over-budget.json')
        lines = release.known_gap_notes()
        self.assertEqual(lines[0], '## Known gaps')
        self.assertIn(gaps['follow_up'], lines[2])
        self.assertEqual(sum(line.startswith('- ') for line in lines),
                         len({step['scenario'] for step in gaps['steps']}))

    def test_inventory_matches_files_and_has_owners(self):
        entries = read('scenario-inventory.json')['scenarios']
        self.assertEqual(len(entries), len({e['file'] for e in entries}))
        self.assertEqual({e['file'] for e in entries},
                         {str(p.relative_to(ROOT)) for p in (ROOT / 'ci/scenarios').glob('*.json')})
        self.assertTrue(all(e['owners'] for e in entries))

    def test_acceptance_has_numeric_output_and_error_path(self):
        entries = read('scenario-inventory.json')['scenarios']
        self.assertTrue(next(e for e in entries if e['id'] == 'bracket')['acceptance'])
        for entry in entries:
            if entry['acceptance']:
                steps = json.loads((ROOT / entry['file']).read_text())['steps']
                self.assertTrue(any('approx' in c or 'equals' in c for s in steps for c in s.get('checks', [])))
                self.assertTrue(any('refused' in s or 'refusal' in s for s in steps))


class Routing(unittest.TestCase):
    """The frozen fixtures still state every rule, so they read the selection a Windows catch-up gets."""

    @classmethod
    def setUpClass(cls):
        cls.packages = selector.graph(ROOT)
        cls.sources = staticmethod(lambda p: [selector.content(ROOT, p, 'HEAD'), selector.content(ROOT, p)])

    def test_frozen_fixtures(self):
        for name, sha in [('routing-cases.json', '5360929b3dd5d42d103faa99866ffb686d466abe111d4167b5c55bb442aff40d'),
                          ('gate-cases.json', 'c66b54349c735d237663849692c9df802caabf8b9f33d6e03940134d89d23c05')]:
            self.assertEqual(hashlib.sha256((ROOT / 'ci' / name).read_bytes()).hexdigest(), sha)
        for name, sha in read('routing-cases.json')['snapshots'].items():
            self.assertEqual(hashlib.sha256((ROOT / 'ci/routing-replays' / name).read_bytes()).hexdigest(), sha)

    def resolve_source(self, spec):
        if spec is None:
            return ''
        if 'text' in spec:
            return spec['text']
        text = (ROOT / 'ci/routing-replays' / spec['snapshot']).read_text()
        for old, new, count in spec['edits']:
            self.assertEqual(text.count(old), count)
            text = text.replace(old, new)
        return text

    def case_sources(self, case):
        return lambda path: ([self.resolve_source(s) for s in case['sources'][path]]
                             if path in case.get('sources', {}) else self.sources(path))

    def test_routing_fixture(self):
        fixture = read('routing-cases.json')
        for case in fixture['cases']:
            with self.subTest(case['id']):
                selected = normal_selection(case['changes'], case['event'], case.get('ref', ''), self.packages,
                                           self.case_sources(case), windows=True)
                must = case['must']
                if isinstance(must, str):
                    must = {'units': fixture['units'], 'all': fixture['units'] + fixture['package_units']}[must]
                self.assertLessEqual(set(must), set(selected))
                self.assertFalse(set(case['must_not']) & set(selected))

    def test_round2_cases_name_every_unit(self):
        # Must and must_not partition all 15 units, so the routing fixture compares exact sets.
        fixture = read('routing-cases.json')
        rows = [c for c in fixture['cases'] if c['id'].startswith('round2-')]
        rows += read('routing-replays/round2-replays.json')['cases']
        changed = {'HN03', 'HN04', 'HN05', 'HN06', 'HN07', 'HN08', 'H08', 'H09', 'N03'}
        rows += [c for c in fixture['cases'] if c['id'].split('-')[0] in changed]
        self.assertEqual(len(rows), 58)
        for case in rows:
            with self.subTest(case['id']):
                self.assertEqual(set(case['must']) | set(case['must_not']), everything())
                self.assertFalse(set(case['must']) & set(case['must_not']))
        replay = next(c for c in fixture['cases'] if c['id'] == 'R72-replay-pr72')
        self.assertEqual(replay['must'], 'units')
        self.assertEqual(set(replay['must_not']), set(fixture['package_units']))

    def test_consumer_table(self):
        for path, units in read('routing-cases.json')['consumers'].items():
            with self.subTest(path):
                selected = normal_selection([['M', path]], 'pull_request', '', self.packages, self.sources, windows=True)
                self.assertEqual(set(selected), everything() if units == 'all' else {'lint', *units})

    def test_map_covers_every_ci_file(self):
        self.assertEqual(list(read('check-consumers.json')['patterns']), [r'ci/scenarios/[^/]+\.json'])
        consumers = selector.parse_consumers((ROOT / POLICY).read_text())
        files = selector.git(ROOT, 'ls-files', '--cached', '--others', '--exclude-standard', '-z', 'ci')
        paths = [p for p in files.decode().split('\0') if p]
        self.assertIn('ci/ownership.toml', paths)
        self.assertEqual([p for p in paths if selector.mapped_units(p, consumers) is None], [])

    def test_refusal_fixture(self):
        for case in read('routing-cases.json')['refusals']:
            with self.subTest(case['id']):
                text = case.get('policy') or ''
                sources = self.sources
                if 'policy' in case:
                    sources = lambda path, text=text: ['', text] if path == POLICY else self.sources(path)
                with self.assertRaisesRegex(selector.PolicyError, re.escape(case['diagnostic'])):
                    normal_selection(case['changes'], case['event'], case.get('ref', ''), self.packages, sources)

    def test_round2_replays(self):
        script = FROZEN_BASE.read_text()
        for case in read('routing-replays/round2-replays.json')['cases']:
            with self.subTest(case['id']):
                packages = ({name: set(deps) for name, deps in case['packages'].items()}
                            if 'packages' in case else self.packages)
                sources = self.case_sources(case)
                selected = normal_selection(case['changes'], case['event'], '', packages, sources, windows=True)
                self.assertEqual(set(selected), set(case['must']))
                union = selector.with_base_rules(selected, script, case['changes'], case['event'], '',
                                                 packages, sources)
                self.assertEqual(set(union), everything())

    def test_real_base_selector_keeps_its_obligations(self):
        script = FROZEN_BASE.read_text()
        ownership = [['M', 'ci/ownership.toml']]
        candidate = selector.select(ownership, 'pull_request', '', self.packages, self.sources)
        self.assertEqual(set(candidate), {'lint'})
        union = selector.with_base_rules(candidate, script, ownership, 'pull_request', '', self.packages, self.sources)
        self.assertEqual(set(union), everything())
        self.assertIn('base: check policy: ci/ownership.toml', union['package-linux'])
        case = next(c for c in read('routing-cases.json')['cases'] if c['id'] == 'round2-L02-catallax-table-added')
        sources = self.case_sources(case)
        candidate = selector.select(case['changes'], case['event'], '', self.packages, sources)
        self.assertEqual(set(candidate), selector.LINUX)
        union = selector.with_base_rules(candidate, script, case['changes'], case['event'], '', self.packages, sources)
        self.assertEqual(set(union), selector.UNITS)

    def round2_map(self, *entries):
        return json.dumps({'format': 'soliddraft-check-consumers/1', 'files': dict(entries), 'patterns': {}})

    def test_accepted_map_keeps_its_own_consumer(self):
        base, head = (self.round2_map(('ci/linux-helper.py', [unit])) for unit in ('test-macos', 'application-loop'))
        sources = lambda path: [base, head] if path == POLICY else ['', '']
        changes = [['M', 'ci/linux-helper.py'], ['M', 'README.md']]
        candidate = selector.select(changes, 'pull_request', '', self.packages, sources)
        self.assertEqual(set(candidate), {'lint', 'application-loop'})
        union = selector.with_base_rules(candidate, (ROOT / 'ci/select-checks.py').read_text(), changes,
                                         'pull_request', '', self.packages, sources)
        self.assertEqual(set(union), {'lint', 'application-loop', 'test-macos'})
        self.assertEqual(union['test-macos'], ['base: CI consumers: ci/linux-helper.py'])

    def test_accepted_map_that_cannot_judge_selects_everything(self):
        script = (ROOT / 'ci/select-checks.py').read_text()
        head = self.round2_map(('ci/new.py', ['lint']))
        for base, words in ((self.round2_map(), 'ci/new.py'), ('{', 'JSON')):
            with self.subTest(base):
                sources = lambda path: [base, head] if path == POLICY else ['', '']
                changes = [['A', 'ci/new.py'], ['M', 'README.md']]
                candidate = selector.select(changes, 'pull_request', '', self.packages, sources)
                self.assertEqual(set(candidate), {'lint'})
                union = selector.with_base_rules(candidate, script, changes, 'pull_request', '', self.packages, sources)
                self.assertEqual(set(union), qualified())
                self.assertIn(words, union['package-linux'][0])
                self.assertTrue(union['package-linux'][0].startswith('base: accepted map cannot judge'))

    def test_base_rules_still_apply(self):
        case = next(c for c in read('routing-cases.json')['cases'] if c['id'] == 'K01-catallax-lock-rev-only')
        sources = self.case_sources(case)
        candidate = selector.select(case['changes'], case['event'], '', self.packages, sources)
        script = ("def select(changes, event, ref, packages, read_sources):\n"
                  "    return {'test-windows': ['sentinel']}\n")
        result = selector.with_base_rules(candidate, script, case['changes'], case['event'], '', self.packages, sources)
        self.assertEqual(set(result), selector.LINUX | {'test-windows'})
        self.assertEqual(result['test-windows'], ['base: sentinel'])
        candidate = selector.select(case['changes'], case['event'], '', self.packages, sources)
        self.assertEqual(selector.with_base_rules(candidate.copy(), '', case['changes'], case['event'], '',
                                                  self.packages, sources), candidate)

    def test_multiline_runner_edits_select_the_changed_job(self):
        for runner in ('\n      - ubuntu-latest', ' |\n      ubuntu-latest',
                       ' [self-hosted,\n      ubuntu-latest]', ' "runner\n      ubuntu-latest"',
                       ' runner\n      ubuntu-latest'):
            base = ('name: Hosted CI\non: pull_request\njobs:\n  test-linux:\n    runs-on:' + runner
                    + '\n    steps:\n      - run: cargo test --workspace\n')
            case = {'changes': [['M', '.github/workflows/hosted.yml']],
                    'sources': {'.github/workflows/hosted.yml': [
                        {'text': base}, {'text': base.replace('ubuntu-latest', 'windows-latest')}]}}
            with self.subTest(runner=runner):
                selected = selector.select(case['changes'], 'pull_request', '', self.packages,
                                           self.case_sources(case), windows=True)
                self.assertEqual(set(selected), {'lint', 'test-linux'})

    def test_every_platform_file_selects_platform_checks(self):
        tracked = selector.git(ROOT, 'ls-files', '-z').decode().split('\0')
        platform = [p for p in tracked if p and selector.platform_path(p, self.sources(p))]
        required = ['crates/soliddraft-app/src/file.rs', 'crates/soliddraft-assistant/src/key.rs',
                    'crates/soliddraft-document/src/journal.rs', 'crates/soliddraft-document/src/save.rs',
                    'crates/soliddraft-cli/tests/compatibility.rs', 'crates/soliddraft-cli/tests/fuzz_arguments.rs',
                    'crates/soliddraft-assistant/Cargo.toml']
        self.assertLessEqual(set(required), set(platform))
        for path in platform:
            with self.subTest(path):
                selected = normal_selection([['M', path]], 'pull_request', '', self.packages, self.sources, windows=True)
                self.assertLessEqual(selector.PLATFORM, set(selected))


class CiOnly(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.packages = selector.graph(ROOT)
        cls.workflow = (ROOT / '.github/workflows/hosted.yml').read_text()

    def select(self, changes, overrides=None, **options):
        sources = lambda path: (overrides or {}).get(path, [selector.content(ROOT, path)] * 2)
        return selector.select(changes, options.get('event', 'pull_request'), options.get('ref', ''),
                               self.packages, sources)

    def test_ci_files_and_both_sides_of_renames(self):
        for changes in ([['M', 'ci/select-checks.py']], [['M', POLICY]],
                        [['A', 'ci/tools']], [['D', 'ci/old-tool']],
                        [['M', 'docs/ci.md'], ['M', 'ci/run']],
                        [['R100', 'ci/old-tool', 'ci/new-tool']]):
            with self.subTest(changes=changes):
                for options in ({}, {'event': 'push', 'ref': 'refs/heads/main'}):
                    self.assertEqual(set(self.select(changes, **options)), {'lint'})
        for changes in ([['M', 'ci/run'], ['M', CAMERA]],
                        [['R100', 'ci/run', CAMERA]]):
            self.assertEqual(set(self.select(changes)), APP | {'test-macos'})
        self.assertEqual(set(self.select([['M', 'ci/run'], ['M', 'Cargo.lock']])),
                         selector.UNITS - WINDOWS)

    def test_tool_installer_keeps_consumers_in_mixed_changes(self):
        self.assertEqual(set(self.select([['M', 'ci/bootstrap']])), {'lint'})
        self.assertEqual(set(self.select([['M', 'ci/bootstrap'], ['M', 'README.md']])),
                         {'lint', 'test-linux', 'test-macos'})

    def test_only_changed_hosted_jobs_run(self):
        old = self.workflow
        replacements = {
            'kernel': ('cargo test -p soliddraft-kernel-catallax', 'cargo test --verbose -p soliddraft-kernel-catallax'),
            'test-macos': ('    runs-on: macos-latest', '    runs-on: macos-15'),
            'gate': ('    if: always()', '    if: ${{ always() }}'),
        }
        for changed, expected in ((['kernel'], {'lint', 'kernel'}),
                                  (['test-macos'], {'lint', 'test-macos'}),
                                  (['kernel', 'test-macos', 'gate'], {'lint', 'kernel', 'test-macos'}),
                                  (['gate'], {'lint'})):
            new = old
            for job in changed:
                start = new.index('\n  ' + job + ':\n')
                end = re.search(r'\n  [\w-]+:\n', new[start + 1:])
                stop = start + 1 + end.start() if end else len(new)
                body = new[start:stop].replace(*replacements[job])
                new = new[:start] + body + new[stop:]
            overrides = {'.github/workflows/hosted.yml': [old, new]}
            self.assertEqual(set(self.select([['M', '.github/workflows/hosted.yml'],
                                              ['M', 'ci/select-checks.py']], overrides)), expected)

    def test_shared_settings_and_unreadable_workflows_keep_coverage(self):
        path = '.github/workflows/hosted.yml'
        for new in (self.workflow.replace('CARGO_INCREMENTAL: 0', 'CARGO_INCREMENTAL: 1'),
                    self.workflow.replace('env:\n', 'env: &shared\n', 1),
                    'jobs:\n  broken workflow\n'):
            self.assertEqual(set(self.select([['M', path]], {path: [self.workflow, new]})), qualified())
        renamed = self.workflow.replace('name: Hosted CI', 'name: CI')
        self.assertEqual(set(self.select([['M', path]], {path: [self.workflow, renamed]})), {'lint'})

    def test_new_build_job_uses_its_registered_unit(self):
        registry = json.loads((ROOT / 'ci/check-registry.json').read_text())
        old_registry = json.dumps(registry)
        registry['jobs']['test-linux'] = ['test-linux-build', 'test-linux']
        new = self.workflow + '\n  test-linux-build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo nextest archive\n'
        overrides = {'.github/workflows/hosted.yml': [self.workflow, new],
                     'ci/check-registry.json': [old_registry, json.dumps(registry)]}
        self.assertEqual(set(self.select([['M', '.github/workflows/hosted.yml'],
                                          ['M', 'ci/check-registry.json']], overrides)), {'lint', 'test-linux'})

    def test_packages_only_run_the_changed_package_definitions(self):
        path = '.github/workflows/packages.yml'
        old = (ROOT / path).read_text()
        for before, after, expected in (
                ('cargo-packager@0.11.8', 'cargo-packager@0.11.9', {'lint', 'package-linux', 'package-macos'}),
                ('trunk@0.21.14', 'trunk@0.21.15', {'lint', 'package-web'})):
            self.assertEqual(set(self.select([['M', path]], {path: [old, old.replace(before, after)]})), expected)

    def test_base_policy_keeps_this_transition_full_once(self):
        changes = [['M', 'ci/select-checks.py']]
        sources = lambda path: [selector.content(ROOT, path)] * 2
        candidate = self.select(changes)
        base = FROZEN_BASE.read_text()
        self.assertEqual(set(selector.with_base_rules(candidate, base, changes, 'pull_request', '',
                                                      self.packages, sources)), everything())
        current = (ROOT / 'ci/select-checks.py').read_text()
        self.assertEqual(set(selector.with_base_rules(self.select(changes), current, changes,
                                                      'pull_request', '', self.packages, sources)), {'lint'})


class SharedViewport(unittest.TestCase):
    """Five reviewed viewport files run Linux only; backend, shaders, setup and planted faults keep the desktop."""

    @classmethod
    def setUpClass(cls):
        cls.packages = selector.graph(ROOT)

    def select(self, changes, sources=None, event='pull_request', ref=''):
        read = lambda path: list((sources or {}).get(path) or [selector.content(ROOT, path)] * 2)
        return selector.select(changes, event, ref, self.packages, read)

    def assertSelects(self, changes, expected, sources=None, **options):
        self.assertEqual(set(self.select(changes, sources, **options)), expected)

    def test_shared_files_select_linux_only(self):
        for name in ('camera', 'dimensions', 'environment', 'mod', 'pick'):
            with self.subTest(name):
                self.assertSelects([['M', f'{VIEWPORT}{name}.rs']], APP)

    def test_backend_files_and_shaders_keep_the_desktop(self):
        for name in ('assembly.rs', 'look.rs', 'offscreen.rs', 'render.rs', 'assembly.wgsl', 'cull.wgsl', 'scene.wgsl'):
            with self.subTest(name):
                self.assertTrue((ROOT / VIEWPORT / name).is_file())
                self.assertSelects([['M', VIEWPORT + name]], APP | DESKTOP)

    def test_new_and_removed_viewport_paths_keep_the_desktop(self):
        cull = selector.content(ROOT, VIEWPORT + 'cull.wgsl')
        for change, sources in ((['A', VIEWPORT + 'new_backend.rs'], ['', 'pub fn draw() {}\n']),
                                (['A', VIEWPORT + 'grid/lines.rs'], ['', 'pub fn lines() {}\n']),
                                (['A', VIEWPORT + 'grid.wgsl'], ['', '@vertex fn main() {}\n']),
                                (['D', VIEWPORT + 'cull.wgsl'], [cull, ''])):
            with self.subTest(change[1]):
                self.assertSelects([change], APP | DESKTOP, {change[1]: sources})

    def test_native_setup_keeps_the_desktop(self):
        for name in ('main.rs', 'app/workspace.rs', 'platform.rs'):
            with self.subTest(name):
                self.assertSelects([['M', 'crates/soliddraft-app/src/' + name]], APP | DESKTOP)
        self.assertSelects([['M', 'crates/soliddraft-cli/src/screenshot.rs']],
                           {'application-loop', 'interactive-linux', 'interactive-macos', 'lint', 'step-opencascade',
                            'test-linux', 'test-macos'})

    def test_f1_backend_tier_fault_keeps_the_desktop(self):
        path = VIEWPORT + 'look.rs'
        text = selector.content(ROOT, path)
        self.assertEqual(text.count(F1[0]), 1)
        self.assertSelects([['M', path]], APP | DESKTOP, {path: [text, text.replace(*F1)]})

    def test_f2_platform_text_in_a_shared_file_keeps_the_desktop(self):
        text = selector.content(ROOT, CAMERA)
        for tag, addition in F2.items():
            for side in (0, 1):
                with self.subTest(tag, side=side):
                    sources = [text, text]
                    sources[side] += addition
                    self.assertSelects([['M', CAMERA]], APP | DESKTOP, {CAMERA: sources})

    def test_cfg_attr_outside_the_viewport_keeps_the_desktop(self):
        path = 'crates/soliddraft-app/src/theme.rs'
        text = selector.content(ROOT, path)
        self.assertSelects([['M', path]], APP)
        self.assertSelects([['M', path]], APP | DESKTOP, {path: [text, text + F2['cfg-attr']]})

    def test_renames_count_both_endpoints(self):
        render = VIEWPORT + 'render.rs'
        for change in (['R100', render, CAMERA], ['R100', CAMERA, render]):
            with self.subTest(change):
                self.assertSelects([change], APP | DESKTOP)

    def test_other_obligations_are_never_subtracted(self):
        for other in (VIEWPORT + 'render.rs', 'crates/soliddraft-document/src/save.rs'):
            with self.subTest(other):
                alone = set(self.select([['M', other]]))
                self.assertLessEqual(DESKTOP, alone)
                self.assertEqual(set(self.select([['M', CAMERA], ['M', other]])), APP | alone)

    def test_events(self):
        self.assertSelects([['M', CAMERA]], APP | {'kernel', 'msrv'}, event='push', ref='refs/heads/main')
        for event, ref in (('workflow_dispatch', ''), ('push', 'refs/heads/release/0.1.0')):
            with self.subTest(event):
                self.assertSelects([['M', CAMERA]], qualified(), event=event, ref=ref)
        self.assertSelects(None, qualified())
        self.assertSelects([['M', 'ci/select-checks.py']], {'lint'})

    def test_accepted_main_selector_keeps_the_desktop(self):
        script = ROUND3_BASE.read_text()
        self.assertEqual(hashlib.sha256(script.encode()).hexdigest(),
                         'dfa51adb04a1a35924561297f57039cc465e562ced0fcb29993e52f102c37519')
        candidate = self.select([['M', CAMERA]])
        self.assertEqual(set(candidate), APP)
        read = lambda path: [selector.content(ROOT, path)] * 2
        union = selector.with_base_rules(candidate, script, [['M', CAMERA]], 'pull_request', '', self.packages, read)
        # Main's selector before the deferral still selected Windows; a change cannot narrow that.
        self.assertEqual(set(union), APP | DESKTOP | {'test-windows'})
        for unit in DESKTOP | {'test-windows'}:
            self.assertEqual(union[unit], [f'base: platform: {CAMERA}'])


class Cli(unittest.TestCase):
    """The real command over a throwaway repository holding a copy of this tree."""

    @classmethod
    def setUpClass(cls):
        files = selector.git(ROOT, 'ls-files', '--cached', '--others', '--exclude-standard', '-z').decode()
        cls.directory = tempfile.TemporaryDirectory(prefix='.scratch-', dir=ROOT)
        cls.addClassCleanup(cls.directory.cleanup)
        cls.root = pathlib.Path(cls.directory.name) / 'repo'
        cls.output = pathlib.Path(cls.directory.name) / 'selection.json'
        for path in filter(None, files.split('\0')):
            if (ROOT / path).is_file():
                (cls.root / path).parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(ROOT / path, cls.root / path)
        cls.git('init', '-q', '-b', 'main')
        cls.base = cls.commit({})

    def tearDown(self):
        self.git('reset', '-q', '--hard', self.base)
        self.git('clean', '-q', '-fd')

    @classmethod
    def git(cls, *args):
        return selector.git(cls.root, '-c', 'user.name=ci', '-c', 'user.email=ci@example.invalid',
                            '-c', 'commit.gpgsign=false', *args).decode().strip()

    @classmethod
    def commit(cls, files):
        for path, text in files.items():
            file = cls.root / path
            if text is None:
                file.unlink()
            else:
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(text)
        cls.git('add', '-A')
        cls.git('commit', '-q', '--allow-empty', '-m', 'change')
        return cls.git('rev-parse', 'HEAD')

    def select(self, base, event='pull_request', ref='', windows=''):
        self.output.unlink(missing_ok=True)
        return subprocess.run(['python3', str(ROOT / 'ci/select-checks.py'), '--root', str(self.root), '--base', base,
                               '--event', event, '--ref', ref, '--windows', windows, '--output', str(self.output)],
                              capture_output=True, text=True)

    def selection(self, base, **options):
        result = self.select(base, **options)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(self.output.read_text())

    def edited(self, path, old, new):
        return {path: (self.root / path).read_text().replace(old, new)}

    def test_refusals_publish_nothing(self):
        for case in read('routing-cases.json')['refusals']:
            with self.subTest(case['id']):
                files = {change[1]: 'new\n' for change in case['changes'] if change[0] == 'A'}
                if 'policy' in case:
                    files[POLICY] = case['policy']
                self.commit(files)
                result = self.select(self.base, case['event'], case.get('ref', ''))
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(case['diagnostic'], result.stderr)
                self.assertFalse(self.output.exists())
                self.tearDown()

    def test_unmapped_file_outside_the_diff_refuses(self):
        base = self.commit({'ci/unmapped.py': 'new\n'})
        for path in ('docs/ci.md', 'ci/select-checks.py'):
            with self.subTest(path):
                self.commit({path: (self.root / path).read_text() + '\n'})
                result = self.select(base)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn('ci/unmapped.py', result.stderr)
                self.assertFalse(self.output.exists())

    def test_selection_after_this_policy_is_accepted(self):
        policy = (self.root / POLICY).read_text()
        lock = (self.root / 'Cargo.lock').read_text()
        pin = re.search(r'catallax\?rev=([0-9a-f]{40})', lock)[1]
        source = f'source = "git+https://github.com/BohnBawerick/catallax?rev={"a" * 40}#{"a" * 40}"\n'
        edge = source + 'dependencies = [\n'
        bumped = lock.replace(pin, 'a' * 40)
        self.assertIn(edge, bumped)
        # A name no real lockfile holds, so a later pin bump cannot turn this into a duplicate table.
        bumped = bumped.replace(edge, edge + ' "catallax-routing-fixture",\n', 1)
        bumped += f'\n[[package]]\nname = "catallax-routing-fixture"\nversion = "0.1.0"\n{source}'
        ownership = {'ci/ownership.toml': (self.root / 'ci/ownership.toml').read_text() + '\n'}
        full = sorted(qualified())
        linux = sorted(selector.LINUX)
        for name, files, options, expected in (
                ('ownership', ownership, {}, ['lint']),
                ('lock', {'Cargo.lock': bumped}, {}, linux),
                ('map', {POLICY: policy.replace('"ci/cargo-cache.sh": []', '"ci/cargo-cache.sh": ["lint"]')}, {}, ['lint']),
                ('selector', {'ci/select-checks.py': (self.root / 'ci/select-checks.py').read_text() + '\n'}, {}, ['lint']),
                ('main', ownership, {'event': 'push', 'ref': 'refs/heads/main'}, ['lint']),
                ('release', ownership, {'event': 'push', 'ref': 'refs/heads/release/0.1.0'}, full)):
            with self.subTest(name):
                self.commit(files)
                self.assertEqual(self.selection(self.base, **options)['selected'], expected)
                self.tearDown()

    def test_shared_viewport_edit_and_planted_faults(self):
        camera = (self.root / CAMERA).read_text()
        for name, files, expected in (
                ('shared', {CAMERA: camera + '// shared edit\n'}, APP),
                ('f1', self.edited(VIEWPORT + 'look.rs', *F1), APP | DESKTOP),
                ('f2', {CAMERA: camera + F2['cfg']}, APP | DESKTOP),
                ('workspace', {'crates/soliddraft-app/src/app/workspace.rs':
                               (self.root / 'crates/soliddraft-app/src/app/workspace.rs').read_text() + '\n'},
                 APP | DESKTOP)):
            with self.subTest(name):
                self.commit(files)
                self.assertEqual(self.selection(self.base)['selected'], sorted(expected))
                self.tearDown()

    def test_base_map_comes_from_the_base_commit(self):
        policy = (self.root / POLICY).read_text()
        helper = '"ci/cargo-cache.sh": [],\n    "ci/linux-helper.py": '
        base = self.commit({'ci/linux-helper.py': 'one\n', POLICY: policy.replace(
            '"ci/cargo-cache.sh": [],\n    ', helper + '["test-macos"],\n    ')})
        self.commit({'ci/linux-helper.py': 'two\n', 'README.md': 'ordinary documentation edit\n', POLICY: policy.replace(
            '"ci/cargo-cache.sh": [],\n    ', helper + '["application-loop"],\n    ')})
        selection = self.selection(base)
        self.assertEqual(set(selection['selected']), qualified())
        self.assertIn('base: CI consumers: ci/linux-helper.py', selection['reasons']['test-macos'])

    def test_new_ci_file_with_its_map_entry_runs_only_lint(self):
        entry = self.edited(POLICY, '"ci/cargo-cache.sh": []', '"ci/cargo-cache.sh": [],\n    "ci/new.py": ["lint"]')
        self.commit({'ci/new.py': 'new\n', **entry})
        selection = self.selection(self.base)
        self.assertEqual(set(selection['selected']), {'lint'})
        self.assertTrue(all('CI-only' in reason for reason in selection['reasons']['lint']))

    def test_ci_only_real_diff_runs_just_the_changed_job_and_lint(self):
        workflow = (self.root / '.github/workflows/hosted.yml').read_text()
        workflow = workflow.replace('cargo test -p soliddraft-kernel-catallax',
                                    'cargo test --verbose -p soliddraft-kernel-catallax')
        self.commit({'.github/workflows/hosted.yml': workflow, 'docs/ci.md': 'CI documentation edit\n'})
        self.assertEqual(set(self.selection(self.base)['selected']), {'lint', 'kernel'})

    def test_only_a_manual_input_selects_windows(self):
        for name, options, expected in (
                ('manual, on', {'event': 'workflow_dispatch', 'windows': 'true'}, everything()),
                ('manual, off', {'event': 'workflow_dispatch', 'windows': 'false'}, qualified()),
                ('manual, unset', {'event': 'workflow_dispatch'}, qualified()),
                ('release cannot ask', {'event': 'push', 'ref': 'refs/heads/release/0.1.0', 'windows': 'true'},
                 qualified()),
                ('main cannot ask', {'event': 'push', 'ref': 'refs/heads/main', 'windows': 'true'}, selector.LINUX),
                ('pull request cannot ask', {'windows': 'true'}, {'lint'})):
            with self.subTest(name):
                self.assertEqual(set(self.selection(self.base, **options)['selected']), expected)

    def test_first_map_needs_no_base_map(self):
        current = {path: (self.root / path).read_text() for path in (POLICY, 'ci/select-checks.py')}
        base = self.commit({POLICY: None, 'ci/select-checks.py': FROZEN_BASE.read_text()})
        self.commit(current)
        self.assertEqual(set(self.selection(base)['selected']), everything())


class WindowsDeferred(unittest.TestCase):
    """Windows is deferred until a catch-up: nothing selects it unless a manual run asks."""

    NATIVE = APP | DESKTOP
    COMPILER = {'application-loop', 'interactive-linux', 'interactive-macos', 'kernel', 'lint', 'msrv',
                'step-opencascade', 'test-linux', 'test-macos', 'web'}
    FULL = COMPILER | {'package-linux', 'package-macos', 'package-web'}

    @classmethod
    def setUpClass(cls):
        cls.packages = selector.graph(ROOT)

    def select(self, changes, event='pull_request', ref='', sources=None, **options):
        read = lambda path: list((sources or {}).get(path) or [selector.content(ROOT, path)] * 2)
        return set(selector.select(changes, event, ref, self.packages, read, **options))

    def test_edits_that_selected_windows_no_longer_do(self):
        fault = selector.content(ROOT, CAMERA) + F2['cfg']
        for path, sources, expected in (
                (VIEWPORT + 'render.rs', None, self.NATIVE),
                (VIEWPORT + 'scene.wgsl', None, self.NATIVE),
                (CAMERA, {CAMERA: [fault, fault]}, self.NATIVE),
                ('crates/soliddraft-app/src/app/workspace.rs', None, self.NATIVE),
                ('crates/soliddraft-app/src/main.rs', None, self.NATIVE),
                ('Cargo.toml', None, self.COMPILER),
                ('Cargo.lock', None, self.COMPILER),
                ('rust-toolchain.toml', None, self.COMPILER),
                ('crates/soliddraft-app/build.rs', None, self.COMPILER),
                ('packaging/release.py', None, self.FULL),
                ('packaging/windows/package.ps1', None, self.FULL),
                ('.github/workflows/packages.yml', None, {'lint'}),
                ('.github/workflows/release.yml', None, {'lint'}),
                ('docs/RELEASING.md', None, {'lint', 'package-linux', 'package-macos', 'package-web'}),
                ('ci/select-checks.py', None, {'lint'}),
                ('somewhere/unknown', None, self.FULL)):
            with self.subTest(path):
                self.assertEqual(self.select([['M', path]], sources=sources), expected)

    def test_events_that_selected_windows_no_longer_do(self):
        render = [['M', VIEWPORT + 'render.rs']]
        self.assertEqual(self.select(render, 'push', 'refs/heads/main'), self.NATIVE | {'kernel', 'msrv'})
        for name, changes, event, ref in (('release branch', render, 'push', 'refs/heads/release/0.1.0'),
                                          ('manual dispatch', render, 'workflow_dispatch', ''),
                                          ('unavailable diff', None, 'pull_request', '')):
            with self.subTest(name):
                self.assertEqual(self.select(changes, event, ref), self.FULL)

    def test_the_manual_input_selects_exactly_the_windows_units(self):
        self.assertEqual(self.select(None, 'workflow_dispatch', windows=True), self.FULL | WINDOWS)

    def test_every_frozen_case_only_loses_the_windows_units(self):
        routing = Routing('test_frozen_fixtures')
        routing.sources = lambda p: [selector.content(ROOT, p, 'HEAD'), selector.content(ROOT, p)]
        for case in read('routing-cases.json')['cases']:
            with self.subTest(case['id']):
                arguments = (case['changes'], case['event'], case.get('ref', ''), self.packages,
                             routing.case_sources(case))
                self.assertEqual(set(selector.select(*arguments)),
                                 set(selector.select(*arguments, windows=True)) - WINDOWS)

    def test_the_landed_deferral_binds_the_accepted_selector_too(self):
        changes = [['M', VIEWPORT + 'render.rs']]
        sources = lambda path: [selector.content(ROOT, path)] * 2
        candidate = selector.select(changes, 'pull_request', '', self.packages, sources)
        script = (ROOT / 'ci/select-checks.py').read_text()
        union = selector.with_base_rules(candidate, script, changes, 'pull_request', '', self.packages, sources)
        self.assertEqual(set(union), self.NATIVE)


class Gate(unittest.TestCase):
    def gate_exit(self, selected, needs):
        with tempfile.TemporaryDirectory() as directory:
            folder = pathlib.Path(directory)
            (folder / 'selection.json').write_text(json.dumps({'selected': selected}))
            (folder / 'needs.json').write_text(json.dumps({job: {'result': result} for job, result in needs.items()}))
            registry = str(ROOT / 'ci/check-registry.json')
            return subprocess.run(['python3', str(ROOT / 'ci/check-gate.py'), '--registry', registry,
                                   '--base-registry', registry, '--selection', str(folder / 'selection.json'),
                                   '--needs', str(folder / 'needs.json')], capture_output=True, text=True).returncode

    def test_windows_unselected_passes_and_selected_must_be_green(self):
        tests, package = ['lint', 'test-windows'], ['lint', 'package-windows']
        for name, selected, windows, packaging, code in (
                ('unselected', ['lint'], 'skipped', 'skipped', 0),
                ('other packages', ['lint', 'package-linux'], 'skipped', 'success', 0),
                ('unselected but red', ['lint'], 'failure', 'skipped', 1),
                ('tests green', tests, 'success', 'skipped', 0),
                ('tests red', tests, 'failure', 'skipped', 1),
                ('tests cancelled', tests, 'cancelled', 'skipped', 1),
                ('tests skipped', tests, 'skipped', 'skipped', 1),
                ('packages green', package, 'skipped', 'success', 0),
                ('packages red', package, 'skipped', 'failure', 1),
                ('packages skipped', package, 'skipped', 'skipped', 1)):
            with self.subTest(name):
                needs = {'select': 'success', 'lint': 'success', 'test-windows': windows, 'packaging': packaging}
                self.assertEqual(self.gate_exit(selected, needs), code)
        self.assertEqual(self.gate_exit(tests, {'select': 'success', 'lint': 'success', 'packaging': 'skipped'}), 1)

    def test_workspace_unit_requires_both_build_and_shards(self):
        for platform in ('linux', 'macos'):
            build, shard = f'build-{platform}', f'test-{platform}'
            for build_result, shard_result, expected in (
                    ('success', 'success', 0), ('failure', 'success', 1),
                    ('cancelled', 'success', 1), ('skipped', 'success', 1),
                    ('success', 'failure', 1), ('success', 'cancelled', 1),
                    ('success', 'skipped', 1), (None, 'success', 1), ('success', None, 1)):
                with self.subTest(platform=platform, build=build_result, shards=shard_result):
                    needs = {'select': 'success', 'lint': 'success'}
                    if build_result is not None:
                        needs[build] = build_result
                    if shard_result is not None:
                        needs[shard] = shard_result
                    self.assertEqual(self.gate_exit(['lint', shard], needs), expected)

    def test_gate_fixture(self):
        fixture = read('gate-cases.json')
        for case in fixture['cases']:
            with self.subTest(case['id']):
                result = gate.verdict(fixture['registry'], case['base_registry'] or fixture['registry'],
                                      case['retired'], case['selected'], case['needs'])
                self.assertEqual(result['pass'], case['pass'])
                for name in case['names']:
                    self.assertIn(name, '\n'.join(result['messages'] + result['failures']))


class Wiring(unittest.TestCase):
    def test_gate_always_needs_every_job(self):
        workflow = (ROOT / '.github/workflows/hosted.yml').read_text()
        parsed = jobs(workflow)
        self.assertRegex(parsed['gate'], r'(?m)^    if: (?:\$\{\{ )?always\(\)')
        needs = re.search(r'^    needs: \[(.*?)\]', parsed['gate'], re.M)[1]
        self.assertEqual({n.strip() for n in needs.split(',')}, set(parsed) - {'gate'})
        registered = {job for definition in read('check-registry.json')['jobs'].values()
                      for job in ([definition] if isinstance(definition, str) else definition)}
        self.assertEqual(registered, set(parsed) - {'gate'})

    def test_workspace_shards_reuse_archives_and_keep_documentation_tests(self):
        parsed = jobs((ROOT / '.github/workflows/hosted.yml').read_text())
        for platform, count in (('linux', 4), ('macos', 2)):
            with self.subTest(platform=platform):
                build, shards = parsed[f'build-{platform}'], parsed[f'test-{platform}']
                self.assertIn('ci/run test-build', build)
                self.assertIn('ci/run test-doc', build)
                self.assertIn(f'needs: [select, build-{platform}]', shards)
                self.assertIn('fail-fast: false', shards)
                matrix = re.search(r'shard: \[(.*?)\]', shards)[1]
                self.assertEqual([int(n) for n in matrix.split(',')], list(range(1, count + 1)))
                self.assertIn('ci/run test-shard', shards)
                self.assertIn('SOLIDDRAFT_REQUIRE_GPU: "1"', shards)
                self.assertIn('LP_NUM_THREADS: "1"', shards)
                self.assertNotIn('ci/run test-build', shards)
                self.assertNotIn('cargo test', shards)
                self.assertNotIn('Swatinem/rust-cache', shards)

    def test_windows_jobs_stay_defined_behind_one_manual_input(self):
        hosted = (ROOT / '.github/workflows/hosted.yml').read_text()
        self.assertEqual(re.findall(r'^      (\w+):\n        description:', hosted, re.M),
                         ['refresh_catallax_views', 'windows_catch_up'])
        self.assertRegex(hosted, r'      windows_catch_up:\n        description: .*\n        type: boolean\n'
                                 r'        default: false\n')
        parsed = jobs(hosted)
        self.assertIn('WINDOWS: ${{ inputs.windows_catch_up }}', parsed['select'])
        self.assertIn('--windows "$WINDOWS"', parsed['select'])
        self.assertIn("if: contains(fromJSON(needs.select.outputs.selected), 'test-windows')", parsed['test-windows'])
        self.assertIn('runs-on: windows-latest', parsed['test-windows'])
        self.assertIn("windows: ${{ contains(fromJSON(needs.select.outputs.selected), 'package-windows') }}",
                      parsed['packaging'])
        registry = read('check-registry.json')['jobs']
        self.assertEqual((registry['test-windows'], registry['package-windows']), ('test-windows', 'packaging'))

    def test_a_package_run_builds_windows_only_when_asked(self):
        packages = (ROOT / '.github/workflows/packages.yml').read_text()
        self.assertRegex(packages, r'      windows:\n        description: .*\n        type: boolean\n'
                                   r'        default: false\n')
        parsed = jobs(packages)
        self.assertIn('include: ${{ fromJSON(needs.plan.outputs.desktop) }}', parsed['desktop'])
        self.assertNotIn('windows-2022', packages)
        self.assertRegex(parsed['assemble'], r'Require the portable Windows executable.*\n        if: inputs\.windows\n')
        flag = "${{ inputs.windows && '--windows' || '' }}"
        for command in (f'python3 packaging/release.py desktop {flag}', f'note_args=({flag})',
                        f'python3 packaging/release.py checksums {flag}'):
            self.assertIn(command, packages)
        # A tag or a release dry run passes no input, so it has no way to build Windows.
        self.assertNotIn('with:', jobs((ROOT / '.github/workflows/release.yml').read_text())['assemble'])

    def test_no_silent_failure(self):
        self.assertNotIn('continue-on-error', (ROOT / '.github/workflows/hosted.yml').read_text())

    def test_release_requires_exact_successful_hosted_run(self):
        parsed = jobs((ROOT / '.github/workflows/release.yml').read_text())
        self.assertRegex(parsed['publish'], r'needs: \[plan, assemble\]')
        self.assertIn('needs: plan', parsed['assemble'])
        self.assertIn('uses: ./.github/workflows/packages.yml', parsed['assemble'])
        plan = parsed['plan']
        self.assertIn('branch=$(python3 packaging/release.py check-source "$GITHUB_REF_NAME")', plan)
        self.assertIn('actions/workflows/hosted.yml/runs?head_sha=$GITHUB_SHA&event=push&branch=$branch', plan)
        self.assertIn("any(.conclusion == \"success\")' | grep -x true", plan)
        self.assertIn('"$GITHUB_REF" == refs/tags/v*', plan)


if __name__ == '__main__':
    unittest.main()
