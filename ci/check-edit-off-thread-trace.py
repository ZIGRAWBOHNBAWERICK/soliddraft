#!/usr/bin/env python3
"""Check the renderer main-thread budget for the bracket edit after Cancel."""
import gzip
import json
import math
import sys

EDIT = 'Changing the base depth to 16 mm after the cancel'
# The hosted flake used 193 ms of CPU in a 379 ms frame (1.07× its 355 ms budget).
# Keep the CPU budget and give elapsed time 15% headroom for scheduling and GPU waits.
WALL_HEADROOM = 1.15


def check_trace(path):
    with gzip.open(path, 'rt', encoding='utf-8') as trace:
        events = json.load(trace)['traceEvents']

    def mark(edge):
        name = f'{EDIT}: {edge}'
        (event,) = (event for event in events if event.get('cat') == 'blink.user_timing'
                    and event.get('ph') == 'I' and event['name'] == name)
        return event

    start, end = mark('start'), mark('end')
    thread = start['pid'], start['tid']
    if thread != (end['pid'], end['tid']) or start['ts'] >= end['ts']:
        raise ValueError('invalid edit window')
    bound = float(start['args']['data']['detail'])
    if not math.isfinite(bound) or bound <= 0:
        raise ValueError('invalid calibrated budget')

    # Marks come from the page, so only its renderer thread is measured. Work on
    # the kernel worker is expected; a synchronous wait for it counts as page wall time.
    longest_cpu = longest_wall = 0
    for event in events:
        if (event.get('name') == 'RunTask' and event.get('ph') == 'X'
                and (event['pid'], event['tid']) == thread
                and start['ts'] <= event['ts'] < end['ts']):
            wall = event['dur']
            longest_wall = max(longest_wall, wall)
            # Missing thread time must not let a slow task evade the check.
            longest_cpu = max(longest_cpu, event.get('tdur', wall))
    if not longest_wall:
        raise ValueError('no renderer tasks in the edit window')

    longest_cpu /= 1000
    longest_wall /= 1000
    wall_bound = bound * WALL_HEADROOM
    print(f'{EDIT}: longest page task {longest_cpu:.1f} ms CPU (bound {bound:.1f}), '
          f'{longest_wall:.1f} ms elapsed (bound {wall_bound:.1f})')
    if longest_cpu >= bound or longest_wall >= wall_bound:
        sys.exit(f'Regression: {EDIT} blocked the page for {longest_wall:.1f} ms elapsed '
                 f'(bound {wall_bound:.1f} ms) and used {longest_cpu:.1f} ms of '
                 f'page-thread CPU time (bound {bound:.1f} ms)')


if __name__ == '__main__':
    try:
        check_trace(sys.argv[1])
    except (OSError, EOFError, KeyError, TypeError, ValueError) as error:
        sys.exit(f'Cannot check the edit after Cancel: {error}')
