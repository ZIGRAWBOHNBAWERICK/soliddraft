// Recent documents and crash recovery in the browser build, driven in headless Chrome as
// a visitor drives them. Usage: node ci/browser-documents.mjs CHROME URL OUTDIR
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol.
//
// Visitor A opens the site on the start page, makes a part, saves it (a download) and
// edits it again. Autosave keeps a recovery copy in the browser. Chrome is then killed
// with SIGKILL in the middle of the editing and started again on the same profile: the
// start page lists the saved part, and the Recover documents window offers the unsaved
// work. Visitor B, another browser profile on the same site at the same time, sees no
// recent documents and nothing to recover. Nothing is sent to the server.
//
// Run Chrome on WebGL: a headless screenshot cannot read a WebGPU canvas. The click goes
// to the New part tile at a fixed point of a 1400 x 813 page; tests/documents_start.rs
// checks the tile is there.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const [chrome, url, out] = process.argv.slice(2);
if (!chrome || !url || !out) throw new Error('usage: browser-documents.mjs CHROME URL OUTDIR');
fs.mkdirSync(out, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const NEW_PART = [124, 143];
// A copy after every edit, so the test does not wait out the two-minute default.
const FAST_AUTOSAVE = JSON.stringify({ documents: { autosave: { enabled: true, interval_seconds: 1, edits: 1 } } });

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

// What the page keeps in IndexedDB, as plain data.
const STORED = `(async () => {
  const d = await new Promise((resolve, reject) => {
    const r = indexedDB.open('soliddraft-documents', 1);
    r.onupgradeneeded = () => { r.result.createObjectStore('recovery'); r.result.createObjectStore('recent'); };
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const all = (store, what) => new Promise((resolve, reject) => {
    const r = d.transaction(store).objectStore(store)[what]();
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  const recovery = await all('recovery', 'getAllKeys');
  const recent = (await all('recent', 'getAll')).map(e => ({ name: e.name, kind: e.kind, bytes: e.bytes.length }));
  d.close();
  return { recovery, recent };
})()`;

class Browser {
  constructor(profile) {
    this.profile = profile;
    this.process = spawn(chrome, [
      '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
      '--no-first-run', '--no-default-browser-check', `--window-size=${SIZE.width},${SIZE.height}`, ...extra, 'about:blank',
    ], { stdio: ['ignore', 'ignore', 'pipe'] });
    this.stderr = '';
    this.process.stderr.on('data', d => { this.stderr += d; });
    this.requests = [];
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
      }
      if (m.method === 'Network.requestWillBeSent') this.requests.push(m.params.request);
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

  // A tab: in this profile's own storage, or in a fresh profile of its own when `separate`.
  async visitor(name, separate) {
    const context = separate ? await this.send('Target.createBrowserContext') : {};
    const { targetId } = await this.send('Target.createTarget', { url: 'about:blank', ...context });
    const { sessionId } = await this.send('Target.attachToTarget', { targetId, flatten: true });
    const call = (method, params) => this.send(method, params, sessionId);
    await call('Page.enable');
    await call('Network.enable');
    await call('Emulation.setDeviceMetricsOverride', { ...SIZE, deviceScaleFactor: 1, mobile: false });
    // The app draws on animation frames, which a page in the background does not get.
    await call('Emulation.setFocusEmulationEnabled', { enabled: true });
    await call('Page.bringToFront');
    // Saving downloads the file; the test needs only what the page keeps.
    await this.send('Browser.setDownloadBehavior', { behavior: 'deny', ...context });
    const v = {
      name,
      async eval(expression) {
        const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
        if (r.exceptionDetails) throw new Error(`${name}: ${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
        return r.result.value;
      },
      async load(address = url) {
        await call('Page.bringToFront');
        await call('Page.navigate', { url: address });
        for (let t = 0; t < 240; t++) {
          await sleep(500);
          const ready = await v.eval(`document.readyState === 'complete' && !!document.querySelector('#status.hidden')`).catch(() => false);
          if (ready) {
            await sleep(3000);
            return;
          }
        }
        await v.shot('not-started');
        throw new Error(`${name}: the app did not start`);
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
        await v.settle(800);
      },
      // E opens Extrude on its default rectangle; Enter applies it.
      async extrude() {
        await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 700, y: 450 });
        await v.key('e', 'KeyE', 69, 0, 'e');
        await v.settle(1500);
        await v.key('Enter', 'Enter', 13, 0, '\r');
        await v.settle(3000);
      },
      async stored() {
        return v.eval(STORED);
      },
      async storedWhen(label, test) {
        let s;
        for (let t = 0; t < 60; t++) {
          s = await v.stored();
          if (test(s)) return s;
          await sleep(500);
        }
        await v.shot(`waited-${label}`);
        throw new Error(`${name}: ${label}: ${JSON.stringify(s)}`);
      },
      async shot(label) {
        const { data } = await call('Page.captureScreenshot', { format: 'png' });
        fs.writeFileSync(path.join(out, `${name}-${label}.png`), Buffer.from(data, 'base64'));
      },
    };
    return v;
  }

  close() {
    this.ws?.close();
    this.process.kill();
  }
}

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-documents-'));
const open = [];

async function main() {
  // A, on a new browser: a fast autosave set as Preferences would keep it.
  let browser = await new Browser(profile).connect();
  open.push(browser);
  const a = await browser.visitor('a');
  await a.load();
  await a.eval(`localStorage.setItem('soliddraft-settings', ${JSON.stringify(FAST_AUTOSAVE)})`);
  await a.load();
  await a.shot('1-start-page-new-browser');
  const first = await a.stored();
  check(first.recent.length === 0 && first.recovery.length === 0, `a new browser keeps no documents: ${JSON.stringify(first)}`);

  await a.click(NEW_PART);
  await a.extrude();
  await a.storedWhen('the edit has a recovery copy', s => s.recovery.length === 1);
  await a.shot('2-edited');
  // Ctrl+S: the part downloads, and the browser keeps it for the start page.
  await a.key('s', 'KeyS', 83, 2, 's');
  const saved = await a.storedWhen('the saved part is recent, its copy gone', s => s.recent.length === 1 && s.recovery.length === 0);
  check(saved.recent[0].kind === 'dwprt' && saved.recent[0].bytes > 0, `the part file itself is kept: ${JSON.stringify(saved.recent)}`);
  await a.extrude();
  const editing = await a.storedWhen('the new edit has a recovery copy', s => s.recovery.length === 1);
  console.log(`A before the kill: ${JSON.stringify(editing)}`);
  await a.shot('3-unsaved-edit');

  // Killed mid-edit: no page teardown runs.
  await a.key('e', 'KeyE', 69, 0, 'e');
  browser.process.kill('SIGKILL');
  await new Promise(resolve => browser.process.once('exit', resolve));
  open.pop();

  // The same browser again.
  browser = await new Browser(profile).connect();
  open.push(browser);
  const again = await browser.visitor('a-again');
  await again.load();
  await again.settle(2000);
  await again.shot('4-after-kill');
  const after = await again.stored();
  console.log(`A after the kill: ${JSON.stringify(after)}`);
  check(after.recovery.length === 1 && after.recovery[0].includes('/offered/'), `the unsaved work is offered: ${JSON.stringify(after.recovery)}`);
  check(after.recent.length === 1 && after.recent[0].name === 'Part1.dwprt', `the saved part is still recent: ${JSON.stringify(after.recent)}`);

  // B: another browser profile on the same site, at the same time.
  const b = await browser.visitor('b', true);
  await b.load();
  await b.shot('5-other-browser');
  const other = await b.stored();
  check(other.recent.length === 0 && other.recovery.length === 0, `another browser sees none of A's documents: ${JSON.stringify(other)}`);
  await again.eval('1');
  check((await again.stored()).recovery.length === 1, 'and A keeps its own');

  const network = browser.requests.filter(r => /^(https?|wss?):/.test(r.url));
  const sent = network.filter(r => r.method !== 'GET' || !r.url.startsWith(new URL(url).origin));
  check(network.length > 0, 'the requests were recorded');
  check(sent.length === 0, `documents never leave the browser: ${JSON.stringify(sent.map(r => [r.method, r.url]))}`);
}

try {
  await main();
  console.log('PASS documents kept per browser, offered after a kill, never shared');
} catch (error) {
  console.error(error.stack || error);
  for (const b of open) console.error(b.stderr.split('\n').slice(-20).join('\n'));
  process.exitCode = 1;
} finally {
  for (const b of open) b.close();
}
