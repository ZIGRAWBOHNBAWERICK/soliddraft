// Manufacturing information in the browser build, driven in headless Chrome as a visitor
// drives it. Usage: node ci/browser-manufacturing-information.mjs CHROME URL OUTDIR STEP
//
// STEP is the P1 bracket with its fourteen items of manufacturing information, as
// SolidDraft writes it (ci/fixtures/pmi/p1-bracket-pmi.step). The visitor makes a part,
// runs Import STEP and uploads the file, accepts the import and opens Inspect >
// Manufacturing information. The panel must list the fourteen items with the part's
// notes, each on the face the file put it on. The visitor saves the part (a download),
// loads the site again, opens the saved file and opens the panel again: the same
// fourteen items must be on the same faces. Last, Inspect > Drawing and files must hand
// over one download holding the STEP file, the PDF and the DXF.
//
// Before the drawing, the visitor drills a tapped hole in the reopened part: Hole from
// the command palette, a click on the plate's top, Thread set to Tapped, OK. The panel
// must list the thread as a fifteenth item, `M8x1.25-6H THRU` on the new wall, the
// viewport must draw the thread's marks (the read-only `shown_threads` export), and both
// must come back after a second save and a fresh reopen. The face is found on screen
// through `part_screen_point`, and the form's controls through `shown_controls`.
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. The canvas has no text to read, so the
// check reads the panel's rows through the read-only `shown_manufacturing_information`
// export: each item with its callout, and each face with its surface and its centroid.
//
// Run Chrome on WebGL: a headless screenshot cannot read a WebGPU canvas. The clicks go
// to the New part and Open tiles of the start page and to the import review's Accept
// button at fixed points of a 1400 x 813 page; commands run through the command palette
// by name.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, out, file] = process.argv.slice(2);
if (!chrome || !url || !out || !file) {
  throw new Error('usage: browser-manufacturing-information.mjs CHROME URL OUTDIR STEP');
}
fs.mkdirSync(out, { recursive: true });
const downloads = path.join(path.resolve(out), 'downloads');
fs.rmSync(downloads, { recursive: true, force: true });
fs.mkdirSync(downloads, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const NEW_PART = [124, 143];
const OPEN = [663, 147];
const ACCEPT_IMPORT = [56, 147];

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
// What the visitor is doing, on stderr, so a run that stops says where.
const started = Date.now();
function step(what) { console.error(`[${((Date.now() - started) / 1000).toFixed(1)} s] ${what}`); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

// The bracket's items in the part's order, each with its callout and where its faces
// are: surface and centroid in millimetres.
const P = (x, y, z) => ['Plane', x, y, z];
const C = (x, y, z) => ['Cylinder', x, y, z];
const A = P(30, 19.847, 0), B = P(30, 0, 14.8), DATUM_C = P(0, 13.269, 8.269);
const BORE = C(30, 25, 7.5), BOSS = C(30, 25, 10), BOSS_TOP = P(30, 25, 15);
const HOLES = [C(10, 12, 2.5), C(10, 32, 2.5), C(50, 12, 2.5), C(50, 32, 2.5)];
const SLOT_LOW = P(30, 2.5, 15), SLOT_HIGH = P(30, 2.5, 21);
const SLOT_ENDS = [C(21.09, 2.5, 18), C(38.91, 2.5, 18)];
const EXPECTED = [
  ['Datum A', 'Datum A', [[A]]],
  ['Datum B', 'Datum B', [[B]]],
  ['Datum C', 'Datum C', [[DATUM_C]]],
  ['Bore diameter', 'Ø8 H7', [[BORE]]],
  ['Bore finish', 'Ra 1.6', [[BORE]]],
  ['Bore position', 'Position Ø0.05 A|B|C', [[BORE]]],
  ['Boss diameter', 'Ø16 ±0.1', [[BOSS]]],
  ['Boss height', '15 ±0.1', [[A], [BOSS_TOP]]],
  ['Hole diameter', '4× Ø4 +0.1/0 THRU', HOLES.map(hole => [hole])],
  ['Hole position', 'Position Ø0.2 A|B|C', HOLES.map(hole => [hole])],
  ['Slot', '20×6 +0.1/0 R3', [[SLOT_LOW], [SLOT_HIGH], [SLOT_ENDS[0]], [SLOT_ENDS[1]]]],
  ['Slot height', '18 ±0.1', [[A], [SLOT_LOW, SLOT_HIGH]]],
  ['Flatness of A', 'Flatness 0.05', [[A]]],
  ['Perpendicularity of B', 'Perpendicularity 0.1 A', [[B]]],
];
const NOTES = { general_tolerance: 'ISO 2768-m', material: '6061-T6', notes: ['Break sharp edges'] };
// The tapped hole: through the 5 mm plate at x 50, y 22, between two mounting holes. The
// form's Tapped takes the coarse thread whose tap drill is nearest the drawn Ø6.6: M8.
const TAPPED_AT = [50.1, 22, 5];
const THREAD = ['Hole1 thread', 'M8x1.25-6H THRU', [[C(50, 22, 2.5)]]];
// The tapped hole removes material from the datum face, moving that face's centroid.
const TAPPED_A = P(29.681, 19.812, 0);
const TAPPED_EXPECTED = EXPECTED.map(([name, callout, features]) => [name, callout,
  features.map(feature => feature.map(face => face === A ? TAPPED_A : face))]);

// `<name> @ <Surface> (x, y, z)` as the panel export writes a face.
function place(face) {
  const m = /@ (\w+) \((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)$/.exec(face);
  check(m, `a face reads as a name, a surface and a centroid: ${face}`);
  return [m[1], Number(m[2]), Number(m[3]), Number(m[4])];
}

// The display mesh gives centroids to a few hundredths of a millimetre.
function at(have, want) {
  return have[0] === want[0] && [1, 2, 3].every(i => Math.abs(have[i] - want[i]) < 0.05);
}

function checkThread(shown, when) {
  check(shown.items.length === EXPECTED.length + 1, `${when}: ${shown.items.length} items, not ${EXPECTED.length + 1}: ${JSON.stringify(shown.items.map(i => i.name))}`);
  checkItems({ ...shown, items: shown.items.slice(0, EXPECTED.length) }, when, TAPPED_EXPECTED);
  const [name, callout, features] = THREAD;
  const item = shown.items[EXPECTED.length];
  check(item.name === name && item.callout === callout, `${when}: the last item is ${item.name} "${item.callout}", not ${name} "${callout}"`);
  check(!item.problem, `${when}: ${name} does not apply: ${item.problem}`);
  const have = item.faces.map(feature => feature.map(place));
  check(have.length === features.length && features.every((feature, f) => feature.every(want => have[f].some(face => at(face, want)))),
    `${when}: ${name} is on ${JSON.stringify(item.faces)}, not on ${JSON.stringify(features)}`);
}

function checkItems(shown, when, expected = EXPECTED) {
  check(shown.items.length === expected.length, `${when}: ${shown.items.length} items, not ${expected.length}: ${JSON.stringify(shown.items.map(i => i.name))}`);
  expected.forEach(([name, callout, features], n) => {
    const item = shown.items[n];
    check(item.name === name && item.callout === callout, `${when}: item ${n + 1} is ${item.name} "${item.callout}", not ${name} "${callout}"`);
    check(!item.problem, `${when}: ${name} does not apply: ${item.problem}`);
    check(item.faces.length === features.length, `${when}: ${name} is on ${item.faces.length} features, not ${features.length}`);
    features.forEach((feature, f) => {
      const have = item.faces[f].map(place);
      check(have.length === feature.length && feature.every(want => have.some(face => at(face, want))),
        `${when}: ${name} is on ${JSON.stringify(item.faces[f])}, not on ${JSON.stringify(feature)}`);
    });
  });
  check(JSON.stringify(shown.notes) === JSON.stringify(NOTES), `${when}: the notes are ${JSON.stringify(shown.notes)}`);
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
      // The panel's rows once it lists `count` items, as the export gives them.
      async panel(label, count) {
        let shown = null;
        for (let t = 0; t < 900; t++) {
          await v.stopped(label);
          const text = await v.eval(`window.wasmBindings.shown_manufacturing_information()`);
          shown = text ? JSON.parse(text) : null;
          if (shown && shown.items.length === count) return shown;
          await sleep(200);
        }
        await v.shot(`panel-${label}`);
        throw new Error(`${label}: the panel lists ${shown ? shown.items.length : 'no'} items, not ${count}`);
      },
      // The page point of a control the last frames drew, by its name in `shown_controls`.
      async control(name) {
        for (let t = 0; t < 100; t++) {
          const shown = JSON.parse(await v.eval(`window.wasmBindings.shown_controls()`) || '{}');
          if (shown[name]) return [shown[name].x, shown[name].y];
          await v.settle(100);
        }
        await v.shot(`no-control-${name.replace(/\W+/g, '-')}`);
        throw new Error(`no control named ${name} was drawn`);
      },
      // How many thread mark segments the viewport draws, once it draws some.
      async threads(label) {
        for (let t = 0; t < 300; t++) {
          const drawn = await v.eval(`window.wasmBindings.shown_threads()`);
          if (drawn > 0) return drawn;
          await sleep(200);
        }
        await v.shot(`no-threads-${label}`);
        throw new Error(`${label}: the viewport draws no thread marks`);
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

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-pmi-'));
let browser;

async function main() {
  browser = await new Browser(profile).connect();
  const v = await browser.visitor();
  step('load the site');
  await v.load();
  check(await v.eval(`typeof window.wasmBindings.shown_manufacturing_information === 'function'`), 'the build has no panel export');

  // A new part, then Import STEP with the bracket's file as the upload.
  step('new part, Import STEP, upload');
  await v.click(NEW_PART);
  await v.settle(1500);
  await v.choose(file, () => v.command('Import STEP'));
  await v.shot('1-import-review');
  step('accept the import');
  await v.click(ACCEPT_IMPORT);

  // The panel lists what the file states, on the faces the file names.
  await v.ready('after the import');
  step('open the panel and read its items');
  await v.command('Manufacturing information');
  const imported = await v.panel('after the import', EXPECTED.length);
  checkItems(imported, 'after the import');
  await v.shot('2-imported-with-its-items');

  // Save (a download), load the site afresh, open the saved file.
  step('save');
  const saved = await v.download('save', () => v.key('s', 'KeyS', 83, 2, 's'));
  check(saved.endsWith('.dwprt'), `the part saves as a .dwprt file, not ${saved}`);
  step('load the site again');
  await v.load();
  step('open the saved part');
  await v.choose(saved, () => v.click(OPEN));
  await v.ready('after the reopen');
  step('open the panel and read its items');
  await v.command('Manufacturing information');
  const reopened = await v.panel('after the reopen', EXPECTED.length);
  checkItems(reopened, 'after the reopen');
  check(JSON.stringify(reopened) === JSON.stringify(imported),
    `the reopened part lists ${JSON.stringify(reopened)}, not ${JSON.stringify(imported)}`);
  await v.shot('3-reopened-with-its-items');

  // A tapped hole on the plate's top, made in the Hole form.
  step('Hole, click the plate top, Tapped, OK');
  await v.command('Hole');
  await v.settle(800);
  const face = await v.eval(`Array.from(window.wasmBindings.part_screen_point(${TAPPED_AT.join(', ')}))`);
  check(face, 'the plate top is on screen');
  await v.click(face);
  await v.click(await v.control('Hole thread'));
  await v.click(await v.control('Hole thread: Tapped'));
  await v.shot('3a-tapped-hole-form');
  await v.click(await v.control('Form OK'));
  await v.ready('after the tapped hole');
  const tapped = await v.panel('after the tapped hole', EXPECTED.length + 1);
  checkThread(tapped, 'after the tapped hole');
  const marks = await v.threads('after the tapped hole');
  await v.shot('3b-tapped-hole');

  // Saved again and reopened afresh, the thread is still stated and still drawn.
  step('save the tapped part');
  const tappedFile = await v.download('save the tapped part', () => v.key('s', 'KeyS', 83, 2, 's'));
  step('load the site again');
  await v.load();
  step('open the tapped part');
  await v.choose(tappedFile, () => v.click(OPEN));
  await v.ready('after reopening the tapped part');
  await v.command('Manufacturing information');
  const tappedAgain = await v.panel('after reopening the tapped part', EXPECTED.length + 1);
  checkThread(tappedAgain, 'after reopening the tapped part');
  check(JSON.stringify(tappedAgain) === JSON.stringify(tapped),
    `the reopened tapped part lists ${JSON.stringify(tappedAgain)}, not ${JSON.stringify(tapped)}`);
  const marksAgain = await v.threads('after reopening the tapped part');
  check(marksAgain === marks, `the reopened part draws ${marksAgain} thread mark segments, not ${marks}`);
  await v.shot('3c-tapped-reopened');

  // One action: the STEP file and a drawing of it as PDF and DXF, in one download.
  step('Drawing and files');
  const pack = await v.download('drawing and files', () => v.command('Drawing and files'));
  check(pack.endsWith('.zip'), `the files arrive as one .zip, not ${pack}`);
  const bytes = fs.readFileSync(pack);
  // A stored zip names its files in the clear.
  const names = ['.step', '.pdf', '.dxf'].filter(extension => bytes.includes(Buffer.from(extension)));
  check(names.length === 3, `the download holds ${names.join(', ')} of .step, .pdf and .dxf`);
  await v.settle(3000);
  await v.stopped('after Drawing and files');
  await v.shot('4-drawing');

  fs.writeFileSync(path.join(out, 'manufacturing-information.json'), JSON.stringify({ imported, reopened, tapped, tappedAgain, marks, saved: path.basename(saved), download: path.basename(pack), bytes: bytes.length }, null, 1));
  console.log(`Manufacturing information in the browser: ${imported.items.length} items read from the uploaded STEP file, the same ${reopened.items.length} on the same faces after save and a fresh reopen, a tapped hole stated as ${THREAD[1]} and drawn with ${marks} mark segments before and after a second save and reopen, and ${path.basename(pack)} (${bytes.length} bytes) with the STEP file, the PDF and the DXF.`);
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
