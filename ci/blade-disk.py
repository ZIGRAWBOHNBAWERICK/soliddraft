#!/usr/bin/env python3
"""Measure a Blade job's workspace, temporary files and writable container layer."""
import http.client
import json
import os
import pathlib
import shutil
import signal
import socket
import subprocess
import sys
import time
import uuid

INTERVAL = 10


def layer_bytes():
    if not pathlib.Path('/.dockerenv').exists():
        return 0
    connection = http.client.HTTPConnection('localhost', timeout=10)
    connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.sock.settimeout(10)
    connection.sock.connect('/var/run/docker.sock')
    try:
        connection.request('GET', f'/containers/{socket.gethostname()}/json?size=true')
        response = connection.getresponse()
        if response.status != 200:
            raise RuntimeError(f'cannot measure own container: Docker HTTP {response.status}')
        return json.load(response)['SizeRw']
    finally:
        connection.close()


def disk_bytes(paths):
    seen, total = set(), 0
    for path in paths:
        info = path.stat()
        identity = (info.st_dev, info.st_ino)
        if identity in seen:
            continue
        seen.add(identity)
        result = subprocess.run(['du', '-sx', '--block-size=1', str(path)],
                                capture_output=True, text=True, timeout=30, env=dict(os.environ, LC_ALL='C'))
        if result.returncode and (result.returncode != 1 or not result.stderr
                                  or any('No such file or directory' not in line
                                         for line in result.stderr.splitlines())):
            raise RuntimeError(f'cannot measure {path}: {result.stderr}')
        total += int(result.stdout.split()[0])
    return total


def sample(state, workspace, temporary, canonical):
    layer = layer_bytes()
    bound = disk_bytes([workspace, temporary, canonical])
    state['peak_bytes'] = max(state.get('peak_bytes', 0), bound + layer)
    state['peak_bound_bytes'] = max(state.get('peak_bound_bytes', 0), bound)
    state['peak_writable_layer_bytes'] = max(state.get('peak_writable_layer_bytes', 0), layer)
    state['min_free_bytes'] = min(state.get('min_free_bytes', float('inf')),
                                  shutil.disk_usage(workspace).free)
    state['samples'] = state.get('samples', 0) + 1


def save(path, state):
    pending = path.with_suffix('.tmp')
    pending.write_text(json.dumps(state, indent=2) + '\n')
    pending.replace(path)


def monitor(report, token):
    state = {'job': os.environ['GITHUB_JOB'], 'sample_interval_seconds': INTERVAL,
             'container_layer_measured': pathlib.Path('/.dockerenv').exists()}
    roots = [pathlib.Path(os.environ[key]) for key in
             ('GITHUB_WORKSPACE', 'RUNNER_TEMP', 'LINUX_ARCHIVE_WORKSPACE')]
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    try:
        while True:
            sample(state, *roots)
            save(report, state)
            time.sleep(INTERVAL)
    except Exception as error:
        state['error'] = str(error)
    finally:
        try:
            sample(state, *roots)
        except Exception as error:
            state['error'] = str(error)
        state['stopped'] = True
        save(report, state)


def start(report, control):
    report.unlink(missing_ok=True)
    token = uuid.uuid4().hex
    with report.with_suffix('.log').open('w') as output:
        process = subprocess.Popen([sys.executable, str(pathlib.Path(__file__).resolve()),
                                    'monitor', str(report), token],
                                   stdin=subprocess.DEVNULL, stdout=output, stderr=output,
                                   start_new_session=True)
    control.write_text(json.dumps({'pid': process.pid, 'token': token}))
    for _ in range(300):
        if report.exists():
            state = json.loads(report.read_text())
            if state.get('error'):
                raise RuntimeError(f'Blade disk measurement failed: {state["error"]}')
            return
        if process.poll() is not None:
            raise RuntimeError('Blade disk monitor exited before its first sample')
        time.sleep(0.1)
    raise RuntimeError('Blade disk monitor did not produce its first sample')


def stop(report, control):
    if not control.exists():
        return
    owner = json.loads(control.read_text())
    command = pathlib.Path(f'/proc/{owner["pid"]}/cmdline')
    if command.exists() and owner['token'].encode() in command.read_bytes().split(b'\0'):
        os.kill(owner['pid'], signal.SIGTERM)
    for _ in range(400):
        if report.exists() and json.loads(report.read_text()).get('stopped'):
            break
        time.sleep(0.1)
    state = json.loads(report.read_text()) if report.exists() else {}
    if state.get('error') or not state.get('stopped'):
        raise RuntimeError(f'Blade disk measurement failed: {state}')
    if not state.get('reported'):
        peak = state['peak_bytes'] / 2**30
        layer = state['peak_writable_layer_bytes'] / 2**30
        free = state['min_free_bytes'] / 2**30
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as summary:
            summary.write(f'\n## Blade disk usage\n\nSampled peak: {peak:.2f} GiB '
                          f'({layer:.2f} GiB writable layer); minimum shared free space: '
                          f'{free:.2f} GiB. Samples every {INTERVAL} seconds, plus cleanup.\n')
        print(json.dumps(state))
        state['reported'] = True
        save(report, state)


def main():
    action = sys.argv[1]
    if action == 'monitor':
        monitor(pathlib.Path(sys.argv[2]), sys.argv[3])
        return
    temporary = pathlib.Path(os.environ['RUNNER_TEMP'])
    report, control = temporary / 'blade-disk.json', temporary / 'blade-disk-control.json'
    if action == 'start':
        start(report, control)
    elif action == 'stop':
        stop(report, control)
    else:
        raise ValueError(f'unknown disk action: {action}')


if __name__ == '__main__':
    main()
