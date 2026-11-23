// Run with chrome-devtools-axi run on the packaged app. Serve the
// display-states*.dwprt produced by the appearance_panel test at /fixtures/.
function check(value, message) {
  if (!value) throw new Error(message);
}

await page.wait('#status.hidden', 60000);
await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();

  const aim = document.createElement('button');
  aim.id = 'sd-test-pointer';
  aim.textContent = 'Test pointer';
  Object.assign(aim.style, { position: 'fixed', opacity: '1', color: 'transparent', background: 'transparent', zIndex: '9999',
    width: '4px', height: '4px', padding: '0', border: '0' });
  for (const type of ['pointermove', 'pointerdown', 'pointerup']) {
    aim.addEventListener(type, event => {
      event.preventDefault();
      canvas.dispatchEvent(new PointerEvent(type, {
        clientX: event.clientX, clientY: event.clientY, bubbles: true, cancelable: true,
        pointerId: event.pointerId, pointerType: event.pointerType, isPrimary: event.isPrimary,
        button: event.button, buttons: event.buttons,
      }));
    });
  }
  document.body.append(aim);
  window.sdAim = (x, y) => {
    aim.style.left = (x - 2) + 'px';
    aim.style.top = (y - 2) + 'px';
    return true;
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
  await page.eval(`sdAim(${at[name].x}, ${at[name].y})`);
  const target = (await page.snapshot()).match(/uid=([^\s]+) button "Test pointer"/);
  check(target, 'Missing test pointer in the accessibility snapshot');
  await page.click('@' + target[1]);
}
async function view(name, bodies, action) {
  return until(viewport, value => value?.display_state === name && value.visible_meshes === bodies,
    `${action}: state ${name ?? 'Default'} did not draw ${bodies} bodies`);
}

await page.press('Control+o');
const opened = await view('Working', 1, 'Open Service fixture');
check(opened.display_mode === 'Shaded', 'The display mode did not reopen');

async function selectState(name, bodies) {
  const item = `Display state: ${name ?? 'Default'}`;
  await click('Display state');
  await click(item);
  await view(name, bodies, `Select ${name ?? 'Default'}`);
}
await selectState(null, 2);
await selectState('Working', 1);

await page.press('Control+s');
await until(() => page.eval(() => window.sdSaved?.size), size => size > 0, 'Save did not produce a part file');
// Open the same part in its other configuration, then reopen the saved download.
await page.eval(() => { window.sdOpen = window.sdFixture('display-states.dwprt'); return true; });
await page.press('Control+o');
await view('Inspection', 2, 'Open Default fixture');
await page.eval(() => { window.sdOpen = ['browser-saved.dwprt', window.sdSaved]; return true; });
await page.press('Control+o');
await until(() => page.eval(() => window.sdOpen === null), Boolean, 'Reopen did not consume the saved file');
await view('Working', 1, 'Reopen the download');
console.log('Opened a configured part, switched display states in the tree, saved it and reopened the saved state.');
