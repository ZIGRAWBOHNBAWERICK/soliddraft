// Run with chrome-devtools-axi run on the packaged app. Serve the
// display-states*.dwprt produced by the appearance_panel test at /fixtures/.
function check(value, message) {
  if (!value) throw new Error(message);
}

await page.wait('#status.hidden', 60000);
await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();

  window.sdClick = (x, y) => {
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true, pointerId: 1,
      pointerType: 'mouse', isPrimary: true, button: 0 };
    canvas.dispatchEvent(new PointerEvent('pointermove', { ...at, buttons: 0 }));
    return new Promise(resolve => setTimeout(() => {
      canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
      setTimeout(() => {
        canvas.dispatchEvent(new PointerEvent('pointerup', { ...at, buttons: 0 }));
        resolve(true);
      }, 150);
    }, 150));
  };

  const click = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.sdOpen) return click.call(this);
    const input = this;
    const [name, pending] = window.sdOpen;
    window.sdOpen = null;
    Promise.resolve(pending).then(blob => {
      const files = new DataTransfer();
      files.items.add(new File([blob], name));
      input.files = files.files;
      input.dispatchEvent(new Event('change'));
    });
  };

  const blobs = new Map();
  const create = URL.createObjectURL.bind(URL);
  URL.createObjectURL = blob => {
    const url = create(blob);
    blobs.set(url, blob);
    return url;
  };
  const download = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    if (this.download.endsWith('.dwprt')) window.sdSaved = blobs.get(this.href);
    return download.call(this);
  };
  window.sdFixture = name => [name, fetch(`/fixtures/${name}`).then(r => {
    if (!r.ok) throw new Error(`Fixture ${name} is missing`);
    return r.blob();
  })];
  window.sdOpen = window.sdFixture('display-states-service.dwprt');
  return true;
});

async function until(read, predicate, message) {
  let value;
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    value = await read();
    if (predicate(value)) return value;
    await page.wait(100);
  }
  throw new Error(`${message}. Last value ${JSON.stringify(value)}`);
}

const viewport = () => page.eval(() => JSON.parse(window.wasmBindings.shown_viewport() || 'null'));
const controls = () => page.eval(() => JSON.parse(window.wasmBindings.shown_controls()));
async function click(name) {
  const at = await until(controls, value => Boolean(value[name]), `Control ${name} is missing`);
  await page.eval(`sdClick(${at[name].x}, ${at[name].y})`);
}
async function view(name, bodies) {
  return until(viewport, value => value?.display_state === name && value.visible_meshes === bodies,
    `State ${name ?? 'Default'} did not draw ${bodies} bodies`);
}

await page.press('Control+o');
const opened = await view('Working', 1);
check(opened.display_mode === 'Shaded', 'The display mode did not reopen');

async function selectState(name, bodies) {
  const item = `Display state: ${name ?? 'Default'}`;
  for (let attempt = 0; attempt < 2; attempt++) {
    if (!(await controls())[item]) await click('Display state');
    await until(controls, value => Boolean(value[item]), `${item} did not open`);
    await page.wait(250);
    await click(item);
    await page.wait(400);
    const current = await viewport();
    if (current?.display_state === name && current.visible_meshes === bodies) return;
  }
  await view(name, bodies);
}
await selectState(null, 2);
await selectState('Working', 1);

await page.press('Control+s');
await until(() => page.eval(() => window.sdSaved?.size), size => size > 0, 'Save did not produce a part file');
// Open the same part in its other configuration, then reopen the saved download.
await page.eval(() => { window.sdOpen = window.sdFixture('display-states.dwprt'); return true; });
await page.press('Control+o');
await view('Inspection', 2);
await page.eval(() => { window.sdOpen = ['browser-saved.dwprt', window.sdSaved]; return true; });
await page.press('Control+o');
await until(() => page.eval(() => window.sdOpen === null), Boolean, 'Reopen did not consume the saved file');
await view('Working', 1);
console.log('Opened a configured part, switched display states in the tree, saved it and reopened the saved state.');
