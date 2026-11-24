// Assemblies in the browser build, driven in headless Chrome as a visitor drives them.
// Usage: node ci/browser-assemblies.mjs CHROME URL OUTDIR TRAY COLLAR RIG BLOCK SHAFT BEARING
//
// TRAY and COLLAR are part files (site/parts/tray.dwprt and collar.dwprt). The page has no
// folder, so every part reaches it as a file handed over. The visitor starts a new
// assembly from the start page, inserts both parts with one upload and the tray again
// with a second, and mates the second tray's underside onto the first tray's floor, picked
// in the viewport from above and from below: the later feature on top, which needs each
// tray's exact solid from the bytes the page holds. The visitor saves the assembly (a
// download), loads the site again and opens the saved file with the two part files in
// one upload: the same three components at the same placements, the mate kept, every
// part found at the revision it pins. Last, the saved file opens alone: the parts still
// show, from the copies the file keeps, and each says its file cannot be found.
//
// RIG, BLOCK and SHAFT are ci/fixtures/universal-joint: two shafts on hinges in two blocks
// locked 40 degrees apart. The visitor opens the rig with both parts, chooses Universal
// Joint in the Mate panel's type list and picks the input shaft, then the output shaft,
// seen from the right. Then one later feature on top: the input turned 30 degrees through
// its Rotate Z placement field drives the output by the Hooke relation,
// tan(output) = tan(input) / cos(40 degrees). The joint is saved and reopened with its
// parts.
//
// Last, a parent assembly that places the saved tray assembly is opened from one upload
// that lists that child first, with the tray and collar parts. Neither opens on its own:
// the page asks which one to open, the visitor chooses the parent, and it opens with the
// child and its parts found. Then one later feature on top: the tray inserted in the
// parent the choice opened. Then the same upload again with the choice left unanswered,
// and the child opened alone through the start page's Open tile, which the choice window leaves uncovered: the old choice is gone, so it cannot
// open over the child.
//
// BEARING is ci/fixtures/universal-joint/bearing.dwasm: the rig with its Universal Joint
// made and the lock between the blocks deleted, so the output block turns freely. The
// visitor selects that block and types 90 degrees into its Rotate X placement field,
// which would set the shafts at right angles. The drag is refused by name and leaves the
// placements, the mates and undo as they were. Then one later feature on top: 60 degrees
// in the same field is taken, the output shaft follows its bore, and Undo puts it back.
//
// Then configurations and the exploded view on the saved tray assembly, opened with its
// parts: the visitor opens the Configurations panel, names a configuration Lifted and adds
// it, activates it, and explodes it with Auto explode. The parts move apart on screen and
// their placements do not; Explode collapses them back. Saved and opened again with its
// parts, the assembly is in Lifted with its exploded view, and activating Default, made
// before the view, shows none: each configuration keeps its own.
//
// Then the Mate Controller on the saved tray assembly: the floor mate is set as the driven
// mate, its value saved as position low, raised 12 mm through the controller's value field
// and saved as high, animated back to low, saved, opened again with both positions, and
// high applied.
// Last, Pack and Go and the robot exports, which download: the rig packs into one zip,
// whose files open again as the rig with every part found beside it and take a shaft
// inserted on top; Export URDF downloads the URDF and its meshes as one zip, Export USD a
// .usda file. Both still describe the assembled poses while the viewport is exploded.
//
// The app draws into one canvas and takes only trusted input: keys and clicks sent as the
// browser's own, through the DevTools protocol. The canvas has no text to read, so the
// check reads the assembly through the read-only `shown_assembly` export, and finds a
// face's place on screen through `assembly_screen_point`. Commands run through the
// command palette by name; Enter adds the mate the Mate panel holds. The clicks go to the
// start page's New assembly and Open tiles at fixed points of a 1400 x 813 page. Other
// controls are found through the read-only `shown_controls` export.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { CanvasBrowser } from './canvas-browser.mjs';

const [chrome, url, out, tray, collar, rig, block, shaft, bearing] = process.argv.slice(2);
if (!chrome || !url || !out || !tray || !collar || !rig || !block || !shaft || !bearing) {
  throw new Error('usage: browser-assemblies.mjs CHROME URL OUTDIR TRAY COLLAR RIG BLOCK SHAFT BEARING');
}
fs.mkdirSync(out, { recursive: true });
const downloads = path.join(path.resolve(out), 'downloads');
fs.rmSync(downloads, { recursive: true, force: true });
fs.mkdirSync(downloads, { recursive: true });
const SIZE = { width: 1400, height: 813 };
const NEW_ASSEMBLY = [304, 145];
const OPEN = [663, 147];
// The tray is 60 x 40 x 20 mm, open at the top, with 2 mm walls: its floor is 2 mm up.
const FLOOR = 2;

const sleep = ms => new Promise(r => setTimeout(r, ms));
function check(value, message) { if (!value) throw new Error(message); }
const started = Date.now();
function step(what) { console.error(`[${((Date.now() - started) / 1000).toFixed(1)} s] ${what}`); }
const extra = (process.env.CHROME_ARGS || '').split(/\s+/).filter(Boolean);

const loaded = shown => shown.open && !shown.opening && !shown.loading;
const parts = shown => shown.components.map(c => c.label.replace(/<.*/, ''));
function placements(shown) { return shown.components.map(c => [c.translation, c.rotation]); }
function near(a, b) { const [p, q] = [a.flat(2), b.flat(2)]; return p.length === q.length && p.every((x, i) => Math.abs(x - q[i]) < 1e-6); }
// Quaternions as [x, y, z, w], as the export gives them.
function multiply([ax, ay, az, aw], [bx, by, bz, bw]) {
  return [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz];
}
function about([x, y, z], angle) { const s = Math.sin(angle / 2); return [x * s, y * s, z * s, Math.cos(angle / 2)]; }
function rotate(q, [x, y, z]) {
  const [, rx, ry, rz] = [0, ...multiply(multiply(q, [x, y, z, 0]), [-q[0], -q[1], -q[2], q[3]])];
  return [rx, ry, rz];
}
// Two turns agree when they take X and Y to the same places.
function sameTurn(p, q, tolerance) {
  return [[1, 0, 0], [0, 1, 0]].every(axis => rotate(p, axis).every((x, i) => Math.abs(x - rotate(q, axis)[i]) < tolerance));
}

const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'soliddraft-assemblies-'));
let browser;

async function main() {
  browser = new CanvasBrowser({ chrome, profile, size: SIZE, url, out, downloads, extra });
  await browser.connect();
  const v = await browser.visitor();
  step('load the site');
  await v.load();
  check(await v.eval(`typeof window.wasmBindings.shown_assembly === 'function' && typeof window.wasmBindings.assembly_screen_point === 'function'`), 'the build has no assembly exports');

  // New assembly from the start page, then both parts in one upload.
  step('new assembly');
  await v.click(NEW_ASSEMBLY);
  await v.assembly('new', s => s.open && s.active && s.components.length === 0);
  step('insert the tray and the collar');
  await v.choose([tray, collar], () => v.command('Insert component'));
  let shown = await v.assembly('inserted', s => loaded(s) && s.components.length === 2);
  check(JSON.stringify(parts(shown)) === '["tray","collar"]', `the components are ${parts(shown)}`);
  check(shown.components.every(c => c.state === 'DisplayReady'), `not every part shows: ${JSON.stringify(shown.components)}`);
  check(JSON.stringify(shown.held) === '["tray.dwprt","collar.dwprt"]', `the page holds ${shown.held}`);
  step('insert the tray again');
  await v.choose(tray, () => v.command('Insert component'));
  shown = await v.assembly('inserted again', s => loaded(s) && s.components.length === 3);
  check(JSON.stringify(parts(shown)) === '["tray","collar","tray"]', `the components are ${parts(shown)}`);
  await v.shot('1-inserted');

  // The later feature on top: a mate, picked in the viewport.
  step('mate the second tray onto the first one\'s floor');
  await v.command('Mate');
  await v.assembly('mate panel', s => s.mate_panel.open);
  const [first, , second] = shown.components.map(c => c.translation);
  await v.command('Top');
  await v.command('Fit');
  await v.pick(await v.screen(first[0], first[1], first[2] + FLOOR));
  shown = await v.assembly('first pick', s => s.mate_panel.a);
  await v.command('Bottom');
  await v.command('Fit');
  await v.pick(await v.screen(second[0], second[1], second[2]));
  shown = await v.assembly('second pick', s => s.mate_panel.b);
  check(shown.mate_panel.a.component !== shown.mate_panel.b.component, `both picks are on one component: ${JSON.stringify(shown.mate_panel)}`);
  await v.shot('2-picked');
  await v.key('Enter', 'Enter', 13, 0, '\r');
  shown = await v.assembly('mated', s => s.mates === 1 || s.mate_panel.error);
  check(shown.mates === 1, `the mate was refused: ${shown.mate_panel.error}`);
  const [a, , b] = shown.components.map(c => c.translation);
  check(Math.abs(b[2] - a[2] - FLOOR) < 1e-9, `the second tray stands ${b[2] - a[2]} mm above the first, not ${FLOOR}`);
  await v.command('Isometric');
  await v.command('Fit');
  await v.shot('3-mated');
  const mated = shown;

  // Save (a download), load the site afresh, open the file with its parts.
  step('save');
  const saved = await v.download('save', () => v.key('s', 'KeyS', 83, 2, 's'));
  check(saved.endsWith('.dwasm'), `the assembly saves as a .dwasm file, not ${saved}`);
  // Later downloads can take the same name, so the configurations and Mate Controller
  // sections get a copy.
  const trayAssembly = path.join(downloads, 'tray-assembly.dwasm');
  fs.copyFileSync(saved, trayAssembly);
  shown = await v.assembly('saved', s => !s.dirty);
  step('load the site again and open the saved assembly with its parts');
  await v.load();
  await v.choose([saved, tray, collar], () => v.click(OPEN));
  shown = await v.assembly('reopened', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(parts(shown)) === JSON.stringify(parts(mated)), `the reopened components are ${parts(shown)}`);
  check(near(placements(shown), placements(mated)), `the reopened placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(placements(mated))}`);
  check(shown.mates === 1, `the reopened assembly has ${shown.mates} mates`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every part is found: ${JSON.stringify(shown.components)}`);
  await v.shot('4-reopened');
  const reopened = shown;

  // The file alone: its copies show, and each part says its file is not here.
  step('load the site again and open the saved assembly alone');
  await v.load();
  await v.choose(saved, () => v.click(OPEN));
  shown = await v.assembly('alone', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(near(placements(shown), placements(mated)), `alone, the placements are ${JSON.stringify(placements(shown))}`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'missing' && c.warning), `alone, the parts are ${JSON.stringify(shown.components)}`);
  await v.settle(1000);
  await v.stopped('at the end');
  await v.shot('5-alone');
  const alone = shown;

  const joint = await universalJoint(v);
  const choice = await chosenAssembly(v, saved, [tray, collar], parts(mated));
  const bearingDrag = await refusedDrag(v);
  const configured = await configuredExplode(v, trayAssembly, [tray, collar], mated);
  const controller = await mateController(v, trayAssembly, [tray, collar], mated);
  const packAndGo = await packAndExport(v);

  fs.writeFileSync(path.join(out, 'assemblies.json'), JSON.stringify({ mated, reopened, alone, saved: path.basename(saved), joint, choice, bearing: bearingDrag, configured, controller, packAndGo }, null, 1));
  console.log(`Assemblies in the browser: three components from two uploads, a mate picked in the viewport putting the second tray ${FLOOR} mm up on the first one's floor, saved as ${path.basename(saved)}, reopened with its parts at the same placements with the mate kept, and opened alone from the copies it keeps. A Universal Joint picked between two shafts meeting at ${BEND} degrees drove the output ${joint.follows.toFixed(6)} degrees for a ${TURN} degree input turn, and reopened with its parts. A parent and its child assembly uploaded child first opened the parent the visitor chose, with ${choice.chosen.held.length} files held for it, and took a tray inserted on top. Turning the joint's free output block to set the shafts at right angles was refused by name with nothing changed, and a 60 degree turn after it was taken and undone. A configuration added and activated in the panel took an auto exploded view that moved ${configured.moved} parts on screen and no placement, saved, reopened in that configuration, and showed no exploded view in Default. The floor mate, driven from the Mate Controller, saved low and 12 mm higher positions, animated back to low, and reopened with both, high applying again. Pack and Go downloaded the rig and its two parts as one zip, which opened again with every part found beside it and took the shaft inserted again; Export URDF downloaded the URDF with ${packAndGo.urdf.entries - 1} meshes as one zip, and Export USD a .usda file.`);
}

// The rig's shafts: the input (component 4) on a hinge at (10, 5, 0) along Z, the output
// (component 9) in a block turned 40 degrees about X.
const BEND = 40;
const TURN = 30;
const INPUT = '4';
const OUTPUT = '9';

async function universalJoint(v) {
  step('load the site again and open the Universal Joint rig with its parts');
  await v.load();
  await v.choose([rig, block, shaft], () => v.click(OPEN));
  let shown = await v.assembly('rig', s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(parts(shown)) === '["block","shaft","block","shaft"]', `the rig's components are ${parts(shown)}`);
  check(shown.mates === 4, `the rig has ${shown.mates} mates, not 4`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every rig part is found: ${JSON.stringify(shown.components)}`);
  const component = (s, at) => s.components.find(c => c.path === at);
  const start = component(shown, OUTPUT);
  const tilt = about([1, 0, 0], BEND * Math.PI / 180);
  check(sameTurn(start.rotation, tilt, 1e-9), `the output shaft starts at ${start.rotation}, not ${BEND} degrees about X`);

  step('choose Universal Joint in the Mate panel');
  await v.command('Mate');
  await v.assembly('mate panel', s => s.mate_panel.open);
  await v.click(await v.control('Mate type'));
  await v.click(await v.control('Mate type: Universal Joint'));
  await v.shot('6-universal-joint-chosen');

  // Each shaft's side at the middle of its axis, seen from the right: from the front the
  // output shaft stands in front of the input.
  step('pick the input shaft, then the output shaft');
  await v.command('Right');
  await v.command('Fit');
  const middle = rotate(tilt, [0, 0, 22]).map((x, i) => x + start.translation[i]);
  await v.pick(await v.screen(10, 5, 22));
  shown = await v.assembly('input pick', s => s.mate_panel.a);
  check(shown.mate_panel.a.component === Number(INPUT) && shown.mate_panel.a.face.startsWith('Boss-Extrude1'), `the first pick is ${JSON.stringify(shown.mate_panel.a)}`);
  await v.pick(await v.screen(...middle));
  shown = await v.assembly('output pick', s => s.mate_panel.b);
  check(shown.mate_panel.b.component === Number(OUTPUT) && shown.mate_panel.b.face.startsWith('Boss-Extrude1'), `the second pick is ${JSON.stringify(shown.mate_panel.b)}`);
  await v.shot('7-shafts-picked');
  await v.key('Enter', 'Enter', 13, 0, '\r');
  shown = await v.assembly('joined', s => s.mates === 5 || s.mate_panel.error);
  check(shown.mates === 5, `the Universal Joint was refused: ${shown.mate_panel.error}`);
  await v.key('Escape', 'Escape', 27);
  await v.assembly('mate panel closed', s => !s.mate_panel.open);

  // The later feature on top: the input turned through its placement field, under mates.
  step(`turn the input shaft ${TURN} degrees through its placement field`);
  await v.pick(await v.screen(10, 5, 22));
  await v.assembly('input selected', s => s.selected === INPUT);
  await v.click(await v.control('Assembly options'));
  await v.click(await v.control('Placement Rotate Z'));
  await v.key('a', 'KeyA', 65, 2, 'a');
  for (const ch of String(TURN)) {
    await v.key(ch, `Digit${ch}`, ch.charCodeAt(0), 0, ch);
  }
  await v.key('Enter', 'Enter', 13, 0, '\r');
  const turn = TURN * Math.PI / 180;
  const input = about([0, 0, 1], turn);
  shown = await v.assembly('driven', s => sameTurn(component(s, INPUT).rotation, input, 1e-7));
  const bend = BEND * Math.PI / 180;
  const follows = Math.atan2(Math.sin(turn), Math.cos(turn) * Math.cos(bend));
  const expected = multiply(about(rotate(tilt, [0, 0, 1]), follows), start.rotation);
  check(sameTurn(component(shown, OUTPUT).rotation, expected, 1e-7), `the output turned to ${component(shown, OUTPUT).rotation}, not ${expected}`);
  await v.shot('8-driven');
  const driven = shown;

  step('save the joint and open it again with its parts');
  const saved = await v.download('save joint', () => v.key('s', 'KeyS', 83, 2, 's'));
  await v.assembly('joint saved', s => !s.dirty);
  await v.load();
  await v.choose([saved, block, shaft], () => v.click(OPEN));
  shown = await v.assembly('joint reopened', s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked'));
  check(shown.mates === 5, `the reopened rig has ${shown.mates} mates, not 5`);
  check(near(placements(shown), placements(driven)), `the reopened placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(placements(driven))}`);
  await v.stopped('joint at the end');
  await v.shot('9-joint-reopened');
  return { driven, reopened: shown, follows: follows * 180 / Math.PI, saved: path.basename(saved) };
}

// A parent assembly placing the assembly `saved`, opened from one upload that lists that
// child first, with `files`, the part files of the components the child places, named in
// `placed`. Neither assembly opens until the visitor chooses the parent.
async function chosenAssembly(v, saved, files, placed) {
  const named = file => path.basename(file);
  // The parent downloads under the name the child did, so each gets a name of its own.
  const child = path.join(downloads, 'child.dwasm');
  fs.copyFileSync(saved, child);
  step('load the site again and start a parent assembly that places a saved assembly');
  await v.load();
  await v.click(NEW_ASSEMBLY);
  await v.assembly('parent new', s => s.open && s.active && s.components.length === 0);
  await v.choose(child, () => v.command('Insert component'));
  await v.assembly('parent', s => loaded(s) && s.components.length === 1 && s.components.every(c => c.freshness !== 'unchecked'));
  const made = await v.download('save parent', () => v.key('s', 'KeyS', 83, 2, 's'));
  await v.assembly('parent saved', s => !s.dirty);
  const parent = path.join(downloads, 'parent.dwasm');
  fs.renameSync(made, parent);

  step('load the site again and open the child, the parent and the parts, the child first');
  await v.load();
  await v.choose([child, parent, ...files], () => v.click(OPEN));
  let shown = await v.assembly('choosing', s => !s.open && !s.opening && s.choosing?.length > 0);
  check(JSON.stringify(shown.choosing) === '["child.dwasm","parent.dwasm"]', `the page offers ${shown.choosing}`);
  check(shown.held.length === 0, `the page holds ${shown.held} before a choice`);
  await v.settle(1000);
  const choosing = JSON.parse(await v.eval(`window.wasmBindings.shown_assembly()`));
  check(!choosing.open && !choosing.opening, `an assembly opened before the choice: ${JSON.stringify(choosing)}`);
  await v.shot('10-choose');

  step('choose the parent');
  await v.click(await v.control('Open assembly: parent.dwasm'));
  const checked = c => c.freshness !== 'unchecked' && c.components.every(checked);
  shown = await v.assembly('chosen', s => loaded(s) && s.components.length === 1 && s.components.every(checked));
  check(shown.file === 'parent.dwasm', `${shown.file} opened, not parent.dwasm`);
  check(shown.choosing.length === 0, `the page still offers ${shown.choosing}`);
  const held = JSON.stringify(['child.dwasm', ...files.map(named)]);
  check(JSON.stringify(shown.held) === held, `the page holds ${shown.held}, not ${held}`);
  // A subassembly has no mesh of its own: its structure is read, and its parts are drawn.
  const [sub] = shown.components;
  check(sub.state === 'MetadataOnly' && sub.freshness === 'current' && !sub.warning, `the child is ${JSON.stringify(sub)}`);
  const inside = JSON.stringify(parts(sub));
  check(inside === JSON.stringify(placed), `the child places ${inside}, not ${JSON.stringify(placed)}`);
  check(sub.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every part in the child is found: ${JSON.stringify(sub.components)}`);
  await v.shot('11-chosen');
  const chosen = shown;

  // One later feature on top: a part inserted in the parent the choice opened.
  const part = files[0];
  step(`insert ${named(part)} in the parent`);
  await v.choose(part, () => v.command('Insert component'));
  shown = await v.assembly('inserted', s => loaded(s) && s.components.length === 2 && s.components.every(c => c.freshness !== 'unchecked'));
  check(shown.file === 'parent.dwasm' && shown.dirty, `the part went to ${shown.file}, dirty ${shown.dirty}`);
  const stem = named(part).replace(/\.dwprt$/, '');
  check(parts(shown)[1] === stem && shown.components[1].freshness === 'current', `the inserted component is ${JSON.stringify(shown.components[1])}`);
  await v.stopped('chosen at the end');
  await v.shot('12-inserted');
  const inserted = shown;

  step('open the child and the parent again, leave the choice, and open the child alone');
  await v.load();
  await v.choose([child, parent, ...files], () => v.click(OPEN));
  await v.assembly('choosing again', s => !s.open && !s.opening && s.choosing?.length === 2);
  await v.choose([child, ...files], () => v.click(OPEN));
  await v.assembly('child alone', s => loaded(s) && s.file === 'child.dwasm' && s.components.every(c => c.freshness !== 'unchecked'));
  await v.settle(1000);
  const replaced = JSON.parse(await v.eval(`window.wasmBindings.shown_assembly()`));
  const offered = Object.keys(JSON.parse(await v.eval(`window.wasmBindings.shown_controls()`))).filter(n => n.startsWith('Open assembly'));
  check(replaced.file === 'child.dwasm', `${replaced.file} is open, not child.dwasm`);
  check(replaced.choosing.length === 0 && offered.length === 0, `the old choice is still offered: ${JSON.stringify(replaced.choosing)}, buttons ${JSON.stringify(offered)}`);
  await v.stopped('choice replaced');
  await v.shot('13-choice-replaced');
  return { choosing, chosen, inserted, replaced };
}

// The bearing fixture: the rig with its Universal Joint U made and the lock between the
// blocks deleted, so the output block (component 7) turns freely and carries the output
// shaft on its hinge.
const BEARING = '7';
// The note shows a refusal as its code, then its message.
const REFUSAL = 'invalid: a universal joint locks with its shafts at right angles; set the shafts at another angle';

// Types `degrees` into the placement field `name` of the selected component, and Enter.
async function placementField(v, name, degrees) {
  await v.click(await v.control(name));
  await v.key('a', 'KeyA', 65, 2, 'a');
  for (const ch of String(degrees)) {
    await v.key(ch, `Digit${ch}`, ch.charCodeAt(0), 0, ch);
  }
  await v.key('Enter', 'Enter', 13, 0, '\r');
}

async function refusedDrag(v) {
  step('load the site again and open the joint whose output block turns freely');
  await v.load();
  await v.choose([bearing, block, shaft], () => v.click(OPEN));
  let shown = await v.assembly('bearing', s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(shown.mate_names) === '["O","H","H2","U"]', `the joint's mates are ${shown.mate_names}`);
  const component = (s, at) => s.components.find(c => c.path === at);
  const tilt = about([1, 0, 0], BEND * Math.PI / 180);
  check(sameTurn(component(shown, BEARING).rotation, tilt, 1e-9), `the bearing block starts at ${component(shown, BEARING).rotation}`);

  // A corner of the block's top, clear of its shaft, seen from above.
  step('select the bearing block');
  await v.command('Top');
  await v.command('Fit');
  const corner = rotate(tilt, [-12, -8, 10]).map((x, i) => x + component(shown, BEARING).translation[i]);
  await v.pick(await v.screen(...corner));
  const before = await v.assembly('bearing selected', s => s.selected === BEARING);
  check(before.note?.kind !== 'Error', `an error shows before the drag: ${before.note?.text}`);
  await v.click(await v.control('Assembly options'));

  step('turn the bearing block to 90 degrees about X, which would set the shafts at right angles');
  await placementField(v, 'Placement Rotate X', 90);
  shown = await v.assembly('refused', s => s.note?.kind === 'Error');
  check(shown.note.text === REFUSAL, `the refusal says "${shown.note.text}", not "${REFUSAL}"`);
  check(JSON.stringify(placements(shown)) === JSON.stringify(placements(before)), `the refused drag moved ${JSON.stringify(placements(shown))} from ${JSON.stringify(placements(before))}`);
  check(JSON.stringify(shown.mate_names) === JSON.stringify(before.mate_names), `the refused drag changed the mates to ${shown.mate_names}`);
  check(JSON.stringify(shown.undo) === JSON.stringify(before.undo), `the refused drag changed undo to ${JSON.stringify(shown.undo)} from ${JSON.stringify(before.undo)}`);
  check(shown.dirty === before.dirty, `the refused drag left the joint dirty: ${shown.dirty}`);
  await v.shot('14-refused');
  const refused = shown;

  // One later feature on top: the same field takes 60 degrees, the output shaft's hinge
  // carries it along, and Undo puts the block back.
  step('turn the bearing block to 60 degrees about X instead');
  await placementField(v, 'Placement Rotate X', 60);
  shown = await v.assembly('turned', s => s.undo.depth === before.undo.depth + 1 || (s.note?.kind === 'Error' && s.note.text !== REFUSAL));
  check(shown.undo.depth === before.undo.depth + 1, `the 60 degree turn was not taken: ${JSON.stringify(shown.note)}`);
  const sixty = about([1, 0, 0], 60 * Math.PI / 180);
  check(sameTurn(component(shown, BEARING).rotation, sixty, 1e-9), `the bearing block turned to ${component(shown, BEARING).rotation}`);
  const axis = rotate(component(shown, OUTPUT).rotation, [0, 0, 1]);
  const bore = rotate(sixty, [0, 0, 1]);
  check(axis.every((x, i) => Math.abs(x - bore[i]) < 1e-9), `the output shaft points along ${axis}, not its bore ${bore}`);
  await v.shot('15-turned');
  const turned = shown;

  step('undo the turn');
  await v.command('Undo');
  shown = await v.assembly('undone', s => s.undo.depth === before.undo.depth);
  check(near(placements(shown), placements(before)), `after Undo the placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(placements(before))}`);
  check(JSON.stringify(shown.mate_names) === JSON.stringify(before.mate_names), `after Undo the mates are ${shown.mate_names}`);
  await v.stopped('bearing at the end');
  return { refused, turned, undone: shown };
}

// Types `text` into the focused field, a key at a time.
// Reads a zip whose entries are stored as they are, as Pack and Go and the exports write
// them, into `dir`; answers the entries' names in file order.
function unzip(file, dir) {
  const zip = fs.readFileSync(file);
  let end = zip.length - 22;
  while (end >= 0 && zip.readUInt32LE(end) !== 0x06054b50) end--;
  check(end >= 0, `${file} is not a zip`);
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  const names = [];
  for (let i = 0; i < count; i++) {
    check(zip.readUInt32LE(at) === 0x02014b50, `${file}: entry ${i} has no directory record`);
    check(zip.readUInt16LE(at + 10) === 0, `${file}: entry ${i} is compressed`);
    const size = zip.readUInt32LE(at + 20);
    const nameLength = zip.readUInt16LE(at + 28);
    const skip = zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.toString('utf8', at + 46, at + 46 + nameLength);
    const data = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const to = path.join(dir, name);
    fs.mkdirSync(path.dirname(to), { recursive: true });
    fs.writeFileSync(to, zip.subarray(data, data + size));
    names.push(name);
    at += 46 + nameLength + skip;
  }
  return names;
}

// Pack and Go and the robot exports, where the page has no folders: each arrives as a
// download. The Universal Joint rig, opened with its two parts, packs into one zip of the
// rig and both parts. Loaded afresh, the three unpacked files open as the rig did, every
// part found beside it and current, and take a later feature on top: the shaft inserted
// again. Export URDF downloads one zip of the URDF and its meshes folder, every mesh the
// URDF names inside it; Export USD downloads one .usda file with its joints.
async function packAndExport(v) {
  step('open the Universal Joint rig with its parts and an unrelated upload named block.dwprt');
  const duplicate = path.join(downloads, 'unrelated', 'block.dwprt');
  fs.mkdirSync(path.dirname(duplicate), { recursive: true });
  fs.copyFileSync(collar, duplicate);
  await v.load();
  await v.choose([rig, duplicate, block, shaft], () => v.click(OPEN));
  const ready = s => loaded(s) && s.components.length === 4 && s.components.every(c => c.freshness !== 'unchecked');
  const opened = await v.assembly('rig to pack', ready);
  check(opened.held.filter(name => name === 'block.dwprt').length === 2, `the page holds ${opened.held}`);

  step('Pack and Go downloads one zip');
  const zip = await v.download('pack', () => v.command('Pack and Go'));
  check(path.basename(zip) === 'rig.zip', `Pack and Go downloads rig.zip, not ${path.basename(zip)}`);
  const unpacked = path.join(downloads, 'unpacked');
  const packed = unzip(zip, unpacked).sort();
  check(JSON.stringify(packed) === '["block.dwprt","rig.dwasm","shaft.dwprt"]', `the zip holds ${packed}`);
  check(fs.readFileSync(path.join(unpacked, 'block.dwprt')).equals(fs.readFileSync(block)), 'the zip holds the wrong block.dwprt upload');

  step('load the site again and open the unpacked rig with its parts');
  await v.load();
  await v.choose(packed.map(n => path.join(unpacked, n)), () => v.click(OPEN));
  let shown = await v.assembly('unpacked', ready);
  check(JSON.stringify(parts(shown)) === JSON.stringify(parts(opened)), `the unpacked rig's components are ${parts(shown)}`);
  check(near(placements(shown), placements(opened)), `the unpacked placements are ${JSON.stringify(placements(shown))}`);
  check(shown.mates === opened.mates, `the unpacked rig has ${shown.mates} mates, not ${opened.mates}`);
  check(shown.components.every(c => c.state === 'DisplayReady' && c.freshness === 'current' && !c.warning), `not every unpacked part is found: ${JSON.stringify(shown.components)}`);
  await v.shot('16-unpacked');

  step('insert the unpacked shaft again');
  await v.choose(path.join(unpacked, 'shaft.dwprt'), () => v.command('Insert component'));
  shown = await v.assembly('inserted in the unpacked rig', s => loaded(s) && s.components.length === 5);
  check(parts(shown)[4] === 'shaft', `the fifth component is ${parts(shown)[4]}`);

  const assembled = placements(shown);
  const assembledZip = await v.download('urdf-assembled', () => v.command('Export URDF'));
  const assembledRobot = path.join(downloads, 'robot-assembled');
  const assembledName = unzip(assembledZip, assembledRobot).find(n => n.endsWith('.urdf'));
  check(assembledName, 'the assembled export contains a URDF');
  const assembledUrdf = fs.readFileSync(path.join(assembledRobot, assembledName), 'utf8');
  const assembledUsd = fs.readFileSync(await v.download('usd-assembled', () => v.command('Export USD')), 'utf8');
  await v.command('Configurations');
  await v.assembly('configurations panel', s => s.configure_open);
  await v.click(await v.control('Auto explode'));
  shown = await v.assembly('rig exploded', s => s.exploded || s.note?.kind === 'Error');
  check(shown.exploded && !near(placements(shown), assembled), 'Auto explode leaves at least one part visibly displaced');

  step('Export URDF downloads the URDF and its meshes as one zip');
  const urdfZip = await v.download('urdf', () => v.command('Export URDF'));
  check(urdfZip.endsWith('.zip'), `Export URDF downloads a zip, not ${path.basename(urdfZip)}`);
  const robot = path.join(downloads, 'robot');
  const entries = unzip(urdfZip, robot);
  const urdfName = entries.find(n => n.endsWith('.urdf'));
  check(urdfName && !urdfName.includes('/'), `the zip has no URDF at its top: ${entries}`);
  const urdf = fs.readFileSync(path.join(robot, urdfName), 'utf8');
  check(urdf === assembledUrdf, 'the URDF changes when the assembly is exploded');
  const meshes = [...urdf.matchAll(/filename="([^"]+)"/g)].map(m => m[1]);
  check(meshes.length >= 5, `the URDF names ${meshes.length} meshes`);
  check(meshes.every(m => entries.includes(m)), `the zip lacks meshes the URDF names: ${meshes.filter(m => !entries.includes(m))}`);
  const moving = [...urdf.matchAll(/<joint [^>]*type="(revolute|continuous)"/g)].length;
  check(moving === 2, `the URDF has ${moving} moving joints for the two hinges`);

  step('Export USD downloads one .usda file');
  const usd = await v.download('usd', () => v.command('Export USD'));
  check(usd.endsWith('.usda'), `Export USD downloads a .usda file, not ${path.basename(usd)}`);
  const usda = fs.readFileSync(usd, 'utf8');
  check(usda.startsWith('#usda') && /Physics(Revolute|Fixed)Joint/.test(usda), 'the .usda file has no joints');
  check(usda === assembledUsd, 'the USD changes when the assembly is exploded');
  await v.stopped('after the exports');
  return { packed, unpacked: placements(shown), urdf: { file: urdfName, entries: entries.length, moving }, usd: path.basename(usd) };
}

async function type(v, text) {
  for (const ch of text) {
    const code = /[0-9]/.test(ch) ? `Digit${ch}` : `Key${ch.toUpperCase()}`;
    await v.key(ch, code, ch.toUpperCase().charCodeAt(0), 0, ch);
  }
}

async function configuredExplode(v, saved, files, mated) {
  step('load the site again and open the saved assembly with its parts');
  await v.load();
  await v.choose([saved, ...files], () => v.click(OPEN));
  let shown = await v.assembly('configurations', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(shown.configurations.length === 0 && shown.explode_steps === 0, `the tray assembly starts with ${JSON.stringify(shown.configurations)} and ${shown.explode_steps} steps`);
  const assembled = placements(shown);

  step('add a configuration called Lifted in the Configurations panel');
  await v.command('Configurations');
  await v.assembly('configurations panel', s => s.configure_open);
  await v.click(await v.control('Configuration name'));
  await type(v, 'Lifted');
  await v.click(await v.control('Add configuration'));
  shown = await v.assembly('added', s => s.configurations.length === 2 || s.note?.kind === 'Error');
  check(JSON.stringify(shown.configurations) === '["Default","Lifted"]', `the configurations are ${JSON.stringify(shown.configurations)}: ${JSON.stringify(shown.note)}`);
  check(shown.active_configuration === 'Default', `${shown.active_configuration} is active after Add`);

  step('activate Lifted and explode it');
  await v.click(await v.control('Activate Lifted'));
  shown = await v.assembly('lifted', s => s.active_configuration === 'Lifted');
  await v.click(await v.control('Auto explode'));
  shown = await v.assembly('exploded', s => s.exploded || s.note?.kind === 'Error');
  check(shown.exploded && shown.explode_steps >= 1, `Auto explode did not explode: ${JSON.stringify(shown.note)}`);
  const moved = shown.components.filter((c, i) => !near([c.translation], [assembled[i][0]])).length;
  check(moved >= 1, `no part moved on screen: ${JSON.stringify(placements(shown))}`);
  await v.command('Isometric');
  await v.command('Fit');
  await v.shot('16-exploded');

  step('collapse the exploded view');
  await v.click(await v.control('Explode or collapse'));
  shown = await v.assembly('collapsed', s => !s.exploded);
  check(near(placements(shown), assembled), `collapsed, the placements are ${JSON.stringify(placements(shown))}, not ${JSON.stringify(assembled)}`);
  check(near(placements(shown), placements(mated)), 'the exploded view moved a placement');

  step('save, open again and switch back to Default');
  const again = await v.download('save configured', () => v.key('s', 'KeyS', 83, 2, 's'));
  await v.load();
  await v.choose([again, ...files], () => v.click(OPEN));
  shown = await v.assembly('configured reopened', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(shown.active_configuration === 'Lifted' && shown.explode_steps >= 1, `reopened in ${shown.active_configuration} with ${shown.explode_steps} steps`);
  await v.command('Configurations');
  await v.assembly('configurations panel again', s => s.configure_open);
  await v.click(await v.control('Activate Default'));
  shown = await v.assembly('default', s => s.active_configuration === 'Default');
  check(shown.explode_steps === 0, `Default has ${shown.explode_steps} explode steps of Lifted's`);
  check(near(placements(shown), assembled), `Default's placements are ${JSON.stringify(placements(shown))}`);
  await v.stopped('configured at the end');
  return { moved, reopened: shown.configurations };
}


// The Mate Controller on the saved tray assembly: the floor mate driven between two named
// positions, an animation back to the first, saved and opened again with both positions.
async function mateController(v, saved, files, mated) {
  step('open the saved tray assembly and drive its floor mate from the Mate Controller');
  await v.load();
  await v.choose([saved, ...files], () => v.click(OPEN));
  let shown = await v.assembly('controller opened', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  const floor = mated.mate_names[0];
  const height = s => { const [a, , b] = s.components.map(c => c.translation); return b[2] - a[2]; };
  const start = height(shown);
  await v.command('Mate');
  await v.assembly('mate panel', s => s.mate_panel.open);
  await v.click(await v.control('Mate more'));
  await v.click(await v.control('Controller mates'));
  await type(v, floor);
  await v.click(await v.control('Set driven mates'));
  shown = await v.assembly('driven', s => s.controller.mates.length === 1 || s.mate_panel.error);
  check(JSON.stringify(shown.controller.mates) === JSON.stringify([floor]), `the controller drives ${JSON.stringify(shown.controller.mates)}: ${shown.mate_panel.error}`);
  await v.click(await v.control('Position name'));
  await type(v, 'low');
  await v.click(await v.control('Save position'));
  shown = await v.assembly('low saved', s => s.controller.positions.length === 1);

  step('raise the second tray 12 mm through the controller and save that too');
  await v.click(await v.control('Controller value 1'));
  await v.key('a', 'KeyA', 65, 2, 'a');
  await type(v, '12');
  await v.click(await v.control('Apply values'));
  shown = await v.assembly('raised', s => Math.abs(height(s) - start - 12) < 1e-9 || s.mate_panel.error);
  check(Math.abs(height(shown) - start - 12) < 1e-9, `the tray stands ${height(shown)} mm up, not ${start + 12}: ${shown.mate_panel.error}`);
  await v.click(await v.control('Position name'));
  await v.key('a', 'KeyA', 65, 2, 'a');
  await type(v, 'high');
  await v.click(await v.control('Save position'));
  shown = await v.assembly('high saved', s => s.controller.positions.length === 2);
  check(JSON.stringify(shown.controller.positions) === '["low","high"]', `the positions are ${JSON.stringify(shown.controller.positions)}`);
  await v.shot('17-controller-high');

  step('animate back to low');
  await v.click(await v.control('Animate to low'));
  shown = await v.assembly('animated', s => !s.controller.animating && Math.abs(height(s) - start) < 1e-9);

  step('save, open again and apply high');
  const again = await v.download('save controller', () => v.key('s', 'KeyS', 83, 2, 's'));
  await v.load();
  await v.choose([again, ...files], () => v.click(OPEN));
  shown = await v.assembly('controller reopened', s => loaded(s) && s.components.length === 3 && s.components.every(c => c.freshness !== 'unchecked'));
  check(JSON.stringify(shown.controller.positions) === '["low","high"]', `reopened with ${JSON.stringify(shown.controller.positions)}`);
  check(Math.abs(height(shown) - start) < 1e-9, `reopened ${height(shown)} mm up`);
  await v.command('Mate');
  await v.assembly('mate panel again', s => s.mate_panel.open);
  await v.click(await v.control('Mate more'));
  await v.click(await v.control('Apply high'));
  shown = await v.assembly('high applied', s => Math.abs(height(s) - start - 12) < 1e-9 || s.mate_panel.error);
  check(Math.abs(height(shown) - start - 12) < 1e-9, `Apply high left the tray ${height(shown)} mm up: ${shown.mate_panel.error}`);
  await v.stopped('controller at the end');
  return { floor, positions: shown.controller.positions };
}

try {
  await main();
} catch (error) {
  console.error(error.stack || String(error));
  if (browser?.stderr) console.error(browser.stderr.split('\n').slice(-20).join('\n'));
  process.exitCode = 1;
} finally {
  await browser?.close();
  fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
