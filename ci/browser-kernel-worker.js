// Run through chrome-devtools-axi run after opening the packaged application at /?demo,
// on an origin with no earlier sessions: a second load would leave the first session's
// crash journal window over the Inspect strip.
//
// Mass properties run in the browser's kernel worker, not on the interface thread: the
// page asks, the worker rebuilds the part on its own kernel and answers, and no task on
// the page's thread runs long meanwhile. Each measurement ends when the Details panel
// draws the worker's volume, read from the rows it drew (`shown_details`), since the
// canvas has no text. A worker that fails reports its reason, the page keeps its part,
// and the next request starts a new worker. An answer that arrives after its part was
// replaced is not shown, and a measured block's volume is not shown for an identical
// block in another new part, whose kernel gives it the same body id.
//
// The clicks go to fixed points of a 1400 x 813 page: the strip's Part and Inspect
// buttons, the feature tree's rows, Mass props, Details, Close options and Extrude's
// Rectangle button.
function check(value, message) { if (!value) throw new Error(message); }
const PART = [19, 132], INSPECT = [19, 241], MASS = [220, 112], DETAILS = [317, 52], CLOSE = [347, 92];
const RECTANGLE = [630, 52];
const ROW = { first: 203, second: 229, third: 255 };

check(await page.eval(() => location.search === '?demo'), 'Open the application at /?demo first');
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
  // Every worker the page starts, and what each one answers, with the time it arrived.
  window.kernelWorkers = [];
  window.kernelAnswers = [];
  const Original = window.Worker;
  window.Worker = class extends Original {
    constructor(...args) {
      super(...args);
      window.kernelWorkers.push(this);
      this.addEventListener('message', event => {
        window.kernelAnswers.push({ ...JSON.parse(JSON.stringify(event.data)), at: performance.now() });
      });
    }
  };
  window.longTasks = [];
  new PerformanceObserver(list => {
    for (const entry of list.getEntries()) window.longTasks.push(Math.round(entry.duration));
  }).observe({ type: 'longtask' });
  return true;
});

const click = ([x, y]) => page.eval(`sdClick(${x}, ${y})`);
async function answered(count, what) {
  for (let i = 0; i < 600; i++) {
    const answers = await page.eval(() => window.kernelAnswers.filter(answer => 'answer' in answer || 'fatal' in answer));
    if (answers.length >= count) return answers[count - 1];
    await page.wait(100);
  }
  throw new Error(`No kernel worker answer for ${what}`);
}
async function details() {
  const shown = await page.eval(() => window.wasmBindings.shown_details());
  return shown ? Object.fromEntries(JSON.parse(shown).rows) : {};
}
async function volumeShown(volume, what) {
  const wanted = `${Math.round(volume)} mm³`;
  let rows = {};
  for (let i = 0; i < 100; i++) {
    rows = await details();
    if (rows.Volume === wanted) return;
    await page.wait(100);
  }
  throw new Error(`${what}: Details shows ${JSON.stringify(rows.Volume)}, not ${wanted}`);
}

// One timing window: select the row, ask for mass properties and wait until Details
// draws the answer. Nothing on the page's thread may run 250 ms or more meanwhile.
async function measure(row, count, what) {
  await page.eval(() => { window.longTasks = []; return true; });
  await click([380, row]);
  await page.wait(300);
  await click(MASS);
  const answer = await answered(count, what);
  check('answer' in answer, `${what} failed in the worker: ${JSON.stringify(answer)}`);
  const result = JSON.parse(answer.answer);
  check(result.ok && result.ok.volume > 0, `${what} answered no properties: ${answer.answer}`);
  await click(DETAILS);
  await volumeShown(result.ok.volume, what);
  const longest = await page.eval(() => Math.max(0, ...window.longTasks));
  console.log(`${what}: volume ${result.ok.volume} mm³ drawn; longest page task ${longest} ms`);
  check(longest < 250, `${what} blocked the page for ${longest} ms`);
  await click(CLOSE);
  return result.ok.volume;
}

await click(INSPECT);
await page.wait(500);
const plate = await measure(ROW.first, 1, 'The sample part\'s plate');
check(plate === 108000, `The 120 x 90 x 10 mm plate measured ${plate} mm³`);
check(await page.eval(() => window.kernelWorkers.length) === 1, 'Mass properties did not start one kernel worker');

// Starting the worker again on a module that is not the application fails inside the
// worker, which reports it and closes, as a trap does. The page keeps running.
await page.eval(() => {
  window.kernelWorkers[0].postMessage({ init: { glue: 'data:text/javascript,', module: null } });
  return true;
});
const fatal = await answered(2, 'the failed worker');
check('fatal' in fatal, `The worker did not report its failure: ${JSON.stringify(fatal)}`);
check(await page.eval(() => document.getElementById('status').classList.contains('hidden')), 'The page stopped with its worker');
const flanged = await measure(ROW.second, 3, 'The plate and flange, after the worker stopped');
check(await page.eval(() => window.kernelWorkers.length) === 2, 'The next request did not start a new kernel worker');
check(flanged > plate, `The plate and flange (${flanged}) are not larger than the plate (${plate})`);

// Ask for the whole part, then replace it with a new part before the answer comes.
await click([380, ROW.third]);
await page.wait(300);
await click(MASS);
await page.press('Control+n');
const replaced = await page.eval(() => performance.now());
const late = await answered(4, 'the replaced part');
check(late.at > replaced, 'The whole part\'s answer came before the new part replaced it');

// A new part with one 60 x 40 x 20 mm block, selected, with Details open. A new part's
// kernel numbers its bodies from the start, so two such blocks have the same body id.
async function block(what) {
  await click(PART);
  await page.wait(500);
  await page.press('e');
  await page.wait(1500);
  await click(RECTANGLE);
  await page.press('Enter');
  await page.wait(3000);
  await click(INSPECT);
  await page.wait(500);
  await click([380, ROW.first]);
  await page.wait(300);
  await click(DETAILS);
  await page.wait(500);
  const rows = await details();
  check(rows['Profile'] === 'Rectangle 60 × 40 on Top plane', `${what} is not the block: ${JSON.stringify(rows)}`);
  check(!('Volume' in rows), `${what} shows a volume it never measured: ${rows.Volume}`);
  await click(CLOSE);
}

await block('The part that replaced the sample');
const first = await measure(ROW.first, 5, 'The first new part\'s block');
check(first === 48000, `The 60 x 40 x 20 mm block measured ${first} mm³`);
await page.press('Control+n');
await page.wait(1500);
await block('A second new part with the same body ids');
console.log('Kernel worker check passed');
