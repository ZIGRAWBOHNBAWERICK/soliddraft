// Run with chrome-devtools-axi run < ci/browser-graphics.js after opening /?demo
// on a fresh origin. The demo has a cylindrical extrusion.
const url = await page.eval(() => location.href);
if (!new URL(url).searchParams.has('demo')) throw new Error('Open the app at /?demo');

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
    window.graphicsClick = (x, y) => {
      const at = {
        clientX: x, clientY: y, bubbles: true, cancelable: true,
        pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0,
      };
      canvas.dispatchEvent(new PointerEvent('pointermove', { ...at, buttons: 0 }));
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
    () => page.eval(`JSON.parse(window.wasmBindings.shown_controls())[${JSON.stringify(label)}]`),
    Boolean,
    `Control ${label} was not drawn`,
  );
  await page.eval(`window.graphicsClick(${control.x}, ${control.y})`);
}

await load();
const initial = await until(viewport, view => view?.triangles > 0, 'The demo part did not draw');
await page.press('Control+Alt+p');
await page.type('Preferences');
await page.press('Enter');
await click('Preferences: Graphics');
await click('Graphics preset: Performance');
await until(settings, s => s?.preset === 'performance' && s.curved_surfaces === 'coarse', 'Performance did not save');
const coarse = await until(viewport, v => v?.angle_deg === 15 && v.chord_mm > initial.chord_mm
  && v.triangles > 0, 'Coarse mesh did not draw');

await click('Graphics preset: Quality');
await until(settings, s => s?.preset === 'quality' && s.curved_surfaces === 'fine', 'Quality did not save');
const fine = await until(viewport, v => v?.angle_deg === 10 && v.chord_mm < coarse.chord_mm
  && v.triangles > coarse.triangles, 'Fine mesh did not replace the coarse mesh');
console.log(`Performance drew ${coarse.triangles} triangles. Quality drew ${fine.triangles} triangles.`);

await page.open(url);
await load();
await until(settings, s => s?.preset === 'quality' && s.curved_surfaces === 'fine', 'Reload lost the graphics preset');
await until(viewport, v => v?.angle_deg === 10 && v.triangles > coarse.triangles, 'Reload did not draw the restored quality');
console.log('PASS graphics presets change the live mesh and the browser restores the selected preset');
