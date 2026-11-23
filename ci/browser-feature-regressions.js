// Run with chrome-devtools-axi run in a fresh 1600 x 1000 page on the packaged app.
// Set the URL fragment to depth, offset, positions, formula, or visibility.
// Serve the application test fixtures written to SOLIDDRAFT_BROWSER_FIXTURES at /fixtures/.
await page.wait('#status.hidden', 60000);
const scenario = await page.eval(() => location.hash.slice(1));

await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();
  const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
  window.featureClick = async ({ x, y }, count) => {
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true,
      pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 };
    canvas.dispatchEvent(new PointerEvent('pointermove', at));
    await frame();
    for (let i = 0; i < count; i++) {
      canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
      await frame();
      canvas.dispatchEvent(new PointerEvent('pointerup', at));
      await frame();
    }
    return true;
  };

  const choose = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.featureUpload) return choose.call(this);
    const files = new DataTransfer();
    files.items.add(window.featureUpload);
    window.featureUpload = null;
    this.files = files.files;
    this.dispatchEvent(new Event('change'));
  };
  const download = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (!this.download.endsWith('.dwprt')) return download.call(this);
    fetch(this.href).then(response => response.blob()).then(blob => {
      window.featureSaved = new File([blob], this.download);
    });
  };
  window.featureSavedPart = async () => {
    const bytes = await window.featureSaved.arrayBuffer();
    const zip = new DataView(bytes);
    // ZIP end record and central directory give the compressed member's size and offset.
    let end = bytes.byteLength - 22;
    while (end >= 0 && zip.getUint32(end, true) !== 0x06054b50) end--;
    if (end < 0) throw new Error('The download is missing its ZIP end record');
    let entry = zip.getUint32(end + 16, true);
    for (let i = 0; i < zip.getUint16(end + 10, true); i++) {
      const nameSize = zip.getUint16(entry + 28, true);
      const name = new TextDecoder().decode(new Uint8Array(bytes, entry + 46, nameSize));
      if (name === 'part.json') {
        const local = zip.getUint32(entry + 42, true);
        const start = local + 30 + zip.getUint16(local + 26, true) + zip.getUint16(local + 28, true);
        const data = new Uint8Array(bytes, start, zip.getUint32(entry + 20, true));
        const blob = new Blob([data]);
        if (zip.getUint16(entry + 10, true) === 0) return JSON.parse(await blob.text());
        return new Response(blob.stream().pipeThrough(new DecompressionStream('deflate-raw'))).json();
      }
      entry += 46 + nameSize + zip.getUint16(entry + 30, true) + zip.getUint16(entry + 32, true);
    }
    throw new Error('The download has no part.json');
  };
  return true;
});

function check(condition, message) {
  if (!condition) throw new Error(message);
}
const shown = name => page.eval(`JSON.parse(window.wasmBindings.${name}() || 'null')`);
const controls = () => shown('shown_controls');
const threads = () => page.eval(() => window.wasmBindings.shown_threads());
async function until(read, accept, message) {
  let last;
  const end = Date.now() + 25000;
  do {
    last = await read();
    if (accept(last)) return last;
    await page.wait(100);
  } while (Date.now() < end);
  throw new Error(`${message}. Last value ${JSON.stringify(last)}`);
}
async function click(name, count = 1) {
  const at = (await until(controls, value => value?.[name], `Missing ${name}`))[name];
  await page.eval(`featureClick(${JSON.stringify(at)}, ${count})`);
  await page.wait(300);
}
async function command(label, id) {
  await page.press('Control+Shift+p');
  await until(controls, value => value?.['Command palette'], 'The command palette did not open');
  await page.type(label);
  await click(`Command: ${id}`);
}
async function replace(name, value) {
  await click(name);
  await page.press('Control+a');
  await page.type(value);
  await page.press('Enter');
}
async function open(name) {
  if (name) {
    await page.eval(`(async () => {
      const response = await fetch('/fixtures/' + ${JSON.stringify(name)});
      if (!response.ok) throw new Error('Fixture is missing');
      window.featureUpload = new File([await response.blob()], ${JSON.stringify(name)});
      return true;
    })()`);
  } else {
    await page.eval(() => { window.featureUpload = window.featureSaved; return true; });
  }
  await page.press('Control+o');
  await until(() => page.eval(() => window.featureUpload === null), Boolean, 'Open did not consume the selected file');
  await until(() => shown('shown_build'), value => value && !value.pending && !value.reading, 'Part is still building');
  await page.wait(500);
}
async function save() {
  await page.eval(() => { window.featureSaved = null; return true; });
  await page.press('Control+s');
  await until(() => page.eval(() => window.featureSaved?.size), size => size > 0, 'Save did not download the part');
  return page.eval(() => window.featureSavedPart());
}
async function callout() {
  await command('Manufacturing information', 'manufacturing_information');
  const pmi = await until(() => shown('shown_manufacturing_information'), value => value?.items.length, 'No PMI was drawn');
  return pmi.items[0].callout;
}

switch (scenario) {
  case 'depth': {
    await open('wizard-blind-depth.dwprt');
    await command('Parameters', 'configuration_grid');
    await replace('Filter parameters', 'Hole1.Depth');
    await replace('Default Hole1.Depth', '5 mm');
    await until(() => shown('shown_build'), value => value && !value.pending && !value.reading, 'Edited part is still building');
    const text = await callout();
    check(text === 'M6x1-6H DEEP 5', `The edited hole reads ${text}`);
    break;
  }
  case 'offset': {
    await open('wizard-tapped-offset.dwprt');
    const text = await callout();
    check(text === 'M6x1-6H DEEP 15', `The blind surface-offset hole reads ${text}`);
    break;
  }
  case 'positions': {
    await open('wizard-legacy-positions.dwprt');
    await click('Tree: Hole1', 2);
    await click('Hole Wizard Positions tab');
    await click('Point Patterns');
    await click('Apply linear hole pattern');
    await click('Apply Hole Wizard');
    await until(controls, value => !value['Apply Hole Wizard'], 'The hole form stayed open');
    const stored = await save();
    const hole = stored.features.find(f => f.kind.type === 'hole');
    const sketch = stored.features.find(f => f.id === hole.kind.sketch);
    check(sketch?.kind.sketch.patterns?.length === 1, 'The saved hole lost its position pattern');
    await open();
    const reopened = await save();
    const reloaded = reopened.features.find(f => f.id === sketch.id);
    check(reloaded?.kind.sketch.patterns?.length === 1, 'The reopened hole lost its position pattern');
    break;
  }
  case 'formula': {
    await open('formula-case-distinct.dwprt');
    await command('Parameters', 'configuration_grid');
    await replace('Filter parameters', 'Plate');
    await replace('Name Plate', 'Other');
    const stored = await save();
    const overrides = stored.configurations.rows.find(row => row.name === 'Thin').overrides;
    check(overrides['BossExtrude1.Depth'] === '=plate', `The depth now refers to ${overrides['BossExtrude1.Depth']}`);
    check(overrides.Width === '=Other', `The width now refers to ${overrides.Width}`);
    break;
  }
  case 'visibility': {
    await open('thread-hidden.dwprt');
    const view = () => shown('shown_viewport');
    await until(view, value => value?.display_state === 'Hidden' && value.visible_meshes === 0, 'The body remained visible');
    check(await threads() === 0, 'The hidden solid still drew thread marks');
    await click('Display state');
    await click('Display state: Default');
    await until(threads, count => count > 0, 'The visible solid did not draw its thread marks');
    break;
  }
  default:
    throw new Error(`Unknown feature regression ${scenario}`);
}
console.log(`Passed browser feature regression ${scenario}`);
