// Chromium input and read-only canvas observations shared by the browser scenarios.
import fs from 'node:fs';
import path from 'node:path';
import { startChrome, stopChrome } from './chrome.mjs';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function check(value, message) { if (!value) throw new Error(message); }

export class CanvasBrowser {
  constructor({ chrome, profile, size, url, out, downloads, extra }) {
    this.site = { size, url, out, downloads };
    this.profile = profile;
    this.process = startChrome(chrome, [
      '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
      '--no-first-run', '--no-default-browser-check', `--window-size=${size.width},${size.height}`, ...extra, 'about:blank',
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
    const { size, url, out, downloads } = this.site;
    const { targetId } = await this.send('Target.createTarget', { url: 'about:blank' });
    const { sessionId } = await this.send('Target.attachToTarget', { targetId, flatten: true });
    const call = (method, params) => this.send(method, params, sessionId);
    const events = this.events;
    await call('Page.enable');
    await call('Emulation.setDeviceMetricsOverride', { ...size, deviceScaleFactor: 1, mobile: false });
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
      async load(timeout = Infinity) {
        await call('Page.bringToFront');
        await call('Page.navigate', { url });
        const deadline = Date.now() + timeout;
        for (let t = 0; t < 240 && Date.now() < deadline; t++) {
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
      // A click on a face in the viewport. The pointer rests on it first, so the id
      // buffer read back around it is current, and the press and release go together:
      // a software renderer can take a second a frame, and a press held across frames
      // reads as the start of a drag.
      async pick([x, y]) {
        await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
        await v.settle(1500);
        await call('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
        await call('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
        await v.settle(1000);
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
      // Runs `trigger`, which opens a file dialog, and answers the dialog with `files`.
      async choose(files, trigger) {
        await call('Page.setInterceptFileChooserDialog', { enabled: true });
        const from = events.length;
        await trigger();
        for (let t = 0; t < 100; t++) {
          const opened = events.slice(from).find(e => e.method === 'Page.fileChooserOpened');
          if (opened) {
            await call('DOM.setFileInputFiles', { files: [files].flat().map(f => path.resolve(f)), backendNodeId: opened.params.backendNodeId });
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
      async download(label, trigger, timeout = Infinity) {
        const from = events.length;
        await trigger();
        const deadline = Date.now() + timeout;
        for (let t = 0; t < 600 && Date.now() < deadline; t++) {
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
      // The assembly as the last frame showed it, once `done` says it is what the step
      // waits for.
      async assembly(label, done) {
        let shown = null;
        for (let t = 0; t < 900; t++) {
          await v.stopped(label);
          const text = await v.eval(`window.wasmBindings.shown_assembly()`);
          shown = text ? JSON.parse(text) : null;
          if (shown && done(shown)) return shown;
          await sleep(200);
        }
        await v.shot(`assembly-${label}`);
        throw new Error(`${label}: the assembly is ${JSON.stringify(shown)}`);
      },
      // The centre of the control `name` in the last frame, once it is drawn.
      async control(name, timeout = 10000) {
        const deadline = Date.now() + timeout;
        while (Date.now() < deadline) {
          const text = await v.eval(`window.wasmBindings.shown_controls()`);
          const at = text ? JSON.parse(text)[name] : null;
          if (at) return [at.x, at.y];
          await sleep(100);
        }
        await v.shot(`no-control-${name.replace(/\W+/g, '-')}`);
        throw new Error(`no control named ${name} is drawn`);
      },
      // Where a point of the assembly was drawn, in the page's pixels.
      async screen(x, y, z) {
        const at = await v.eval(`(() => { const p = window.wasmBindings.assembly_screen_point(${x}, ${y}, ${z}); return p ? Array.from(p) : null; })()`);
        check(at, `the point (${x}, ${y}, ${z}) is not on screen`);
        return at;
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
