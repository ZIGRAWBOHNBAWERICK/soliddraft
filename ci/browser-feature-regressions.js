// Run with chrome-devtools-axi run in a fresh 1600 x 1000 page on the packaged app.
// Set the URL fragment to depth, offset, positions, formula, visibility, callouts, notes, or
// curved-from.
// Serve the application test fixtures written to SOLIDDRAFT_BROWSER_FIXTURES at /fixtures/.
await page.wait('#status.hidden', 60000);
const scenario = await page.eval(() => location.hash.slice(1));

await page.eval(() => {
  const canvas = document.getElementById('soliddraft');
  canvas.focus();
  const frame = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  window.featureClick = async ({ x, y }) => {
    const at = { clientX: x, clientY: y, bubbles: true, cancelable: true,
      pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 };
    canvas.dispatchEvent(new MouseEvent('mousemove', at));
    await frame();
    // eframe runs app logic synchronously on both edges. Keep one click even
    // when rendering between edges would exceed egui's click duration.
    const down = new PointerEvent('pointerdown', { ...at, buttons: 1 });
    const up = new PointerEvent('pointerup', at);
    canvas.dispatchEvent(down);
    canvas.dispatchEvent(up);
    return true;
  };
  window.featureFieldHover = async name => {
    const locate = () => JSON.parse(window.wasmBindings.shown_controls())[name];
    const at = locate();
    if (!at) return { at: null, hovered: false };
    canvas.dispatchEvent(new MouseEvent('mousemove', { clientX: at.x, clientY: at.y, bubbles: true }));
    await frame();
    const current = locate();
    return {
      at: current ?? null,
      hovered: current?.x === at.x && current.y === at.y && canvas.style.cursor === 'text',
    };
  };
  window.featureCommands = [];
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'action') window.featureCommands.push(detail.command);
  });

  const choose = HTMLInputElement.prototype.click;
  HTMLInputElement.prototype.click = function () {
    if (this.type !== 'file' || !window.featureUpload) return choose.call(this);
    const files = new DataTransfer();
    files.items.add(window.featureUpload);
    window.featureUpload = null;
    this.files = files.files;
    this.dispatchEvent(new Event('change'));
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
    if (!/\.dw(prt|drw)$/.test(this.download)) return download.call(this);
    window.featureSaved = new File([blobs.get(this.href)], this.download);
  };
  window.featureSavedJson = async member => {
    const bytes = await window.featureSaved.arrayBuffer();
    const zip = new DataView(bytes);
    // ZIP end record and central directory give the compressed member's size and offset.
    let end = bytes.byteLength - 22;
    while (end >= 0 && zip.getUint32(end, true) !== 0x06054b50) end--;
    if (end < 0) throw new Error('The download is missing its ZIP end record');
    let entry = zip.getUint32(end + 16, true);
    for (let i = 0; i < zip.getUint16(end + 10, true); i++) {
      const nameSize = zip.getUint16(entry + 28, true);
      const name = new TextDecoder().decode(new Uint8Array(bytes, entry + 46, nameSize));
      if (name === member) {
        const local = zip.getUint32(entry + 42, true);
        const start = local + 30 + zip.getUint16(local + 26, true) + zip.getUint16(local + 28, true);
        const data = new Uint8Array(bytes, start, zip.getUint32(entry + 20, true));
        const blob = new Blob([data]);
        if (zip.getUint16(entry + 10, true) === 0) return JSON.parse(await blob.text());
        return new Response(blob.stream().pipeThrough(new DecompressionStream('deflate-raw'))).json();
      }
      entry += 46 + nameSize + zip.getUint16(entry + 30, true) + zip.getUint16(entry + 32, true);
    }
    throw new Error(`The download has no ${member}`);
  };
  return true;
});

function check(condition, message) {
  if (!condition) throw new Error(message);
}
const shown = name => page.eval(`JSON.parse(window.wasmBindings.${name}() || 'null')`);
const controls = () => shown('shown_controls');
const threads = () => page.eval(() => window.wasmBindings.shown_threads());
const textFocused = () => page.eval(() =>
  document.activeElement instanceof HTMLInputElement && document.activeElement.type === 'text');
async function until(read, accept, message, limit = 25000) {
  let last;
  const end = Date.now() + limit;
  do {
    last = await read();
    if (accept(last)) return last;
    await page.wait(100);
  } while (Date.now() < end);
  const state = await page.eval(() => ({
    focused: document.activeElement?.tagName,
    commands: window.featureCommands,
    build: JSON.parse(window.wasmBindings.shown_build() || 'null'),
    controls: JSON.parse(window.wasmBindings.shown_controls()),
  }));
  throw new Error(`${message}. Last value ${JSON.stringify(last)}; app ${JSON.stringify(state)}`);
}
async function clickAt(at) {
  await page.eval(`featureClick(${JSON.stringify(at)})`);
}
async function command(label, id) {
  await page.press('Control+Shift+p');
  await until(controls, value => value?.['Command palette'], 'The command palette did not open');
  await until(textFocused, Boolean, 'The command palette did not focus its browser text input');
  await page.type(label);
  const result = `Command: ${id}`;
  await until(controls, shown => {
    const first = Object.entries(shown).filter(([key]) => key.startsWith('Command: '))
      .reduce((top, entry) => !top || entry[1].y < top[1].y ? entry : top, null);
    return first?.[0] === result;
  }, `${label} never became the first palette result`);
  const before = await page.eval(() => window.featureCommands.length);
  await page.press('Enter');
  const received = await until(
    () => page.eval(`window.featureCommands[${before}] ?? ''`), Boolean, `No command dispatched for ${label}`,
  );
  check(received === id, `Expected ${id}, but the app dispatched ${received}`);
  await until(controls, value => !value?.['Command palette'], `The palette stayed open for ${label}`);
  await until(textFocused, focused => !focused, `The palette kept browser text focus after ${label}`);
}
async function replace(name, value) {
  // The first modal frame can publish the cell before egui will hit-test it.
  const field = await until(
    () => page.eval(`featureFieldHover(${JSON.stringify(name)})`),
    value => value.hovered,
    `${name} was not ready for a click`,
  );
  await clickAt(field.at);
  await until(textFocused, Boolean, `${name} did not take browser text focus`);
  await page.press('Control+a');
  if (value.length === 1) await page.press(value);
  else await page.type(value);
  await page.press('Enter');
  await until(textFocused, focused => !focused, `${name} did not end editing`);
}
async function parameters() {
  await command('Parameters', 'configuration_grid');
  // Expand the pane so its rows have room below the new-global controls.
  const docked = await until(controls, value => value?.Expand, 'The parameters pane did not open');
  await clickAt(docked.Expand);
  await until(controls, value => value?.['Back to dock'], 'The expanded parameters pane did not open');
}
async function closeParameters() {
  const expanded = await until(controls, value => value?.['Back to dock'], 'The expanded parameters pane did not open');
  await clickAt(expanded['Back to dock']);
  await until(controls, value => value?.Expand, 'The expanded parameters pane stayed open');
}
async function open(name, building = 25000) {
  await page.eval(`(async () => {
    const response = await fetch('/fixtures/' + ${JSON.stringify(name)});
    if (!response.ok) throw new Error('Fixture is missing');
    window.featureUpload = new File([await response.blob()], ${JSON.stringify(name)});
    return true;
  })()`);
  await page.press('Control+o');
  await until(() => page.eval(() => window.featureUpload === null), Boolean, 'Open did not consume the selected file');
  if (name.endsWith('.dwprt')) {
    await until(controls, value => value?.[`Tab ${name}`], `${name} did not open`, building);
    await until(() => shown('shown_build'), value => value && !value.pending && !value.reading, 'Part is still building',
      building);
  }
}
async function save(drawing = false) {
  await page.eval(() => { window.featureSaved = null; return true; });
  if (drawing) await command('Save drawing', 'drawing_save');
  else await page.press('Control+s');
  await until(() => page.eval(() => window.featureSaved?.size), size => size > 0, 'Save did not download the file');
  return page.eval(`featureSavedJson('${drawing ? 'drawing' : 'part'}.json')`);
}
async function callout() {
  await command('Manufacturing information', 'manufacturing_information');
  const pmi = await until(() => shown('shown_manufacturing_information'), value => value?.items.length, 'No PMI was drawn');
  return pmi.items[0].callout;
}
const drawing = () => shown('shown_drawing');
async function makeDrawing() {
  await command('Make drawing', 'make_drawing');
  await page.press('Enter');
  return until(drawing, value => value?.up_to_date, 'The drawing did not open');
}

switch (scenario) {
  case 'depth': {
    await open('wizard-blind-depth.dwprt');
    await parameters();
    await replace('Default Hole1.Depth', '5');
    await closeParameters();
    await until(() => shown('shown_build'), value => value && !value.pending && !value.reading, 'Edited part is still building');
    const stored = await save();
    const hole = stored.features.find(f => f.kind.type === 'hole');
    check(hole.kind.depth.depth === 5, 'The Parameters edit did not commit the 5 mm depth');
    const text = await callout();
    check(text === 'M6x1-6H DEEP 5', `The edited hole reads ${text}`);
    break;
  }
  case 'offset': {
    await open('wizard-tapped-offset.dwprt');
    const text = await callout();
    check(text === 'M6x1-6H DEEP 15', `The blind surface-offset hole reads ${text}`);
    break;
  }
  case 'positions': {
    // The application test edits the legacy hole through Positions and saves this file.
    await open('wizard-edited-positions.dwprt');
    const stored = await save();
    const hole = stored.features.find(f => f.kind.type === 'hole');
    const sketch = stored.features.find(f => f.id === hole.kind.sketch);
    check(sketch?.kind.sketch.patterns?.length === 1, 'The reopened hole lost its position pattern');
    check(sketch.kind.sketch.constraints.length > 0, 'The reopened hole lost its position constraints');
    break;
  }
  case 'formula': {
    await open('formula-case-distinct.dwprt');
    await parameters();
    await replace('Name Plate', 'Other');
    await until(controls, value => value?.['Name Other'], 'The global rename did not commit');
    await closeParameters();
    const stored = await save();
    check(stored.values.entries.some(entry => entry.name === 'Other'), 'The global rename did not commit');
    const overrides = stored.configurations.rows.find(row => row.name === 'Thin').overrides;
    check(overrides['BossExtrude1.Depth'] === '=plate', `The depth now refers to ${overrides['BossExtrude1.Depth']}`);
    check(overrides.Width === '=Other', `The width now refers to ${overrides.Width}`);
    break;
  }
  case 'visibility': {
    await open('wizard-tapped-offset.dwprt');
    await until(threads, count => count > 0, 'The visible solid did not draw its thread marks');
    await open('thread-hidden.dwprt');
    const view = () => shown('shown_viewport');
    await until(view, value => value?.display_state === 'Hidden' && value.visible_meshes === 0, 'The body remained visible');
    check(await threads() === 0, 'The hidden solid still drew thread marks');
    break;
  }
  case 'callouts': {
    await open('drawing-skimmed-counterbores.dwprt');
    const initial = await makeDrawing();
    let annotations = initial.annotations;
    for (const [x, z] of [[20, 9], [50, 10]]) {
      await command('Hole callout', 'drawing_hole_callout');
      const [sx, sy] = await page.eval(`Array.from(window.wasmBindings.drawing_screen_point('Top view', ${x}, 20, ${z}))`);
      await clickAt({ x: sx, y: sy });
      annotations++;
      await until(drawing, value => value?.annotations === annotations, `No callout was added at ${x}, ${z}`);
    }
    const stored = await save(true);
    const callouts = stored.sheets[0].annotations.filter(a => a.body.type === 'hole_callout').map(a => a.body.text);
    check(callouts.every(text => !/^2\s*×/.test(text)) &&
      callouts.some(text => / 7\n.* 2$/.test(text)) &&
      callouts.some(text => / 8\n.* 3$/.test(text)), `Counterbore callouts read ${JSON.stringify(callouts)}`);
    break;
  }
  case 'notes': {
    // The application test removes the skim and refreshes the drawing before saving both files.
    await open('Tapped plate.dwprt');
    await open('drawing-merged-threads.dwdrw');
    await until(drawing, value => value?.annotations > 0, 'The refreshed drawing did not open');
    const notes = stored => stored.sheets[0].annotations.filter(a => a.source?.type === 'thread').map(a => a.body.text);
    const after = notes(await save(true));
    check(after.length === 1 && after[0].startsWith('2× M6x1-6H') &&
      after[0].endsWith(' 6'), `Rebuilt thread notes read ${JSON.stringify(after)}`);
    break;
  }
  case 'curved-from': {
    // The headless application test builds a boss from the rod's curved top, then a later post.
    // The web build takes about two minutes to build this part (seconds natively); the
    // longer wait keeps the check on the result until that is fixed.
    await open('extrude-from-curved.dwprt', 240000);
    const view = () => shown('shown_viewport');
    await until(view, value => value?.visible_meshes > 0, 'The part with a curved start did not draw');
    // The rod alone draws too: the boss itself must be in the tree with no Refused or
    // Working mark, which a built feature never has.
    const bossBuilt = async () => {
      const [tree, marks] = [await controls(), await shown('shown_marks')];
      return Boolean(tree?.['Tree: Boss-Extrude2']) && Array.isArray(marks) &&
        !marks.some(mark => mark.feature === 'Boss-Extrude2' || mark.mark === 'Refused');
    };
    await until(bossBuilt, built => built, 'The boss on the curved face did not build');
    await parameters();
    await replace('Default BossExtrude2.Depth', '9');
    await closeParameters();
    await until(() => shown('shown_build'), value => value && !value.pending && !value.reading, 'Edited part is still building',
      240000);
    await until(view, value => value?.visible_meshes > 0, 'The rebuilt part did not draw');
    await until(bossBuilt, built => built, 'The edited boss on the curved face did not build');
    const stored = await save();
    const boss = stored.features.find(f => f.kind.type === 'extrude' && f.kind.from);
    check(boss?.kind.from.type === 'surface' && boss.kind.from.plane.on === 'face',
      `The curved start did not survive: ${JSON.stringify(boss?.kind.from)}`);
    check(boss.kind.end.depth === 9, `The Parameters edit left depth ${JSON.stringify(boss.kind.end)}`);
    break;
  }
  default:
    throw new Error(`Unknown feature regression ${scenario}`);
}
console.log(`Passed browser feature regression ${scenario}`);
