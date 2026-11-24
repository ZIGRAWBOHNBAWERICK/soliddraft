// The Hole Wizard in the browser build, driven in headless Chrome as a visitor drives it.
// Usage: node ci/browser-hole-wizard.mjs CHROME URL OUTDIR STEP
//
// STEP is the P1 bracket (ci/fixtures/pmi/p1-bracket-pmi.step), a plate with a boss and
// fourteen items of manufacturing information. The visitor makes a part, imports the file
// and opens Inspect > Manufacturing information. Then Hole from the command palette opens
// the Hole Wizard panel in the tree's place. A click on the plate's top places the first
// hole there; the panel's Straight Tap tile makes it an ISO M6 coarse tapped hole through
// all with a cosmetic thread; on the Positions tab a second click on the plate adds a
// second hole; Apply commits the feature. The panel must then list the thread as a
// fifteenth item, `2× M6x1-6H THRU` on two hole walls where the clicks were, and the
// viewport must draw the thread's marks. The visitor saves the part (a download), loads the
// site again and opens the saved file: the same item on the same walls, and the same marks.
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. The panel's tiles, tabs and Apply button
// are found through the read-only `shown_controls` export, the plate through
// `part_screen_point`, and the hole through `shown_manufacturing_information` and
// `shown_threads`.
//
// Run Chrome on WebGL: a headless screenshot cannot read a WebGPU canvas.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, out, file] = process.argv.slice(2);
if (!chrome || !url || !out || !file) {
  throw new Error('usage: browser-hole-wizard.mjs CHROME URL OUTDIR STEP');
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

// The bracket's own items; the hole adds one more.
const IMPORTED = 14;
// The two holes: through the 5 mm base plate at y 22, 6 mm apart, both in view.
const FIRST = [50, 22, 5];
const SECOND = [56, 22, 5];
const THREAD = '2× M6x1-6H THRU';

// `<name> @ <Surface> (x, y, z)` as the panel export writes a face.
function place(face) {
  const m = /@ (\w+) \((-?[\d.]+), (-?[\d.]+), (-?[\d.]+)\)$/.exec(face);
  check(m, `a face reads as a name, a surface and a centroid: ${face}`);
  return [m[1], Number(m[2]), Number(m[3]), Number(m[4])];
}

// The hole's item: the thread on two cylinders, each at one of the clicks. A click lands
// to a tenth of a millimetre and the display mesh moves a centroid a little more.
function checkHole(shown, when) {
  check(shown.items.length === IMPORTED + 1, `${when}: ${shown.items.length} items, not ${IMPORTED + 1}`);
  const item = shown.items[IMPORTED];
  check(item.name === 'Hole1 thread', `${when}: the last item is ${item.name}, not Hole1 thread`);
  check(item.callout === THREAD, `${when}: the hole reads "${item.callout}", not ${THREAD}`);
  check(!item.problem, `${when}: the thread does not apply: ${item.problem}`);
  check(item.faces.length === 2, `${when}: the thread is on ${item.faces.length} holes, not 2: ${JSON.stringify(item.faces)}`);
  const walls = item.faces.map(feature => place(feature[0]));
  for (const [x, y] of [FIRST, SECOND]) {
    check(walls.some(([surface, wx, wy]) => surface === 'Cylinder' && Math.abs(wx - x) < 0.3 && Math.abs(wy - y) < 0.3),
      `${when}: no hole wall at (${x}, ${y}): ${JSON.stringify(item.faces)}`);
  }
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
      // Rests the pointer on a point.
      async hover([x, y]) {
        await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
        await v.settle(500);
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

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-hole-wizard-'));
let browser;

async function main() {
  browser = await new Browser(profile).connect();
  const v = await browser.visitor();
  step('load the site');
  await v.load();
  check(await v.eval(`typeof window.wasmBindings.shown_controls === 'function'`), 'the build has no control export');

  step('new part, Import STEP, upload, accept');
  await v.click(NEW_PART);
  await v.settle(1500);
  await v.choose(file, () => v.command('Import STEP'));
  await v.click(ACCEPT_IMPORT);
  await v.ready('after the import');
  await v.command('Manufacturing information');
  await v.panel('after the import', IMPORTED);

  // A click on the plate, as a careful visitor makes it: once the preview has rebuilt,
  // with the pointer resting on the spot first, so the viewport has read what is under it.
  const clickPlate = async (label, at) => {
    for (let t = 0, quiet = 0; quiet < 3; t++) {
      check(t < 300, `${label}: the preview did not finish building`);
      const build = JSON.parse(await v.eval(`window.wasmBindings.shown_build()`) || '{}');
      quiet = build.pending === false && !build.reading ? quiet + 1 : 0;
      await sleep(200);
    }
    const point = await v.eval(`(() => {
      const point = window.wasmBindings.part_screen_point(${at.join(', ')});
      return point == null ? null : Array.from(point);
    })()`);
    check(point, `${label}: the plate at ${at} is on screen`);
    await v.hover([point[0] + 1, point[1]]);
    await v.hover(point);
    await v.click(point);
  };

  step('Hole, click the plate top');
  await v.command('Hole');
  await v.settle(800);
  await clickPlate('the first hole', FIRST);
  step('Straight Tap tile');
  await v.click(await v.control('Hole type: Straight Tap'));
  await v.settle(500);
  await v.shot('1-type-tab');
  step('Positions tab, click the plate again');
  await v.click(await v.control('Hole Wizard Positions tab'));
  await v.settle(500);
  await clickPlate('the second hole', SECOND);
  await v.shot('2-positions-tab');
  step('Apply');
  await v.click(await v.control('Apply Hole Wizard'));
  await v.ready('after the hole');
  const made = await v.panel('after the hole', IMPORTED + 1);
  checkHole(made, 'after the hole');
  const marks = await v.threads('after the hole');
  await v.shot('3-holes');

  step('save');
  const saved = await v.download('save', () => v.key('s', 'KeyS', 83, 2, 's'));
  check(saved.endsWith('.dwprt'), `the part saves as a .dwprt file, not ${saved}`);
  step('load the site again');
  await v.load();
  step('open the saved part');
  await v.choose(saved, () => v.click(OPEN));
  await v.ready('after the reopen');
  await v.command('Manufacturing information');
  const reopened = await v.panel('after the reopen', IMPORTED + 1);
  checkHole(reopened, 'after the reopen');
  check(JSON.stringify(reopened) === JSON.stringify(made),
    `the reopened part lists ${JSON.stringify(reopened)}, not ${JSON.stringify(made)}`);
  const marksAgain = await v.threads('after the reopen');
  check(marksAgain === marks, `the reopened part draws ${marksAgain} thread mark segments, not ${marks}`);
  await v.shot('4-reopened');

  fs.writeFileSync(path.join(out, 'hole-wizard.json'), JSON.stringify({ made: made.items[IMPORTED], reopened: reopened.items[IMPORTED], marks, saved: path.basename(saved) }, null, 1));
  console.log(`Hole Wizard in the browser: a Straight Tap placed by two clicks on the plate reads "${made.items[IMPORTED].callout}" on two hole walls with ${marks} thread mark segments, the same after save and a fresh reopen.`);
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
