// Assemblies in the browser build, driven in headless Chrome as a visitor drives them.
// Usage: node ci/browser-assemblies.mjs CHROME URL OUTDIR TRAY COLLAR RIG BLOCK SHAFT BEARING
//
// TRAY and COLLAR are part files (site/parts/tray.dwprt and collar.dwprt). The page has no
// folder, so every part reaches it as a file handed over. The visitor starts a new
// assembly from the start page, inserts both parts with one upload and the tray again
// with a second, and mates the second tray's underside onto the first tray's floor, picked
// in the viewport from above and from below: the later feature on top, which needs each
// tray's exact solid from the bytes the page holds. The visitor saves the assembly (a
// download), loads the site again and opens the saved file with the two part files in
// one upload: the same three components at the same placements, the mate kept, every
// part found at the revision it pins. Last, the saved file opens alone: the parts still
// show, from the copies the file keeps, and each says its file cannot be found.
//
// RIG, BLOCK and SHAFT are ci/fixtures/universal-joint: two shafts on hinges in two blocks
// locked 40 degrees apart. The visitor opens the rig with both parts, chooses Universal
// Joint in the Mate panel's type list and picks the input shaft, then the output shaft,
// seen from the right. Then one later feature on top: the input turned 30 degrees through
// its Rotate Z placement field drives the output by the Hooke relation,
// tan(output) = tan(input) / cos(40 degrees). The joint is saved and reopened with its
// parts.
//
// Last, a parent assembly that places the saved tray assembly is opened from one upload
// that lists that child first, with the tray and collar parts. Neither opens on its own:
// the page asks which one to open, the visitor chooses the parent, and it opens with the
// child and its parts found. Then one later feature on top: the tray inserted in the
// parent the choice opened. Then the same upload again with the choice left unanswered,
// and the child opened alone through the start page's Open tile, which the choice window leaves uncovered: the old choice is gone, so it cannot
// open over the child.
//
// BEARING is ci/fixtures/universal-joint/bearing.dwasm: the rig with its Universal Joint
// made and the lock between the blocks deleted, so the output block turns freely. The
// visitor selects that block and types 90 degrees into its Rotate X placement field,
// which would set the shafts at right angles. The drag is refused by name and leaves the
// placements, the mates and undo as they were. Then one later feature on top: 60 degrees
// in the same field is taken, the output shaft follows its bore, and Undo puts it back.
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. The canvas has no text to read, so the
// check reads the assembly through the read-only `shown_assembly` export, and finds a
// face's place on screen through `assembly_screen_point`. Commands run through the
// command palette by name; Enter adds the mate the Mate panel holds. The clicks go to the
// start page's New assembly and Open tiles at fixed points of a 1400 x 813 page. Other
// controls are found through the read-only `shown_controls` export.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, out, tray, collar, rig, block, shaft, bearing] = process.argv.slice(2);
if (!chrome || !url || !out || !tray || !collar || !rig || !block || !shaft || !bearing) {
  throw new Error('usage: browser-assemblies.mjs CHROME URL OUTDIR TRAY COLLAR RIG BLOCK SHAFT BEARING');
}
fs.mkdirSync(out, { recursive: true });
const downloads = path.join(path.resolve(out), 'downloads');
fs.rmSync(downloads, { recursive: true, force: true });
fs.mkdirSync(downloads, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const NEW_ASSEMBLY = [304, 145];
const OPEN = [663, 147];
// The tray is 60 x 40 x 20 mm, open at the top, with 2 mm walls: its floor is 2 mm up.
const FLOOR = 2;

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
const started = Date.now();
function step(what) { console.error(`[${((Date.now() - started) / 1000).toFixed(1)} s] ${what}`); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

const loaded = shown => shown.open && !shown.opening && !shown.loading;
const parts = shown => shown.components.map(c => c.label.replace(/<.*/, ''));
function placements(shown) { return shown.components.map(c => [c.translation, c.rotation]); }
function near(a, b) { const [p, q] = [a.flat(2), b.flat(2)]; return p.length === q.length && p.every((x, i) => Math.abs(x - q[i]) < 1e-6); }
// Quaternions as [x, y, z, w], as the export gives them.
function multiply([ax, ay, az, aw], [bx, by, bz, bw]) {
  return [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz];
}
function about([x, y, z], angle) { const s = Math.sin(angle / 2); return [x * s, y * s, z * s, Math.cos(angle / 2)]; }
function rotate(q, [x, y, z]) {
  const [, rx, ry, rz] = [0, ...multiply(multiply(q, [x, y, z, 0]), [-q[0], -q[1], -q[2], q[3]])];
  return [rx, ry, rz];
}
// Two turns agree when they take X and Y to the same places.
function sameTurn(p, q, tolerance) {
  return [[1, 0, 0], [0, 1, 0]].every(axis => rotate(p, axis).every((x, i) => Math.abs(x - rotate(q, axis)[i]) < tolerance));
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
      async control(name) {
        for (let t = 0; t < 100; t++) {
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

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-assemblies-'));
let browser;

async function main() {
  browser = await new Browser(profile).connect();
  const v = await browser.visitor();
  step('load the site');
  await v.load();
  check(await v.eval(`typeof window.wasmBindings.shown_assembly === 'function' && typeof window.wasmBindings.assembly_screen_point === 'function'`), 'the build has no assembly exports');

  // New assembly from the start page, then both parts in one upload.
  step('new assembly');
  await v.click(NEW_ASSEMBLY);
  await v.assembly('new', s => s.open && s.active && s.components.length === 0);
  step('insert the tray and the collar');
  await v.choose([tray, collar], () => v.command('Insert component'));
  let shown = await v.assembly('inserted', s => loaded(s) && s.components.length === 2);
  check(JSON.stringify(parts(shown)) === '["tray","collar"]', `the components are ${parts(shown)}`);
  check(shown.components.every(c => c.state === 'DisplayReady'), `not every part shows: ${JSON.stringify(shown.components)}`);
  check(JSON.stringify(shown.held) === '["tray.dwprt","collar.dwprt"]', `the page holds ${shown.held}`);
  step('insert the tray again');
  await v.choose(tray, () => v.command('Insert component'));
  shown = await v.assembly('inserted again', s => loaded(s) && s.components.length === 3);
  check(JSON.stringify(parts(shown)) === '["tray","collar","tray"]', `the components are ${parts(shown)}`);
  await v.shot('1-inserted');

  // The later feature on top: a mate, picked in the viewport.
  step('mate the second tray onto the first one\'s floor');
  await v.command('Mate');
  await v.assembly('mate panel', s => s.mate_panel.open);
  const [first, , second] = shown.components.map(c => c.translation);
  await v.command('Top');
  await v.command('Fit');
  await v.pick(await v.screen(first[0], first[1], first[2] + FLOOR));
  shown = await v.assembly('first pick', s => s.mate_panel.a);
  await v.command('Bottom');
  await v.command('Fit');
  await v.pick(await v.screen(second[0], second[1], second[2]));
  shown = await v.assembly('second pick', s => s.mate_panel.b);
  check(shown.mate_panel.a.component !== shown.mate_panel.b.component, `both picks are on one component: ${JSON.stringify(shown.mate_panel)}`);
  await v.shot('2-picked');
  await v.key('Enter', 'Enter', 13, 0, '\r');
  shown = await v.assembly('mated', s => s.mates === 1 || s.mate_panel.error);
  check(shown.mates === 1, `the mate was refused: ${shown.mate_panel.error}`);
  const [a, , b] = shown.components.map(c => c.translation);
  check(Math.abs(b[2] - a[2] - FLOOR) < 1e-9, `the second tray stands ${b[2] - a[2]} mm above the first, not ${FLOOR}`);
  await v.command('Isometric');
  await v.command('Fit');
  await v.shot('3-mated');
  const mated = shown;

  // Save (a download), load the site afresh, open the file with its parts.
  step('save');
  const saved = await v.download('save', () => v.key('s', 'KeyS', 83, 2, 's'));
  check(saved.endsWith('.dwasm'), `the assembly saves as a .dwasm file, not ${saved}`);
  shown = await v.assembly('saved', s => !s.dirty);
  step('load the site again and open the saved assembly with its parts');
  await v.load();
  await v.choose([saved, tray, collar], () => v.click(OPEN));
  shown = await v.assembly('reopened', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(parts(shown)) === JSON.stringify(parts(mated)), `the reopened components are ${parts(shown)}`);
  check(near(placements(shown), placements(mated)), `the reopened placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(placements(mated))}`);
  check(shown.mates === 1, `the reopened assembly has ${shown.mates} mates`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every part is found: ${JSON.stringify(shown.components)}`);
  await v.shot('4-reopened');
  const reopened = shown;

  // The file alone: its copies show, and each part says its file is not here.
  step('load the site again and open the saved assembly alone');
  await v.load();
  await v.choose(saved, () => v.click(OPEN));
  shown = await v.assembly('alone', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(near(placements(shown), placements(mated)), `alone, the placements are ${JSON.stringify(placements(shown))}`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'missing' && c.warning), `alone, the parts are ${JSON.stringify(shown.components)}`);
  await v.settle(1000);
  await v.stopped('at the end');
  await v.shot('5-alone');
  const alone = shown;

  const joint = await universalJoint(v);
  const choice = await chosenAssembly(v, saved, [tray, collar], parts(mated));
  const bearingDrag = await refusedDrag(v);

  fs.writeFileSync(path.join(out, 'assemblies.json'), JSON.stringify({ mated, reopened, alone, saved: path.basename(saved), joint, choice, bearing: bearingDrag }, null, 1));
  console.log(`Assemblies in the browser: three components from two uploads, a mate picked in the viewport putting the second tray ${FLOOR} mm up on the first one's floor, saved as ${path.basename(saved)}, reopened with its parts at the same placements with the mate kept, and opened alone from the copies it keeps. A Universal Joint picked between two shafts meeting at ${BEND} degrees drove the output ${joint.follows.toFixed(6)} degrees for a ${TURN} degree input turn, and reopened with its parts. A parent and its child assembly uploaded child first opened the parent the visitor chose, with ${choice.chosen.held.length} files held for it, and took a tray inserted on top. Turning the joint's free output block to set the shafts at right angles was refused by name with nothing changed, and a 60 degree turn after it was taken and undone.`);
}

// The rig's shafts: the input (component 4) on a hinge at (10, 5, 0) along Z, the output
// (component 9) in a block turned 40 degrees about X.
const BEND = 40;
const TURN = 30;
const INPUT = '4';
const OUTPUT = '9';

async function universalJoint(v) {
  step('load the site again and open the Universal Joint rig with its parts');
  await v.load();
  await v.choose([rig, block, shaft], () => v.click(OPEN));
  let shown = await v.assembly('rig', s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(parts(shown)) === '["block","shaft","block","shaft"]', `the rig's components are ${parts(shown)}`);
  check(shown.mates === 4, `the rig has ${shown.mates} mates, not 4`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every rig part is found: ${JSON.stringify(shown.components)}`);
  const component = (s, at) => s.components.find(c => c.path === at);
  const start = component(shown, OUTPUT);
  const tilt = about([1, 0, 0], BEND * Math.PI / 180);
  check(sameTurn(start.rotation, tilt, 1e-9), `the output shaft starts at ${start.rotation}, not ${BEND} degrees about X`);

  step('choose Universal Joint in the Mate panel');
  await v.command('Mate');
  await v.assembly('mate panel', s => s.mate_panel.open);
  await v.click(await v.control('Mate type'));
  await v.click(await v.control('Mate type: Universal Joint'));
  await v.shot('6-universal-joint-chosen');

  // Each shaft's side at the middle of its axis, seen from the right: from the front the
  // output shaft stands in front of the input.
  step('pick the input shaft, then the output shaft');
  await v.command('Right');
  await v.command('Fit');
  const middle = rotate(tilt, [0, 0, 22]).map((x, i) => x + start.translation[i]);
  await v.pick(await v.screen(10, 5, 22));
  shown = await v.assembly('input pick', s => s.mate_panel.a);
  check(shown.mate_panel.a.component === Number(INPUT) && shown.mate_panel.a.face.startsWith('Boss-Extrude1'), `the first pick is ${JSON.stringify(shown.mate_panel.a)}`);
  await v.pick(await v.screen(...middle));
  shown = await v.assembly('output pick', s => s.mate_panel.b);
  check(shown.mate_panel.b.component === Number(OUTPUT) && shown.mate_panel.b.face.startsWith('Boss-Extrude1'), `the second pick is ${JSON.stringify(shown.mate_panel.b)}`);
  await v.shot('7-shafts-picked');
  await v.key('Enter', 'Enter', 13, 0, '\r');
  shown = await v.assembly('joined', s => s.mates === 5 || s.mate_panel.error);
  check(shown.mates === 5, `the Universal Joint was refused: ${shown.mate_panel.error}`);
  await v.key('Escape', 'Escape', 27);
  await v.assembly('mate panel closed', s => !s.mate_panel.open);

  // The later feature on top: the input turned through its placement field, under mates.
  step(`turn the input shaft ${TURN} degrees through its placement field`);
  await v.pick(await v.screen(10, 5, 22));
  await v.assembly('input selected', s => s.selected === INPUT);
  await v.click(await v.control('Assembly options'));
  await v.click(await v.control('Placement Rotate Z'));
  await v.key('a', 'KeyA', 65, 2, 'a');
  for (const ch of String(TURN)) {
    await v.key(ch, `Digit${ch}`, ch.charCodeAt(0), 0, ch);
  }
  await v.key('Enter', 'Enter', 13, 0, '\r');
  const turn = TURN * Math.PI / 180;
  const input = about([0, 0, 1], turn);
  shown = await v.assembly('driven', s => sameTurn(component(s, INPUT).rotation, input, 1e-7));
  const bend = BEND * Math.PI / 180;
  const follows = Math.atan2(Math.sin(turn), Math.cos(turn) * Math.cos(bend));
  const expected = multiply(about(rotate(tilt, [0, 0, 1]), follows), start.rotation);
  check(sameTurn(component(shown, OUTPUT).rotation, expected, 1e-7), `the output turned to ${component(shown, OUTPUT).rotation}, not ${expected}`);
  await v.shot('8-driven');
  const driven = shown;

  step('save the joint and open it again with its parts');
  const saved = await v.download('save joint', () => v.key('s', 'KeyS', 83, 2, 's'));
  await v.assembly('joint saved', s => !s.dirty);
  await v.load();
  await v.choose([saved, block, shaft], () => v.click(OPEN));
  shown = await v.assembly('joint reopened', s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked'));
  check(shown.mates === 5, `the reopened rig has ${shown.mates} mates, not 5`);
  check(near(placements(shown), placements(driven)), `the reopened placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(placements(driven))}`);
  await v.stopped('joint at the end');
  await v.shot('9-joint-reopened');
  return { driven, reopened: shown, follows: follows * 180 / Math.PI, saved: path.basename(saved) };
}

// A parent assembly placing the assembly `saved`, opened from one upload that lists that
// child first, with `files`, the part files of the components the child places, named in
// `placed`. Neither assembly opens until the visitor chooses the parent.
async function chosenAssembly(v, saved, files, placed) {
  const named = file => path.basename(file);
  // The parent downloads under the name the child did, so each gets a name of its own.
  const child = path.join(downloads, 'child.dwasm');
  fs.copyFileSync(saved, child);
  step('load the site again and start a parent assembly that places a saved assembly');
  await v.load();
  await v.click(NEW_ASSEMBLY);
  await v.assembly('parent new', s => s.open && s.active && s.components.length === 0);
  await v.choose(child, () => v.command('Insert component'));
  await v.assembly('parent', s => loaded(s) && s.components.length === 1 && s.components.every(c => c.freshness !== 'unchecked'));
  const made = await v.download('save parent', () => v.key('s', 'KeyS', 83, 2, 's'));
  await v.assembly('parent saved', s => !s.dirty);
  const parent = path.join(downloads, 'parent.dwasm');
  fs.renameSync(made, parent);

  step('load the site again and open the child, the parent and the parts, the child first');
  await v.load();
  await v.choose([child, parent, ...files], () => v.click(OPEN));
  let shown = await v.assembly('choosing', s => !s.open && !s.opening && s.choosing?.length > 0);
  check(JSON.stringify(shown.choosing) === '["child.dwasm","parent.dwasm"]', `the page offers ${shown.choosing}`);
  check(shown.held.length === 0, `the page holds ${shown.held} before a choice`);
  await v.settle(1000);
  const choosing = JSON.parse(await v.eval(`window.wasmBindings.shown_assembly()`));
  check(!choosing.open && !choosing.opening, `an assembly opened before the choice: ${JSON.stringify(choosing)}`);
  await v.shot('10-choose');

  step('choose the parent');
  await v.click(await v.control('Open assembly: parent.dwasm'));
  const checked = c => c.freshness !== 'unchecked' && c.components.every(checked);
  shown = await v.assembly('chosen', s => loaded(s) && s.components.length === 1 && s.components.every(checked));
  check(shown.file === 'parent.dwasm', `${shown.file} opened, not parent.dwasm`);
  check(shown.choosing.length === 0, `the page still offers ${shown.choosing}`);
  const held = JSON.stringify(['child.dwasm', ...files.map(named)]);
  check(JSON.stringify(shown.held) === held, `the page holds ${shown.held}, not ${held}`);
  // A subassembly has no mesh of its own: its structure is read, and its parts are drawn.
  const [sub] = shown.components;
  check(sub.state === 'MetadataOnly' && sub.freshness === 'current' && !sub.warning, `the child is ${JSON.stringify(sub)}`);
  const inside = JSON.stringify(parts(sub));
  check(inside === JSON.stringify(placed), `the child places ${inside}, not ${JSON.stringify(placed)}`);
  check(sub.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every part in the child is found: ${JSON.stringify(sub.components)}`);
  await v.shot('11-chosen');
  const chosen = shown;

  // One later feature on top: a part inserted in the parent the choice opened.
  const part = files[0];
  step(`insert ${named(part)} in the parent`);
  await v.choose(part, () => v.command('Insert component'));
  shown = await v.assembly('inserted', s => loaded(s) && s.components.length === 2 && s.components.every(c => c.freshness !== 'unchecked'));
  check(shown.file === 'parent.dwasm' && shown.dirty, `the part went to ${shown.file}, dirty ${shown.dirty}`);
  const stem = named(part).replace(/\.dwprt$/, '');
  check(parts(shown)[1] === stem && shown.components[1].freshness === 'current', `the inserted component is ${JSON.stringify(shown.components[1])}`);
  await v.stopped('chosen at the end');
  await v.shot('12-inserted');
  const inserted = shown;

  step('open the child and the parent again, leave the choice, and open the child alone');
  await v.load();
  await v.choose([child, parent, ...files], () => v.click(OPEN));
  await v.assembly('choosing again', s => !s.open && !s.opening && s.choosing?.length === 2);
  await v.choose([child, ...files], () => v.click(OPEN));
  await v.assembly('child alone', s => loaded(s) && s.file === 'child.dwasm' && s.components.every(c => c.freshness !== 'unchecked'));
  await v.settle(1000);
  const replaced = JSON.parse(await v.eval(`window.wasmBindings.shown_assembly()`));
  const offered = Object.keys(JSON.parse(await v.eval(`window.wasmBindings.shown_controls()`))).filter(n => n.startsWith('Open assembly'));
  check(replaced.file === 'child.dwasm', `${replaced.file} is open, not child.dwasm`);
  check(replaced.choosing.length === 0 && offered.length === 0, `the old choice is still offered: ${JSON.stringify(replaced.choosing)}, buttons ${JSON.stringify(offered)}`);
  await v.stopped('choice replaced');
  await v.shot('13-choice-replaced');
  return { choosing, chosen, inserted, replaced };
}

// The bearing fixture: the rig with its Universal Joint U made and the lock between the
// blocks deleted, so the output block (component 7) turns freely and carries the output
// shaft on its hinge.
const BEARING = '7';
// The note shows a refusal as its code, then its message.
const REFUSAL = 'invalid: a universal joint locks with its shafts at right angles; set the shafts at another angle';

// Types `degrees` into the placement field `name` of the selected component, and Enter.
async function placementField(v, name, degrees) {
  await v.click(await v.control(name));
  await v.key('a', 'KeyA', 65, 2, 'a');
  for (const ch of String(degrees)) {
    await v.key(ch, `Digit${ch}`, ch.charCodeAt(0), 0, ch);
  }
  await v.key('Enter', 'Enter', 13, 0, '\r');
}

async function refusedDrag(v) {
  step('load the site again and open the joint whose output block turns freely');
  await v.load();
  await v.choose([bearing, block, shaft], () => v.click(OPEN));
  let shown = await v.assembly('bearing', s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(shown.mate_names) === '["O","H","H2","U"]', `the joint's mates are ${shown.mate_names}`);
  const component = (s, at) => s.components.find(c => c.path === at);
  const tilt = about([1, 0, 0], BEND * Math.PI / 180);
  check(sameTurn(component(shown, BEARING).rotation, tilt, 1e-9), `the bearing block starts at ${component(shown, BEARING).rotation}`);

  // A corner of the block's top, clear of its shaft, seen from above.
  step('select the bearing block');
  await v.command('Top');
  await v.command('Fit');
  const corner = rotate(tilt, [-12, -8, 10]).map((x, i) => x + component(shown, BEARING).translation[i]);
  await v.pick(await v.screen(...corner));
  const before = await v.assembly('bearing selected', s => s.selected === BEARING);
  check(before.note?.kind !== 'Error', `an error shows before the drag: ${before.note?.text}`);
  await v.click(await v.control('Assembly options'));

  step('turn the bearing block to 90 degrees about X, which would set the shafts at right angles');
  await placementField(v, 'Placement Rotate X', 90);
  shown = await v.assembly('refused', s => s.note?.kind === 'Error');
  check(shown.note.text === REFUSAL, `the refusal says "${shown.note.text}", not "${REFUSAL}"`);
  check(JSON.stringify(placements(shown)) === JSON.stringify(placements(before)), `the refused drag moved ${JSON.stringify(placements(shown))} from ${JSON.stringify(placements(before))}`);
  check(JSON.stringify(shown.mate_names) === JSON.stringify(before.mate_names), `the refused drag changed the mates to ${shown.mate_names}`);
  check(JSON.stringify(shown.undo) === JSON.stringify(before.undo), `the refused drag changed undo to ${JSON.stringify(shown.undo)} from ${JSON.stringify(before.undo)}`);
  check(shown.dirty === before.dirty, `the refused drag left the joint dirty: ${shown.dirty}`);
  await v.shot('14-refused');
  const refused = shown;

  // One later feature on top: the same field takes 60 degrees, the output shaft's hinge
  // carries it along, and Undo puts the block back.
  step('turn the bearing block to 60 degrees about X instead');
  await placementField(v, 'Placement Rotate X', 60);
  shown = await v.assembly('turned', s => s.undo.depth === before.undo.depth + 1 || (s.note?.kind === 'Error' && s.note.text !== REFUSAL));
  check(shown.undo.depth === before.undo.depth + 1, `the 60 degree turn was not taken: ${JSON.stringify(shown.note)}`);
  const sixty = about([1, 0, 0], 60 * Math.PI / 180);
  check(sameTurn(component(shown, BEARING).rotation, sixty, 1e-9), `the bearing block turned to ${component(shown, BEARING).rotation}`);
  const axis = rotate(component(shown, OUTPUT).rotation, [0, 0, 1]);
  const bore = rotate(sixty, [0, 0, 1]);
  check(axis.every((x, i) => Math.abs(x - bore[i]) < 1e-9), `the output shaft points along ${axis}, not its bore ${bore}`);
  await v.shot('15-turned');
  const turned = shown;

  step('undo the turn');
  await v.command('Undo');
  shown = await v.assembly('undone', s => s.undo.depth === before.undo.depth);
  check(near(placements(shown), placements(before)), `after Undo the placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(placements(before))}`);
  check(JSON.stringify(shown.mate_names) === JSON.stringify(before.mate_names), `after Undo the mates are ${shown.mate_names}`);
  await v.stopped('bearing at the end');
  return { refused, turned, undone: shown };
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
