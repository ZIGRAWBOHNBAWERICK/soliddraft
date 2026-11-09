#!/usr/bin/env python3
"""Balance the current inventory using recorded durations, never a frozen test list."""
import argparse
import datetime
import hashlib
import json
import math
import pathlib
import re
import statistics
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent


def partition(items, timings, count):
    if count < 1 or len(items) != len(set(items)):
        raise ValueError('invalid shard count or duplicate inventory')
    loads, shards = [0.0] * count, [[] for _ in range(count)]
    for item in sorted(items, key=lambda item: (-timings.get(item, 1.0), item)):
        duration = timings.get(item, 1.0)
        if not math.isfinite(duration) or duration < 0:
            raise ValueError('invalid duration')
        shard = min(range(count), key=lambda i: (loads[i], len(shards[i]), i))
        shards[shard].append(item)
        loads[shard] += duration
    return shards, loads


def binary_partition(items, timings, count):
    if len(items) != len(set(items)):
        raise ValueError('duplicate inventory')
    groups, weights = {}, {}
    for item in sorted(items):
        binary = item.split('\t', 1)[0]
        duration = timings.get(item, 1.0)
        if not math.isfinite(duration) or duration < 0:
            raise ValueError('invalid duration')
        groups.setdefault(binary, []).append(item)
        weights[binary] = weights.get(binary, 0) + duration
    binaries, loads = partition(list(groups), weights, count)
    return [[item for binary in shard for item in sorted(groups[binary])]
            for shard in binaries], loads


def matcher(value):
    escapes = {'\\': '\\\\', ')': '\\)', ',': '\\,', '\n': '\\n', '\r': '\\r', '\t': '\\t'}
    return '=' + ''.join(escapes.get(c, c) for c in value)


def expression(item):
    binary, name = item.split('\t', 1)
    return f'(binary_id({matcher(binary)}) & test({matcher(name)}))'


def filterset(items):
    groups = {}
    for item in items:
        binary, name = item.split('\t', 1)
        groups.setdefault(binary, []).append(f'test({matcher(name)})')
    return ' | '.join(f'(binary_id({matcher(binary)}) & (' + ' | '.join(tests) + '))'
                      for binary, tests in sorted(groups.items())) or 'none()'


def inventory(listing):
    return [binary + '\t' + name
            for binary, suite in listing['rust-suites'].items()
            for name, case in suite['testcases'].items()
            if not case['ignored'] and case['filter-match']['status'] == 'matches']


def inventory_digest(items):
    return hashlib.sha256(json.dumps(sorted(items)).encode()).hexdigest()


def timing_path():
    name = 'test-durations-macos.json' if sys.platform == 'darwin' else 'test-durations.json'
    return ROOT / 'ci' / name


def run_shard(archive, shard, metadata=None, full_inventory=None):
    index, count = map(int, shard.split('/'))
    if not 1 <= index <= count:
        raise ValueError('shard must be K/N with 1 <= K <= N')
    reuse = ['--archive-file', archive, '--workspace-remap', str(ROOT),
             '--extract-to', str(ROOT), '--extract-overwrite']
    result = subprocess.run(['cargo', 'nextest', 'list', *reuse, '--message-format', 'json'],
                            cwd=ROOT, capture_output=True, text=True, check=True)
    listing = json.loads(result.stdout)
    local_items = inventory(listing)
    items = inventory(json.loads(full_inventory.read_text())) if full_inventory else local_items
    timings = json.loads(timing_path().read_text())['seconds']
    assign = binary_partition if full_inventory else partition
    shards, loads = assign(items, timings, count)
    chosen = shards[index - 1]
    if not set(chosen) <= set(local_items):
        raise ValueError('archive is missing selected tests')
    print(f'Shard {shard}: {len(chosen)}/{len(items)} tests; estimated seconds {loads}', flush=True)
    # Only selected tests enter the filter. New and renamed tests have a one-second weight.
    filters = filterset(chosen)
    with tempfile.TemporaryDirectory(prefix='nextest-balanced-') as temporary:
        config = pathlib.Path(temporary) / 'nextest.toml'
        text = (ROOT / 'ci/nextest.toml').read_text()
        longest = max((timings.get(item, 1.0) for item in chosen), default=1.0) or 1.0
        for item in chosen:
            priority = max(1, min(100, math.ceil(100 * timings.get(item, 1.0) / longest)))
            text += ('\n[[profile.ci.overrides]]\nfilter = ' + json.dumps(expression(item))
                     + f'\npriority = {priority}\n')
        config.write_text(text)
        # List extracts once; reuse the extracted binaries without another decompression.
        if metadata is not None:
            (ROOT / 'target/nextest/ci/junit.xml').unlink(missing_ok=True)
        result = subprocess.run(['cargo', 'nextest', 'run', '--config-file', str(config),
                               '--profile', 'ci', '--cargo-metadata', str(ROOT / 'target/nextest/cargo-metadata.json'),
                               '--binaries-metadata', str(ROOT / 'target/nextest/binaries-metadata.json'),
                               '--target-dir-remap', str(ROOT / 'target'),
                               '--workspace-remap', str(ROOT), '--filter-expr', filters,
                               '--no-tests', 'pass', '--no-fail-fast'], cwd=ROOT)
        if metadata is not None:
            metadata.parent.mkdir(parents=True, exist_ok=True)
            metadata.write_text(json.dumps({
                'format': 'soliddraft-partition/1', 'partition': index, 'partitions': count,
                'inventory_count': len(items), 'inventory_sha256': inventory_digest(items),
                'selected_count': len(chosen), 'selected_sha256': inventory_digest(chosen),
                'returncode': result.returncode,
                'finished_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            }, indent=2) + '\n')
        return result.returncode


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    record = sub.add_parser('record')
    record.add_argument('--source', required=True)
    record.add_argument('--output', type=pathlib.Path, required=True)
    record.add_argument('junit', nargs='+', type=pathlib.Path)
    scenarios = sub.add_parser('record-scenarios')
    scenarios.add_argument('--source', required=True)
    scenarios.add_argument('--output', type=pathlib.Path, required=True)
    scenarios.add_argument('inputs', nargs='+', type=pathlib.Path)
    run = sub.add_parser('run')
    run.add_argument('archive')
    run.add_argument('shard')
    run.add_argument('--metadata', type=pathlib.Path)
    run.add_argument('--inventory', type=pathlib.Path)
    args = parser.parse_args()
    if args.command == 'run':
        return run_shard(args.archive, args.shard, args.metadata, args.inventory)
    timings = {}
    if args.command == 'record-scenarios':
        for path in args.inputs:
            if path.suffix == '.json':
                for key, duration in json.loads(path.read_text())['seconds'].items():
                    timings[key] = max(timings.get(key, 0), duration)
            else:
                previous = None
                for line in path.read_text().splitlines():
                    if '\tRun the selected application scenarios and feature-count latency trends\t' not in line:
                        continue
                    match = re.search(r'(\d{4}-\S+Z) (.*)', line)
                    if not match:
                        continue
                    stamp = datetime.datetime.fromisoformat(match[1].replace('Z', '+00:00'))
                    if previous is None:
                        previous = stamp
                    try:
                        report = json.loads(match[2])
                    except ValueError:
                        continue
                    if 'bundle' in report:
                        key = 'ci/scenarios/' + pathlib.Path(report['bundle']).name + '.json'
                        timings[key] = round((stamp - previous).total_seconds(), 3)
                        previous = stamp
        if not timings:
            raise ValueError('no scenario timings in log')
        args.output.write_text(json.dumps({'source': args.source, 'seconds': dict(sorted(timings.items()))}, indent=2) + '\n')
        return 0
    samples = {}
    for path in args.junit:
        for test in ET.parse(path).getroot().iter('testcase'):
            if test.find('skipped') is None:
                key = test.attrib['classname'] + '\t' + test.attrib['name']
                samples.setdefault(key, []).append(float(test.attrib['time']))
    timings = {key: statistics.median(values) for key, values in samples.items()}
    args.output.write_text(json.dumps({'source': args.source, 'seconds': dict(sorted(timings.items()))}, indent=2) + '\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
