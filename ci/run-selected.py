#!/usr/bin/env python3
"""Run a validated sub-job plan without interpreting its data as shell commands."""
import argparse
import json
import pathlib
import re
import subprocess
import sys
import time

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


def doctest_packages(plan, metadata):
    """Find selected library roots; Cargo still decides which doctests to execute."""
    members = set(metadata['workspace_members'])
    packages = {p['name']: p for p in metadata['packages'] if p['id'] in members}
    selected = set(packages) if plan['packages'] is None else set(plan['packages'])
    missing = selected - packages.keys()
    if missing:
        raise ValueError(f'unknown selected workspace packages: {", ".join(sorted(missing))}')
    library_kinds = {'lib', 'rlib', 'dylib', 'cdylib', 'staticlib', 'proc-macro'}
    return sorted(name for name in selected if any(
        library_kinds.intersection(target['kind'])
        for target in packages[name]['targets']))


def scenario_paths(plan, root):
    all_paths = sorted(str(p.relative_to(root)) for p in (root / 'ci/scenarios').glob('*.json'))
    paths = all_paths if plan['scenarios'] is None else plan['scenarios']
    if not paths or not set(paths) <= set(all_paths):
        raise ValueError('empty scenario inventory or missing selected scenario')
    return paths


def build_shards(plan, listing, directory, output):
    from balance import binary_partition, inventory, matcher, read_timings
    items = inventory(listing)
    if not items:
        raise ValueError('selected archive must contain runnable tests')
    timings = read_timings(ROOT)
    # Five-minute execution target, with four threads per worker.
    total = sum(timings.get(item, 1.0) for item in items)
    count = 8 if total > 4 * 4 * 300 else 4
    shards, loads = binary_partition(items, timings, count)
    print(f'Binary partitions: {count}; estimated seconds {loads}', flush=True)
    records = []
    for index, chosen in enumerate(shards, 1):
        folder = directory / str(index)
        folder.mkdir(parents=True, exist_ok=True)
        (folder / 'listing.json').write_text(json.dumps(listing) + '\n')
        # Empty workers still need a valid native archive for nextest's no-tests mode.
        binaries = sorted({item.split('\t', 1)[0] for item in chosen}) or [items[0].split('\t', 1)[0]]
        filters = ' | '.join(f'binary_id({matcher(binary)})' for binary in binaries)
        archive = folder / 'workspace-tests.tar.zst'
        command = ['cargo', 'nextest', 'archive', *cargo_args(plan), '-E', filters,
                   '--archive-file', str(archive)]
        print(json.dumps(command), flush=True)
        result = subprocess.run(command, cwd=ROOT)
        if result.returncode:
            return result.returncode
        records.append({'partition': index, 'binaries': len(binaries),
                        'tests': len(chosen), 'bytes': archive.stat().st_size})
    print(json.dumps({'archive_partitions': records, 'total_bytes': sum(r['bytes'] for r in records)}), flush=True)
    with output.open('a') as stream:
        stream.write(f'count={count}\npartitions={json.dumps(list(range(1, count + 1)))}\n')
    return 0


def run(args):
    plan = read_plan(args.selection)
    if args.command in ('test-build', 'test-build-shards'):
        # Shards may be empty, but the archive as a whole must contain real tests.
        # Listing compiles the same selection; archive reuses those build outputs.
        listing = ['cargo', 'nextest', 'list', *cargo_args(plan), '--message-format', 'json']
        print(json.dumps(listing), flush=True)
        result = subprocess.run(listing, cwd=ROOT, capture_output=True, text=True)
        if result.returncode:
            print(result.stderr, file=sys.stderr)
            return result.returncode
        live = json.loads(result.stdout)
        count = live['test-count']
        if type(count) is not int or count <= 0:
            raise ValueError('selected archive must contain at least one test')
        print(f'Selected archive contains {count} tests', flush=True)
        if args.command == 'test-build-shards':
            return build_shards(plan, live, args.directory, args.partition_output)
        command = ['cargo', 'nextest', 'archive', *cargo_args(plan), '--archive-file', args.archive]
        print(json.dumps(command), flush=True)
        return subprocess.run(command, cwd=ROOT).returncode
    if args.command == 'test-doc':
        command = ['cargo', 'metadata', '--format-version', '1', '--no-deps', '--locked']
        print(json.dumps(command), flush=True)
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
        if result.stderr:
            print(result.stderr, end='', file=sys.stderr)
        if result.returncode:
            return result.returncode
        eligible = doctest_packages(plan, json.loads(result.stdout))
        if not eligible:
            print('No selected library doctest targets; documentation tests are not applicable.', flush=True)
            return 0
        print('Selected library doctest packages: ' + json.dumps(eligible), flush=True)
        command = ['cargo', 'test', *cargo_args(plan, docs=True), '--doc', '--no-fail-fast']
        print(json.dumps(command), flush=True)
        return subprocess.run(command, cwd=ROOT).returncode
    paths = scenario_paths(plan, ROOT)
    if getattr(args, 'shard', None):
        from balance import partition
        index, count = map(int, args.shard.split('/'))
        if not 1 <= index <= count:
            raise ValueError('shard must be K/N with 1 <= K <= N')
        timings = json.loads((ROOT / 'ci/scenario-durations.json').read_text())['seconds']
        shards, loads = partition(paths, timings, count)
        paths = shards[index - 1]
        print(f'Scenario shard {args.shard}: estimated seconds {loads}', flush=True)
    print(f'Running {len(paths)} application scenarios: {json.dumps(paths)}', flush=True)
    failed = False
    durations = {}
    args.out.mkdir(parents=True, exist_ok=True)
    for path in paths:
        command = [sys.executable, str(ROOT / 'ci/run-scenarios.py'), path,
                   '--soliddraft', 'target/release/soliddraft',
                   '--catallax-verify', 'target/release/catallax-verify',
                   '--out', str(args.out / pathlib.PurePosixPath(path).stem)]
        started = time.monotonic()
        if subprocess.run(command, cwd=ROOT).returncode:
            failed = True
        durations[path] = round(time.monotonic() - started, 3)
        (args.out / 'scenario-times.json').write_text(json.dumps({'seconds': durations}, indent=2) + '\n')
    if not paths:
        (args.out / 'scenario-times.json').write_text('{"seconds": {}}\n')
    return int(failed)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('test-build', 'test-build-shards', 'test-doc', 'scenarios'):
        command = sub.add_parser(name)
        command.add_argument('--selection', type=pathlib.Path)
        if name == 'test-build':
            command.add_argument('archive')
        if name == 'test-build-shards':
            command.add_argument('directory', type=pathlib.Path)
            command.add_argument('--partition-output', required=True, type=pathlib.Path)
        if name == 'scenarios':
            command.add_argument('--out', type=pathlib.Path, required=True)
            command.add_argument('--shard', help='duration-balanced K/N partition of selected scenarios')
    return run(parser.parse_args())


if __name__ == '__main__':
    sys.exit(main())
