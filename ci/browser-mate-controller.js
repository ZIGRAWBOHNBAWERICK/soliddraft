// Open the assembly produced by the app's controller reorder test.
await page.wait('#status.hidden', 60000);
await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();
  window.sdClick = async ({ x, y }) => {
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true, pointerId: 1,
      pointerType: 'mouse', isPrimary: true, button: 0 };
    canvas.dispatchEvent(new MouseEvent('mousemove', at));
    canvas.dispatchEvent(new PointerEvent('pointermove', at));
    await new Promise(resolve => setTimeout(resolve, 300));
    canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
    await new Promise(resolve => setTimeout(resolve, 50));
    canvas.dispatchEvent(new PointerEvent('pointerup', at));
    canvas.focus();
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

await fill('Controller value 1', '18');
await fill('Controller mates', 'D2,D1');
await click('Set driven mates');
await until(assembly, value => value.controller.mates.join() === 'D2,D1',
  'The controller did not reorder the driven mates');
await fill('Controller value 2', '15');
await click('Apply values');
await until(assembly, value => atHeights(value, 21, 46), 'A draft applied to the wrong mate after reordering');

await click('Apply Low');
await until(assembly, value => atHeights(value, 16, 26), 'Low did not apply');
await fill('Controller value 1', '18');
await click('Apply Low');
await fill('Controller value 2', '15');
await click('Apply values');
await until(assembly, value => atHeights(value, 21, 26), 'An old draft overrode the saved position');
console.log('Reordered the driven mates and applied a saved position with no stale draft values.');
