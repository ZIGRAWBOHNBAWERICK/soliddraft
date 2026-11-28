#!/usr/bin/env python3
"""GPU preview acceptance: the phase 5 speed scenarios show their preview from the GPU mesher.

native: replays each scenario through `ci/run-scenarios.py` with the release `soliddraft`,
on the adapter the CLI selects. Every displayed step must run on a hardware adapter with the
GPU preview on, the scenario must show GPU previews, and the viewport must ask the kernel for
no CPU preview. Faces the mesher refuses are drawn by the CPU preview and are reported apart.
The certified-mesh calls each revision made are summed from the kernel logs.

safari: opens each scenario's last saved part in the shipping Wasm app in Safari, on the
canvas's own WebGPU adapter, and reads the page's GPU preview counts.

Run on a Mac only. Software and fallback adapters fail.
"""

import argparse
import json
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


def native_scenario(args, name, out):
    bundle = out / name
    command = [sys.executable, str(HERE.parent / 'run-scenarios.py'), str(HERE.parent / 'scenarios' / f'{name}.json'),
               '--out', str(bundle), '--soliddraft', str(args.soliddraft), '--catallax-verify', str(args.catallax_verify)]
    with (out / f'{name}.log').open('wb') as log:
        finished = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    record = summarize(name, bundle, finished.returncode)
    parts = sorted((bundle / 'commands').glob('*/part.dwprt')) or sorted(bundle.glob('**/part.dwprt'))
    if parts:
        fixture = out / 'fixtures' / f'{name}.dwprt'
        fixture.parent.mkdir(exist_ok=True)
        fixture.write_bytes(parts[-1].read_bytes())
    return record


def summarize(name, bundle, exit_code):
    """The acceptance record of one scenario bundle."""
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
    result = evaluate(f'window.gpuAcceptance.drop({json.dumps(name)}).then(() => true)')
    if result is not True:
        raise RuntimeError(f'Dropping {name}: {result}')
    gate.wait_drawn(lambda: state() and (record.get('document') or {}).get('count', 0) > 0, f'{name} to open',
                    record, seconds=600)
    gate.wait_drawn(lambda: state() and (record.get('preview') or {}).get('gpu_previews', 0) > 0,
                    f'{name} to show a GPU preview', record, seconds=300)
    time.sleep(3)
    state()
    screenshot(out / f'{name}.png')
    preview = record.get('preview') or {}
    problems = []
    if not (preview.get('status') or '').startswith('GPU preview on'):
        problems.append(f"GPU preview not on: {preview.get('status')}")
    if preview.get('cpu_previews'):
        problems.append(f"{preview['cpu_previews']} CPU previews for the display")
    record['problems'] = problems
    record['accepted'] = not problems
    return record


def safari(args, out, records):
    fixtures = args.fixtures.resolve()
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

                for name in args.scenarios:
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
    parser.add_argument('--dist', type=Path, help='shipping Wasm site (safari)')
    parser.add_argument('--fixtures', type=Path, help='the native gate\'s fixtures folder (safari)')
    parser.add_argument('--scenarios', nargs='+', default=list(SCENARIOS))
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('GPU preview acceptance runs on a Mac')
    if args.gate == 'native' and not (args.soliddraft and args.catallax_verify):
        parser.error('native needs --soliddraft and --catallax-verify')
    if args.gate == 'safari' and not (args.dist and args.fixtures):
        parser.error('safari needs --dist and --fixtures')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    records = []
    summary = {'gate': args.gate, 'scenarios': records}
    try:
        if args.gate == 'native':
            for name in args.scenarios:
                records.append(native_scenario(args, name, out))
                (out / 'result.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
        else:
            summary['browser'] = safari(args, out, records)
    finally:
        summary['pass'] = bool(records) and all(record.get('accepted') for record in records)
        (out / 'result.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'pass': summary['pass'], 'failed': [r['scenario'] for r in records if not r.get('accepted')],
                      'not_strict': [r['scenario'] for r in records if r.get('strict') is False]}))
    return 0 if summary['pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
