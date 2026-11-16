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
// `shown_details`, since the canvas has no text. A cancelled edit's answer still comes
// from the worker; the page drops the builds it carries, as its kernel trace records,
// so the next edit builds from its own.
//
// Each edit also logs the time from Enter until no build is owed, when the edited part
// is usable. Last, Measure, the first command after the edits that needs exact geometry,
// logs the time until it ran and its longest page task. It has no bound, so deferring
// work from the edits cannot hide it, but a page task of 250 ms or more must come
// behind the busy state the page draws while it reads the part.
function check(value, message) { if (!value) throw new Error(message); }
const CLOSE = [347, 92], BASE = [380, 203], DEPTH = [515, 268];
const INSPECT = [19, 241], MEASURE = [92, 112];
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
  // Each kernel job's steps on the page, which the page records once this exists.
  window.sdKernelTrace = [];
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
  return page.eval(() => performance.now());
}
const now = () => page.eval(() => performance.now());
async function usable(what, entered) {
  await idle(what);
  console.log(`${what}: usable ${Math.round((await now()) - entered)} ms after Enter`);
}

await window_('Opening the bracket', async () => {
  await page.eval(() => { window.sdOpen = 'bracket.dwprt'; return true; });
  await page.press('Control+o');
  await until(() => page.eval(() => window.featureCount), count => count === 10, 'The bracket did not open');
  await idle('The opened bracket');
});

// The options bar draws Details for a new selection only in a later frame, so the
// pointer moves until a frame shows both, and Details is clicked where it was drawn.
await click(BASE);
const nudge = () => page.eval(() => {
  document.getElementById('soliddraft').dispatchEvent(new PointerEvent('pointermove',
    { clientX: 900, clientY: 450, bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true }));
  return true;
});
const options = await until(async () => {
  await nudge();
  return shown('shown_options');
}, drawn => drawn && drawn.selected === 'Boss-Extrude1' && drawn.details, 'Boss-Extrude1 is not selected with Details drawn');
await click([Math.round(options.details.x), Math.round(options.details.y)]);
await depthShown(10, 'The opened bracket');

let sawBusy = false;
await window_('Changing the base depth to 12 mm', async () => {
  const entered = await typeDepth(12);
  // The opened part already reports a finished build, so wait for the edit's own.
  const state = await until(building, state => state && state.pending, 'The edit drew no busy state', 50);
  sawBusy = Boolean(state.cancel);
  await usable('The depth edit', entered);
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
  const before = await page.eval(() => window.sdKernelTrace.length);
  await typeDepth(14);
  const state = await until(building, state => state && state.pending && state.cancel, 'The edit showed no Cancel', 100);
  await click([state.cancel.x, state.cancel.y]);
  // Cancel is drawn over the viewport, where the pointer would have the viewport pick
  // under it every frame: a GPU readback that software rendering takes most of a
  // second to answer. The pointer moves to the side panel, as a user's would.
  await page.eval(() => {
    document.getElementById('soliddraft').dispatchEvent(new PointerEvent('pointermove',
      { clientX: 150, clientY: 450, bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true }));
    return true;
  });
  await idle('The cancelled edit');
  await depthShown(12, 'The cancelled edit');
  // The worker answers the cancelled edit later; the page drops the builds it carries.
  const dropped = () => page.eval(`window.sdKernelTrace.slice(${before}).some(step => step.stage === 'page dropped builds')`);
  await until(dropped, Boolean, 'The cancelled edit\'s answer never came, or its builds were offered');
  await idle('After the cancelled edit\'s answer');
  await depthShown(12, 'After the cancelled edit\'s answer');
});

await window_('Changing the base depth to 16 mm after the cancel', async () => {
  const entered = await typeDepth(16);
  await until(building, state => state && state.pending, 'The edit after the cancel drew no busy state', 50);
  await usable('The edit after the cancel', entered);
  await depthShown(16, 'The edit after the cancel');
});

await click(CLOSE);
await click(INSPECT);
await page.wait(500);
// The busy state may last one frame before the read, so the page notes it each frame.
await page.eval(() => {
  window.longTasks = [];
  window.sawReading = false;
  window.watching = true;
  const watch = () => {
    const drawn = JSON.parse(window.wasmBindings.shown_build() || 'null');
    if (drawn && drawn.reading) window.sawReading = true;
    if (window.watching) requestAnimationFrame(watch);
  };
  requestAnimationFrame(watch);
  return true;
});
const asked = await now();
await click(MEASURE);
// A few frames for the page to take the click and hold Measure while it reads.
await page.wait(300);
await until(building, state => state && !state.pending && !state.reading, 'Measure did not run');
await page.wait(500);
const sawReading = await page.eval(() => { window.watching = false; return window.sawReading; });
const longest = await page.eval(() => Math.max(0, ...window.longTasks));
console.log(`The first exact action, Measure: ran ${Math.round((await now()) - asked)} ms after the click; longest page task ${longest} ms; busy state ${sawReading ? 'drawn' : 'not drawn'}`);
check(longest < LONG || sawReading, `Measure blocked the page for ${longest} ms with no busy state`);
console.log('Edit off the page thread check passed');
