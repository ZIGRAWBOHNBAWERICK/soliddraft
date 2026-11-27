// Run through chrome-devtools-axi on a fresh 1400 x 813 browser session at /?demo.
// Serve ci/fixtures/import/block-closed.step as /import-field-block.step.
// Lattice, Gyroid and CAD commands are available on a reviewed STEP solid.
function check(value, message) { if (!value) throw new Error(message); }
const controls = () => page.eval(() => JSON.parse(window.wasmBindings.shown_controls()));
async function until(read, matches, label) {
  let value;
  for (let i = 0; i < 300; i++) {
    value = await read();
    const match = matches(value);
    if (match) return match;
    await page.wait(100);
  }
  const state = await page.eval(() => ({
    focused: document.activeElement?.tagName,
    build: JSON.parse(window.wasmBindings.shown_build() || 'null'),
    controls: JSON.parse(window.wasmBindings.shown_controls()),
    importFields: window.importFields,
  }));
  throw new Error(`${label}. Last value ${JSON.stringify(value)}; app ${JSON.stringify(state)}`);
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
  await until(
    () => page.eval(() => document.activeElement instanceof HTMLInputElement),
    focused => !focused,
    'The dismissed palette kept browser text focus',
  );
}
async function click({ x, y }) {
  await page.eval(`window.clickImportFields(${x}, ${y})`);
}
async function command(label, id) {
  await search(label);
  const result = `Command: ${id}`;
  await until(controls, shown => {
    const first = Object.entries(shown).filter(([key]) => key.startsWith('Command: '))
      .reduce((top, entry) => !top || entry[1].y < top[1].y ? entry : top, null);
    return first?.[0] === result;
  }, `${label} never became the first palette result`);
  const before = await page.eval(() => window.importFields.actions.length);
  await page.press('Enter');
  const received = await until(
    () => page.eval(`window.importFields.actions[${before}] ?? ''`), command => command, `No command dispatched for ${label}`,
  );
  check(received === id, `Expected ${id}, but the app dispatched ${received}`);
  await until(controls, shown => !shown['Command palette'], `The palette stayed open for ${label}`);
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
    canvas.dispatchEvent(new MouseEvent('mousemove', at));
    const frame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    await frame();
    const down = new PointerEvent('pointerdown', { ...at, buttons: 1 });
    const up = new PointerEvent('pointerup', at);
    canvas.dispatchEvent(down);
    canvas.dispatchEvent(up);
    return true;
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
const documentChanges = await page.eval(() => window.importFields.counts.length);
await command('New part', 'new_part');
await until(
  () => page.eval(`window.importFields.counts.length > ${documentChanges} ? window.importFields.counts.at(-1) : null`),
  count => count === 0,
  'New part did not open',
);
await command('Import STEP', 'import_step');
await until(() => page.eval(() => window.importFields.uploaded), Boolean, 'Import STEP did not ask for the fixture');
await click(await until(controls, shown => shown['Accept import'], 'Import review did not offer Accept'));
await until(() => page.eval(() => window.importFields.counts.at(-1)), count => count === 1, 'The STEP solid was not imported');

for (const field of ['Lattice', 'Gyroid']) {
  const id = field.toLowerCase();
  await command(field, id);
  await until(controls, shown => shown['Form OK'], `${field} did not open its editor on the imported solid`);
  await page.press('Escape');
  await until(controls, shown => !shown['Form OK'], `${field} editor did not close`);
}
await command('Chamfer', 'chamfer');
console.log('The imported STEP solid offers Lattice, Gyroid and Chamfer');
