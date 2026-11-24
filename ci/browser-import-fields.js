// Run through chrome-devtools-axi on a fresh 1400 x 813 browser session at /?demo.
// Serve ci/fixtures/import/block-closed.step as /import-field-block.step.
// The pinned stage cannot read import bytes. Lattice and Gyroid should disappear
// from command search after a reviewed STEP import, while CAD commands still run.
function check(value, message) { if (!value) throw new Error(message); }
const controls = () => page.eval(() => JSON.parse(window.wasmBindings.shown_controls()));
async function until(read, matches, label) {
  for (let i = 0; i < 300; i++) {
    const value = await read();
    const match = matches(value);
    if (match) return match;
    await page.wait(100);
  }
  throw new Error(label);
}
async function search(name) {
  await page.press('Control+Shift+P');
  await until(controls, shown => shown['Command palette'], 'The command palette did not open');
  await until(
    () => page.eval(() => document.activeElement instanceof HTMLInputElement && document.activeElement.type === 'text'),
    Boolean,
    'The command palette did not focus its browser text input',
  );
  await page.type(name);
}
async function dismiss() {
  await page.press('Escape');
  await until(controls, shown => !shown['Command palette'], 'The command palette did not close');
}
async function click({ x, y }) {
  await page.eval(`window.clickImportFields(${x}, ${y})`);
}
async function command(label, id) {
  await search(label);
  const at = await until(controls, shown => shown[`Command: ${id}`], `No search result for ${label}`);
  await click(at);
}

check(await page.eval(() => innerWidth === 1400 && innerHeight === 813), 'Set a 1400 x 813 viewport first');
await page.wait('#status.hidden', 60000);
await page.eval(async () => {
  const response = await fetch('/import-field-block.step');
  if (!response.ok) throw new Error('The STEP fixture is missing');
  const source = await response.blob();
  window.importFields = { counts: [], actions: [], uploaded: false };
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') window.importFields.counts.push(detail.count);
    if (detail.type === 'action') window.importFields.actions.push(detail.command);
  });
  window.clickImportFields = async (x, y) => {
    const canvas = document.getElementById('soliddraft');
    const at = { clientX: x, clientY: y, bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true };
    canvas.dispatchEvent(new PointerEvent('pointermove', at));
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
    return new Promise(resolve => setTimeout(() => {
      canvas.dispatchEvent(new PointerEvent('pointerup', at));
      resolve(true);
    }, 50));
  };
  // Answer the file picker that the real Import STEP command opens.
  const click = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !this.accept.includes('.step')) return click.call(this);
    const transfer = new DataTransfer();
    transfer.items.add(new File([source], 'block.step'));
    this.files = transfer.files;
    this.dispatchEvent(new Event('change'));
    window.importFields.uploaded = true;
  };
  return true;
});

for (const field of ['Lattice', 'Gyroid']) {
  await search(field);
  await until(controls, shown => shown[`Command: ${field.toLowerCase()}`], `The sample part did not offer ${field}`);
  await dismiss();
}
await command('New part', 'new_part');
await until(() => page.eval(() => window.importFields.counts.at(-1)), count => count === 0, 'New part did not open');
await command('Import STEP', 'import_step');
await until(() => page.eval(() => window.importFields.uploaded), Boolean, 'Import STEP did not ask for the fixture');
await click(await until(controls, shown => shown['Accept import'], 'Import review did not offer Accept'));
await until(() => page.eval(() => window.importFields.counts.at(-1)), count => count === 1, 'The STEP solid was not imported');

for (const field of ['Lattice', 'Gyroid']) {
  await search(field);
  await page.wait(250);
  check(!(await controls())[`Command: ${field.toLowerCase()}`], `Command search still offers ${field} on the imported solid`);
  await dismiss();
}
await command('Chamfer', 'chamfer');
await until(() => page.eval(() => window.importFields.actions.includes('chamfer')), Boolean,
  'Chamfer did not start on the imported solid');
console.log('The browser offers CAD on the imported solid and hides both unavailable field commands');
