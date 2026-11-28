// Field preview and first display in the browser build, driven in headless Chrome as a
// visitor drives it. Usage:
//   node ci/browser-field-preview.mjs CHROME URL OUTDIR TRAY LATTICE GYROID PIECES GRADED
//
// Roadmap 4.1: the app draws a field preview only for the field-only features, lattices and
// gyroids; the boundary model previews everything else, and a preview never stands in for an
// export. Each file is a saved part, opened afresh from the start page:
//
// - TRAY (site/parts/tray.dwprt), a boundary-model part. Its first display reports no
//   field preview and no status mark. Its STL export runs the boundary-model print
//   route, and the file saved is the checked print mesh, marked as that route in its header.
// - LATTICE (ci/fixtures/field-preview/lattice.dwprt). The first display reports a
//   field preview and no mark. The STL export runs the distance-field route, and
//   the file saved is the certified field mesh, marked as that route in its header.
// - GYROID (ci/fixtures/field-preview/gyroid.dwprt), the network gyroid G0. Its first display
//   reports a field preview and no mark. Its export is not run here: a certified gyroid mesh takes
//   minutes in the browser's debug build.
// - PIECES (ci/fixtures/field-preview/gyroid-refused-export.dwprt), a network gyroid at level
//   1/2 in a box it leaves a separate corner piece in, at 2 mm. The screen shows its field
//   preview, but Catallax cannot settle the boundary within the minimum detail, so the STL
//   export is refused by name, offers no Save, and writes nothing.
// - GRADED (ci/fixtures/field-preview/gyroid-graded-refused.dwprt), a gyroid level graded
//   past the range Catallax certifies. The tree marks the gyroid Refused with its reason, the
//   viewport stops reporting a field preview, and the STL command opens no export.
//
// The label is the frame probe's name for a field sample on screen. The page itself shows
// no words about checking.
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. The canvas has no text to read, so the check
// reads what the frame drew through the read-only `shown_first_part_frame`,
// `shown_print_check` and `shown_controls` exports. The Open tile sits at a fixed point of
// a 1400 x 813 page; commands run through the command palette by name. The fixtures are saved
// without display data, so each first display is made in the page.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, out, tray, lattice, gyroid, pieces, graded] = process.argv.slice(2);
if (!chrome || !url || !out || !tray || !lattice || !gyroid || !pieces || !graded) {
  throw new Error('usage: browser-field-preview.mjs CHROME URL OUTDIR TRAY LATTICE GYROID PIECES GRADED');
}
fs.mkdirSync(out, { recursive: true });
const downloads = path.join(path.resolve(out), 'downloads');
fs.rmSync(downloads, { recursive: true, force: true });
fs.mkdirSync(downloads, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const OPEN = [663, 147];

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
const started = Date.now();
function step(what) { console.error(`[${((Date.now() - started) / 1000).toFixed(1)} s] ${what}`); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

// A binary STL's header and triangle count. A checked print mesh starts CXPRINT1, and its
// ninth byte names the route: 1 the boundary model, 2 the distance field.
function stl(file) {
  const bytes = fs.readFileSync(file);
  check(bytes.length >= 84, `${file} is shorter than an STL header`);
  return { magic: bytes.subarray(0, 8).toString('latin1'), route: bytes[8], triangles: bytes.readUInt32LE(80), bytes: bytes.length };
}

class Browser {
  constructor(profile) {
    this.profile = profile;
    this.process = startChrome(chrome, [
      '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
      '--no-first-run', '--no-default-browser-check', `--window-size=${SIZE.width},${SIZE.height}`, ...extra, 'about:blank',
    ]);
    this.stderr = '';
    this.process.stderr.on('data', d => { this.stderr += d; });
    this.events = [];
  }

  async connect() {
    const file = path.join(this.profile, 'DevToolsActivePort');
    let port;
    for (let t = 0; t < 100 && !port; t++) {
      if (fs.existsSync(file)) [port] = fs.readFileSync(file, 'utf8').split('\n');
      if (!port) await sleep(100);
    }
    check(port, 'Chrome did not open its DevTools port');
    const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
    this.ws = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { this.ws.onopen = resolve; this.ws.onerror = reject; });
    this.id = 0;
    this.waiting = new Map();
    this.ws.onmessage = event => {
      const m = JSON.parse(event.data);
      if (m.id && this.waiting.has(m.id)) {
        const { resolve, reject } = this.waiting.get(m.id);
        this.waiting.delete(m.id);
        if (m.error) reject(new Error(`${m.error.message} ${m.error.data || ''}`)); else resolve(m.result);
      } else if (m.method) {
        this.events.push(m);
      }
    };
    return this;
  }

  send(method, params = {}, sessionId) {
    return new Promise((resolve, reject) => {
      const n = ++this.id;
      this.waiting.set(n, { resolve, reject });
      this.ws.send(JSON.stringify({ id: n, method, params, sessionId }));
    });
  }

  async visitor() {
    const { targetId } = await this.send('Target.createTarget', { url: 'about:blank' });
    const { sessionId } = await this.send('Target.attachToTarget', { targetId, flatten: true });
    const call = (method, params) => this.send(method, params, sessionId);
    const events = this.events;
    await call('Page.enable');
    await call('Emulation.setDeviceMetricsOverride', { ...SIZE, deviceScaleFactor: 1, mobile: false });
    // The app draws on animation frames, which a page in the background does not get.
    await call('Emulation.setFocusEmulationEnabled', { enabled: true });
    await call('Page.bringToFront');
    await this.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, eventsEnabled: true });
    const v = {
      async eval(expression) {
        const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
        if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
        return r.result.value;
      },
      async load() {
        await call('Page.bringToFront');
        await call('Page.navigate', { url });
        for (let t = 0; t < 240; t++) {
          await sleep(500);
          const ready = await v.eval(`document.readyState === 'complete' && !!document.querySelector('#status.hidden')`).catch(() => false);
          if (ready) {
            await sleep(3000);
            return;
          }
        }
        await v.shot('not-started');
        throw new Error('the app did not start');
      },
      async frames(n = 3) {
        await v.eval(`new Promise(done => { let left = ${n}; const tick = () => --left ? requestAnimationFrame(tick) : done(); requestAnimationFrame(tick); })`);
      },
      async settle(ms) {
        await sleep(ms);
        await v.frames();
      },
      async click([x, y]) {
        await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
        await v.settle(300);
        await call('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
        await sleep(80);
        await call('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
        await v.settle(600);
      },
      async key(key, code, keyCode, modifiers = 0, text) {
        const base = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
        await call('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', ...base, text });
        await call('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
        await v.settle(600);
      },
      // Ctrl+Shift+P, the command's name, Enter.
      async command(name, enter = () => v.key('Enter', 'Enter', 13, 0, '\r')) {
        await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1000, y: 450 });
        await v.key('P', 'KeyP', 80, 2 | 8, 'P');
        for (const ch of name) {
          await call('Input.dispatchKeyEvent', { type: 'char', text: ch });
          await sleep(30);
        }
        await v.settle(500);
        await enter();
      },
      // Runs `trigger`, which opens a file dialog, and answers the dialog with `file`.
      async choose(file, trigger) {
        await call('Page.setInterceptFileChooserDialog', { enabled: true });
        const from = events.length;
        await trigger();
        for (let t = 0; t < 100; t++) {
          const opened = events.slice(from).find(e => e.method === 'Page.fileChooserOpened');
          if (opened) {
            await call('DOM.setFileInputFiles', { files: [path.resolve(file)], backendNodeId: opened.params.backendNodeId });
            await call('Page.setInterceptFileChooserDialog', { enabled: false });
            await v.settle(1500);
            return;
          }
          await sleep(100);
        }
        await v.shot('no-file-dialog');
        throw new Error('no file dialog opened');
      },
      // Runs `trigger` and waits for the download it starts; answers the file's path.
      async download(label, trigger) {
        const from = events.length;
        await trigger();
        for (let t = 0; t < 600; t++) {
          const mine = events.slice(from);
          const begun = mine.find(e => e.method === 'Browser.downloadWillBegin');
          const done = begun && mine.find(e => e.method === 'Browser.downloadProgress' && e.params.guid === begun.params.guid && e.params.state === 'completed');
          if (done) {
            // Chrome keeps the file under its download id.
            const kept = path.join(downloads, begun.params.guid);
            const named = path.join(downloads, begun.params.suggestedFilename);
            if (fs.existsSync(kept)) fs.renameSync(kept, named);
            check(fs.existsSync(named), `${label}: ${begun.params.suggestedFilename} is not in ${downloads}`);
            return named;
          }
          await v.stopped(label);
          await sleep(200);
        }
        await v.shot(`no-download-${label}`);
        throw new Error(`${label}: no download arrived`);
      },
      // The page shows its status element again when it stops.
      async stopped(label) {
        const status = await v.eval(`(() => { const s = document.getElementById('status'); return s.classList.contains('hidden') ? null : s.textContent; })()`);
        check(status === null, `${label}: the page stopped: ${status}`);
      },
      // Waits until a part is open and nothing is being built or read: the options bar
      // is drawn, and three looks in a row find no build owed. A command sent while a
      // part is still opening would be lost.
      async ready(label) {
        let quiet = 0;
        for (let t = 0; t < 1500; t++) {
          await v.stopped(label);
          const [options, build] = await v.eval(`[window.wasmBindings.shown_options(), window.wasmBindings.shown_build()]`);
          const state = build ? JSON.parse(build) : null;
          quiet = options && state && !state.pending && !state.reading ? quiet + 1 : 0;
          if (quiet === 3) return;
          await sleep(200);
        }
        await v.shot(`not-ready-${label}`);
        throw new Error(`${label}: the part did not finish opening`);
      },
      async shot(label) {
        const { data } = await call('Page.captureScreenshot', { format: 'png' });
        fs.writeFileSync(path.join(out, `${label}.png`), Buffer.from(data, 'base64'));
      },
    };
    return v;
  }

  async close() {
    const connected = this.ws?.readyState === WebSocket.OPEN;
    await stopChrome(this.process, connected ? () => this.send('Browser.close') : null);
    this.ws?.close();
  }
}


const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-field-preview-'));
let browser;
const evidence = {};

// Reads one of the page's read-only exports as JSON, or null when the last frame drew none.
async function shown(v, name) {
  const text = await v.eval(`window.wasmBindings.${name}()`);
  return text ? JSON.parse(text) : null;
}

// Waits for `done` to hold of what `name` shows; answers that value.
async function until(v, label, name, done, tries = 300) {
  let value = null;
  for (let t = 0; t < tries; t++) {
    await v.stopped(label);
    value = await shown(v, name);
    if (value !== null && done(value)) return value;
    await sleep(200);
  }
  await v.shot(`stuck-${label}`);
  throw new Error(`${label}: ${name} shows ${JSON.stringify(value)}`);
}

// Loads the site afresh and opens `file` from the start page; answers its first display.
async function open(v, label, file) {
  step(`open ${label}`);
  await v.load();
  await v.choose(file, () => v.click(OPEN));
  await v.ready(label);
  const first = await shown(v, 'shown_first_part_frame');
  check(first?.viewport.triangles > 0, `${label}: the first viewport frame drew no triangles: ${JSON.stringify(first)}`);
  check(first.viewport.pending === false, `${label}: the first viewport frame still owed a build`);
  const settled = await until(v, label, 'shown_viewport', frame => frame.triangles > 0);
  check(settled.field_preview === first.viewport.field_preview, `${label}: the field preview changed after the first frame`);
  const marks = await shown(v, 'shown_marks');
  check(JSON.stringify(marks) === JSON.stringify(first.marks), `${label}: the tree marks changed after the first frame`);
  await v.shot(`${label}-opened`);
  return first;
}

// Runs the STL command and waits for the Print check window's outcome.
async function exportStl(v, label) {
  step(`${label}: export STL`);
  await v.command('STL');
  return until(v, label, 'shown_print_check', shown => shown.state !== 'checking', 900);
}

async function withoutDownload(label, run) {
  const from = browser.events.length;
  const files = fs.readdirSync(downloads).sort();
  const result = await run();
  const started = browser.events.slice(from).filter(e => e.method === 'Browser.downloadWillBegin');
  check(started.length === 0, `${label}: a refused export started a download: ${started.map(e => e.params.suggestedFilename)}`);
  check(JSON.stringify(fs.readdirSync(downloads).sort()) === JSON.stringify(files), `${label}: a refused export changed the download directory`);
  return result;
}

async function save(v, label) {
  const controls = await shown(v, 'shown_controls');
  const button = controls && controls['Save export'];
  check(button, `${label}: the Print check window drew no Save export button`);
  const file = await v.download(label, () => v.click([button.x, button.y]));
  check(file.endsWith('.stl'), `${label}: the export saved ${file}, not an STL file`);
  return stl(file);
}

async function field(v, label, file) {
  const first = await open(v, label, file);
  check(first.viewport.field_preview === true, `${label}: the first display is not a field preview`);
  check(first.marks.length === 0, `${label}: a built part shows marks ${JSON.stringify(first.marks)}`);
  const exported = await exportStl(v, label);
  check(exported.route === 'distance field' && exported.state === 'checked',
    `${label}: the STL export shows ${JSON.stringify(exported)}, not a checked distance-field route`);
  await v.shot(`${label}-print-check`);
  const file_ = await save(v, label);
  check(file_.magic === 'CXPRINT1' && file_.route === 2, `${label}: the saved STL is ${JSON.stringify(file_)}, not the certified field mesh`);
  check(file_.triangles !== first.viewport.triangles,
    `${label}: the saved STL has the screen's ${file_.triangles} triangles; the export must not be the preview`);
  evidence[label] = { first, exported, saved: file_ };
}

async function main() {
  browser = await new Browser(profile).connect();
  const v = await browser.visitor();
  step('load the site');
  await v.load();
  for (const name of ['shown_first_part_frame', 'shown_viewport', 'shown_marks', 'shown_print_check', 'shown_controls']) {
    check(await v.eval(`typeof window.wasmBindings.${name} === 'function'`), `the build has no ${name} export`);
  }

  // A boundary-model part: no field preview, the boundary-model print route.
  const plain = await open(v, 'tray', tray);
  check(plain.viewport.field_preview === false, 'tray: a boundary-model part reports a field preview');
  check(plain.marks.length === 0, `tray: a built part shows marks ${JSON.stringify(plain.marks)}`);
  await v.command('Hide body');
  check((await shown(v, 'shown_viewport'))?.triangles === 0, 'tray: the hidden body still reports triangles');
  await v.command('Show all bodies');
  check((await shown(v, 'shown_viewport'))?.triangles > 0, 'tray: the restored body reports no triangles');
  await v.command('Wireframe');
  check((await shown(v, 'shown_viewport'))?.triangles === 0, 'tray: wireframe still reports triangles');
  await v.command('Shaded, edges');
  check((await shown(v, 'shown_viewport'))?.triangles > 0, 'tray: the shaded body reports no triangles');
  const plainExport = await exportStl(v, 'tray');
  check(plainExport.route === 'boundary model' && plainExport.state === 'checked',
    `tray: the STL export shows ${JSON.stringify(plainExport)}, not a checked boundary-model route`);
  const plainFile = await save(v, 'tray');
  check(plainFile.magic === 'CXPRINT1' && plainFile.route === 1, `tray: the saved STL is ${JSON.stringify(plainFile)}, not the checked print mesh`);
  evidence.tray = { first: plain, exported: plainExport, saved: plainFile };

  await v.command('New assembly');
  check((await shown(v, 'shown_assembly'))?.active === true, 'the new assembly did not take the viewport');
  check(await shown(v, 'shown_viewport') === null, 'the assembly still reports a part viewport');
  check(await v.eval(`window.wasmBindings.part_screen_point(0, 0, 0) == null`), 'the assembly still reports a part camera');
  check(await v.eval(`window.wasmBindings.shown_threads()`) === 0, 'the assembly still reports part thread marks');

  // A lattice: the field sample on screen, the certified field mesh in the file.
  await field(v, 'lattice', lattice);

  // The gyroid G0: the field sample on screen.
  const g0 = await open(v, 'gyroid', gyroid);
  check(g0.viewport.field_preview === true, 'gyroid: the first display is not a field preview');
  check(g0.marks.length === 0, `gyroid: a built part shows marks ${JSON.stringify(g0.marks)}`);
  evidence.gyroid = { first: g0 };

  // A preview on screen does not make an export: Catallax refuses this one.
  const split = await open(v, 'pieces', pieces);
  check(split.viewport.field_preview === true, 'pieces: the display is not a field preview');
  const refused = await withoutDownload('pieces', async () => {
    const outcome = await exportStl(v, 'pieces');
    check(outcome.state === 'refused' && outcome.route === 'distance field', `pieces: the STL export shows ${JSON.stringify(outcome)}, not a refusal`);
    check(/could not be reached/.test(outcome.message), `pieces: the refusal reads "${outcome.message}"`);
    const controls = await shown(v, 'shown_controls');
    check(!controls['Save export'], 'pieces: a refused export offers Save export');
    await v.shot('pieces-refused-export');
    await v.click([controls['Print check: Cancel'].x, controls['Print check: Cancel'].y]);
    await v.settle(2000);
    return outcome;
  });
  evidence.pieces = { first: split, exported: refused };

  // A refused field: marked, no field preview, no export opens.
  const bad = await open(v, 'graded', graded);
  const mark = bad.marks.find(m => m.feature === 'Gyroid1');
  check(mark && mark.mark === 'Refused' && mark.reason, `graded: the tree marks ${JSON.stringify(bad.marks)}, not Gyroid1 Refused with a reason`);
  check(bad.viewport.field_preview === false, 'graded: a refused gyroid still reports a field preview');
  await withoutDownload('graded', async () => {
    await v.command('STL');
    await v.settle(2000);
    check(await shown(v, 'shown_print_check') === null, 'graded: the STL command opened an export over a refused feature');
  });
  await v.shot('graded-refused');
  evidence.graded = { first: bad };

  fs.writeFileSync(path.join(out, 'field-preview.json'), JSON.stringify(evidence, null, 1));
  console.log(`Field preview in the browser: the tray shows no field preview and exports through the boundary model (${plainFile.triangles} triangles); the lattice shows a field sample of ${evidence.lattice.first.viewport.triangles} triangles and exports a certified field mesh of ${evidence.lattice.saved.triangles}; the gyroid shows a field sample of ${g0.viewport.triangles}; the split gyroid shows its preview and its export is refused with nothing written; the refused grade is marked "${mark.reason}" and opens no export.`);
}

try {
  await main();
} catch (error) {
  console.error(error.stack || String(error));
  if (browser?.stderr) console.error(browser.stderr.split('\n').slice(-20).join('\n'));
  process.exitCode = 1;
} finally {
  await browser?.close();
  fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
