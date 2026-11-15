// Run through chrome-devtools-axi run after opening the packaged application at /?demo
// in a 1400 x 813 page, on an origin with no earlier sessions: a second load would leave
// the first session's recovery window over the canvas.
//
// Inspect > Check builds each solid step of the sample part again, in the boundary model
// and in the distance field. In the browser it runs on the page's thread. Catallax's
// kernel observations once read a clock the browser does not have, so the page stopped
// as soon as Check ran. The check clicks Inspect, then Check, and waits until the Part
// check window draws its outcome, read through the read-only `shown_part_check` export
// since the canvas has no text. Every solid step of the sample part must be confirmed by
// both models, and the page must report no crash. The page must then still take input:
// the window closes, and a feature tree row selected afterwards draws in Details.
function check(value, message) { if (!value) throw new Error(message); }
const INSPECT = [19, 241], CHECK = [87, 185], CLOSE_REPORT = [536, 35], DETAILS = [317, 52], BOSS = [380, 255];

check(await page.eval(() => location.search === '?demo'), 'Open the application at /?demo first');
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
  window.diagnostics = [];
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'diagnostic') window.diagnostics.push(detail.code);
  });
  return true;
});
const click = ([x, y]) => page.eval(`sdClick(${x}, ${y})`);

await click(INSPECT);
await page.wait(500);
await click(CHECK);
let shown = null;
for (let i = 0; i < 600 && !(shown && 'solid' in shown); i++) {
  const stopped = await page.eval(() => {
    const status = document.getElementById('status');
    return status.classList.contains('hidden') ? null : status.textContent;
  });
  check(stopped === null, `The page stopped during Check: ${stopped}`);
  const drawn = await page.eval(() => window.wasmBindings.shown_part_check());
  shown = drawn ? JSON.parse(drawn) : null;
  check(!(shown && 'error' in shown), `Check could not confirm the part: ${shown && shown.error}`);
  if (!(shown && 'solid' in shown)) await page.wait(100);
}
check(shown && 'solid' in shown, `The Part check window drew no outcome: ${JSON.stringify(shown)}`);
const diagnostics = await page.eval(() => window.diagnostics);
check(!diagnostics.includes('crash'), `The page reported ${diagnostics.join(', ')}`);
check(shown.solid > 0 && shown.confirmed === shown.solid,
  `Check confirmed ${shown.confirmed} of ${shown.solid} solid steps of the sample part`);

async function until(read, done, what) {
  let value = null;
  for (let i = 0; i < 100; i++) {
    value = await read();
    if (done(value)) return;
    await page.wait(100);
  }
  throw new Error(`After Check, ${what}: ${JSON.stringify(value)}`);
}
await click(CLOSE_REPORT);
await until(() => page.eval(() => window.wasmBindings.shown_part_check()), drawn => drawn === '',
  'the Part check window did not close');
await click(BOSS);
await page.wait(300);
await click(DETAILS);
await until(() => page.eval(() => window.wasmBindings.shown_details()),
  drawn => drawn !== '' && JSON.parse(drawn).title === 'Boss-Extrude3', 'Details does not show the selected Boss-Extrude3');
console.log(`Part check passed: ${shown.confirmed} of ${shown.solid} solid steps confirmed`);
