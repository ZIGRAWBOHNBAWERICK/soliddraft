#!/usr/bin/env python3
"""Balance the current inventory using recorded durations, never a frozen test list."""
import argparse
import datetime
import json
import math
import pathlib
import re
import subprocess
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


def run_shard(archive, shard):
    index, count = map(int, shard.split('/'))
    if not 1 <= index <= count:
        raise ValueError('shard must be K/N with 1 <= K <= N')
    reuse = ['--archive-file', archive, '--workspace-remap', str(ROOT),
             '--extract-to', str(ROOT), '--extract-overwrite']
    result = subprocess.run(['cargo', 'nextest', 'list', *reuse, '--message-format', 'json'],
                            cwd=ROOT, capture_output=True, text=True, check=True)
    listing = json.loads(result.stdout)
    items = inventory(listing)
    timings = json.loads((ROOT / 'ci/test-durations.json').read_text())['seconds']
    shards, loads = partition(items, timings, count)
    chosen = shards[index - 1]
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
        return subprocess.run(['cargo', 'nextest', 'run', '--config-file', str(config),
                               '--profile', 'ci', '--cargo-metadata', str(ROOT / 'target/nextest/cargo-metadata.json'),
                               '--binaries-metadata', str(ROOT / 'target/nextest/binaries-metadata.json'),
                               '--target-dir-remap', str(ROOT / 'target'),
                               '--workspace-remap', str(ROOT), '--filter-expr', filters,
                               '--no-tests', 'pass', '--no-fail-fast'], cwd=ROOT).returncode


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
    args = parser.parse_args()
    if args.command == 'run':
        return run_shard(args.archive, args.shard)
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
    for path in args.junit:
        for test in ET.parse(path).getroot().iter('testcase'):
            if test.find('skipped') is None:
                key = test.attrib['classname'] + '\t' + test.attrib['name']
                timings[key] = max(timings.get(key, 0), float(test.attrib['time']))
    args.output.write_text(json.dumps({'source': args.source, 'seconds': dict(sorted(timings.items()))}, indent=2) + '\n')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
