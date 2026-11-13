#!/usr/bin/env python3
"""Select a healthy runner without sending outside contributions to private machines."""
import json
import os
import time


def fresh(value, now):
    try:
        stamp = int(value)
    except (TypeError, ValueError):
        return False
    return 0 <= now - stamp < 15 * 60


def runners(event, actor, starter, repository, boat_enabled, boat_at, bohn_at, now, blade_at=None):
    owner = 'BohnBawerick'
    pr = event.get('pull_request')
    trusted = (repository == 'ZIGRAWBOHNBAWERICK/soliddraft'
               and actor == owner and starter == owner
               and (pr is None or (pr.get('user', {}).get('login') == owner
                    and (pr.get('head', {}).get('repo') or {}).get('full_name') == repository)))
    boat = trusted and boat_enabled == 'true' and fresh(boat_at, now)
    bohn = trusted and fresh(bohn_at, now)
    blade = trusted and fresh(blade_at, now)
    blade_runner = ['self-hosted', 'blade-sd'] if blade else ['ubuntu-latest']
    # Blade uses Ubuntu job containers. NixOS remains limited to plain Cargo jobs.
    return {'blade': blade_runner,
            'archive': ['boat-large'] if boat else blade_runner,
            'plain': ['boat-large'] if boat else ['self-hosted', 'bohn-03'] if bohn else ['ubuntu-latest'],
            'application': ['boat-large'] if boat else blade_runner,
            'plain_jobs': '1' if bohn and not boat else '4'}


def main():
    with open(os.environ['GITHUB_EVENT_PATH']) as source:
        event = json.load(source)
    result = runners(event, os.environ['GITHUB_ACTOR'], os.environ['GITHUB_TRIGGERING_ACTOR'],
                     os.environ['GITHUB_REPOSITORY'], os.environ.get('USE_BOAT'),
                     os.environ.get('RUNNER_BOAT_OK_AT'), os.environ.get('RUNNER_BOHN03_OK_AT'),
                     int(time.time()), os.environ.get('RUNNER_BLADE_OK_AT'))
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        for key, value in result.items():
            output.write(f'{key}={json.dumps(value) if isinstance(value, list) else value}\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
