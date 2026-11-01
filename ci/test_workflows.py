"""Every workflow cancels its branch's superseded runs and never cancels a run on main."""

import json
import os
import pathlib
import re
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
    python = re.sub(r"github((?:\.\w+)+)",
                    lambda m: "g" + "".join(f"[{key!r}]" for key in m[1][1:].split('.')),
                    expr).replace("&&", " and ").replace("||", " or ")
    python = re.sub(r"vars\.(\w+)", r"v.get('\1', '')", python)
    python = re.sub(r"inputs\.(\w+)", r"i.get('\1', False)", python)
    python = re.sub(r"!(?!=)", " not ", python)
    context = {"workflow": "Hosted CI", "head_ref": "", "run_id": "1", **github}
    return eval(python, {"__builtins__": {}}, {
        "g": context,
        "i": inputs,
        "v": variables,
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

    def test_a_manual_run_on_a_branch_is_grouped_by_the_branch(self):
        for path in WORKFLOWS:
            group, cancel, _ = settings(path)
            run = dict(ref="refs/heads/feature", run_id="5")
            self.assertEqual(render(group, **run), render(group, **dict(run, run_id="6")), path.name)
            self.assertEqual(render(cancel, **run), "true", path.name)

    def test_a_run_on_main_is_never_cancelled_nor_shares_a_group(self):
        for path in WORKFLOWS:
            group, cancel, _ = settings(path)
            pushes = [render(group, ref=MAIN, run_id=str(n)) for n in (200, 201, 202)]
            self.assertEqual(len(set(pushes)), 3, f"{path.name}: main pushes share a group, so a waiting one is replaced")
            self.assertEqual(render(cancel, ref=MAIN, run_id="200"), "false", path.name)

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
    def test_only_an_explicit_refresh_dispatch_changes_the_gate(self):
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


class BoatRunners(unittest.TestCase):
    def test_only_owner_runs_from_this_repository_use_boat(self):
        text = next(p for p in WORKFLOWS if p.name == 'hosted.yml').read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:|\Z)",
                               text, re.M | re.S))
        moved = {'lint', 'kernel', 'step-opencascade', 'build-linux', 'test-linux',
                 'web', 'msrv', 'interactive-linux', 'application-loop'}
        self.assertEqual({job for job, block in jobs.items() if 'vars.USE_BOAT' in block}, moved)
        self.assertEqual(len(moved) - 1 + 4, 12)
        self.assertIn('shard: [1, 2, 3, 4]', jobs['test-linux'])
        owner = 'BohnBawerick'
        repo = 'ZIGRAWBOHNBAWERICK/soliddraft'
        for job in moved:
            expression = re.search(r'^    runs-on: \$\{\{ (.*) \}\}$', jobs[job], re.M)[1]
            size = re.search(r"&& '(boat(?:-large)?)' \|\| 'ubuntu-latest'$", expression)[1]
            self.assertNotIn('USE_LOCAL_RUNNERS', expression)
            for event in ('push', 'workflow_dispatch', 'pull_request'):
                for enabled in ('true', 'false', ''):
                    for repository in (repo, 'elsewhere/soliddraft'):
                        for actor in (owner, 'dependabot[bot]'):
                            for starter in (owner, 'another-user'):
                                for author in (owner, 'another-user'):
                                    for source in (repo, 'fork/soliddraft'):
                                        values = dict(variables={'USE_BOAT': enabled},
                                                      repository=repository, actor=actor,
                                                      triggering_actor=starter, event_name=event,
                                                      event={'pull_request': {
                                                          'user': {'login': author},
                                                          'head': {'repo': {'full_name': source}}}})
                                        allowed = (enabled == 'true' and repository == repo
                                                   and actor == owner and starter == owner
                                                   and (event != 'pull_request'
                                                        or (author == owner and source == repo)))
                                        with self.subTest(job=job, **values):
                                            self.assertEqual(evaluate(expression, **values),
                                                             size if allowed else 'ubuntu-latest')


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
        self.assertEqual(count, 13)
        self.assertEqual(keys, {'native-dev', 'native-test-opt2', 'wasm-dev',
                                'native-release', 'wasm-release'})


if __name__ == "__main__":
    unittest.main()
