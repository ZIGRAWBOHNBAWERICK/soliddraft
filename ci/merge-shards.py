#!/usr/bin/env python3
"""Validate every execution partition before publishing four required shard reports."""
import argparse
import copy
import datetime
import json
import pathlib
import xml.etree.ElementTree as ET

from balance import inventory_digest


def merge(directory, group, partitions):
    index, groups = map(int, group.split('/'))
    if not 1 <= index <= groups or partitions < groups or partitions % groups:
        raise ValueError('invalid shard grouping')
    reports, expected, all_items = [], set(), []
    for partition in range(1, partitions + 1):
        folder = directory / f'workspace-partition-linux-{partition}'
        metadata = json.loads((folder / 'inventory.json').read_text())
        if (metadata['format'] != 'soliddraft-partition/1'
                or metadata['partition'] != partition or metadata['partitions'] != partitions
                or metadata['returncode'] != 0):
            raise ValueError(f'partition {partition} did not succeed')
        expected.add((metadata['inventory_count'], metadata['inventory_sha256']))
        report = folder / 'junit.xml'
        if not report.exists() and metadata['selected_count'] == 0:
            root = ET.Element('testsuites', tests='0', failures='0', errors='0', skipped='0',
                              time='0', timestamp=metadata['finished_at'])
        else:
            root = ET.parse(report).getroot()
        if int(root.get('failures', '0')) or int(root.get('errors', '0')):
            raise ValueError(f'partition {partition} has failed tests')
        cases = list(root.iter('testcase'))
        if any(not c.get('classname') or not c.get('name') for c in cases):
            raise ValueError('missing test identity')
        items = [c.get('classname') + '\t' + c.get('name') for c in cases]
        if (int(root.get('tests')) != len(items) or metadata['selected_count'] != len(items)
                or metadata['selected_sha256'] != inventory_digest(items)):
            raise ValueError(f'partition {partition} omitted or changed its selected tests')
        all_items.extend(items)
        reports.append(root)
    if (len(all_items) != len(set(all_items)) or len(expected) != 1
            or next(iter(expected)) != (len(all_items), inventory_digest(all_items))):
        raise ValueError('execution partitions do not cover the live inventory exactly once')
    chosen = reports[index - 1::groups]
    starts = [datetime.datetime.fromisoformat(r.get('timestamp').replace('Z', '+00:00')) for r in chosen]
    ends = [s + datetime.timedelta(seconds=float(r.get('time'))) for s, r in zip(starts, chosen)]
    merged = ET.Element('testsuites', name='nextest-run', timestamp=min(starts).isoformat(),
                        time=str((max(ends) - min(starts)).total_seconds()),
                        tests=str(sum(int(r.get('tests')) for r in chosen)), failures='0', errors='0',
                        skipped=str(sum(int(r.get('skipped', '0')) for r in chosen)))
    for root in chosen:
        for suite in root:
            merged.append(copy.deepcopy(suite))
    return merged


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('directory', type=pathlib.Path)
    parser.add_argument('group')
    parser.add_argument('partitions', type=int)
    parser.add_argument('output', type=pathlib.Path)
    args = parser.parse_args()
    root = merge(args.directory, args.group, args.partitions)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(args.output, encoding='utf-8', xml_declaration=True)
    print(f'Validated {args.partitions} partitions; shard {args.group}: {root.get("tests")} tests')


if __name__ == '__main__':
    main()
