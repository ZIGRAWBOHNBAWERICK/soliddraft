// Exercise the four drawing annotation tools with trusted Chrome input.
// The part was saved by drawing_annotations_follow_the_part_through_save_and_reopen
// before the plate edit. Its 6 mm bevel is visible in Top view.
// Usage: node ci/browser-drawing-annotations.mjs CHROME URL OUTDIR PART
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { startChrome, stopChrome } from './chrome.mjs';

const [chrome, url, output, part] = process.argv.slice(2);
assert(chrome && url && output && part, 'expected CHROME URL OUTDIR PART');
const out = path.resolve(output);
const downloads = path.join(out, 'downloads');
fs.mkdirSync(downloads, { recursive: true });
const profile = fs.mkdtempSync(path.join(out, 'chrome-'));
const viewport = { width: 1440, height: 900 };
const child = startChrome(chrome, [
  '--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
  ...((process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean)), 'about:blank',
]);
child.stderr.pipe(fs.createWriteStream(path.join(out, 'chrome.log')));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(label, read, ms = 120_000) {
  const deadline = Date.now() + ms;
  let result;
  do {
    result = await read();
    if (result) return result;
    await sleep(200);
  } while (Date.now() < deadline);
  throw new Error(`Timed out waiting for ${label}`);
}

let send;
let call;
try {
  const portFile = path.join(profile, 'DevToolsActivePort');
  const port = await until('Chrome', () => fs.existsSync(portFile) && fs.readFileSync(portFile, 'utf8').split('\n')[0]);
  const endpoint = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  const socket = new WebSocket(endpoint.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let nextId = 0;
  let chooser;
  let download;
  const pending = new Map();
  socket.onmessage = ({ data }) => {
    const { id, result, error, method, params } = JSON.parse(data);
    if (method === 'Page.fileChooserOpened') chooser = params.backendNodeId;
    if (method === 'Browser.downloadWillBegin') download = params.suggestedFilename;
    const handle = pending.get(id);
    if (handle) {
      pending.delete(id);
      if (error) handle.reject(new Error(JSON.stringify(error))); else handle.resolve(result);
    }
  };
  send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  call = (method, params) => send(method, params, sessionId);
  await call('Page.enable');
  await call('Emulation.setDeviceMetricsOverride', { ...viewport, deviceScaleFactor: 1, mobile: false });
  await call('Emulation.setFocusEmulationEnabled', { enabled: true });
  await call('Page.bringToFront');
  await call('Page.setInterceptFileChooserDialog', { enabled: true });
  await send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, eventsEnabled: true });
  await call('Page.navigate', { url });

  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  const shown = () => evaluate("JSON.parse(window.wasmBindings?.shown_drawing?.() || 'null')");
  const sheet = (label, check) => until(label, async () => {
    const state = await shown();
    return state && check(state) && state;
  }, 25_000);
  const click = async ([x, y]) => {
    await call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await sleep(240);
    await call('Input.dispatchMouseEvent', { type: 'mousePressed', button: 'left', buttons: 1, clickCount: 1, x, y });
    await call('Input.dispatchMouseEvent', { type: 'mouseReleased', button: 'left', buttons: 0, clickCount: 1, x, y });
    await sleep(400);
  };
  const key = async (key, code, windowsVirtualKeyCode, modifiers = 0, text) => {
    const params = { key, code, windowsVirtualKeyCode, modifiers };
    await call('Input.dispatchKeyEvent', { ...params, type: text ? 'keyDown' : 'rawKeyDown', text });
    await call('Input.dispatchKeyEvent', { ...params, type: 'keyUp' });
    await sleep(400);
  };
  const enter = () => key('Enter', 'Enter', 13, 0, '\r');
  const moveToCanvas = () => call('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1000, y: 450 });
  const command = async name => {
    await moveToCanvas();
    await key('P', 'KeyP', 80, 2 | 8, 'P');
    for (const ch of name) {
      await call('Input.dispatchKeyEvent', { type: 'char', text: ch });
      await sleep(30);
    }
    await sleep(400);
    await enter();
  };
  const open = async (file, action) => {
    chooser = undefined;
    await action();
    const backendNodeId = await until('file picker', () => chooser);
    await call('DOM.setFileInputFiles', { files: [path.resolve(file)], backendNodeId });
  };
  const model = (view, x, y, z) => until(`${view} model point`, () => evaluate(`(() => {
    const point = window.wasmBindings.drawing_screen_point(${JSON.stringify(view)}, ${x}, ${y}, ${z});
    return point && Array.from(point);
  })()`), 25_000);
  const pick = async (...args) => click(await model(...args));
  const tool = async (commandName, label = commandName) => {
    await command(commandName);
    await sheet(label, s => s.tool === label);
  };
  const save = async () => {
    download = undefined;
    await command('Save drawing');
    const name = await until('drawing download', () => download);
    assert(name.endsWith('.dwdrw'), name);
    const file = path.join(downloads, name);
    await until('saved drawing', () => fs.existsSync(file));
    const json = execFileSync('python3', ['-c',
      'import sys, zipfile; sys.stdout.buffer.write(zipfile.ZipFile(sys.argv[1]).read("drawing.json"))', file]);
    return { file, sheet: JSON.parse(json).sheets[0] };
  };
  const verify = ({ annotations, tables }) => {
    assert.equal(annotations.filter(a => a.body.kind?.type === 'chamfer')[0].body.chamfer_angle, 45);
    assert.equal(annotations.filter(a => a.body.type === 'centreline').length, 1);
    assert.equal(annotations.filter(a => a.body.type === 'weld').length, 1);
    assert.deepEqual(tables[0].holes.map(h => h.diameter), [9, 9, 12, 9, 9]);
  };

  await until('start page', () => evaluate("document.querySelector('#status.hidden') !== null"));
  // New drawing asks for the part, then opens Drawing options after the worker finishes.
  await evaluate(`document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') window.annotationTestDocument = detail;
  })`);
  await open(part, () => click([484, 147]));
  await until('proved bracket', () => evaluate(`(() => {
    const build = JSON.parse(window.wasmBindings.shown_build() || 'null');
    return window.annotationTestDocument?.extension === '.dwprt'
      && window.annotationTestDocument?.count === 12 && build && !build.pending;
  })()`), 25_000);
  await until('drawing options', () => evaluate(`Boolean(
    JSON.parse(window.wasmBindings.shown_controls())['Drawing options']
  )`), 25_000);
  await key('Enter', 'Enter', 13);
  const initial = await sheet('new drawing', s => s.up_to_date);
  console.error('Created a drawing of the proved bracket.');
  await tool('Chamfer', 'Chamfer dimension');
  await pick('Top view', 117, 3, 10);
  await pick('Top view', 90, 0, 10);
  await pick('Top view', 130, -13, 10);
  await sheet('chamfer dimension', s => s.annotations === initial.annotations + 1);

  await tool('Centreline');
  await pick('Front view', 100, 0, 0);
  await pick('Front view', 100, 0, 10);
  await sheet('centreline', s => s.annotations === initial.annotations + 2);

  await tool('Weld', 'Weld symbol');
  await pick('Front view', 100, 0, 10);
  await pick('Front view', 100, 0, 27);
  await sheet('weld symbol', s => s.annotations === initial.annotations + 3);

  await tool('Hole table');
  await pick('Top view', 25, 23, 10);
  const [x, y] = await model('Top view', 25, 23, 10);
  await click([x + 174, y - 70]);
  await sheet('hole table', s => s.tables === 1);
  console.error('Placed all four annotations.');
  await key('z', 'KeyZ', 90, 2, 'z');
  await sheet('undone table', s => s.tables === 0);
  await key('y', 'KeyY', 89, 2, 'y');
  await sheet('redone table', s => s.tables === 1);
  await command('Rebuild');
  await sheet('rebuilt annotations', s => s.up_to_date && s.dangling === 0);

  const saved = await save();
  verify(saved.sheet);
  await command('Close drawing');
  await until('closed drawing', async () => (await shown()) === null);
  await open(saved.file, () => key('o', 'KeyO', 79, 2, 'o'));
  await sheet('reopened drawing', s => s.annotations === initial.annotations + 3 && s.tables === 1);
  await command('Rebuild');
  await sheet('reopened rebuild', s => s.up_to_date && s.dangling === 0);

  await moveToCanvas();
  await key('d', 'KeyD', 68, 0, 'd');
  await sheet('smart dimension tool', s => s.tool === 'Smart dimension');
  const [hx, hy] = await model('Top view', 29.5, 23, 10);
  await click([hx, hy]);
  await click([hx + 28, hy - 20]);
  await sheet('diameter dimension', s => s.annotations === initial.annotations + 4);
  fs.renameSync(saved.file, path.join(out, 'before-reopen.dwdrw'));
  const final = await save();
  verify(final.sheet);
  assert(final.sheet.annotations.some(a => a.body.kind?.type === 'diameter' && a.body.value.value === 9));
  const { data } = await call('Page.captureScreenshot', { format: 'png' });
  fs.writeFileSync(path.join(out, 'drawing.png'), Buffer.from(data, 'base64'));
  console.error('Four annotations survived rebuild, undo, save and reopen. A 9 mm diameter was added after reopen.');
} catch (error) {
  if (call) {
    const shot = await call('Page.captureScreenshot', { format: 'png' }).catch(() => null);
    if (shot) fs.writeFileSync(path.join(out, 'failed.png'), Buffer.from(shot.data, 'base64'));
  }
  throw error;
} finally {
  await stopChrome(child, send && (() => send('Browser.close')));
  fs.rmSync(profile, { recursive: true, force: true });
}
