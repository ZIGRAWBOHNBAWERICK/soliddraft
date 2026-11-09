#!/usr/bin/env python3
"""Reduce repeated test-binary data without changing the archive payload."""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

HASHER = ('import hashlib, sys\n'
          'print(hashlib.file_digest(sys.stdin.buffer, "sha256").hexdigest())\n')


def pipe(left, right, output, deadline):
    children = []
    try:
        reader = subprocess.Popen(left, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        children.append(reader)
        writer = subprocess.Popen(right, stdin=reader.stdout, stdout=output, stderr=subprocess.DEVNULL)
        children.append(writer)
        reader.stdout.close()
        result, _ = writer.communicate(timeout=max(0.001, deadline - time.monotonic()))
        reader.wait(timeout=max(0.001, deadline - time.monotonic()))
        if reader.returncode or writer.returncode:
            raise subprocess.SubprocessError('codec process failed')
        return result
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


def compact(archive, *, threads=0, timeout=120, minimum=64 * 1024 * 1024):
    archive = pathlib.Path(archive)
    started = time.monotonic()
    size = archive.stat().st_size
    result = {'before_bytes': size, 'after_bytes': size, 'changed': False}
    temporary = None
    try:
        codec = shutil.which('zstd')
        if size < minimum:
            result['reason'] = 'small archive'
        elif codec is None:
            result['reason'] = 'zstd unavailable'
        else:
            deadline = started + timeout
            with tempfile.NamedTemporaryFile(dir=archive.parent, prefix=archive.name + '.', delete=False) as output:
                temporary = pathlib.Path(output.name)
                pipe([codec, '-q', '-d', '-c', str(archive)],
                     [codec, '-q', '-3', f'-T{threads}', '--long=27', '-c'], output, deadline)
            result['candidate_bytes'] = temporary.stat().st_size
            if temporary.stat().st_size >= size:
                result['reason'] = 'not smaller'
            else:
                digests = [pipe([codec, '-q', '-d', '-c', str(path)],
                                [sys.executable, '-c', HASHER], subprocess.PIPE, deadline).strip()
                           for path in (archive, temporary)]
                if digests[0] != digests[1]:
                    raise subprocess.SubprocessError('payload digest mismatch')
                os.replace(temporary, archive)
                result.update(after_bytes=archive.stat().st_size, changed=True,
                              payload_sha256=digests[0].decode(), reason='smaller verified archive')
    except subprocess.TimeoutExpired:
        result['reason'] = 'timeout; kept original'
    except (OSError, subprocess.SubprocessError):
        result['reason'] = 'compaction failed; kept original'
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    result['seconds'] = round(time.monotonic() - started, 3)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('archive', type=pathlib.Path)
    parser.add_argument('--partition-output', type=pathlib.Path)
    args = parser.parse_args()
    result = compact(args.archive)
    if args.partition_output is not None:
        count = execution_partitions(result)
        with args.partition_output.open('a') as output:
            output.write(f'count={count}\npartitions={json.dumps(list(range(1, count + 1)))}\n')
    print(json.dumps(result), flush=True)


def execution_partitions(report):
    before, after = report['before_bytes'], max(1, report['after_bytes'])
    if 3 * after <= before:
        return 12
    if 3 * after <= 2 * before:
        return 8
    return 4


if __name__ == '__main__':
    main()
