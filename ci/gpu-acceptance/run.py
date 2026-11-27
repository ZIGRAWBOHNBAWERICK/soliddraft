#!/usr/bin/env python3
"""Run one hardware viewport gate and retain the selected adapter and P1 frame."""

import argparse
import base64
from contextlib import contextmanager
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import socket
import subprocess
import threading
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
import uuid

HERE = Path(__file__).resolve().parent
SCENARIO = HERE.parent / 'scenarios' / 'canonical-p1.json'
SOFTWARE = re.compile(r'swiftshader|llvmpipe|lavapipe|softpipe|software|warp\b|microsoft basic render', re.I)
SELECTED = re.compile(r'(?:wgpu adapter:|Software rasterizer detected[^.]*\.) (backend: .+)$', re.M)
REFERENCE = HERE / 'p1-reference.pgm'


def picture(empty, part):
    """Extract the filled model against the empty viewport, omitting window chrome."""
    from PIL import Image, ImageChops, ImageFilter, ImageOps

    with Image.open(empty) as before, Image.open(part) as after:
        if before.size != after.size:
            raise ValueError(f'Viewport resized between captures: {before.size} != {after.size}')
        width, height = after.size
        crop = (int(width * .24), int(height * .27), int(width * .94), int(height * .86))
        rendered = after.crop(crop).convert('RGB')
        delta = ImageChops.difference(before.crop(crop).convert('RGB'), rendered)
    red, green, blue = delta.split()
    mask = ImageChops.lighter(ImageChops.lighter(red, green), blue).point(lambda n: 255 if n >= 30 else 0)
    # The default light P1 material is blue-grey; construction planes are warm and
    # can move between the two fits. Do not mistake them for the body.
    red, _, blue = rendered.split()
    material = ImageChops.subtract(blue, red).point(lambda n: 255 if n >= 8 else 0)
    mask = ImageChops.darker(mask, material)
    # Ignore one-pixel differences from antialiasing, the grid and overlays.
    normalized = (420, round(mask.height * 420 / mask.width))
    mask = mask.resize(normalized).point(lambda n: 255 if n >= 128 else 0)
    mask = mask.filter(ImageFilter.MinFilter(5)).filter(ImageFilter.MaxFilter(5))
    bounds = mask.getbbox()
    if not bounds or mask.histogram()[255] < 1000:
        raise ValueError('No filled part in the viewport crop')
    if bounds[0] == 0 or bounds[1] == 0 or bounds[2] == mask.width or bounds[3] == mask.height:
        raise ValueError(f'Foreground touches the viewport crop boundary: {bounds}')
    return ImageOps.pad(mask.crop(bounds), (64, 64), method=Image.Resampling.NEAREST, color=0)


def compare_pictures(out):
    from PIL import Image, ImageChops

    actual = picture(out / 'empty.png', out / 'p1.png')
    actual.save(out / 'p1-mask.png')
    with Image.open(REFERENCE) as image:
        expected = image.convert('L')
    intersection = ImageChops.darker(actual, expected).histogram()[255]
    union = ImageChops.lighter(actual, expected).histogram()[255]
    overlap = intersection / union
    if overlap < .84:
        raise ValueError(f'P1 silhouette overlap {overlap:.3f} is below 0.84')
    return {'reference': REFERENCE.name, 'silhouette_iou': round(overlap, 4)}


def require_hardware(adapter, backends):
    if not adapter or adapter.get('backend') not in backends:
        raise ValueError(f'Expected {sorted(backends)}, got {adapter}')
    identity = ' '.join(adapter.get(key) or '' for key in ('name', 'vendor', 'architecture', 'device', 'description'))
    if SOFTWARE.search(identity) or adapter.get('fallback') is True or adapter.get('device_type') == 'Cpu':
        raise ValueError(f'Software or fallback adapter: {adapter}')
    if not identity.strip():
        raise ValueError(f'Selected adapter has no identity: {adapter}')
    if adapter['backend'] != 'WebGPU' and adapter.get('device_type') not in {'IntegratedGpu', 'DiscreteGpu'}:
        raise ValueError(f'Adapter is not a physical GPU: {adapter}')
    # Some Safari versions do not expose either spelling of the fallback bit. Require
    # positive hardware identity there rather than treating a missing bit as false.
    if adapter['backend'] == 'WebGPU' and adapter.get('fallback') is not False:
        if adapter.get('fallback') is not None or not re.fullmatch(r'apple(?:[- ][\w ]+)?', adapter.get('vendor', ''), re.I):
            raise ValueError(f'Cannot establish a non-fallback adapter: {adapter}')


def selected_native(log):
    matches = SELECTED.findall(log.read_text(errors='replace'))
    if len(set(matches)) != 1:
        raise ValueError(f'Expected a single selected wgpu adapter in {log}, got {matches}')
    summary = matches[0]
    fields = re.match(r'backend: (\w+), device_type: (\w+)(?:, name: ("(?:[^"\\]|\\.)*"))?', summary)
    if not fields:
        raise ValueError(f'Unrecognized wgpu adapter record: {summary}')
    backend, device_type, name = fields.groups()
    return {'backend': backend, 'device_type': device_type, 'name': name or '', 'summary': summary}


def save_record(out, record):
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n', encoding='utf-8')


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def request(url, payload=None, method=None, timeout=120):
    data = json.dumps(payload).encode() if payload is not None else None
    with urlopen(Request(url, data, {'Content-Type': 'application/json'}, method=method), timeout=timeout) as response:
        return json.load(response) if response.status != 202 else None


def wait_for(action, description, seconds=45):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        result = action()
        if result:
            return result
        time.sleep(0.3)
    raise RuntimeError(f'Timed out waiting for {description}')


@contextmanager
def child(command, logfile, env=None):
    with logfile.open('wb') as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        try:
            yield process
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


def native_env(out):
    # No adapter/backend override: this is the same selection the app ships.
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith('WGPU_')}
    config = out / 'config'
    config.mkdir(exist_ok=True)
    env.update(SOLIDDRAFT_CONFIG_DIR=str(config), RUST_LOG='egui_wgpu=debug,soliddraft_app::app::workspace=info')
    return env


def build_fixture(app, out, env):
    port = free_port()
    url = f'http://127.0.0.1:{port}/mcp'
    seq = 0

    def rpc(method, params=None, timeout=120):
        nonlocal seq
        seq += 1
        reply = request(url, {'jsonrpc': '2.0', 'id': seq, 'method': method, 'params': params or {}}, timeout=timeout)
        if reply.get('id') != seq or 'error' in reply:
            raise RuntimeError(f'{method}: {reply}')
        return reply['result']

    def tool(name, arguments):
        result = rpc('tools/call', {'name': name, 'arguments': arguments})
        if result.get('isError'):
            raise RuntimeError(f'{name}: {result}')

    scenario = json.loads(SCENARIO.read_text(encoding='utf-8'))
    log = out / 'fixture.log'
    with child([app, '--mcp', str(port), '--size', '1200x780', '--no-assistant'], log, env) as process:
        def initialize():
            if process.poll() is not None:
                raise RuntimeError(f'SolidDraft exited before fixture replay; see {log}')
            try:
                return rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                          'clientInfo': {'name': 'gpu-acceptance', 'version': '1'}}, timeout=3)
            except URLError:
                return None

        wait_for(initialize, 'desktop MCP')
        request(url, {'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        tool('save', {'path': str(out / 'empty.dwprt')})
        for step in scenario['steps']:
            tool(step['tool'], step['arguments'])
            if step.get('label') == 'original':
                break
        else:
            raise ValueError('Canonical P1 scenario has no original checkpoint')
        tool('save', {'path': str(out / 'p1.dwprt')})


def native(args, out, record):
    system = platform.system()
    backends = {'Darwin': {'Metal'}, 'Windows': {'Dx12', 'Vulkan'}}.get(system)
    if backends is None:
        raise ValueError('Native GPU acceptance runs only on macOS and native Windows')
    env = native_env(out)
    build_fixture(args.app.resolve(), out, env)
    if system == 'Darwin':
        env['SOLIDDRAFT_GPU_ACCEPTANCE_OUT'] = str(out)
        with (out / 'metal.log').open('wb') as log:
            subprocess.run([args.metal_harness.resolve(), '--ignored', '--exact', 'capture_metal_p1'],
                           env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=180)
        record['capture'] = 'offscreen'
        record['renderer'] = json.loads((out / 'adapter.json').read_text(encoding='utf-8'))
        require_hardware(record['renderer'], backends)
        return
    record['capture'] = 'window'
    for name in ('empty', 'p1'):
        logfile = out / f'{name}.log'
        png = out / f'{name}.png'
        with logfile.open('wb') as log:
            subprocess.run([args.app.resolve(), str(out / f'{name}.dwprt'), '--no-assistant', '--size', '1200x780',
                            '--preset', 'fast', '--theme', 'light', '--screenshot', str(png)],
                           env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=45)
        adapter = selected_native(logfile)
        record[name] = adapter
        require_hardware(adapter, backends)
        if not png.is_file():
            raise RuntimeError(f'SolidDraft did not write {png}; see {logfile}')
    if record['empty'] != record['p1']:
        raise ValueError('The empty and P1 frames did not run on the same adapter')


class GateServer(SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        local = self.path.partition('?')[0]
        payload = {
            '/.gpu-acceptance/probe.js': (HERE / 'probe.js', 'text/javascript'),
            '/.gpu-acceptance/empty.dwprt': (self.server.fixture / 'empty.dwprt', 'application/octet-stream'),
            '/.gpu-acceptance/p1.dwprt': (self.server.fixture / 'p1.dwprt', 'application/octet-stream'),
        }.get(local)
        if local in ('/', '/index.html'):
            source = (Path(self.directory) / 'index.html').read_bytes()
            body = source.replace(b'<head>', b'<head>\n<script src="/.gpu-acceptance/probe.js"></script>', 1)
            if body == source:
                self.send_error(500, 'Shipping index has no insertion point for the acceptance probe')
                return
            kind = 'text/html; charset=utf-8'
        elif payload:
            path, kind = payload
            body = path.read_bytes()
        else:
            return super().do_GET()
        self.send_response(200)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@contextmanager
def app_server(dist, fixture):
    for name in ('empty', 'p1'):
        if not (fixture / f'{name}.dwprt').is_file():
            raise ValueError(f'Run the native gate first to create {fixture / (name + ".dwprt")}')
    with ThreadingHTTPServer(('127.0.0.1', 0), partial(GateServer, directory=dist)) as server:
        server.fixture = fixture
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield f'http://127.0.0.1:{server.server_port}/?preset=fast&theme=light'
        finally:
            server.shutdown()
            worker.join()


@contextmanager
def safari(out, url, record):
    port = free_port()
    origin = f'http://127.0.0.1:{port}'

    def driver(path, payload=None, method=None):
        try:
            return request(origin + path, payload, method, timeout=35)['value']
        except HTTPError as error:
            raise RuntimeError(json.load(error)['value']['message']) from error

    def ready():
        try:
            return driver('/status')
        except URLError:
            return None

    with child(['/usr/bin/safaridriver', '-p', str(port)], out / 'driver.log'):
        wait_for(ready, 'SafariDriver')
        session = driver('/session', {'capabilities': {'alwaysMatch': {'browserName': 'safari'}}})
        path = f"/session/{session['sessionId']}"
        try:
            version = session['capabilities']['browserVersion']
            record['browser'] = f'Safari {version}'
            if int(version.split('.')[0]) < 26:
                raise ValueError(f'Safari 26 or later required, got {version}')
            driver(path + '/window/rect', {'width': 1200, 'height': 870})
            driver(path + '/url', {'url': url})

            def evaluate(script):
                return driver(path + '/execute/async', {'args': [], 'script':
                              'const done = arguments[0]; Promise.resolve().then(() => (' + script
                              + ')).then(done, error => done({error: String(error)}));'})

            def screenshot(file):
                file.write_bytes(base64.b64decode(driver(path + '/screenshot')))

            yield evaluate, screenshot
        finally:
            driver(path, method='DELETE')


@contextmanager
def chrome(out, url, record, force):
    env = os.environ.copy()
    for key in ('AUTO_CONNECT', 'BROWSER_URL', 'CHANNEL', 'CHROME_ARGS', 'EXECUTABLE_PATH', 'USER_DATA_DIR'):
        env.pop('CHROME_DEVTOOLS_AXI_' + key, None)
    env.update(CHROME_DEVTOOLS_AXI_SESSION='gpu-acceptance-' + uuid.uuid4().hex[:12],
               CHROME_DEVTOOLS_AXI_HEADED='1',
               CHROME_DEVTOOLS_AXI_CHROME_ARGS='--window-size=1200,870' +
               (' --enable-unsafe-swiftshader --use-angle=swiftshader --enable-unsafe-webgpu' if force else ''))

    def axi(*args):
        # npm exposes a .cmd shim on native Windows.
        executable = 'chrome-devtools-axi.cmd' if os.name == 'nt' else 'chrome-devtools-axi'
        return subprocess.check_output([executable, *args], text=True, env=env, timeout=45)

    def evaluate(script):
        output = axi('eval', f'''async () => {{
            const bytes = new TextEncoder().encode(JSON.stringify(await ({script})));
            return 'GPU_ACCEPTANCE_VALUE:' + btoa(String.fromCharCode(...bytes));
        }}''')
        value = re.search(r'GPU_ACCEPTANCE_VALUE:([A-Za-z0-9+/=]+)', output)
        if not value:
            raise RuntimeError(f'Chrome did not return the adapter probe: {output}')
        return json.loads(base64.b64decode(value[1]))

    try:
        axi('open', url)
        record['browser'] = evaluate('navigator.userAgent')
        if not re.search(r'\bChrome/\d+', record['browser']):
            raise ValueError(f"Expected Chrome, got {record['browser']}")
        yield evaluate, lambda file: axi('screenshot', str(file))
    finally:
        axi('stop')


def web(args, out, record):
    expected = 'Darwin' if args.gate == 'safari' or args.force_software else 'Windows'
    if platform.system() != expected:
        raise ValueError(f'{args.gate} gate requires {expected}')
    with app_server(args.dist, args.fixture) as url:
        if args.force_software:
            url += '&force-software'
        browser = safari(out, url, record) if args.gate == 'safari' else chrome(out, url, record, args.force_software)
        with browser as (evaluate, screenshot):
            snapshot = '''({
              renderer: window.gpuAcceptance?.renderer,
              requests: window.gpuAcceptance?.requests,
              document: window.gpuAcceptance?.document,
              ready: document.querySelector('#status')?.classList.contains('hidden'),
              status: document.querySelector('#status')?.textContent
            })'''

            def state():
                record.update(evaluate(snapshot))
                if args.force_software and any(a and (a.get('fallback') or SOFTWARE.search(a.get('architecture', '')))
                                               for a in record.get('requests', [])):
                    raise ValueError(f'Forced software adapter rejected: {record["requests"]}')
                if record.get('renderer'):
                    require_hardware(record['renderer'], {'WebGPU'})
                return record.get('renderer') and record.get('ready')

            def drop(part):
                result = evaluate(f'window.gpuAcceptance.drop({json.dumps(part)}).then(() => true)')
                if result is not True:
                    raise RuntimeError(f'Dropping {part}: {result}')

            wait_for(state, 'the shipping Wasm viewport')
            drop('empty')
            wait_for(lambda: state() and record['document'] == {'type': 'document', 'extension': '.dwprt', 'count': 0},
                     'the empty part to open')
            evaluate('window.gpuAcceptance.painted()')
            screenshot(out / 'empty.png')
            drop('p1')
            wait_for(lambda: state() and (record.get('document') or {}).get('count', 0) > 0,
                     'P1 features to open', seconds=120)
            evaluate('window.gpuAcceptance.painted()')
            last_error = None

            def rendered():
                nonlocal last_error
                state()
                screenshot(out / 'p1.png')
                try:
                    return compare_pictures(out)
                except ValueError as error:
                    last_error = error
                    return None

            try:
                return wait_for(rendered, 'the canonical P1 picture', seconds=45)
            except RuntimeError as error:
                if last_error:
                    raise ValueError(str(last_error)) from error
                raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('gate', choices=('native', 'safari', 'chrome'))
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--app', type=Path)
    parser.add_argument('--metal-harness', type=Path, help='Mac test executable from cargo test --no-run')
    parser.add_argument('--dist', type=Path)
    parser.add_argument('--fixture', type=Path)
    parser.add_argument('--force-software', action='store_true')
    args = parser.parse_args()
    if args.gate == 'native' and (not args.app or args.force_software):
        parser.error('native requires --app and cannot use --force-software')
    if args.gate == 'native' and platform.system() == 'Darwin' and not args.metal_harness:
        parser.error('the Mac native gate requires --metal-harness')
    if args.gate != 'native' and (not args.dist or not args.fixture):
        parser.error('browser gates require --dist and --fixture from the native gate')
    if args.force_software and args.gate != 'chrome':
        parser.error('--force-software is a separate Chrome negative test')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    record = {'gate': args.gate, 'system': platform.system(), 'pass': False, 'forced_software': args.force_software}
    try:
        if args.gate == 'native':
            native(args, out, record)
            record['picture'] = compare_pictures(out)
        else:
            record['picture'] = web(args, out, record)
        if args.force_software:
            raise ValueError('Forced software run did not establish a software adapter')
        record['pass'] = True
    except (ImportError, OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        record['error'] = str(error)
    finally:
        save_record(out, record)
    print(json.dumps(record, indent=2))
    return 0 if record['pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
