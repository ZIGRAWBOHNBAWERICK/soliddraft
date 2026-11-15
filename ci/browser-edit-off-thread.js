// Run through chrome-devtools-axi run after opening the packaged application at / in a
// 1400 x 813 page, on an origin with no earlier sessions, that also serves the bracket
// part at /fixtures/bracket.dwprt.
//
// Building a part in the browser runs in the kernel worker, not on the page's thread:
// opening the bracket, changing a dimension, Undo and Redo once each held the page for
// 2.7 to 7 s in headless Chrome. The page shows that it is building, with a Cancel that
// returns the part to its last accepted state, and nothing on the page's thread may run
// 250 ms or more meanwhile. The check reads the build state through the read-only
// `shown_build` export, which also places the Cancel button, and Details through
// `shown_details`, since the canvas has no text.
function check(value, message) { if (!value) throw new Error(message); }
const DETAILS = [317, 52], CLOSE = [347, 92], BASE = [380, 203], DEPTH = [515, 268];
const LONG = 250;

check(await page.eval(() => location.pathname === '/' && location.search === ''), 'Open the application at / first');
const size = await page.eval(() => [innerWidth, innerHeight]);
check(size[0] === 1400 && size[1] === 813, `The page is ${size.join(' x ')}; set a 1400 x 813 viewport first`);
await page.wait('#status.hidden', 60000);
await page.eval(() => {
  window.sdClick = (x, y) => {
    const canvas = document.getElementById('soliddraft');
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true, pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 };
    canvas.dispatchEvent(new PointerEvent('pointermove', { ...at, buttons: 0 }));
    canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
    return new Promise(resolve => setTimeout(() => {
      canvas.dispatchEvent(new PointerEvent('pointerup', { ...at, buttons: 0 }));
      resolve(true);
    }, 50));
  };
  // File > Open asks for a file through a detached input; answer it with the bracket.
  const click = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.sdOpen) return click.call(this);
    const input = this, name = window.sdOpen;
    window.sdOpen = null;
    fetch(`/fixtures/${name}`).then(response => response.arrayBuffer()).then(bytes => {
      const files = new DataTransfer();
      files.items.add(new File([bytes], name));
      input.files = files.files;
      input.dispatchEvent(new Event('change'));
    });
  };
  window.featureCount = null;
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') window.featureCount = detail.count;
  });
  window.longTasks = [];
  new PerformanceObserver(list => {
    for (const entry of list.getEntries()) window.longTasks.push(Math.round(entry.duration));
  }).observe({ type: 'longtask' });
  const canvas = document.getElementById('soliddraft');
  canvas.focus();
  canvas.dispatchEvent(new PointerEvent('pointermove', { clientX: 900, clientY: 450, bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true }));
  return true;
});
const click = ([x, y]) => page.eval(`sdClick(${x}, ${y})`);
const shown = async name => {
  const drawn = await page.eval(`window.wasmBindings.${name}()`);
  return drawn ? JSON.parse(drawn) : null;
};
async function until(read, done, what, tries = 600) {
  let value = null;
  for (let i = 0; i < tries; i++) {
    value = await read();
    if (done(value)) return value;
    await page.wait(100);
  }
  throw new Error(`${what}: ${JSON.stringify(value)}`);
}
const building = () => shown('shown_build');
const idle = what => until(building, state => state && !state.pending, `${what} did not finish building`);
async function window_(what, act) {
  await page.eval(() => { window.longTasks = []; return true; });
  await act();
  const longest = await page.eval(() => Math.max(0, ...window.longTasks));
  console.log(`${what}: longest page task ${longest} ms`);
  check(longest < LONG, `${what} blocked the page for ${longest} ms`);
}
async function depthShown(depth, what) {
  await until(async () => (await shown('shown_details')) || {}, drawn => drawn.title === 'Boss-Extrude1'
    && Object.fromEntries(drawn.rows || []).Depth === `${depth} mm`, `${what}: Details does not show Depth ${depth} mm`);
}
async function typeDepth(depth) {
  await click(DEPTH);
  await page.wait(300);
  await page.press('Control+a');
  await page.type(String(depth));
  await page.press('Enter');
}

await window_('Opening the bracket', async () => {
  await page.eval(() => { window.sdOpen = 'bracket.dwprt'; return true; });
  await page.press('Control+o');
  await until(() => page.eval(() => window.featureCount), count => count === 10, 'The bracket did not open');
  await idle('The opened bracket');
});

await click(BASE);
await page.wait(300);
await click(DETAILS);
await depthShown(10, 'The opened bracket');

let sawBusy = false;
await window_('Changing the base depth to 12 mm', async () => {
  await typeDepth(12);
  const state = await until(building, state => state !== null, 'No build state was drawn', 50);
  sawBusy = state.pending && state.cancel !== undefined;
  await idle('The depth edit');
  await depthShown(12, 'The depth edit');
});
check(sawBusy, 'The page showed no busy state with a Cancel while the edit built');

await window_('Undo', async () => {
  await page.press('Control+z');
  await until(building, state => state && state.pending, 'Undo started no build', 50).catch(() => null);
  await idle('Undo');
  await depthShown(10, 'Undo');
});

await window_('Redo', async () => {
  await page.press('Control+y');
  await idle('Redo');
  await depthShown(12, 'Redo');
});

await window_('Cancelling a depth edit', async () => {
  await typeDepth(14);
  const state = await until(building, state => state && state.pending && state.cancel, 'The edit showed no Cancel', 100);
  await click([state.cancel.x, state.cancel.y]);
  await idle('The cancelled edit');
  await depthShown(12, 'The cancelled edit');
});

await click(CLOSE);
console.log('Edit off the page thread check passed');
