#!/usr/bin/env python3
"""Install a pinned tool once, then verify and reuse its cached executable."""
import hashlib
import http.client
import io
import json
import os
import pathlib
import platform
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.request


def specification(root, name):
    tool = json.loads((root / 'ci/tools').read_text())[name]
    system = platform.system()
    tool.setdefault('binary', name + ('.exe' if system == 'Windows' else ''))
    key = f'{system}-{platform.machine()}'
    asset = tool.get('platforms', {}).get(key, tool.get('platforms', {}).get(system))
    if asset is None and tool.get('source') != 'cargo':
        sys.exit(f'No pinned {name} binary for {key}')
    return tool, asset or {}, key


def identity(tool, asset):
    return {'version': tool['version'], 'archive': asset.get('sha256', 'cargo-locked')}


def installed(prefix, name, tool, asset):
    binary = prefix / 'bin' / tool.get('binary', name)
    marker = prefix / f'.{name}.json'
    try:
        recorded = json.loads(marker.read_text())
        return (all(recorded.get(k) == v for k, v in identity(tool, asset).items())
                and os.access(binary, os.X_OK)
                and hashlib.sha256(binary.read_bytes()).hexdigest() == recorded['binary_sha256'])
    except (OSError, ValueError, KeyError):
        return False


def download(url, name):
    attempts = 5
    for attempt in range(1, attempts + 1):
        try:
            request = urllib.request.Request(url, headers={
                'Accept': 'application/octet-stream', 'User-Agent': 'soliddraft-ci-tools'})
            with urllib.request.urlopen(request, timeout=120) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code < 500 and error.code != 429:
                sys.exit(f'{name} cache miss: download failed (HTTP {error.code})')
            reason = f'HTTP {error.code}'
        except (OSError, http.client.HTTPException) as error:
            reason = type(error).__name__
        if attempt == attempts:
            sys.exit(f'{name} cache miss: download failed after {attempts} attempts ({reason})')
        delay = 5 * 2 ** (attempt - 1)
        print(f'{name} download attempt {attempt} failed ({reason}); retrying in {delay} s', file=sys.stderr)
        time.sleep(delay)


def install(root, prefix, name):
    tool, asset, _ = specification(root, name)
    if installed(prefix, name, tool, asset):
        print(f'Reusing verified {name} {tool["version"]} in {prefix / "bin"}')
        return
    executable = tool.get('binary', name)
    if tool.get('source') == 'cargo':
        # This tool publishes no release binaries. Cache its locked source build.
        command = ['cargo', 'install', name, '--version', tool['version'], '--locked',
                   '--root', str(prefix), '--force']
        for attempt in range(3):
            if subprocess.run(command).returncode == 0:
                break
            if attempt == 2:
                sys.exit(f'{name} cache miss: pinned source installation failed after 3 attempts')
            time.sleep(5 * 2 ** attempt)
    else:
        target = asset['target']
        filename = f'cargo-nextest-{tool["version"]}-{target}.tar.gz'
        url = asset.get('url', f'https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-{tool["version"]}/{filename}')
        payload = download(url, f'{name} {tool["version"]}')
        if hashlib.sha256(payload).hexdigest() != asset['sha256']:
            sys.exit(f'{name} checksum mismatch')
        with tarfile.open(fileobj=io.BytesIO(payload), mode='r:gz') as archive:
            members = [m for m in archive.getmembers()
                       if m.isfile() and pathlib.PurePosixPath(m.name).name == executable]
            if len(members) != 1:
                sys.exit(f'{name} archive must contain exactly one executable')
            binary = archive.extractfile(members[0]).read()
        bindir = prefix / 'bin'
        bindir.mkdir(parents=True, exist_ok=True)
        temporary = bindir / f'.{executable}.tmp'
        temporary.write_bytes(binary)
        temporary.chmod(0o755)
        temporary.replace(bindir / executable)
    marker = identity(tool, asset)
    marker['binary_sha256'] = hashlib.sha256((prefix / 'bin' / executable).read_bytes()).hexdigest()
    temporary = prefix / f'.{name}.json.tmp'
    temporary.write_text(json.dumps(marker) + '\n')
    temporary.replace(prefix / f'.{name}.json')
    print(f'Installed {name} {tool["version"]} in {prefix / "bin"}')


def main():
    root, destination = pathlib.Path(sys.argv[1]), sys.argv[2]
    name = sys.argv[3] if len(sys.argv) > 3 else 'cargo-nextest'
    if destination == '--info':
        tool, asset, host = specification(root, name)
        prefix = pathlib.Path(os.environ['RUNNER_TOOL_CACHE']) / 'soliddraft-tools' / name / tool['version'] / host
        values = {'prefix': str(prefix), 'cache-key': f'soliddraft-tool-v1-{host}-{name}-{tool["version"]}-{identity(tool, asset)["archive"]}',
                  'installed': str(installed(prefix, name, tool, asset)).lower()}
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            for key, value in values.items():
                output.write(f'{key}={value}\n')
    else:
        install(root, pathlib.Path(destination), name)


if __name__ == '__main__':
    main()
