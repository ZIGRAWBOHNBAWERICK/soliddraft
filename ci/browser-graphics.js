// Run with chrome-devtools-axi run < ci/browser-graphics.js at /?demo on a fresh
// origin, then open /?demo&graphics=restart on the same origin and run it again.
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

async function load() {
  await page.wait('#status.hidden', 60000);
  await page.eval(() => {
    const canvas = document.getElementById('soliddraft');
    canvas.focus();
    window.graphicsClick = async (x, y) => {
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

const viewport = () => page.eval(() => JSON.parse(window.wasmBindings.shown_viewport() || 'null'));
const settings = () => page.eval(() => JSON.parse(localStorage.getItem('soliddraft-settings'))?.graphics);
async function click(label) {
  const control = await until(
    () => page.eval(`() => JSON.parse(window.wasmBindings.shown_controls())[${JSON.stringify(label)}] ?? null`),
    value => Number.isFinite(value?.x) && Number.isFinite(value?.y),
    `Control ${label} was not drawn`,
  );
  await page.eval(`() => window.graphicsClick(${Math.round(control.x)}, ${Math.round(control.y)})`);
}

await load();
const initial = await until(viewport, view => view?.curves_settled && view.triangles > 0, 'The demo part did not draw');
if (query.get('graphics') === 'restart') {
  await until(settings, s => s?.preset === 'quality' && s.curved_surfaces === 'fine', 'Reload lost the graphics preset');
  const baseline = await page.eval(() => JSON.parse(sessionStorage.getItem('graphics-check-baseline')));
  if (!baseline) throw new Error('Run the graphics presets before the restart check');
  const restored = await until(viewport, view => view?.curves_settled && view.angle_deg === 10
    && view.triangles > baseline.coarse && view.triangles === baseline.fine,
  'Reload did not draw the restored quality');
  console.log(`PASS restart restored Quality and drew ${restored.triangles} triangles`);
} else {
  await page.press('Control+Shift+P');
  await until(
    () => page.eval(() => document.activeElement instanceof HTMLInputElement),
    value => value === true,
    'The command palette did not take keyboard focus',
  );
  await page.type('Preferences');
  await page.eval(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(() => done(true)))));
  await page.press('Enter');
  for (let attempt = 0; attempt < 3; attempt++) {
    await page.wait(350);
    await click('Preferences: Graphics');
    await page.wait(350);
    const opened = await page.eval(() => Boolean(
      JSON.parse(window.wasmBindings.shown_controls())['Graphics preset: Performance'],
    ));
    if (opened) break;
    if (attempt === 2) throw new Error('The Graphics page did not open');
  }
  await click('Graphics preset: Performance');
  await until(settings, s => s?.preset === 'performance' && s.curved_surfaces === 'coarse', 'Performance did not save');
  const coarse = await until(viewport, v => v?.curves_settled && v.angle_deg === 15
    && v.standard_chord_mm === initial.standard_chord_mm * 4 && v.triangles > 0, 'Coarse mesh did not draw');

  await click('Graphics preset: Quality');
  await until(settings, s => s?.preset === 'quality' && s.curved_surfaces === 'fine', 'Quality did not save');
  const fine = await until(viewport, v => v?.angle_deg === 10 && v.chord_mm < coarse.chord_mm
    && v.triangles > coarse.triangles, 'Fine mesh did not replace the coarse mesh');
  await page.eval(`() => {
    sessionStorage.setItem('graphics-check-baseline', JSON.stringify({ coarse: ${coarse.triangles}, fine: ${fine.triangles} }));
    return true;
  }`);
  console.log(`Performance drew ${coarse.triangles} triangles. Quality drew ${fine.triangles} triangles.`);
  console.log('PASS graphics presets change the live mesh');
}
