// Run with chrome-devtools-axi run < ci/browser-appearance.js at /?demo on a fresh
// origin, then at /?demo&appearance=import and /?demo&appearance=restart on the same origin.
const query = new URL(await page.eval(() => location.href)).searchParams;
if (!query.has('demo')) throw new Error('Open the app at /?demo');

async function until(read, accept, description) {
  let value;
  for (let frame = 0; frame < 300; frame++) {
    value = await read();
    if (accept(value)) return value;
    await page.wait(200);
  }
  throw new Error(`${description}: ${JSON.stringify(value)}`);
}

const control = name => page.eval(
  `() => JSON.parse(window.wasmBindings.shown_controls())[${JSON.stringify(name)}] ?? null`,
);
const saved = () => page.eval(() => JSON.parse(localStorage.getItem('soliddraft-settings'))?.theme_preferences ?? null);
const viewport = () => page.eval(() => JSON.parse(window.wasmBindings.shown_viewport() || 'null'));

async function load() {
  await page.wait('#status.hidden', 60000);
  await until(viewport, view => view?.curves_settled && view.triangles > 0, 'The demo part did not draw');
  await page.eval(() => {
    const canvas = document.getElementById('soliddraft');
    canvas.focus();
    window.appearanceClick = async (x, y) => {
      const at = {
        clientX: x, clientY: y, bubbles: true, cancelable: true,
        pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0,
      };
      canvas.dispatchEvent(new PointerEvent('pointermove', { ...at, buttons: 0 }));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
      return new Promise(resolve => setTimeout(() => {
        canvas.dispatchEvent(new PointerEvent('pointerup', { ...at, buttons: 0 }));
        resolve(true);
      }, 50));
    };
    return true;
  });
}

async function click(name) {
  const at = await until(() => control(name), value => Number.isFinite(value?.x) && Number.isFinite(value?.y),
    `Control ${name} was not drawn`);
  await page.eval(`() => window.appearanceClick(${Math.round(at.x)}, ${Math.round(at.y)})`);
}

async function selected(id) {
  await until(() => control(`theme-selected-${id}`), Boolean, `Theme ${id} was not selected in the gallery`);
}

async function openAppearance() {
  await page.press('Control+Shift+P');
  await until(
    () => page.eval(() => document.activeElement instanceof HTMLInputElement),
    value => value === true,
    'The command palette did not take keyboard focus',
  );
  await page.type('Preferences');
  await page.eval(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(() => done(true)))));
  await page.press('Enter');
  await click('Preferences: Appearance');
}

async function provideFile(kind) {
  // Replace the OS chooser for one click. The application's input/change and file-reading paths still run.
  await page.eval(`async () => {
    const kind = ${JSON.stringify(kind)};
    let file;
    if (kind === 'theme') {
      const theme = {
        format: 'soliddraft-theme/1', name: 'Workshop', base: 'clear_bench_dark',
        accent: [255, 201, 78], viewport_top: [35, 42, 52], viewport_bottom: [20, 25, 35],
      };
      file = new File([JSON.stringify(theme)], 'workshop.json', { type: 'application/json' });
    } else {
      const canvas = document.createElement('canvas');
      canvas.width = 8;
      canvas.height = 4;
      const context = canvas.getContext('2d');
      context.fillStyle = 'rgb(32, 106, 120)';
      context.fillRect(0, 0, 8, 4);
      const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
      file = new File([blob], 'teal.png', { type: 'image/png' });
    }
    const original = HTMLInputElement.prototype.click;
    HTMLInputElement.prototype.click = function () {
      if (this.type !== 'file') return original.call(this);
      HTMLInputElement.prototype.click = original;
      const transfer = new DataTransfer();
      transfer.items.add(file);
      this.files = transfer.files;
      this.dispatchEvent(new Event('change', { bubbles: true }));
    };
    return true;
  }`);
}

await load();
await openAppearance();
const phase = query.get('appearance') ?? 'choose';
if (phase === 'choose') {
  await selected('clear_bench');
  await click('theme-filter-Light');
  await until(() => control('theme-gruvbox-dark'), v => v === null, 'Light filter showed Gruvbox Dark');
  await click('theme-filter-Dark');
  await until(() => control('theme-gruvbox-light'), v => v === null, 'Dark filter showed Gruvbox Light');
  await click('theme-gruvbox-dark');
  await selected('gruvbox-dark');
  await click('Preferences: Cancel');
  await openAppearance();
  await selected('clear_bench');

  await click('theme-gruvbox-dark');
  await selected('gruvbox-dark');
  await click('Preferences: OK');
  await until(saved, appearance => appearance?.selected?.built_in === 'gruvbox_dark', 'The chosen theme was not saved');
  console.log('PASS light and dark filters, live preview, Cancel and save');
} else if (phase === 'import') {
  await selected('gruvbox-dark');

  await provideFile('theme');
  await click('theme-import');
  await selected('imported');
  await provideFile('picture');
  await click('theme-picture-import');
  await until(() => control('theme-background-Picture'), Boolean, 'The imported picture did not become available');
  await click('Preferences: OK');
  const imported = await until(saved, appearance => appearance?.format === 'soliddraft-appearance/1'
    && appearance.selected === 'imported' && appearance.imported?.name === 'Workshop'
    && appearance.background === 'picture' && appearance.picture?.png?.length > 0,
  'Theme and picture were not saved');
  await page.eval(`() => {
    sessionStorage.setItem('appearance-check-baseline', ${JSON.stringify(JSON.stringify(imported))});
    return true;
  }`);
  console.log('PASS restart restored Gruvbox Dark, JSON theme and PNG picture imported and saved');
} else if (phase === 'restart') {
  await selected('imported');
  await until(() => control('theme-background-Picture'), Boolean, 'Reload lost the imported picture');
  const expected = await page.eval(() => sessionStorage.getItem('appearance-check-baseline'));
  const reopened = await saved();
  if (!expected || JSON.stringify(reopened) !== expected) throw new Error('Reload changed the saved appearance');
  await click('Preferences: OK');
  console.log('PASS restart restored the imported theme and PNG background');
} else {
  throw new Error(`Unknown Appearance phase: ${phase}`);
}
