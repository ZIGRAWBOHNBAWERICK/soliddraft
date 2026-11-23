// Open the assembly produced by the app's controller reorder test.
function check(condition, message) {
  if (!condition) throw new Error(message);
}

await page.wait('#status.hidden', 60000);
await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();
  window.sdClick = async ({ x, y }) => {
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true, pointerId: 1,
      pointerType: 'mouse', isPrimary: true, button: 0 };
    const frame = () => new Promise(resolve => requestAnimationFrame(() => resolve()));
    canvas.dispatchEvent(new PointerEvent('pointermove', { ...at, buttons: 0 }));
    await frame();
    canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
    await frame();
    canvas.dispatchEvent(new PointerEvent('pointerup', { ...at, buttons: 0 }));
    await frame();
    return true;
  };
  const inputClick = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.sdControllerUpload) return inputClick.call(this);
    const files = window.sdControllerUpload;
    window.sdControllerUpload = null;
    files.then(list => {
      const transfer = new DataTransfer();
      for (const file of list) transfer.items.add(file);
      this.files = transfer.files;
      this.dispatchEvent(new Event('change'));
    });
  };
  window.sdControllerUpload = Promise.all(['controller-rig.dwasm', 'controller-block.dwprt'].map(async name => {
    const response = await fetch(`/fixtures/${name}`);
    if (!response.ok) throw new Error(`Missing fixture ${name}`);
    return new File([await response.blob()], name);
  }));
  return true;
});

const shown = name => page.eval(`JSON.parse(window.wasmBindings.${name}() || '{}')`);
const assembly = () => shown('shown_assembly');
const controls = () => shown('shown_controls');
async function until(read, accept, message) {
  let value;
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    value = await read();
    if (accept(value)) return value;
    await page.wait(50);
  }
  throw new Error(`${message}. Last state ${JSON.stringify(value)}`);
}
async function click(name) {
  const at = (await until(controls, value => value[name], `Missing ${name}`))[name];
  await page.eval(`sdClick(${JSON.stringify(at)})`);
}
async function fill(name, text) {
  await click(name);
  await page.press('Control+a');
  await page.type(text);
}
const heights = value => value.components.slice(1).map(c => c.translation[2]);
const near = (a, b) => Math.abs(a - b) < 1e-7;
const atHeights = (value, a, b) => {
  const [first, second] = heights(value);
  return near(first, a) && near(second, b);
};

await page.press('Control+o');
await until(assembly, value => value.components?.length === 3 && !value.loading &&
  value.components.every(c => c.state === 'DisplayReady') && atHeights(value, 36, 46), 'Rig did not open');
await page.press('Control+Shift+P');
await until(controls, value => value['Command palette'], 'Command palette did not open');
await page.type('Mate');
await click('Command: assembly_mate');
await click('Mate more');

await fill('Controller mates', 'D2, D1');
await click('Animate to Low');
await click('Set driven mates');
const preview = await until(assembly, value => value.controller.mates.join() === 'D2,D1' &&
  value.controller.animating && heights(value)[0] < 35, 'No animation preview after reordering');
const [first, second] = heights(preview);
check(near(second - first, 10), `Reordering applied a preview to the wrong mates: ${first}, ${second}`);
await until(assembly, value => !value.controller.animating && atHeights(value, 16, 26),
  'Animation committed values to the wrong mates');

await fill('Controller value 1', '18');
await click('Apply Low');
await fill('Controller value 2', '15');
await click('Apply values');
await until(assembly, value => atHeights(value, 21, 26), 'An old draft overrode the saved position');
console.log('Reordered the driven mates during animation and applied a saved position with no stale draft values.');
