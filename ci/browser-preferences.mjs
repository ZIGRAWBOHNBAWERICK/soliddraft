// Preferences in the browser build, as two visitors use them.
//
//   node ci/browser-preferences.mjs CHROME URL OUTDIR
//
// Starts Chrome itself and drives it over the DevTools protocol, because the app draws
// into one canvas and takes only trusted input: keys and clicks sent as the browser's own.
// Each visitor is a separate browser context, which has its own storage, as two people
// on two computers, or two browser profiles, do.
//
// Visitor A chooses the SOLIDWORKS preset and hides the grid with real clicks; both land
// in A's storage. A reload keeps them: the grid is gone from the picture, and a further
// change saves the navigation the app read back. Visitor B, on the same site at the same
// time, starts with empty storage and the grid shown, and B's choice leaves A's alone.
// The page sends nothing but GET requests for its own files.
//
// Run Chrome on WebGL: a headless screenshot cannot read a WebGPU canvas.
// The clicks go to fixed points of a 1400 x 813 page. When the Preferences window's
// layout changes, the frames this writes to OUTDIR show where its controls moved.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const [chrome, url, out] = process.argv.slice(2);
if (!chrome || !url || !out) throw new Error('usage: browser-preferences.mjs CHROME URL OUTDIR');
fs.mkdirSync(out, { recursive: true });
const KEY = 'soliddraft-settings';
const SIZE = { width: 1400, height: 813 };
// Where the controls are, in CSS pixels.
const AT = {
  solidworks: [468, 152], onshape: [575, 152],
  orbitAndZoom: [80, 156], gridAndPlanes: [80, 227],
  showGrid: [245, 133], wheelInvert: [244, 242], close: [977, 35],
  // The start page's New part tile; tests/documents_start.rs checks it is there.
  newPart: [124, 143],
};
// Floor right of the origin planes and above the feedback launcher.
const FLOOR = { x: 1200, y: 400, width: 190, height: 200 };

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-preferences-'));
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);
const browser = spawn(chrome, [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  '--no-first-run', '--no-default-browser-check', `--window-size=${SIZE.width},${SIZE.height}`, ...extra, 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let stderr = '';
browser.stderr.on('data', d => { stderr += d; });

let ws;
try {
  await main();
  console.log('PASS preferences kept per browser, back after a reload, never shared');
} catch (error) {
  console.error(error.stack || error);
  console.error(stderr.split('\n').slice(-20).join('\n'));
  process.exitCode = 1;
} finally {
  ws?.close();
  browser.kill();
}

async function main() {
  const port = await devtoolsPort();
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  ws = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let id = 0;
  const waiting = new Map();
  const requests = [];
  ws.onmessage = event => {
    const m = JSON.parse(event.data);
    if (m.id && waiting.has(m.id)) {
      const { resolve, reject } = waiting.get(m.id);
      waiting.delete(m.id);
      if (m.error) reject(new Error(`${m.error.message} ${m.error.data || ''}`)); else resolve(m.result);
    }
    if (m.method === 'Network.requestWillBeSent') requests.push({ session: m.sessionId, ...m.params.request });
  };
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const n = ++id;
    waiting.set(n, { resolve, reject });
    ws.send(JSON.stringify({ id: n, method, params, sessionId }));
  });

  async function visitor(name) {
    const { browserContextId } = await send('Target.createBrowserContext');
    const { targetId } = await send('Target.createTarget', { url: 'about:blank', browserContextId });
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
    const call = (method, params) => send(method, params, sessionId);
    await call('Page.enable');
    await call('Network.enable');
    await call('Emulation.setDeviceMetricsOverride', { ...SIZE, deviceScaleFactor: 1, mobile: false });
    // The app draws on animation frames, which a page in the background does not get.
    await call('Emulation.setFocusEmulationEnabled', { enabled: true });
    await call('Page.bringToFront');
    const v = {
      name, sessionId,
      async eval(expression) {
        const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
        if (r.exceptionDetails) throw new Error(`${name}: ${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
        return r.result.value;
      },
      async front() {
        await call('Page.bringToFront');
      },
      async load(reload) {
        await v.front();
        if (reload) await call('Page.reload'); else await call('Page.navigate', { url });
        for (let t = 0; t < 240; t++) {
          await sleep(500);
          const ready = await v.eval(`document.readyState === 'complete' && !!document.querySelector('#status.hidden')`).catch(() => false);
          if (ready) {
            await sleep(3000);
            const size = await v.eval('[innerWidth, innerHeight]');
            check(size[0] === SIZE.width && size[1] === SIZE.height, `${name}: the page is ${size}, the clicks need ${SIZE.width} x ${SIZE.height}`);
            // The page opens on the start page; New part shows the empty part and its grid.
            await v.click(AT.newPart);
            return;
          }
        }
        await v.shot('not-started');
        throw new Error(`${name}: the app did not start`);
      },
      async stored() {
        const text = await v.eval(`localStorage.getItem(${JSON.stringify(KEY)})`);
        return text === null ? null : JSON.parse(text);
      },
      // Lets the app draw `n` frames. A debug build can take over a second for one, and an
      // input event is only seen by the frame after it.
      async frames(n = 3) {
        await v.eval(`new Promise(done => { let left = ${n}; const tick = () => --left ? requestAnimationFrame(tick) : done(); requestAnimationFrame(tick); })`);
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
      // At least `ms`, and three frames.
      async settle(ms) {
        await sleep(ms);
        await v.frames();
      },
      // The stored settings once `test` holds of them, or what is stored after 20 seconds.
      async storedWhen(test) {
        let stored;
        for (let t = 0; t < 40; t++) {
          stored = await v.stored();
          if (test(stored)) return stored;
          await sleep(500);
        }
        await v.shot('waited');
        return stored;
      },
      async key(key, code, keyCode, modifiers = 0, text) {
        const base = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
        await call('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', ...base, text });
        await call('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
        await v.frames(1);
      },
      async type(text) {
        for (const c of text) {
          const upper = c.toUpperCase();
          await v.key(c, `Key${upper}`, upper.charCodeAt(0), 0, c);
        }
      },
      async openPreferences() {
        await v.key('P', 'KeyP', 80, 2 | 8);
        await v.settle(1000);
        await v.type('Preferences');
        await v.settle(1000);
        await v.key('Enter', 'Enter', 13, 0, '\r');
        await v.settle(1500);
      },
      async shot(label) {
        const { data } = await call('Page.captureScreenshot', { format: 'png' });
        fs.writeFileSync(path.join(out, `${name}-${label}.png`), Buffer.from(data, 'base64'));
        return data;
      },
      // The share of floor pixels that stand out from their neighbours: the grid's lines.
      async lines(label) {
        const data = await v.shot(label);
        return v.eval(`(async () => {
          const image = new Image();
          image.src = 'data:image/png;base64,${data}';
          await image.decode();
          const c = new OffscreenCanvas(image.width, image.height);
          const g = c.getContext('2d');
          g.drawImage(image, 0, 0);
          const f = ${JSON.stringify(FLOOR)};
          const px = g.getImageData(f.x - 3, f.y - 3, f.width + 6, f.height + 6);
          const w = px.width;
          const lum = (x, y) => { const i = (y * w + x) * 4; return 0.3 * px.data[i] + 0.59 * px.data[i + 1] + 0.11 * px.data[i + 2]; };
          let marked = 0;
          for (let y = 3; y < f.height + 3; y++) for (let x = 3; x < f.width + 3; x++) {
            const around = (lum(x - 3, y) + lum(x + 3, y) + lum(x, y - 3) + lum(x, y + 3)) / 4;
            if (Math.abs(lum(x, y) - around) > 4) marked++;
          }
          return marked / (f.width * f.height);
        })()`);
      },
    };
    return v;
  }

  // A, on a new browser: nothing stored yet, then a preset and the grid chosen by hand.
  const a = await visitor('a');
  await a.load();
  check(await a.stored() === null, 'a new browser starts with no settings');
  const shown = await a.lines('first-view');
  console.log(`A first view: ${(shown * 100).toFixed(2)}% of the floor is grid`);
  check(shown > 0.02, `the grid shows on a new browser: ${shown}`);

  await a.openPreferences();
  await a.click(AT.solidworks);
  await a.shot('solidworks');
  check((await a.storedWhen(s => s?.navigation?.preset_id === 'solidworks'))?.navigation?.preset_id === 'solidworks', 'choosing SOLIDWORKS is kept in this browser');
  await a.click(AT.gridAndPlanes);
  await a.click(AT.showGrid);
  await a.shot('grid-hidden');
  check((await a.storedWhen(s => s?.view?.show_grid === false))?.view?.show_grid === false, 'hiding the grid is kept in this browser');

  // The reload reads them back: the grid stays hidden, and a further change saves the
  // preset the app read, not a new browser's Fusion.
  await a.load(true);
  const hidden = await a.lines('reloaded');
  console.log(`A after the reload: ${(hidden * 100).toFixed(2)}% of the floor is grid`);
  check(hidden < shown / 5, `the grid stays hidden after a reload: ${hidden} against ${shown}`);
  await a.openPreferences();
  await a.click(AT.orbitAndZoom);
  await a.click(AT.wheelInvert);
  await a.shot('reloaded-wheel-inverted');
  const kept = await a.storedWhen(s => s?.navigation?.wheel?.invert === true);
  check(kept?.navigation?.wheel?.invert === true, 'the wheel choice is kept');
  check(kept?.navigation?.preset_id === 'solidworks', `the reloaded app still follows SOLIDWORKS: ${kept?.navigation?.preset_id}`);

  // B, on the same site at the same time: its own empty storage, and the grid shown.
  const b = await visitor('b');
  await b.load();
  check(await b.stored() === null, 'another browser sees none of the first one\'s settings');
  const other = await b.lines('first-view');
  console.log(`B first view: ${(other * 100).toFixed(2)}% of the floor is grid`);
  check(other > 0.02, `the other browser shows the grid: ${other}`);
  await b.openPreferences();
  await b.click(AT.onshape);
  check((await b.storedWhen(s => s?.navigation?.preset_id === 'onshape'))?.navigation?.preset_id === 'onshape', 'the other browser keeps its own choice');
  await a.front();
  check((await a.stored())?.navigation?.preset_id === 'solidworks', 'and the first browser keeps its own');

  // Network requests only: the measuring above decodes frames from data: URLs.
  const network = requests.filter(r => /^(https?|wss?):/.test(r.url));
  const sent = network.filter(r => r.method !== 'GET' || !r.url.startsWith(new URL(url).origin));
  check(network.length > 0, 'the requests were recorded');
  check(sent.length === 0, `settings never leave the browser: ${JSON.stringify(sent.map(r => [r.method, r.url]))}`);
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
