#!/usr/bin/env python3
"""Run a validated sub-job plan without interpreting its data as shell commands."""
import argparse
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
KEYS = {'packages', 'test_targets', 'scenarios'}


def read_plan(selection):
    if selection is None:
        return dict.fromkeys(KEYS)
    plan = json.loads(selection.read_text())['work']
    if not isinstance(plan, dict) or set(plan) != KEYS:
        raise ValueError('missing or invalid work plan')
    patterns = {'packages': r'soliddraft-[a-z0-9-]+', 'test_targets': r'[A-Za-z0-9_-]+',
                'scenarios': r'ci/scenarios/[a-z0-9-]+\.json'}
    for key, pattern in patterns.items():
        values = plan[key]
        if values is not None and (not isinstance(values, list) or not values
                                   or any(not isinstance(v, str) or not re.fullmatch(pattern, v)
                                          for v in values) or len(set(values)) != len(values)):
            raise ValueError(f'invalid or empty {key} selection')
    if plan['test_targets'] is not None and (plan['packages'] is None or len(plan['packages']) != 1):
        raise ValueError('test targets require exactly one package')
    return plan


def cargo_args(plan, docs=False):
    args = ['--workspace'] if plan['packages'] is None else [
        arg for package in plan['packages'] for arg in ('-p', package)]
    if not docs and plan['test_targets'] is not None:
        args += [arg for target in plan['test_targets'] for arg in ('--test', target)]
    return args + ['--all-features', '--locked']


def scenario_paths(plan, root):
    all_paths = sorted(str(p.relative_to(root)) for p in (root / 'ci/scenarios').glob('*.json'))
    paths = all_paths if plan['scenarios'] is None else plan['scenarios']
    if not paths or not set(paths) <= set(all_paths):
        raise ValueError('empty scenario inventory or missing selected scenario')
    return paths


def run(args):
    plan = read_plan(args.selection)
    if args.command == 'test-build':
        # Shards may be empty, but the archive as a whole must contain real tests.
        # Listing compiles the same selection; archive reuses those build outputs.
        listing = ['cargo', 'nextest', 'list', *cargo_args(plan), '--message-format', 'json']
        print(json.dumps(listing), flush=True)
        result = subprocess.run(listing, cwd=ROOT, capture_output=True, text=True)
        if result.returncode:
            print(result.stderr, file=sys.stderr)
            return result.returncode
        count = json.loads(result.stdout)['test-count']
        if type(count) is not int or count <= 0:
            raise ValueError('selected archive must contain at least one test')
        print(f'Selected archive contains {count} tests', flush=True)
        command = ['cargo', 'nextest', 'archive', *cargo_args(plan), '--archive-file', args.archive]
        print(json.dumps(command), flush=True)
        return subprocess.run(command, cwd=ROOT).returncode
    if args.command == 'test-doc':
        command = ['cargo', 'test', *cargo_args(plan, docs=True), '--doc', '--no-fail-fast']
        print(json.dumps(command), flush=True)
        return subprocess.run(command, cwd=ROOT).returncode
    paths = scenario_paths(plan, ROOT)
    print(f'Running {len(paths)} application scenarios: {json.dumps(paths)}', flush=True)
    failed = False
    for path in paths:
        command = [sys.executable, str(ROOT / 'ci/run-scenarios.py'), path,
                   '--soliddraft', 'target/release/soliddraft',
                   '--catallax-verify', 'target/release/catallax-verify',
                   '--out', str(args.out / pathlib.PurePosixPath(path).stem)]
        if subprocess.run(command, cwd=ROOT).returncode:
            failed = True
    return int(failed)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('test-build', 'test-doc', 'scenarios'):
        command = sub.add_parser(name)
        command.add_argument('--selection', type=pathlib.Path)
        if name == 'test-build':
            command.add_argument('archive')
        if name == 'scenarios':
            command.add_argument('--out', type=pathlib.Path, required=True)
    return run(parser.parse_args())


if __name__ == '__main__':
    sys.exit(main())
