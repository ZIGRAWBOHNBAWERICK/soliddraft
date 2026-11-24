// Document tabs in the browser build: closing a tab with unsaved work asks Save, Discard
// or Cancel; a drawing's Save writes the drawing; and Export preferences in the shortcut
// menu writes the shortcut menu's settings only. Usage:
//
//   node ci/browser-document-tabs.mjs CHROME URL OUTDIR
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. Controls the canvas draws are found by
// name through the read-only `shown_controls` export. Saving downloads the file, so the
// check reads what was saved from the downloads.
//
// Part1 gets a body and waits in its tab behind a new part. Closing it asks; Cancel keeps
// it, and Save downloads Part1.dwprt and closes it. Part2 gets a body, waits behind
// Part3, and Discard closes it without a download. A drawing of Part3 is saved from the
// top bar: the download is the drawing, not the part. Export preferences, with private
// folders and assistant settings stored beside the shortcut menu, downloads the shortcut
// menu alone.
//
// Run Chrome on WebGL: a headless screenshot cannot read a WebGPU canvas. The New part
// tile and Extrude's Rectangle button are at fixed points of a 1400 x 813 page;
// tests/documents_start.rs checks both are there.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { killGroup, startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, out] = process.argv.slice(2);
if (!chrome || !url || !out) throw new Error('usage: browser-document-tabs.mjs CHROME URL OUTDIR');
fs.mkdirSync(out, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const NEW_PART = [124, 143];
const RECTANGLE = [99, 355];
// A private folder and an assistant choice, stored beside the shortcut menu.
const PRIVATE = '/home/someone/private-projects';
const SETTINGS = JSON.stringify({
  file_locations: { lists: { referenced_documents: [PRIVATE] } },
  assistant: { anthropic_model: 'private-model-choice' },
});

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-document-tabs-'));
const downloads = path.join(profile, 'downloads');
fs.mkdirSync(downloads);
const browser = startChrome(chrome, [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', `--window-size=${SIZE.width},${SIZE.height}`, ...extra, 'about:blank',
]);
let stderr = '';
browser.stderr.on('data', d => { stderr += d; });
let ws;

// Every download the page started, by the file name it suggested, in order.
const started = [];
const finished = new Set();

async function main() {
  const port = await devtoolsPort();
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  ws = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let id = 0;
  const waiting = new Map();
  ws.onmessage = event => {
    const m = JSON.parse(event.data);
    if (m.id && waiting.has(m.id)) {
      const { resolve, reject } = waiting.get(m.id);
      waiting.delete(m.id);
      if (m.error) reject(new Error(`${m.error.message} ${m.error.data || ''}`)); else resolve(m.result);
    }
    if (m.method === 'Browser.downloadWillBegin') started.push({ guid: m.params.guid, name: m.params.suggestedFilename });
    if (m.method === 'Browser.downloadProgress' && m.params.state === 'completed') finished.add(m.params.guid);
  };
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const n = ++id;
    waiting.set(n, { resolve, reject });
    ws.send(JSON.stringify({ id: n, method, params, sessionId }));
  });
  await send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, eventsEnabled: true });
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  const call = (method, params) => send(method, params, sessionId);
  await call('Page.enable');
  await call('Emulation.setDeviceMetricsOverride', { ...SIZE, deviceScaleFactor: 1, mobile: false });
  // The app draws on animation frames, which a page in the background does not get.
  await call('Emulation.setFocusEmulationEnabled', { enabled: true });
  await call('Page.bringToFront');

  const v = {
    async eval(expression) {
      const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
      return r.result.value;
    },
    async load() {
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
      // A press held much longer than this is not a click to the app.
      await call('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
      await sleep(80);
      await call('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
      await v.settle(600);
    },
    async key(key, code, keyCode, modifiers = 0, text) {
      const base = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
      await call('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', ...base, text });
      await call('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
      await v.settle(800);
    },
    // E opens Extrude, which asks for a sketch when none is selected. Its Rectangle
    // button extrudes a sized rectangle instead, and Enter applies it.
    async extrude() {
      await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 700, y: 450 });
      await v.key('e', 'KeyE', 69, 0, 'e');
      await v.settle(1500);
      await v.click(RECTANGLE);
      await v.key('Enter', 'Enter', 13, 0, '\r');
      await v.settle(3000);
    },
    async controls() {
      return JSON.parse(await v.eval('window.wasmBindings.shown_controls()') || '{}');
    },
    // The page point of a control the last frames drew, by its name in `shown_controls`.
    async control(name) {
      for (let t = 0; t < 100; t++) {
        const shown = await v.controls();
        if (shown[name]) return [shown[name].x, shown[name].y];
        await v.settle(100);
      }
      await v.shot(`no-control-${name.replace(/\W+/g, '-')}`);
      throw new Error(`no control named ${name} was drawn`);
    },
    async press(name) {
      await v.click(await v.control(name));
    },
    // Whether a control is drawn, once the next frames have settled.
    async drawn(name) {
      await v.settle(500);
      return name in await v.controls();
    },
    async shot(label) {
      const { data } = await call('Page.captureScreenshot', { format: 'png' });
      fs.writeFileSync(path.join(out, `${label}.png`), Buffer.from(data, 'base64'));
    },
    async newPartTab() {
      await v.press('New tab');
      await v.press('New part (.dwprt)');
      await v.settle(1000);
    },
  };

  // The file a download wrote, once it has finished.
  async function downloaded(name) {
    for (let t = 0; t < 120; t++) {
      const found = started.find(d => d.name === name && finished.has(d.guid));
      if (found) return fs.readFileSync(path.join(downloads, name));
      await sleep(250);
    }
    throw new Error(`no download ${name}: ${JSON.stringify(started.map(d => d.name))}`);
  }

  await v.load();
  await v.eval(`localStorage.setItem('soliddraft-settings', ${JSON.stringify(SETTINGS)})`);
  await v.load();
  check(await v.eval(`typeof window.wasmBindings.shown_controls === 'function'`), 'the build has no control export');
  await v.click(NEW_PART);
  await v.extrude();
  await v.newPartTab();
  await v.shot('1-part1-waits-behind-part2');

  // Closing the unsaved part asks; Cancel keeps it.
  await v.press('Close Part1.dwprt');
  check(await v.drawn('Close question: Discard'), 'closing an unsaved part closed it without asking');
  await v.shot('2-asked');
  await v.press('Close question: Cancel');
  check(!await v.drawn('Close question: Discard'), 'Cancel left the question open');
  check(await v.drawn('Close Part1.dwprt'), 'Cancel closed the part');

  // Save downloads the part, then its tab closes.
  await v.press('Close Part1.dwprt');
  await v.press('Close question: Save');
  const part = await downloaded('Part1.dwprt');
  check(part.length > 0, 'the saved part is empty');
  check(!await v.drawn('Close Part1.dwprt'), 'the saved part stayed open');
  await v.shot('3-saved-and-closed');

  // Discard closes an unsaved part without saving it.
  await v.extrude();
  await v.newPartTab();
  await v.press('Close Part2.dwprt');
  await v.press('Close question: Discard');
  check(!await v.drawn('Close Part2.dwprt'), 'Discard kept the part open');
  check(!started.some(d => d.name === 'Part2.dwprt'), 'Discard saved the part');

  // Export preferences writes the shortcut menu alone, not the folders kept beside it.
  await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 700, y: 450 });
  await v.key('s', 'KeyS', 83, 0, 's');
  await v.press('Customize shortcut menu');
  await v.press('Export preferences');
  const exported = JSON.parse(await downloaded('soliddraft-preferences.json'));
  check(JSON.stringify(Object.keys(exported)) === '["shortcut_menu"]', `the export holds other settings: ${Object.keys(exported)}`);
  check(!JSON.stringify(exported).includes(PRIVATE), 'the export holds a private folder');
  await v.key('Escape', 'Escape', 27);
  await v.key('Escape', 'Escape', 27);

  // A drawing of Part3, saved from the top bar, downloads the drawing.
  await v.extrude();
  await v.press('New tab');
  await v.press('New drawing (.dwdrw)');
  await v.key('Enter', 'Enter', 13, 0, '\r');
  await v.settle(3000);
  await v.shot('4-drawing');
  const before = started.length;
  const save = Object.keys(await v.controls()).find(name => name.startsWith('Quick Save'));
  check(save, 'the top bar draws no Save');
  await v.press(save);
  for (let t = 0; t < 120 && started.length === before; t++) await sleep(250);
  const names = started.slice(before).map(d => d.name);
  check(names.length > 0, 'the top bar Save downloaded nothing');
  check(names.every(name => name.endsWith('.dwdrw')), `the top bar Save of a drawing downloaded ${names}`);
}

async function devtoolsPort() {
  const file = path.join(profile, 'DevToolsActivePort');
  for (let t = 0; t < 100; t++) {
    if (fs.existsSync(file)) {
      const [port] = fs.readFileSync(file, 'utf8').split('\n');
      if (port) return port;
    }
    await sleep(100);
  }
  throw new Error('Chrome did not open its DevTools port');
}

// A step that hangs says where it was instead of waiting for the job to time out.
const deadline = setTimeout(() => {
  console.error('FAIL the document tabs check took over eight minutes');
  killGroup(browser);
  process.exit(1);
}, 8 * 60 * 1000);

try {
  await main();
  console.log('PASS unsaved tabs ask before closing, a drawing saves itself, and the export holds the shortcut menu');
} catch (error) {
  console.error(error.stack || error);
  console.error(stderr.split('\n').slice(-20).join('\n'));
  process.exitCode = 1;
} finally {
  clearTimeout(deadline);
  const connected = ws?.readyState === WebSocket.OPEN;
  await stopChrome(browser, connected ? async () => ws.send(JSON.stringify({ id: 0, method: 'Browser.close' })) : null);
  ws?.close();
  fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  process.exit(process.exitCode ?? 0);
}
