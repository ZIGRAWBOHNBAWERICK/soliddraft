// Run through chrome-devtools-axi run at the packaged app's root in a 1400 x 813 page.
// Serve graded-wall.json as recipe.json inside /fixtures/graded-wall.dwprt.
function check(condition, message) {
  if (!condition) throw new Error(message);
}

await page.wait('#status.hidden', 60000);
await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();
  window.sdClick = async ({ x, y }, count) => {
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true,
      pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 };
    canvas.dispatchEvent(new PointerEvent('pointermove', { ...at, buttons: 0 }));
    for (let i = 0; i < count; i++) {
      canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
      await new Promise(resolve => setTimeout(resolve, 50));
      canvas.dispatchEvent(new PointerEvent('pointerup', { ...at, buttons: 0 }));
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    return true;
  };
  const inputClick = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.sdUpload) return inputClick.call(this);
    const file = window.sdUpload;
    window.sdUpload = null;
    const transfer = new DataTransfer();
    transfer.items.add(file);
    this.files = transfer.files;
    this.dispatchEvent(new Event('change'));
  };
  const linkClick = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (!this.download.endsWith('.dwprt')) return linkClick.call(this);
    const name = this.download;
    fetch(this.href).then(response => response.blob()).then(blob => {
      window.sdSaved = new File([blob], name);
    });
  };
  return true;
});

const drawn = async name => {
  const data = await page.eval(`window.wasmBindings.${name}()`);
  return data ? JSON.parse(data) : {};
};
async function until(read, accept, message) {
  let value;
  for (let i = 0; i < 600; i++) {
    value = await read();
    if (accept(value)) return value;
    await page.wait(100);
  }
  throw new Error(`${message}. Last state: ${JSON.stringify(value)}`);
}
const click = (at, count = 1) => page.eval(`sdClick(${JSON.stringify(at)}, ${count})`);
const control = async name => {
  const controls = await until(() => drawn('shown_controls'), shown => shown[name], `Missing ${name}`);
  return controls[name];
};
const wall = () => control('Wall feature: Shell1');
const builtWall = rows => !rows.Error && rows['Wall thickness'] === '1 mm wall' && Number(rows['Faces made']) > 0;
async function details(accept) {
  await click(await wall());
  const options = await until(() => drawn('shown_options'),
    shown => shown.selected === 'Shell1' && shown.details, 'Shell1 was not selected');
  await click(options.details);
  const detail = await until(() => drawn('shown_details'),
    shown => shown.title === 'Shell1' && accept(Object.fromEntries(shown.rows ?? [])), 'Shell1 Details did not update');
  await click(options.details);
  await until(() => drawn('shown_details'), shown => !shown.title, 'Details stayed open');
  return Object.fromEntries(detail.rows);
}

check(await page.eval(async () => {
  const response = await fetch('/fixtures/graded-wall.dwprt');
  if (!response.ok) return false;
  window.sdUpload = new File([await response.blob()], 'graded-wall.dwprt');
  return true;
}), 'The graded wall fixture did not load');
await page.press('Control+o');
const refused = await details(rows => rows.Error?.includes('Field-driven thickness is unavailable'));
check(refused['Wall thickness'] === '1 mm to 3 mm along x from -5 to 5 mm wall', 'The saved grade changed');

await click(await wall(), 2);
await click(await control('Use constant'));
await control('Thickness value');
await page.press('Enter');
await until(() => drawn('shown_controls'), shown => !shown['Thickness value'], 'The wall editor stayed open');
await details(builtWall);

await page.press('Control+s');
await until(() => page.eval(() => window.sdSaved?.size ?? 0), size => size > 0, 'The edited part did not download');
await page.press('Control+n');
await until(() => drawn('shown_controls'), shown => !shown['Wall feature: Shell1'], 'The new part did not open');
await page.eval(() => { window.sdUpload = window.sdSaved; return true; });
await page.press('Control+o');
await details(builtWall);
console.log('The browser refused the saved grade and reopened the repaired constant shell');
