// The feature property panel in the browser build, driven in headless Chrome as a visitor
// drives it. Usage: node ci/browser-feature-panel.mjs CHROME URL OUTDIR
//
// The visitor makes a part and runs Extrude from the command palette. The side area turns
// into the extrusion's property panel, which asks for a sketch or a built-in shape. The
// visitor takes the Rectangle, ticks Direction 2 (the section opens), and applies with
// OK. A second Extrude takes the Circle with Merge result off, so it is a second body. The
// visitor saves the part (a download): the file must be format 4, keep Direction 2 on
// the first extrusion and put the second in its own body. The visitor loads the site
// again, opens the saved file, selects Boss-Extrude1 in the tree and runs Edit feature:
// the panel must open with Direction 2 ticked and open. Escape cancels. Last, a later
// Extrude on top of the reopened part must build, and a second save must hold all three.
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. The canvas has no text to read, so the
// check reads the panel through the read-only `shown_feature_panel` export and finds its
// controls through `shown_controls`. The New part and Open tiles of the start page are
// clicked at fixed points of a 1400 x 813 page; commands run through the command palette.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, out] = process.argv.slice(2);
if (!chrome || !url || !out) {
  throw new Error('usage: browser-feature-panel.mjs CHROME URL OUTDIR');
}
fs.mkdirSync(out, { recursive: true });
const downloads = path.join(path.resolve(out), 'downloads');
fs.rmSync(downloads, { recursive: true, force: true });
fs.mkdirSync(downloads, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const NEW_PART = [124, 143];
const OPEN = [663, 147];

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
// What the visitor is doing, on stderr, so a run that stops says where.
const started = Date.now();
function step(what) { console.error(`[${((Date.now() - started) / 1000).toFixed(1)} s] ${what}`); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

// part.json of a saved part file: the zip's local entry, stored or deflated.
function partJson(file) {
  const bytes = fs.readFileSync(file);
  for (let at = 0; at + 30 < bytes.length;) {
    check(bytes.readUInt32LE(at) === 0x04034b50, `${file}: no zip entry at ${at}`);
    const method = bytes.readUInt16LE(at + 8);
    const size = bytes.readUInt32LE(at + 18);
    const nameLength = bytes.readUInt16LE(at + 26);
    const extraLength = bytes.readUInt16LE(at + 28);
    const name = bytes.toString('utf8', at + 30, at + 30 + nameLength);
    const start = at + 30 + nameLength + extraLength;
    if (name === 'part.json') {
      const data = bytes.subarray(start, start + size);
      return JSON.parse((method === 8 ? zlib.inflateRawSync(data) : data).toString('utf8'));
    }
    at = start + size;
  }
  throw new Error(`${file} holds no part.json`);
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


const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-feature-panel-'));
let browser;

async function main() {
  browser = await new Browser(profile).connect();
  const v = await browser.visitor();
  // The panel as the last frame drew it, once `done` holds.
  const panel = async (label, done) => {
    let shown = null;
    for (let t = 0; t < 300; t++) {
      await v.stopped(label);
      const text = await v.eval(`window.wasmBindings.shown_feature_panel()`);
      shown = text ? JSON.parse(text) : null;
      if (done(shown)) return shown;
      await sleep(200);
    }
    await v.shot(`panel-${label.replace(/\W+/g, '-')}`);
    throw new Error(`${label}: the panel is ${JSON.stringify(shown)}`);
  };
  const section = (shown, title) => shown?.sections?.find(s => s.title === title);
  const refused = async () => JSON.parse(await v.eval(`window.wasmBindings.shown_marks()`) || '[]').filter(m => m.mark === 'Refused');

  step('load the site');
  await v.load();
  check(await v.eval(`typeof window.wasmBindings.shown_feature_panel === 'function'`), 'the build has no panel export');
  step('new part, Extrude');
  await v.click(NEW_PART);
  await v.settle(1500);
  await v.command('Extrude');
  await panel('the extrusion asks for its profile', shown => shown?.title === 'Boss-Extrude');
  await v.click(await v.control('Rectangle'));
  const opened = await panel('the rectangle extrusion', shown => section(shown, 'Direction 2'));
  check(section(opened, 'Direction 1')?.open === true, `Direction 1 starts open: ${JSON.stringify(opened)}`);
  for (const folded of ['From', 'Direction 2', 'Thin feature']) {
    check(section(opened, folded)?.open === false, `${folded} starts folded: ${JSON.stringify(opened)}`);
  }
  check(section(opened, 'Direction 2').on === false, 'Direction 2 starts off');
  await v.shot('1-panel');
  step('tick Direction 2, OK');
  await v.click(await v.control('Direction 2'));
  const ticked = await panel('Direction 2 ticked', shown => section(shown, 'Direction 2')?.on === true);
  check(section(ticked, 'Direction 2').open, 'ticking Direction 2 opens it');
  await v.shot('2-direction-2');
  await v.click(await v.control('Form OK'));
  await v.ready('after the first extrusion');
  await panel('the panel closes after OK', shown => shown === null);

  step('Extrude a circle with Merge result off');
  await v.command('Extrude');
  await panel('the second extrusion asks for its profile', shown => shown?.title === 'Boss-Extrude');
  await v.click(await v.control('Circle'));
  await panel('the circle extrusion', shown => section(shown, 'Direction 1'));
  await v.click(await v.control('Merge result'));
  await v.shot('3-merge-off');
  await v.click(await v.control('Form OK'));
  await v.ready('after the second extrusion');
  check((await refused()).length === 0, `no feature is refused: ${JSON.stringify(await refused())}`);

  step('save');
  const saved = await v.download('save', () => v.key('s', 'KeyS', 83, 2, 's'));
  const truth = partJson(saved);
  check(truth.format === 'soliddraft-part/4', `the part saves as format 4, not ${truth.format}`);
  const extrusions = truth.features.filter(f => f.kind.type === 'extrude');
  check(extrusions.length === 2, `two extrusions are saved: ${JSON.stringify(truth.features)}`);
  check(extrusions[0].kind.direction2?.type === 'blind', `the first keeps Direction 2: ${JSON.stringify(extrusions[0])}`);
  check((extrusions[0].body || 0) !== (extrusions[1].body || 0), `the second is its own body: ${JSON.stringify(extrusions)}`);

  step('load the site again, open the saved part');
  await v.load();
  await v.choose(saved, () => v.click(OPEN));
  await v.ready('after the reopen');
  step('select Boss-Extrude1 in the tree, Edit feature');
  await v.click(await v.control('Tree: Boss-Extrude1'));
  await v.command('Edit feature');
  const edited = await panel('editing Boss-Extrude1', shown => shown?.title === 'Edit Boss-Extrude');
  check(section(edited, 'Direction 2')?.on === true && section(edited, 'Direction 2')?.open === true,
    `Direction 2 is still ticked and open after the reopen: ${JSON.stringify(edited)}`);
  await v.shot('4-reopened-edit');
  await v.key('Escape', 'Escape', 27);
  await panel('Escape closes the panel', shown => shown === null);

  step('a later extrusion on the reopened part');
  await v.command('Extrude');
  await panel('the later extrusion asks for its profile', shown => shown?.title === 'Boss-Extrude');
  await v.click(await v.control('Rectangle'));
  await panel('the later extrusion', shown => section(shown, 'Direction 1'));
  await v.click(await v.control('Form OK'));
  await v.ready('after the later extrusion');
  check((await refused()).length === 0, `no feature is refused after the later one: ${JSON.stringify(await refused())}`);
  const again = partJson(await v.download('save again', () => v.key('s', 'KeyS', 83, 2, 's')));
  check(again.features.filter(f => f.kind.type === 'extrude').length === 3, 'the second save holds three extrusions');
  await v.shot('5-later-feature');

  fs.writeFileSync(path.join(out, 'feature-panel.json'), JSON.stringify({ opened, ticked, edited, saved: truth, again }, null, 1));
  console.log('Feature panel in the browser: sections folded and open as expected, Direction 2 ticked and kept through save and reopen, Merge result off saved a second body, and a later extrusion built on the reopened part.');
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
