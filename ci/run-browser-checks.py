#!/usr/bin/env python3
"""Run the independent Chromium checks with the standard demo browser setup."""
import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess

from registries import fragments

ROOT = pathlib.Path(__file__).resolve().parents[1]


def checks(root):
    result = []
    scripts = set()
    for path, text in fragments(root, 'ci/browser-checks', '.json'):
        case = json.loads(text)
        script = case['script']
        if (not re.fullmatch(r'ci/browser-[a-z0-9-]+\.js', script) or script in scripts
                or not (root / script).is_file()):
            raise ValueError(f'{path}: missing, duplicate or invalid browser script {script!r}')
        if type(case['port']) is not int or not 1024 <= case['port'] <= 65535:
            raise ValueError(f'{path}: invalid HTTP port')
        scripts.add(script)
        result.append(case)
    if not result:
        raise ValueError('ci/browser-checks: no Chromium checks')
    return result


def run(case, root, temp):
    name = pathlib.Path(case['script']).stem.removeprefix('browser-')
    tools = temp / 'browser-tools/node_modules'
    env = dict(os.environ, CHROME_DEVTOOLS_AXI_SESSION=name,
               PATH=f'{tools / ".bin"}:{os.environ["PATH"]}',
               CHROME_DEVTOOLS_AXI_MCP_PATH=str(tools / 'chrome-devtools-mcp/build/src/bin/chrome-devtools-mcp.js'),
               CHROME_DEVTOOLS_AXI_EXECUTABLE_PATH='/usr/bin/google-chrome',
               CHROME_DEVTOOLS_AXI_CHROME_ARGS=(
                   '--no-sandbox --enable-unsafe-webgpu --enable-unsafe-swiftshader --use-angle=swiftshader'))
    if case.get('swiftshader_webgpu'):
        env['CHROME_DEVTOOLS_AXI_CHROME_ARGS'] += ' --use-webgpu-adapter=swiftshader'
    if 'fixture' in case:
        fixture = case['fixture']
        shutil.copyfile(root / fixture['source'], root / 'target/web-dist' / fixture['destination'])
    print(f'::group::{case["name"]}', flush=True)

    def chrome(*args, check=True, **kwargs):
        return subprocess.run(['chrome-devtools-axi', *args], cwd=root, env=env, check=check, **kwargs)

    try:
        with (temp / f'{name}-server.log').open('w') as log:
            server = subprocess.Popen(['python3', '-m', 'http.server', str(case['port']), '--bind', '127.0.0.1',
                                       '--directory', 'target/web-dist'], cwd=root, stdout=log, stderr=log)
            try:
                if chrome('open', 'about:blank', check=False).returncode:
                    chrome('stop', check=False)
                    chrome('open', 'about:blank')
                chrome('emulate', '--viewport', '1400x813x1')
                path = case.get('path', '') + case.get('query', '')
                chrome('open', f'http://127.0.0.1:{case["port"]}/{path}')
                with (root / case['script']).open() as script:
                    chrome('run', stdin=script)
            finally:
                try:
                    chrome('stop', check=False)
                finally:
                    server.terminate()
                    server.wait()
    finally:
        print('::endgroup::', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--list', action='store_true', help='print the configured checks without launching Chromium')
    args = parser.parse_args()
    cases = checks(ROOT)
    if args.list:
        print(json.dumps(cases))
    else:
        for case in cases:
            run(case, ROOT, pathlib.Path(os.environ['RUNNER_TEMP']))


if __name__ == '__main__':
    main()
