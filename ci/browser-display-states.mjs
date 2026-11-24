// Open, switch, save and reopen display states through the packaged app.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { CanvasBrowser } from './canvas-browser.mjs';

const [chrome, url, out, service, inspection] = process.argv.slice(2);
if (!chrome || !url || !out || !service || !inspection) {
  throw new Error('usage: browser-display-states.mjs CHROME URL OUTDIR SERVICE_PART INSPECTION_PART');
}

const downloads = path.resolve(out, 'downloads');
fs.mkdirSync(downloads, { recursive: true });
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-display-states-'));
const browser = new CanvasBrowser({
  chrome, profile, size: { width: 1400, height: 900 }, url, out, downloads,
  extra: (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean),
});

async function run() {
  await browser.connect();
  const visitor = await browser.visitor();
  await visitor.load(60000);
  const open = file => visitor.choose(file, () => visitor.key('o', 'KeyO', 79, 2, 'o'));
  async function view(name, bodies, label) {
    let shown;
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
      await visitor.stopped(label);
      shown = await visitor.eval("JSON.parse(window.wasmBindings.shown_viewport() || 'null')");
      if (shown?.display_state === name && shown.visible_meshes === bodies) return shown;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    await visitor.shot(label);
    throw new Error(`${label}: expected ${name ?? 'Default'} with ${bodies} bodies, saw ${JSON.stringify(shown)}`);
  }
  async function selectState(name, bodies) {
    await visitor.click(await visitor.control('Display state', 20000));
    await visitor.click(await visitor.control(`Display state: ${name ?? 'Default'}`, 20000));
    await view(name, bodies, `select-${name ?? 'Default'}`);
  }

  await open(service);
  const opened = await view('Working', 1, 'open-Service');
  if (opened.display_mode !== 'Shaded') throw new Error('The display mode did not reopen');
  await selectState(null, 2);
  await selectState('Working', 1);

  const saved = await visitor.download('save', () => visitor.key('s', 'KeyS', 83, 2, 's'), 20000);
  if (!saved.endsWith('.dwprt') || !fs.statSync(saved).size) throw new Error('Save did not produce a part file');
  await open(inspection);
  await view('Inspection', 2, 'open-Default');
  await open(saved);
  await view('Working', 1, 'reopen-download');
  console.log('Opened a configured part, switched display states in the tree, saved it and reopened the saved state.');
}

try {
  await run();
} catch (error) {
  console.error(error.stack || String(error));
  if (browser.stderr) console.error(browser.stderr.split('\n').slice(-20).join('\n'));
  process.exitCode = 1;
} finally {
  await browser.close();
  fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
