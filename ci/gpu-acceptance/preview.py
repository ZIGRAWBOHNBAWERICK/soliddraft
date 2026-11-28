#!/usr/bin/env python3
"""GPU preview acceptance: the phase 5 speed scenarios show their preview from the GPU mesher.

native: replays each scenario through `ci/run-scenarios.py` with the release `soliddraft`,
on the adapter the CLI selects. Every displayed step must run on a hardware adapter with the
GPU preview on, the scenario must show GPU previews, and the viewport must ask the kernel for
no CPU preview. Faces the mesher refuses are drawn by the CPU preview and are reported apart.
The certified-mesh calls each revision made are summed from the kernel logs. Then the Metal
harness opens each scenario's last part and every part file in the repository in the app on
its own Metal device, reads the GPU preview back from the mesher's buffers, and checks it
against the checked mesh: no missing face, no crack, every vertex within the preview
tolerance. It keeps the preview and checked frames of each part.

native --gpu-off: the same runs with the GPU preview switched off. It is a control that
shows the CPU preview route, never a gate.

safari: opens each saved part in the shipping Wasm app in Safari, on the canvas's own
WebGPU adapter, and reads the page's GPU preview counts.

Run on a Mac only. Software and fallback adapters fail.
"""

import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run as gate  # noqa: E402

SCENARIOS = (
    'loft-edge-fillet', 'p5-placed-housing', 'p5-flanged-housing', 'canonical-p1', 'draft-oblique-pull',
    'drawing', 'ring-fillet', 'spline-sweep', 'canonical-p5', 'bracket', 'references', 'torus-revolve',
    'sphere-revolve', 'rod-cut-28deg', 'skew-corner-fillet', 'loft', 'graded-fields', 'sketch-trim',
)
# The certified mesh of an analytic body; a spline body's checked mesh is its display result.
CHECKED = ('tessellate', 'tessellate_edges')
CPU_PREVIEW = ('preview', 'preview_edges')
# The step's own kernel log and its display capture. Exports, measurements and drawing
# views are not the viewport's display.
DISPLAY_LOGS = ('kernel.json', 'display.kernel.json')
ROOT = HERE.parent.parent
# The readback harness's hang guard for the whole corpus, not a speed limit.
READBACK_TIMEOUT_S = 7200


def corpus():
    """Every part file in the repository, by a name made from its path."""
    listed = subprocess.run(['git', '-C', str(ROOT), 'ls-files', '-z', '*.dwprt'], check=True,
                            capture_output=True).stdout.decode().split('\0')
    return [('corpus-' + path[:-len('.dwprt')].replace('/', '-'), ROOT / path) for path in sorted(filter(None, listed))]


def kernel_calls(log):
    """(operation, wall ms) of each kernel call a kernel log lists."""
    try:
        value = json.loads(log.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return []
    return [(call.get('operation'), float(call.get('wall_ms') or 0.0)) for call in value.get('calls', [])
            if isinstance(call, dict)]


def adapter_of(adapter):
    return {'backend': adapter.get('backend'), 'device_type': adapter.get('device_type'),
            'name': adapter.get('name') or ''}


def native_env(args):
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith('WGPU_') and k != 'SOLIDDRAFT_GPU_PREVIEW'}
    if args.gpu_off:
        env['SOLIDDRAFT_GPU_PREVIEW'] = 'off'
    return env


def native_scenario(args, name, out):
    bundle = out / name
    command = [sys.executable, str(HERE.parent / 'run-scenarios.py'), str(HERE.parent / 'scenarios' / f'{name}.json'),
               '--out', str(bundle), '--soliddraft', str(args.soliddraft), '--catallax-verify', str(args.catallax_verify)]
    with (out / f'{name}.log').open('wb') as log:
        finished = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=3600, env=native_env(args))
    record = summarize(name, bundle, finished.returncode, control=args.gpu_off)
    parts = sorted((bundle / 'commands').glob('*/part.dwprt')) or sorted(bundle.glob('**/part.dwprt'))
    if parts:
        fixture = out / 'fixtures' / f'{name}.dwprt'
        fixture.parent.mkdir(exist_ok=True)
        fixture.write_bytes(parts[-1].read_bytes())
    return record


def summarize(name, bundle, exit_code, control=False):
    """The acceptance record of one scenario bundle. A control run expects the CPU preview."""
    record = {'scenario': name, 'runner_exit': exit_code, 'steps': 0, 'adapters': [], 'gpu_previews': 0,
              'cpu_previews': 0, 'refused_faces': 0, 'refusals': {}, 'status': None, 'checked_ms': {},
              'kernel_previews': 0}
    result = bundle / 'result.json'
    if result.is_file():
        report = json.loads(result.read_text(encoding='utf-8'))
        record['pass'] = report.get('pass')
        record['revisions'] = len({step.get('revision') for step in report.get('steps', []) if step.get('revision')})
    for display in sorted((bundle / 'commands').glob('*/display.json')):
        # A capture whose renderer installed the GPU preview records its delivery beside
        # display.json; without the record the step had no GPU preview at all.
        delivery = display.with_name('display.gpu.json')
        counts = json.loads(delivery.read_text(encoding='utf-8')) if delivery.is_file() else {}
        record['steps'] += 1
        record['status'] = counts.get('status')
        # The counts are the process's since it started; a fresh process starts again.
        for key in ('gpu_previews', 'cpu_previews', 'refused_faces'):
            record[key] = max(record[key], counts.get(key, 0))
        for reason, count in (counts.get('refusals') or {}).items():
            record['refusals'][reason] = max(record['refusals'].get(reason, 0), count)
        if counts.get('adapter'):
            adapter = adapter_of(counts['adapter'])
            if adapter not in record['adapters']:
                record['adapters'].append(adapter)
    for log in sorted((bundle / 'commands').glob('*/kernel.json')):
        command = log.with_name('command.json')
        revision = json.loads(command.read_text(encoding='utf-8')).get('revision') if command.is_file() else None
        for name in DISPLAY_LOGS:
            for call, elapsed in kernel_calls(log.with_name(name)):
                if call in CHECKED:
                    record['checked_ms'][str(revision)] = record['checked_ms'].get(str(revision), 0.0) + elapsed
                elif call in CPU_PREVIEW:
                    record['kernel_previews'] += 1
    record['checked_ms'] = {revision: round(ms, 1) for revision, ms in record['checked_ms'].items()}
    record['checked_total_ms'] = round(sum(record['checked_ms'].values()), 1)
    # Strict: the step logs show no CPU preview call at all. They also count calls outside
    # the viewport: saved display data and field bodies keep the CPU preview by decision,
    # and faces the GPU mesher refuses fall back to it (`refused_faces`).
    record['strict'] = record['kernel_previews'] == 0
    problems = []
    if not record['steps']:
        problems.append('no displayed step')
    if control:
        # Without an installed GPU preview the capture writes no delivery record, so the
        # status may be absent; the kernel logs show the CPU preview instead.
        if record['status'] and not record['status'].startswith('GPU preview off'):
            problems.append(f"GPU preview not switched off: {record['status']}")
        if record['gpu_previews']:
            problems.append(f"{record['gpu_previews']} GPU previews with the GPU preview off")
        if not record['kernel_previews']:
            problems.append('no CPU preview call in the kernel logs')
        record['problems'] = problems
        record['accepted'] = not problems
        return record
    for adapter in record['adapters']:
        try:
            gate.require_hardware(adapter, {'Metal'})
        except ValueError as error:
            problems.append(str(error))
    if not (record['status'] or '').startswith('GPU preview on'):
        problems.append(f"GPU preview not on: {record['status'] or 'not installed, no display.gpu.json'}")
    if record['steps'] and not record['gpu_previews']:
        problems.append('no GPU preview was shown')
    if record['cpu_previews']:
        problems.append(f"{record['cpu_previews']} CPU previews for the display")
    record['problems'] = problems
    record['accepted'] = not problems
    return record


def excluded(part):
    """Why a part is outside the GPU mesher's corpus, or None. A part with no body shows
    nothing; a lattice or gyroid is drawn by the field display, which keeps its own mesh."""
    if part.get('route') == 'none' and part.get('live_bodies') == 0 and not part.get('problems'):
        return 'the part has no body to display'
    bodies = part.get('bodies') or []
    if part.get('route') == 'cpu' and bodies and all(body.get('field') for body in bodies):
        return 'field bodies: the field display draws them, not the GPU mesher'
    return None


def judge_part(part, adapter, control=False):
    """The problems of one part's readback record. The gate needs the GPU preview on a
    hardware Metal adapter, read back without a missing face, crack or deviation over the
    preview tolerance. A checked mesh that never replaces the preview is a finding, not a
    failure. The control needs the CPU preview and no GPU preview."""
    problems = list(part.get('problems', []))
    counts = part.get('counts') or {}
    status = part.get('status') or ''
    if control:
        if not status.startswith('GPU preview off'):
            problems.append(f'GPU preview not switched off: {status}')
        if part.get('route') != 'cpu':
            problems.append(f"route {part.get('route')}, not the CPU preview")
        if counts.get('gpu_previews'):
            problems.append(f"{counts['gpu_previews']} GPU previews with the GPU preview off")
        return problems
    try:
        gate.require_hardware(adapter, {'Metal'})
    except ValueError as error:
        problems.append(str(error))
    if not (status.startswith('GPU preview on') and 'Metal' in status):
        problems.append(f'GPU preview not on Metal: {status}')
    if part.get('route') != 'gpu':
        reasons = '; '.join(body.get('why', '') for body in part.get('bodies', []) if body.get('route') == 'cpu')
        problems.append(f"no GPU preview shown (route {part.get('route')}){': ' + reasons if reasons else ''}")
    if not counts.get('gpu_previews') and part.get('route') == 'gpu':
        problems.append('the GPU preview count did not move')
    if counts.get('cpu_previews'):
        problems.append(f"{counts['cpu_previews']} CPU previews for the display")
    return problems


def fallback_faces(part):
    """Each face the GPU mesher refused, with its reason and whether the CPU drew it."""
    return [dict(face, body=body.get('body')) for body in part.get('bodies', []) for face in body.get('refused', [])]


def native_readback(args, out, parts):
    """Runs the Metal harness over `parts`, a list of (name, path), and returns its records."""
    folder = out / 'readback'
    folder.mkdir()
    manifest = out / 'parts.json'
    manifest.write_text(json.dumps([{'name': name, 'path': str(path)} for name, path in parts], indent=2) + '\n',
                        encoding='utf-8')
    env = native_env(args)
    env.update(SOLIDDRAFT_GPU_ACCEPTANCE_OUT=str(folder), SOLIDDRAFT_GPU_ACCEPTANCE_PARTS=str(manifest),
               RUST_LOG='soliddraft_app::gpu_preview=info')
    with (out / 'readback.log').open('wb') as log:
        command = [str(args.metal_harness.resolve()), '--ignored', '--exact', 'read_back_metal_previews', '--nocapture']
        finished = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=READBACK_TIMEOUT_S)
    result = folder / 'readback.json'
    if not result.is_file():
        raise RuntimeError(f'the Metal harness wrote no readback.json (exit {finished.returncode}); see '
                           f"{out / 'readback.log'}")
    value = json.loads(result.read_text(encoding='utf-8'))
    value['exit'] = finished.returncode
    return value


def readback_record(part, adapter, control):
    reason = excluded(part)
    if control and reason and part.get('route') != 'none':
        reason = None
    problems = [] if reason else judge_part(part, adapter, control)
    timings = part.get('timings') or {}
    return {
        'route': part.get('route'), 'status': part.get('status'), 'counts': part.get('counts'),
        'tolerance_mm': part.get('tolerance_mm'),
        'deviation_mm': max((body.get('deviation_mm', 0) for body in part.get('bodies', [])), default=None),
        'edges': [body.get('edges') for body in part.get('bodies', []) if body.get('edges')],
        'fallback_faces': fallback_faces(part), 'timings': timings,
        'findings': [f"{body.get('body')}: {finding}" for body in part.get('bodies', [])
                     for finding in body.get('findings', [])] + (
            [f"the checked mesh did not replace the GPU preview: {part.get('checked_missing')}"]
            if part.get('route') == 'gpu' and not part.get('checked_route') else []),
        'pictures': [str(Path(part['path_out']) / name) for name in ('preview.png', 'checked.png')]
        if part.get('path_out') else [], 'picture_difference': part.get('picture_difference'),
        'problems': problems, 'accepted': not problems, **({'excluded': reason} if reason else {}),
    }


class PartServer(gate.GateServer):
    def do_GET(self):
        local = self.path.partition('?')[0]
        prefix = '/.gpu-acceptance/'
        if local.startswith(prefix) and local.endswith('.dwprt'):
            path = self.server.fixture / local[len(prefix):]
            if path.parent != self.server.fixture or not path.is_file():
                self.send_error(404)
                return
            body = path.read_bytes()
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()


def safari_scenario(evaluate, screenshot, navigate, name, out):
    record = {'scenario': name}
    navigate()
    snapshot = '''({
      renderer: window.gpuAcceptance?.renderer,
      document: window.gpuAcceptance?.document,
      ready: document.querySelector('#status')?.classList.contains('hidden'),
      preview: window.sdGpuPreview ?? null,
      visibility: document.visibilityState
    })'''

    def state():
        record.update(evaluate(snapshot) or {})
        return record.get('renderer') and record.get('ready')

    gate.wait_drawn(state, 'the shipping Wasm viewport', record, seconds=90)
    gate.require_hardware(record['renderer'], {'WebGPU'})
    dropped = time.monotonic()
    result = evaluate(f'window.gpuAcceptance.drop({json.dumps(name)}).then(() => true)')
    if result is not True:
        raise RuntimeError(f'Dropping {name}: {result}')
    gate.wait_drawn(lambda: state() and (record.get('document') or {}).get('count', 0) > 0, f'{name} to open',
                    record, seconds=600)
    opened = time.monotonic()
    gate.wait_drawn(lambda: state() and (record.get('preview') or {}).get('gpu_previews', 0) > 0,
                    f'{name} to show a GPU preview', record, seconds=300)
    # Polled from outside the page, so these include the driver's round trips.
    record['timings'] = {'open_ms': round((opened - dropped) * 1000), 'gpu_preview_ms':
                         round((time.monotonic() - dropped) * 1000)}
    time.sleep(3)
    state()
    screenshot(out / f'{name}.png')
    preview = record.get('preview') or {}
    problems = []
    if not ((preview.get('status') or '').startswith('GPU preview on') and 'WebGPU' in preview.get('status', '')):
        problems.append(f"GPU preview not on WebGPU: {preview.get('status')}")
    if preview.get('cpu_previews'):
        problems.append(f"{preview['cpu_previews']} CPU previews for the display")
    record['problems'] = problems
    record['accepted'] = not problems
    return record


def safari(args, out, records):
    fixtures = args.fixtures.resolve()
    # The native gate's corpus parts follow its scenarios, so both gates open the same parts.
    listed = fixtures / 'corpus.json'
    names = list(args.scenarios) + (json.loads(listed.read_text(encoding='utf-8')) if listed.is_file() else [])
    with gate.ThreadingHTTPServer(('127.0.0.1', 0), gate.partial(PartServer, directory=args.dist)) as server:
        server.fixture = fixtures
        worker = gate.threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        url = f'http://127.0.0.1:{server.server_port}/?preset=fast&theme=light'
        try:
            browser_record = {}
            with gate.safari(out, url, browser_record) as (evaluate, screenshot):
                def navigate():
                    # A fresh page per part, so each part's counts start at zero.
                    evaluate(f'(setTimeout(() => {{ location.href = {json.dumps(url)}; }}, 50), true)')
                    time.sleep(3)

                for name in names:
                    if not (fixtures / f'{name}.dwprt').is_file():
                        records.append({'scenario': name, 'accepted': False, 'problems': ['no saved part']})
                        continue
                    try:
                        records.append(safari_scenario(evaluate, screenshot, navigate, name, out))
                    except (ValueError, RuntimeError, TypeError) as error:
                        records.append({'scenario': name, 'accepted': False, 'problems': [str(error)]})
            return browser_record
        finally:
            server.shutdown()
            worker.join()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('gate', choices=('native', 'safari'))
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--soliddraft', type=Path, help='release soliddraft executable (native)')
    parser.add_argument('--catallax-verify', type=Path, help='verifier from the pinned Catallax revision (native)')
    parser.add_argument('--metal-harness', type=Path,
                        help='test executable of cargo test -p soliddraft-app --test gpu_viewport_acceptance (native)')
    parser.add_argument('--gpu-off', action='store_true',
                        help='native control with the GPU preview switched off; never a gate')
    parser.add_argument('--dist', type=Path, help='shipping Wasm site (safari)')
    parser.add_argument('--fixtures', type=Path, help='the native gate\'s fixtures folder (safari)')
    parser.add_argument('--scenarios', nargs='+', default=list(SCENARIOS))
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('GPU preview acceptance runs on a Mac')
    if args.gate == 'native' and not (args.soliddraft and args.catallax_verify and args.metal_harness):
        parser.error('native needs --soliddraft, --catallax-verify and --metal-harness')
    if args.gpu_off and args.gate != 'native':
        parser.error('--gpu-off is a native control')
    if args.gate == 'safari' and not (args.dist and args.fixtures):
        parser.error('safari needs --dist and --fixtures')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    records = []
    parts = []
    summary = {'gate': args.gate, 'scenarios': records, 'parts': parts}
    if args.gpu_off:
        summary.update(gate='native control, GPU preview off', control=True)
    try:
        if args.gate == 'native':
            for name in args.scenarios:
                records.append(native_scenario(args, name, out))
                (out / 'result.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
            fixtures = out / 'fixtures'
            fixtures.mkdir(exist_ok=True)
            listed = []
            for name, path in corpus():
                (fixtures / f'{name}.dwprt').write_bytes(path.read_bytes())
                listed.append(name)
            (fixtures / 'corpus.json').write_text(json.dumps(listed, indent=2) + '\n', encoding='utf-8')
            opened = [(r['scenario'], fixtures / f"{r['scenario']}.dwprt") for r in records
                      if (fixtures / f"{r['scenario']}.dwprt").is_file()]
            opened += [(name, fixtures / f'{name}.dwprt') for name in listed]
            readback = native_readback(args, out, opened)
            summary['renderer'] = readback.get('adapter')
            summary['status'] = readback.get('status')
            summary['harness_exit'] = readback['exit']
            by_name = {part['name']: dict(part, path_out=str(out / 'readback' / part['name']))
                       for part in readback.get('parts', [])}
            for record in records:
                part = by_name.get(record['scenario'])
                if part is None:
                    record['problems'].append('no saved part was read back')
                else:
                    record['readback'] = readback_record(part, readback.get('adapter'), args.gpu_off)
                    record['problems'] += record['readback']['problems']
                record['accepted'] = not record['problems']
            for name in listed:
                part = by_name.get(name)
                if part is None:
                    parts.append({'part': name, 'accepted': False, 'problems': ['not read back']})
                else:
                    parts.append({'part': name, 'path': part.get('path'),
                                  **readback_record(part, readback.get('adapter'), args.gpu_off)})
            if readback['exit'] != 0:
                summary['problems'] = [f"the Metal harness exited {readback['exit']}"]
        else:
            summary['browser'] = safari(args, out, records)
    finally:
        everything = records + parts
        summary['pass'] = (bool(records) and all(record.get('accepted') for record in everything)
                           and not summary.get('problems'))
        (out / 'result.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'pass': summary['pass'], 'control': bool(args.gpu_off),
                      'failed': [r.get('scenario') or r.get('part') for r in everything if not r.get('accepted')],
                      'not_strict': [r['scenario'] for r in records if r.get('strict') is False]}))
    return 0 if summary['pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
