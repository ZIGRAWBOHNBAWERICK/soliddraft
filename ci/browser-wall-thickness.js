// Run through chrome-devtools-axi run at the packaged app's root in a 1400 x 813 page.
// Serve graded-wall.json as recipe.json inside /fixtures/graded-wall.dwprt.
const { axiCanvas } = await import(`${process.cwd()}/ci/canvas-browser.mjs`);
const canvas = axiCanvas(page);
function check(condition, message) {
  if (!condition) throw new Error(message);
}

await page.wait('#status.hidden', 60000);
await page.eval(() => {
  document.getElementById('soliddraft').focus();
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
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    value = await read();
    if (accept(value)) return value;
    await page.wait(100);
  }
  throw new Error(`${message}. Last state: ${JSON.stringify(value)}`);
}
const click = ({ x, y }) => canvas.click([x, y]);
const control = async name => {
  const controls = await until(() => drawn('shown_controls'), shown => shown[name], `Missing ${name}`);
  return controls[name];
};
const wall = () => control('Wall feature: Shell1');
async function command(label, id) {
  await page.press('Control+Shift+P');
  await control('Command palette');
  await page.type(label);
  // The unfiltered palette can already contain this command in a different row.
  await canvas.frames();
  await click(await control(`Command: ${id}`));
}
const builtWall = rows => !rows.Error && rows['Wall thickness'] === '1 mm wall' && Number(rows['Faces made']) > 0;
async function details(accept) {
  await click(await wall());
  const options = await until(() => drawn('shown_options'),
    shown => shown.selected === 'Shell1' && shown.details, 'Shell1 was not selected');
  await click(options.details);
  const detail = await until(() => drawn('shown_details'),
    shown => shown.title === 'Shell1' && accept(Object.fromEntries(shown.rows ?? [])), 'Shell1 Details did not update');
  await click((await drawn('shown_options')).details);
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

// A tree double-click can show dimensions. Open the feature editor explicitly.
await click(await wall());
await command('Edit feature', 'edit_feature');
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

// The desktop command-reachability test covers the native commands.
await page.eval(() => {
  document.addEventListener('soliddraft-feedback-open', () => { window.sdFeedbackOpened = true; }, { once: true });
  return true;
});
await command('Give feedback', 'give_feedback');
await until(() => page.eval(() => window.sdFeedbackOpened === true), Boolean, 'Give feedback did not dispatch');
