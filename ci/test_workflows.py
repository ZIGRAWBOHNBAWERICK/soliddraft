"""Workflow concurrency cancels superseded runs while preserving release evidence."""

import json
import os
import pathlib
import re
import runpy
import subprocess
import tempfile
import textwrap
import unittest

WORKFLOWS = sorted(pathlib.Path(__file__).resolve().parent.parent.joinpath(".github", "workflows").glob("*.yml"))
MAIN = "refs/heads/main"


def top_level(text, key):
    """The value of a top-level `key:` or of its indented lines, without a YAML parser."""
    match = re.search(rf"^{key}:[ \t]*(.*)\n((?:[ \t]+.*\n|[ \t]*\n)*)", text, re.MULTILINE)
    return match and (match.group(1), match.group(2))


def evaluate(expr, **github):
    """Evaluates the expression subset used by the workflow settings under test."""
    inputs = github.pop("inputs", {})
    variables = github.pop("variables", {})
    needs = github.pop("needs", {})
    matrix = github.pop("matrix", {})
    python = re.sub(r"github((?:\.\w+)+)",
                    lambda m: "g" + "".join(f"[{key!r}]" for key in m[1][1:].split('.')),
                    expr).replace("&&", " and ").replace("||", " or ")
    python = re.sub(r"needs\.select\.outputs\.(\w+)", r"n.get('\1', '')", python)
    python = re.sub(r"matrix\.(\w+)", r"m.get('\1', '')", python)
    python = re.sub(r"vars\.(\w+)", r"v.get('\1', '')", python)
    python = re.sub(r"inputs\.(\w+)", r"i.get('\1', False)", python)
    python = re.sub(r"!(?!=)", " not ", python)
    context = {"workflow": "Hosted CI", "head_ref": "", "run_id": "1", **github}
    return eval(python, {"__builtins__": {}}, {
        "g": context,
        "i": inputs,
        "v": variables,
        "n": needs,
        "m": matrix,
        "contains": lambda values, item: item in values,
        "fromJSON": json.loads,
        "startsWith": lambda value, prefix: value.lower().startswith(prefix.lower()),
    })


def render(template, **github):
    """A setting's value with each `${{ ... }}` replaced by its result; booleans as GitHub prints them."""
    def value(match):
        result = evaluate(match.group(1).strip(), **github)
        return str(result).lower() if isinstance(result, bool) else str(result)

    return re.sub(r"\$\{\{(.*?)\}\}", value, template)


def settings(path):
    """The workflow's group and cancel-in-progress templates, and its text."""
    text = path.read_text()
    block = top_level(text, "concurrency")
    assert block, f"{path.name} has no concurrency group"
    fields = dict(re.findall(r"^[ \t]+([\w-]+):[ \t]*(.*)$", block[1], re.MULTILINE))
    return fields["group"], fields["cancel-in-progress"], text


class Concurrency(unittest.TestCase):
    def test_there_are_workflows(self):
        self.assertTrue(WORKFLOWS)

    def test_a_push_to_a_pull_request_branch_cancels_its_older_runs(self):
        for path in WORKFLOWS:
            group, cancel, _ = settings(path)
            first = dict(ref="refs/pull/7/merge", head_ref="feature", run_id="100")
            second = dict(first, run_id="101")
            self.assertEqual(render(group, **first), render(group, **second), path.name)
            self.assertEqual(render(cancel, **second), "true", path.name)

    def test_other_branches_do_not_share_a_group(self):
        for path in WORKFLOWS:
            group = settings(path)[0]
            one = render(group, ref="refs/pull/7/merge", head_ref="feature-one")
            two = render(group, ref="refs/pull/8/merge", head_ref="feature-two")
            self.assertNotEqual(one, two, path.name)

    def test_boat_benchmark_does_not_cancel_the_normal_qualification(self):
        path = next(p for p in WORKFLOWS if p.name == 'hosted.yml')
        group, _, text = settings(path)
        values = dict(ref='refs/heads/feature', event_name='workflow_dispatch')
        self.assertNotEqual(render(group, **values),
                            render(group, **values, inputs={'benchmark_boat': True}))
        boat = re.search(r"^          USE_BOAT: (.*)$", text, re.M)[1]
        for event, requested, enabled, expected in [
            ('pull_request', True, 'false', False),
            ('workflow_dispatch', False, 'false', False),
            ('workflow_dispatch', True, 'false', True),
            ('pull_request', False, 'true', True),
        ]:
            self.assertEqual(evaluate(boat.removeprefix('${{ ').removesuffix(' }}'),
                                      event_name=event, inputs={'benchmark_boat': requested},
                                      variables={'USE_BOAT': enabled}), expected)

    def test_a_manual_run_on_a_branch_is_grouped_by_the_branch(self):
        for path in WORKFLOWS:
            group, cancel, _ = settings(path)
            run = dict(ref="refs/heads/feature", run_id="5")
            self.assertEqual(render(group, **run), render(group, **dict(run, run_id="6")), path.name)
            self.assertEqual(render(cancel, **run), "true", path.name)

    def test_linux_benchmark_is_explicit_manual_full_linux_work(self):
        path = next(p for p in WORKFLOWS if p.name == 'hosted.yml')
        group, _, text = settings(path)
        expression = re.search(r'^          BENCHMARK_LINUX_ONLY: (.*)$', text, re.M)[1]
        expression = expression.removeprefix('${{ ').removesuffix(' }}')
        script = textwrap.dedent(text.split("<<'PYTHON'\n", 1)[1].split('          PYTHON\n', 1)[0])
        full = ['lint', 'kernel', 'step-opencascade', 'test-linux', 'web', 'msrv',
                'interactive-linux', 'application-loop', 'test-macos', 'interactive-macos',
                'package-linux', 'package-macos', 'package-web']
        linux = set(full) - {'test-macos', 'interactive-macos', 'package-linux',
                            'package-macos', 'package-web'}
        for event, requested in [('workflow_dispatch', True), ('workflow_dispatch', False),
                                 ('pull_request', True), ('push', True), ('schedule', True)]:
            with self.subTest(event=event, requested=requested), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                selection, output, summary = (root / p for p in ('selection.json', 'output', 'summary'))
                original = {'selected': full, 'reasons': {u: ['full qualification'] for u in full},
                            'work': {'packages': None, 'test_targets': None, 'scenarios': None}}
                selection.write_text(json.dumps(original))
                enabled = evaluate(expression, event_name=event, inputs={'benchmark_linux_only': requested})
                env = dict(os.environ, BENCHMARK_LINUX_ONLY=str(enabled).lower(),
                           REFRESH_CATALLAX_VIEWS='false', WINDOWS='false')
                result = subprocess.run(['python3', '-c', script, str(selection), str(output), str(summary)],
                                        cwd=path.parent.parent.parent, env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                selected = json.loads(selection.read_text())
                self.assertEqual(set(selected['selected']), linux if enabled else set(full))
                self.assertEqual(selected['work'], original['work'])
        for conflicting in ('REFRESH_CATALLAX_VIEWS', 'WINDOWS'):
            with self.subTest(conflicting=conflicting), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                selection, output, summary = (root / p for p in ('selection.json', 'output', 'summary'))
                selection.write_text(json.dumps(original))
                env = dict(os.environ, BENCHMARK_LINUX_ONLY='true',
                           REFRESH_CATALLAX_VIEWS='false', WINDOWS='false')
                env[conflicting] = 'true'
                result = subprocess.run(['python3', '-c', script, str(selection), str(output), str(summary)],
                                        cwd=path.parent.parent.parent, env=env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('cannot be combined', result.stderr)
                self.assertFalse(output.exists())
        values = dict(ref='refs/heads/feature', event_name='workflow_dispatch')
        self.assertNotEqual(render(group, **values),
                            render(group, **values, inputs={'benchmark_linux_only': True}))
        self.assertNotEqual(render(group, **values, inputs={'benchmark_boat': True}),
                            render(group, **values, inputs={'benchmark_boat': True, 'benchmark_linux_only': True}))

    def test_main_runs_supersede_only_in_hosted_ci(self):
        for path in WORKFLOWS:
            group, cancel, _ = settings(path)
            pushes = [render(group, ref=MAIN, run_id=str(n)) for n in (200, 201, 202)]
            hosted = path.name == "hosted.yml"
            self.assertEqual(len(set(pushes)), 1 if hosted else 3, path.name)
            self.assertEqual(render(cancel, ref=MAIN, run_id="200"), str(hosted).lower(), path.name)

    def test_a_pull_request_from_a_branch_called_main_cannot_reach_main_runs(self):
        for path in WORKFLOWS:
            group = settings(path)[0]
            fork = render(group, ref="refs/pull/9/merge", head_ref="main", run_id="300")
            self.assertNotEqual(fork, render(group, ref=MAIN, run_id="300"), path.name)
            self.assertNotEqual(fork, render(group, ref=MAIN, run_id="301"), path.name)

    def test_release_branch_runs_keep_independent_evidence(self):
        for path in WORKFLOWS:
            group, cancel, _ = settings(path)
            run = dict(ref="refs/heads/release/0.1.0", run_id="350")
            self.assertNotEqual(render(group, **run), render(group, **dict(run, run_id="351")), path.name)
            self.assertEqual(render(cancel, **run), "false", path.name)
            fork = dict(ref="refs/pull/10/merge", head_ref="release/0.1.0", run_id="350")
            self.assertNotEqual(render(group, **run), render(group, **fork), path.name)
            self.assertEqual(render(cancel, **fork), "true", path.name)

    def test_tag_releases_cannot_cancel_or_replace_one_another(self):
        path = pathlib.Path(__file__).resolve().parent.parent / ".github/workflows/release.yml"
        group, cancel, _ = settings(path)
        first = dict(ref="refs/tags/v0.1.0", run_id="400")
        second = dict(first, run_id="401")
        self.assertNotEqual(render(group, **first), render(group, **second))
        self.assertEqual(render(cancel, **first), "false")
        self.assertEqual(render(cancel, **second), "false")

    def test_the_setting_is_not_overridden_per_job(self):
        for path in WORKFLOWS:
            text = settings(path)[2]
            self.assertEqual(len(re.findall(r"^\s*concurrency:", text, re.MULTILINE)), 1, path.name)


class BaselineRefresh(unittest.TestCase):
    def test_refresh_dispatch_has_its_own_gate(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        refresh = re.search(r"REFRESH_CATALLAX_VIEWS: (.*)", text)[1]
        kernel = text.split("\n  kernel:\n", 1)[1].split("\n  step-opencascade:", 1)[0]
        condition = re.search(r"^        if: (github.event_name.*)$", kernel, re.M)[1]
        gate = text.split("\n  gate:\n", 1)[1]
        name = re.search(r"^    name: (.*)$", gate, re.M)[1]
        for event, requested in [('pull_request', False), ('pull_request', True),
                                 ('push', True), ('workflow_dispatch', False),
                                 ('workflow_dispatch', True)]:
            with self.subTest(event=event, requested=requested):
                values = dict(event_name=event, inputs={'refresh_catallax_views': requested})
                refreshing = event == 'workflow_dispatch' and requested
                self.assertEqual(render(refresh, **values), str(refreshing).lower())
                self.assertEqual(evaluate(condition, **values), not refreshing)
                self.assertEqual(render(name, **values),
                                 'Baseline refresh gate' if refreshing else 'Merge gate')

    def test_linux_benchmark_has_its_own_gate(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        gate = text.split('\n  gate:\n', 1)[1]
        name = re.search(r'^    name: (.*)$', gate, re.M)[1]
        for event in ('pull_request', 'push', 'schedule', 'workflow_dispatch'):
            for requested in (False, True):
                with self.subTest(event=event, requested=requested):
                    values = dict(event_name=event, inputs={'benchmark_linux_only': requested})
                    expected = ('Linux benchmark gate' if event == 'workflow_dispatch' and requested
                                else 'Merge gate')
                    self.assertEqual(render(name, **values), expected)

    def test_selection_step_runs_only_generators_for_refresh(self):
        root = pathlib.Path(__file__).resolve().parent.parent
        text = (root / '.github/workflows/hosted.yml').read_text()
        block = text.split('      - id: select\n', 1)[1].split('      - uses:', 1)[0]
        script = textwrap.dedent(block.split('        run: |\n', 1)[1])
        registry = json.loads((root / 'ci/check-registry.json').read_text())['jobs']
        full = set(registry) - {'select', 'test-windows', 'package-windows'}
        for refresh in ('true', 'false'):
            with self.subTest(refresh=refresh), tempfile.TemporaryDirectory() as directory:
                temporary = pathlib.Path(directory)
                env = dict(os.environ, RUNNER_TEMP=directory, BASE_SHA='HEAD',
                           EVENT='workflow_dispatch', REF='', WINDOWS='',
                           REFRESH_CATALLAX_VIEWS=refresh,
                           GITHUB_OUTPUT=str(temporary / 'output'),
                           GITHUB_STEP_SUMMARY=str(temporary / 'summary'))
                subprocess.run(['bash', '-e', '-c', script], cwd=root, env=env,
                               check=True, capture_output=True, text=True)
                result = json.loads((temporary / 'check-selection/selection.json').read_text())
                self.assertEqual(set(result['selected']), {'kernel'} if refresh == 'true' else full)
                self.assertIn('packaging=' + ('false' if refresh == 'true' else 'true'),
                              (temporary / 'output').read_text())


class MacOSQualification(unittest.TestCase):
    def test_pr_exclusion_preserves_other_units_work_and_gate_requirements(self):
        root = pathlib.Path(__file__).resolve().parent.parent
        text = (root / '.github/workflows/hosted.yml').read_text()
        script = textwrap.dedent(text.split("<<'PYTHON'\n", 1)[1].split('          PYTHON\n', 1)[0])
        registry = json.loads((root / 'ci/check-registry.json').read_text())['jobs']
        full = set(registry) - {'select'}
        macos = {'test-macos', 'interactive-macos', 'package-macos'}
        verdict = runpy.run_path(str(root / 'ci/check-gate.py'))['verdict']
        for event, ref in [('pull_request', 'refs/pull/7/merge'), ('push', MAIN),
                           ('schedule', MAIN), ('workflow_dispatch', 'refs/heads/feature'),
                           ('push', 'refs/heads/release/0.1.0')]:
            with self.subTest(event=event, ref=ref), tempfile.TemporaryDirectory() as directory:
                temporary = pathlib.Path(directory)
                selection, output, summary = (temporary / p for p in ('selection.json', 'output', 'summary'))
                original = {'selected': sorted(full), 'reasons': {u: ['base: required'] for u in full},
                            'work': {'packages': ['soliddraft-app'], 'test_targets': None, 'scenarios': None}}
                selection.write_text(json.dumps(original))
                env = dict(os.environ, EVENT=event, REF=ref, BENCHMARK_LINUX_ONLY='false',
                           REFRESH_CATALLAX_VIEWS='false', WINDOWS='false')
                subprocess.run(['python3', '-c', script, str(selection), str(output), str(summary)],
                               cwd=root, env=env, check=True, capture_output=True, text=True)
                result = json.loads(selection.read_text())
                expected = full - macos if event == 'pull_request' else full
                self.assertEqual(set(result['selected']), expected)
                self.assertEqual(set(result['reasons']), expected)
                self.assertEqual(result['work'], original['work'])
                values = dict(line.split('=', 1) for line in output.read_text().splitlines())
                self.assertEqual(set(json.loads(values['selected'])), expected)
                self.assertEqual(values['packaging'], 'true')
                results = dict.fromkeys(registry, 'success')
                if event == 'pull_request':
                    results.update(dict.fromkeys(macos, 'skipped'))
                self.assertTrue(verdict(registry, registry, [], result['selected'], results)['pass'])
                for required in ('test-linux', 'test-windows', 'package-linux', 'package-windows', 'package-web'):
                    with self.subTest(required=required):
                        self.assertFalse(verdict(registry, registry, [], result['selected'],
                                                 dict(results, **{required: 'skipped'}))['pass'])

    def test_package_matrix_and_inventory_use_the_same_event_policy(self):
        root = pathlib.Path(__file__).resolve().parent.parent
        text = (root / '.github/workflows/packages.yml').read_text()
        desktop = re.search(r'python3 packaging/release.py desktop (.*?)\)"', text)[1]
        checksums = re.search(r'python3 packaging/release.py checksums (.*)', text)[1]
        for event in ('pull_request', 'push', 'schedule', 'workflow_dispatch'):
            for windows in (False, True):
                with self.subTest(event=event, windows=windows):
                    values = dict(event_name=event, inputs={'windows': windows})
                    options = render(desktop, **values).split()
                    self.assertEqual(options, render(checksums, **values).split())
                    result = subprocess.run(['python3', 'packaging/release.py', 'desktop', *options],
                                            cwd=root, check=True, capture_output=True, text=True)
                    platforms = {cell['platform'] for cell in json.loads(result.stdout)}
                    self.assertEqual(platforms, {'linux'} | ({'windows'} if windows else set())
                                     | ({'macos'} if event != 'pull_request' else set()))


class NightlyQualification(unittest.TestCase):
    def test_nightly_uses_the_full_selector_and_normal_gate(self):
        root = pathlib.Path(__file__).resolve().parent.parent
        text = (root / '.github/workflows/hosted.yml').read_text()
        self.assertIn("  schedule:\n    - cron: '17 23 * * *'\n", text)
        block = text.split('      - id: select\n', 1)[1].split('      - uses:', 1)[0]
        script = textwrap.dedent(block.split('        run: |\n', 1)[1])
        registry = json.loads((root / 'ci/check-registry.json').read_text())['jobs']
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            env = dict(os.environ, RUNNER_TEMP=directory, BASE_SHA='HEAD', EVENT='schedule',
                       REF=MAIN, WINDOWS='', REFRESH_CATALLAX_VIEWS='false',
                       GITHUB_OUTPUT=str(temporary / 'output'),
                       GITHUB_STEP_SUMMARY=str(temporary / 'summary'))
            subprocess.run(['bash', '-e', '-c', script], cwd=root, env=env,
                           check=True, capture_output=True, text=True)
            selection = json.loads((temporary / 'check-selection/selection.json').read_text())
            self.assertEqual(set(selection['selected']), set(registry) - {'select', 'test-windows', 'package-windows'})
            self.assertEqual(selection['work'], dict.fromkeys(('packages', 'test_targets', 'scenarios')))
            self.assertIn('packaging=true', (temporary / 'output').read_text())


class BoatRunners(unittest.TestCase):
    def test_outside_runs_cannot_override_runner_selection(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:|\Z)", text, re.M | re.S))
        for job in ('lint', 'kernel', 'build-linux', 'step-opencascade', 'application-loop', 'msrv', 'partition-linux', 'test-linux'):
            expression = re.search(r'^    runs-on: \$\{\{ (.*) \}\}$', jobs[job], re.M)[1]
            for actor, author, source, starter in (
                    ('BohnBawerick', 'BohnBawerick', 'ZIGRAWBOHNBAWERICK/soliddraft', 'BohnBawerick'),
                    ('outsider', 'BohnBawerick', 'ZIGRAWBOHNBAWERICK/soliddraft', 'BohnBawerick'),
                    ('BohnBawerick', 'outsider', 'ZIGRAWBOHNBAWERICK/soliddraft', 'BohnBawerick'),
                    ('BohnBawerick', 'BohnBawerick', 'fork/soliddraft', 'BohnBawerick'),
                    ('BohnBawerick', 'BohnBawerick', 'ZIGRAWBOHNBAWERICK/soliddraft', 'outsider')):
                values = dict(repository='ZIGRAWBOHNBAWERICK/soliddraft', actor=actor,
                              triggering_actor=starter, event_name='pull_request',
                              event={'pull_request': {'user': {'login': author},
                                     'head': {'repo': {'full_name': source}}}},
                              needs={k: '["boat-large"]' for k in
                                     ('plain_runner', 'archive_runner', 'application_runner', 'blade_runner')})
                values['matrix'] = {'shard': 2, 'lane': 1}
                trusted = actor == author == starter == 'BohnBawerick' and source == values['repository']
                self.assertEqual(evaluate(expression, **values),
                                 ['boat-large'] if trusted else ['ubuntu-latest'])
        for job in ('select', 'gate'):
            self.assertIn('    runs-on: ubuntu-latest\n', jobs[job])
        self.assertIn('ci/select-runner.py', jobs['select'])
        self.assertIn('needs.select.outputs.plain_runner', jobs['msrv'])
        for job in ('lint', 'kernel'):
            self.assertNotIn('needs.select.outputs.plain_runner', jobs[job])
        self.assertIn('lane: [proof, 1, 2, 3]', jobs['application-loop'])
        self.assertIn("if: matrix.lane == 'proof'", jobs['application-loop'])
        self.assertIn('application-loop-${{ matrix.lane }}', jobs['application-loop'])


class BladeRouting(unittest.TestCase):
    def test_distribution_and_container_follow_the_selected_pool(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:|\Z)", text, re.M | re.S))
        values = dict(repository='ZIGRAWBOHNBAWERICK/soliddraft', actor='BohnBawerick',
                      triggering_actor='BohnBawerick', event_name='push',
                      needs={'blade_runner': '["self-hosted","blade-sd"]',
                             'archive_runner': '["boat-large"]',
                             'application_runner': '["boat-large"]'})
        cases = [(job, {'shard': n}, ['self-hosted', 'blade-sd'] if n % 2 == 0 else ['ubuntu-latest'])
                 for job in ('partition-linux', 'test-linux') for n in range(1, 9)]
        cases += [('application-loop', {'lane': lane},
                   ['self-hosted', 'blade-sd'] if lane == 1 else ['ubuntu-latest'] if lane == 3 else ['boat-large'])
                  for lane in ('proof', 1, 2, 3)]
        cases += [(job, {}, ['self-hosted', 'blade-sd'] if job == 'lint' else ['boat-large'])
                  for job in ('lint', 'kernel', 'build-linux', 'step-opencascade')]
        for job, matrix, expected in cases:
            runner = re.search(r'^    runs-on: \$\{\{ (.*) \}\}$', jobs[job], re.M)[1]
            image = re.search(r'^      image: \$\{\{ (.*) \}\}$', jobs[job], re.M)[1]
            with self.subTest(job=job, matrix=matrix):
                self.assertEqual(evaluate(runner, **values, matrix=matrix), expected)
                selected_image = evaluate(image, **values, matrix=matrix)
                if 'blade-sd' in expected:
                    self.assertRegex(selected_image, r'^ubuntu@sha256:[0-9a-f]{64}$')
                else:
                    self.assertEqual(selected_image, '')
                self.assertEqual(evaluate(image, **dict(values, actor='outsider'), matrix=matrix), '')
                self.assertIn('shell: bash', jobs[job])
                self.assertIn('Prepare Blade Ubuntu container', jobs[job])
                self.assertIn('Clean Blade build output', jobs[job])


class BladeCleanup(unittest.TestCase):
    def test_cleanup_removes_only_this_jobs_build_output(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        blocks = text.split('      - name: Clean Blade build output\n')[1:]
        self.assertTrue(blocks)
        for block in blocks:
            condition = block.splitlines()[0]
            self.assertIn("always() && startsWith(runner.name, 'blade-sd-')", condition)
            script = textwrap.dedent(block.split('        run: |\n', 1)[1].split('\n\n', 1)[0])
            with tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                (root / 'target').mkdir()
                (root / 'target/large-build').write_text('build output')
                (root / 'keep').write_text('source')
                marker = root / 'container-marker'
                marker.touch()
                (root / 'portable/target').mkdir(parents=True)
                binary = root / 'bin'
                binary.mkdir()
                (binary / 'curl').write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$PRUNE_LOG"\n')
                (binary / 'curl').chmod(0o755)
                self.assertIn('test -f /.dockerenv', script)
                subprocess.run(['bash', '-e', '-c', script.replace('/.dockerenv', str(marker))],
                               check=True, env=dict(os.environ, GITHUB_WORKSPACE=str(root),
                                   LINUX_ARCHIVE_WORKSPACE=str(root / 'portable'), RUNNER_TEMP=str(root),
                                   GITHUB_RUN_ID='123', GITHUB_JOB='fixture', PRUNE_LOG=str(root / 'prune.log'),
                                   PATH=f'{binary}:{os.environ["PATH"]}'))
                prune = (root / 'prune.log').read_text()
                self.assertIn('soliddraft.ci.job=123-fixture', prune)
                self.assertIn('"dangling":["true"]', prune)
                self.assertFalse((root / 'portable').exists())
                self.assertFalse((root / 'target').exists())
                self.assertEqual((root / 'keep').read_text(), 'source')


class BrowserStartup(unittest.TestCase):
    def test_only_a_lost_bridge_retries_open_once(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        start = text.index('          open_browser() {')
        end = text.index('          python3 -m http.server', start)
        script = textwrap.dedent(text[start:end]) + '\nopen_browser http://127.0.0.1:8766/\n'
        for mode, expected, calls in [
            ('bridge_once', 0, ['open', 'stop', 'open']),
            ('bridge_always', 7, ['open', 'stop', 'open']),
            ('other', 7, ['open']),
            ('stop_failed', 0, ['open', 'stop', 'open']),
        ]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                tool = root / 'chrome-devtools-axi'
                tool.write_text('#!/usr/bin/env python3\n'
                                'import os, pathlib, sys\n'
                                'log = pathlib.Path(os.environ["CALLS"])\n'
                                'previous = log.read_text().splitlines() if log.exists() else []\n'
                                'command = sys.argv[1]\n'
                                'log.write_text("\\n".join(previous + [command]) + "\\n")\n'
                                'mode = os.environ["MODE"]\n'
                                'if command == "stop": sys.exit(7 if mode == "stop_failed" else 0)\n'
                                'if mode != "other" and mode != "bridge_always" and "open" in previous: sys.exit(0)\n'
                                'print("code: " + ("OTHER_ERROR" if mode == "other" else "BRIDGE_NOT_READY"))\n'
                                'sys.exit(7)\n')
                tool.chmod(0o755)
                env = dict(os.environ, PATH=f'{root}:{os.environ["PATH"]}', MODE=mode,
                           CALLS=str(root / 'calls'), RUNNER_TEMP=str(root))
                result = subprocess.run(['bash', '-e', '-c', script], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual((root / 'calls').read_text().splitlines(), calls)
        for name in ('browser-journal.js', 'browser-recovery-storage.js'):
            self.assertEqual(text.count(f'chrome-devtools-axi run < ci/{name}'), 1)


class ArchiveWorkspace(unittest.TestCase):
    def test_archive_preparation_keeps_the_commit_without_copying_credentials(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        block = text.split('      - name: Prepare the portable archive workspace\n', 1)[1].split('      - uses:', 1)[0]
        script = textwrap.dedent(block.split('        run: |\n', 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / 'producer'
            destination = root / 'hosted/project'
            subprocess.run(['git', 'init', '-q', source], check=True)
            (source / 'fixture').write_text('source')
            subprocess.run(['git', '-C', source, 'add', 'fixture'], check=True)
            subprocess.run(['git', '-C', source, '-c', 'user.name=Fixture', '-c',
                            'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture'], check=True)
            subprocess.run(['git', '-C', source, 'config', 'http.extraheader', 'auth-marker'], check=True)
            (source / 'untracked').write_text('runtime data')
            binaries = root / 'bin'
            binaries.mkdir()
            sudo = binaries / 'sudo'
            sudo.write_text('#!/bin/sh\necho called >> "$SUDO_LOG"\nexec "$@"\n')
            sudo.chmod(0o755)
            env = dict(os.environ, GITHUB_WORKSPACE=str(source),
                       LINUX_ARCHIVE_WORKSPACE=str(destination), SUDO_LOG=str(root / 'sudo.log'),
                       PATH=f'{binaries}:{os.environ["PATH"]}')
            subprocess.run(['bash', '-e', '-c', script], env=env, check=True, capture_output=True)
            self.assertEqual((destination / 'fixture').read_text(), 'source')
            self.assertNotIn('auth-marker', (destination / '.git/config').read_text())
            self.assertFalse((destination / 'untracked').exists())
            self.assertEqual(subprocess.check_output(['git', '-C', source, 'rev-parse', 'HEAD']),
                             subprocess.check_output(['git', '-C', destination, 'rev-parse', 'HEAD']))
            env['GITHUB_WORKSPACE'] = str(destination)
            subprocess.run(['bash', '-e', '-c', script], env=env, check=True, capture_output=True)
            self.assertEqual((root / 'sudo.log').read_text().splitlines(), ['called'])
        linux = text.split('\n  build-linux:', 1)[1].split('\n  test-linux:', 1)[0]
        self.assertEqual(linux.count('working-directory: ${{ env.LINUX_ARCHIVE_WORKSPACE }}'), 3)
        self.assertIn('Prepare the archive consumer workspace', text)


class RustCache(unittest.TestCase):
    def test_compatible_jobs_share_caches_and_only_main_saves(self):
        keys = set()
        count = 0
        for path in WORKFLOWS:
            for block in re.findall(r"      - uses: Swatinem/rust-cache@v2\n(.*?)(?=      - |\Z)",
                                    path.read_text(), re.DOTALL):
                count += 1
                self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' }}", block, path.name)
                key = re.search(r"shared-key: ([\w-]+)", block)
                self.assertIsNotNone(key, path.name)
                keys.add(key[1])
                self.assertNotRegex(block, r"(?m)^          key:")
        self.assertEqual(count, 14)
        self.assertEqual(keys, {'native-dev', 'native-test-opt2', 'wasm-dev',
                                'native-release', 'wasm-release'})


class PagesPayload(unittest.TestCase):
    def test_package_preview_leaves_the_tag_deployment_artifact_to_pages(self):
        text = next(p for p in WORKFLOWS if p.name == "packages.yml").read_text()
        preview = text.split("\n  pages-preview:\n", 1)[1].split("\n  assemble:\n", 1)[0]
        condition = re.search(r"^    if: (.*)$", preview, re.M)[1]
        for event, ref, expected in [
            ("push", "refs/tags/v0.2.0", False),
            ("push", "refs/heads/release/0.2.0", True),
            ("workflow_dispatch", "refs/tags/v0.2.0", True),
            ("workflow_dispatch", "refs/heads/release/0.2.0", True),
            ("pull_request", "refs/pull/7/merge", True),
        ]:
            with self.subTest(event=event, ref=ref):
                self.assertEqual(evaluate(condition, event_name=event, ref=ref), expected)
        self.assertEqual(text.count("actions/upload-pages-artifact@"), 1)
        self.assertIn("path: site/app/dist/pages", preview)


if __name__ == "__main__":
    unittest.main()
