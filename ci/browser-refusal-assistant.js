// Run through chrome-devtools-axi run on a fresh 1400 x 813 packaged app at /.
// Build a block, accept the radius offered for a refused fillet, edit the fillet,
// reopen the downloaded part, and make a reference plane from its geometry.
function check(value, message) { if (!value) throw new Error(message); }
const NEW_PART = [124, 143], RECTANGLE = [630, 52], FILE = [122, 15], NEW_FROM_MENU = [138, 42];
const TOP_EDGE = [923, 406], TOP_FACE = [950, 310], FILLET_ROW = [380, 229];
const COMMAND_LINE = [920, 797], CLOSE_DETAILS = [347, 92];
check(await page.eval(() => location.search === '' && innerWidth === 1400 && innerHeight === 813),
  'Open a fresh 1400 x 813 SolidDraft page at /');
await page.wait('#status.hidden', 60000);

await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  const at = (x, y) => ({ clientX: x, clientY: y, bubbles: true, cancelable: true,
    pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 });
  window.sdMove = (x, y) => {
    canvas.dispatchEvent(new MouseEvent('mousemove', at(x, y)));
    canvas.dispatchEvent(new PointerEvent('pointermove', at(x, y)));
    return true;
  };
  window.sdClick = async (x, y) => {
    window.sdMove(x, y);
    await new Promise(resolve => setTimeout(resolve, 300));
    canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at(x, y), buttons: 1 }));
    await new Promise(resolve => setTimeout(resolve, 50));
    canvas.dispatchEvent(new PointerEvent('pointerup', at(x, y)));
    canvas.focus();
    return true;
  };
  window.sdPart = null;
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') window.sdPart = detail;
  });
  // Keep the downloaded part so File > Open can use the same bytes.
  const download = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (this.download.endsWith('.dwprt')) {
      window.sdSaved = fetch(this.href).then(async response => {
        if (!response.ok) throw new Error('The saved part could not be read');
        return new File([await response.blob()], this.download);
      });
    }
    return download.call(this);
  };
  const choose = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.sdReopen) return choose.call(this);
    window.sdReopen = false;
    window.sdSaved.then(file => {
      const transfer = new DataTransfer();
      transfer.items.add(file);
      this.files = transfer.files;
      this.dispatchEvent(new Event('change'));
    });
  };
  canvas.focus();
  return true;
});

const click = ([x, y]) => page.eval(`sdClick(${x}, ${y})`);
const shown = async name => {
  const drawn = await page.eval(`window.wasmBindings.${name}()`);
  return drawn ? JSON.parse(drawn) : null;
};
async function until(read, ready, what) {
  let value;
  for (let i = 0; i < 300; i++) {
    value = await read();
    if (ready(value)) return value;
    await page.wait(100);
  }
  throw new Error(`${what}: ${JSON.stringify(value)}`);
}
const part = () => page.eval(() => window.sdPart);
const features = count => until(part, doc => doc?.extension === '.dwprt' && doc.count === count, `Expected ${count} features`);
const idle = () => until(() => shown('shown_build'), build => build && !build.pending && !build.reading, 'The build did not finish');
const center = point => [point.x, point.y];
const controls = () => shown('shown_controls');
async function confirmForm() {
  const form = await until(controls, drawn => drawn?.['Form OK'], 'The form did not draw its OK button');
  await click(center(form['Form OK']));
}
async function pick(point, matches, what) {
  // The viewport reads the pick under the mouse back from the renderer.
  await page.eval(`sdMove(${point[0]}, ${point[1]})`);
  let selected;
  for (let attempt = 0; attempt < 3; attempt++) {
    await page.wait(750);
    await click(point);
    await page.wait(250);
    selected = (await shown('shown_options'))?.selected;
    if (matches(selected)) return selected;
  }
  throw new Error(`${what}: ${selected}`);
}
async function typeRadius(radius) {
  const form = await until(controls, drawn => drawn?.['Radius value'], 'Open the Fillet form');
  await click(center(form['Radius value']));
  await page.press('Control+a');
  await page.type(radius);
}
async function command(name) {
  await click(COMMAND_LINE);
  await page.type(name);
  await page.press('Enter');
}
async function showRadius(radius) {
  const picked = await until(() => shown('shown_options'), options => options?.selected === 'Fillet1' && options.details,
    'The built fillet is not selected');
  await click(center(picked.details));
  const details = await until(() => shown('shown_details'), drawn => drawn?.title === 'Fillet1', 'No details for the fillet');
  check(Object.fromEntries(details.rows).Radius === `${radius} mm`, `The fillet has the wrong radius: ${JSON.stringify(details)}`);
  await click(CLOSE_DETAILS);
}

await click(NEW_PART);
await features(0);
await page.eval(() => sdMove(900, 450));
await page.press('e');
await page.wait(500);
await click(RECTANGLE);
await page.press('Enter');
await features(1);
await idle();
await page.press('i');
await page.press('z');
await page.wait(500);
console.log('block built');

await pick(TOP_EDGE, selected => selected?.includes('Edge'), 'Pick the top edge');
await page.press('f');
await typeRadius('50');

const why = await until(() => page.eval(() => window.wasmBindings.shown_feature_refusal()), Boolean,
  'The Fillet form did not explain the refusal');
check(why.includes('Try a radius under') && !why.includes('fillet.radius-too-large'), why);
check((await part()).count === 1, 'The refusal changed the feature tree');
const offers = Object.entries(await controls()).filter(([label]) => /^Apply [\d.]+ mm radius$/.test(label));
check(offers.length === 1, `Expected one correction, got ${JSON.stringify(offers)}`);
const [[offer, at]] = offers;
const radius = offer.split(' ')[1];
check(Number(radius) === 19.99, `The 20 mm high block offered R${radius}`);
console.log(`${why} ${offer}`);
await click(center(at));
await features(2);
await idle();
await showRadius(radius);

await command('edit_feature');
await typeRadius('5');
await confirmForm();
await idle();
await showRadius('5');
await page.press('Control+s');
await until(() => page.eval(async () => (await window.sdSaved)?.size || 0), size => size > 0, 'Save did not download the part');
await click(FILE);
await click(NEW_FROM_MENU);
await features(0);
await page.eval(() => { window.sdReopen = true; return true; });
await page.press('Control+o');
await features(2);
await idle();
await click(FILLET_ROW);
await showRadius('5');

await page.press('i');
await page.press('z');
await page.wait(500);
const face = await pick(TOP_FACE, selected => selected === 'Boss-Extrude1 · End cap', 'Pick the top face');
await command('reference_plane');
await confirmForm();
await features(3);
await idle();
console.log(`Accepted R${radius}, edited it to R5, saved and reopened it, and added a plane on ${face}`);
