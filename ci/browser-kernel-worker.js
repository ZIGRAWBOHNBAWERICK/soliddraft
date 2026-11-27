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
// The clicks go to fixed points of a 1400 x 813 page: the tool strip's Solid and Inspect
// tabs, the feature tree's rows, Mass props, Details, Close options, File > New part
// and Extrude's Rectangle button. Another width moves the strip's tools, so the check
// refuses any other size. New part is clicked, not typed: Chrome
// on Linux keeps Ctrl+N for a new window. Each step waits for the open part's feature
// count, which the page reports in its own feedback events, or for what the last frame
// drew, rather than a fixed time. The options panel covers the feature tree's rows, so
// a row is clicked only once a frame has drawn the panel closed, and its selection is
// confirmed by the title Details draws for it. With software rendering a frame can take
// longer than any fixed pause.
function check(value, message) { if (!value) throw new Error(message); }
const PART = [36, 52], INSPECT = [164, 52], MASS = [114, 97], CLOSE = [109, 184];
const RECTANGLE = [99, 355], FILE = [158, 18], NEW_PART = [170, 49];
// The feature tree's rows, in the side column.
const TREE = 134, ROW = { first: 297, second: 323, third: 349 };

check(await page.eval(() => location.search === '?demo'), 'Open the application at /?demo first');
const size = await page.eval(() => [innerWidth, innerHeight]);
check(size[0] === 1400 && size[1] === 813, `The page is ${size.join(' x ')}; set a 1400 x 813 viewport first`);
await page.wait('#status.hidden', 60000);
check(await page.eval(() =>
  !performance.getEntriesByType('resource').some(entry => entry.name.includes('/soliddraft-kernel')),
), 'The page requested the worker module before a kernel job');
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
  window.holdKernelJob = false;
  window.releaseKernelJob = null;
  const Original = window.Worker;
  window.Worker = class extends Original {
    constructor(...args) {
      super(...args);
      window.kernelWorkers.push(this);
      // An answer's handed-over builds are kept as their size: copied as JSON, a
      // megabyte of bytes becomes an object of a million keys.
      this.addEventListener('message', event => {
        const { hand_over, ...message } = event.data;
        const size = hand_over ? { hand_over: hand_over.byteLength } : {};
        window.kernelAnswers.push({ ...JSON.parse(JSON.stringify(message)), ...size, at: performance.now() });
      });
    }

    postMessage(message, ...options) {
      if (window.holdKernelJob && message.id !== undefined) {
        window.holdKernelJob = false;
        window.releaseKernelJob = () => super.postMessage(message, ...options);
        return;
      }
      super.postMessage(message, ...options);
    }
  };
  // Each kernel job's steps on the page; the worker's arrive among kernelAnswers.
  window.sdKernelTrace = [];
  // The open document's feature count, from the page's own feedback events.
  window.featureCount = null;
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') window.featureCount = detail.count;
  });
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
  const trace = await page.eval(() => [
    ...window.sdKernelTrace,
    ...window.kernelAnswers.map(message => message.trace || { ...message, at: Math.round(performance.timeOrigin + message.at) }),
  ]);
  throw new Error(`No kernel worker answer for ${what}. Trace: ${JSON.stringify(trace)}`);
}
async function features(count, what) {
  let now = null;
  for (let i = 0; i < 300; i++) {
    now = await page.eval(() => window.featureCount);
    if (now === count) return;
    await page.wait(100);
  }
  throw new Error(`${what}: the open part has ${now} features, not ${count}`);
}
// The Details panel the last frame drew, or null when it drew none.
async function shown() {
  const drawn = await page.eval(() => window.wasmBindings.shown_details());
  return drawn ? JSON.parse(drawn) : null;
}
async function details() {
  const drawn = await shown();
  return drawn ? Object.fromEntries(drawn.rows) : {};
}
// Closes the options panel and waits for a frame drawn without it.
async function close(what) {
  await click(CLOSE);
  for (let i = 0; i < 100; i++) {
    if (!(await shown())) return;
    await page.wait(100);
  }
  throw new Error(`${what}: the options panel did not close`);
}
// A frame without a click: the pointer moves over the viewport. The page draws a frame
// for each input, and a changed selection shows Details only in a later frame.
const nudge = () => page.eval(() => {
  document.getElementById('soliddraft').dispatchEvent(new PointerEvent('pointermove',
    { clientX: 900, clientY: 450, bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true }));
  return true;
});
// Waits for a frame whose options bar shows `title` selected and draws Details, logs
// both, and clicks Details once where that frame drew it. A click sent before then lands
// where Details is not drawn yet.
async function openDetails(title, what) {
  let drawn = null;
  for (let i = 0; i < 100; i++) {
    await nudge();
    const options = await page.eval(() => window.wasmBindings.shown_options());
    drawn = options ? JSON.parse(options) : null;
    if (drawn && drawn.selected === title && drawn.details) break;
    await page.wait(100);
  }
  console.log(`${what}: options bar shows ${JSON.stringify(drawn)}`);
  check(drawn && drawn.selected === title && drawn.details, `${what}: ${title} is not selected with Details drawn`);
  await click([Math.round(drawn.details.x), Math.round(drawn.details.y)]);
}
// Selects a feature tree row while the options panel is closed, and waits until Details
// draws the feature `title` for it.
async function select(row, title, what) {
  await click([TREE, row]);
  await openDetails(title, what);
  let drawn = null;
  for (let i = 0; i < 100; i++) {
    drawn = await shown();
    if (drawn && drawn.title === title) return close(what);
    await page.wait(100);
  }
  throw new Error(`${what}: Details shows ${JSON.stringify(drawn && drawn.title)}, not ${title}`);
}
// Waits until the page has sent its `count`th kernel job.
async function sent(count, what) {
  for (let i = 0; i < 100; i++) {
    if (await page.eval(() => window.sdKernelTrace.filter(step => step.stage === 'enqueued').length) >= count) return;
    await page.wait(50);
  }
  const trace = await page.eval(() => window.sdKernelTrace);
  throw new Error(`${what}: Mass props sent no kernel job. Trace: ${JSON.stringify(trace)}`);
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

// One timing window: ask for the selected row's mass properties and wait until Details
// draws the answer. Nothing on the page's thread may run 250 ms or more meanwhile.
async function measure(row, title, count, what) {
  await select(row, title, what);
  await page.eval(() => { window.longTasks = []; return true; });
  await click(MASS);
  const answer = await answered(count, what);
  check('answer' in answer, `${what} failed in the worker: ${JSON.stringify(answer)}`);
  const result = JSON.parse(answer.answer);
  check(result.ok && result.ok.volume > 0, `${what} answered no properties: ${answer.answer}`);
  await openDetails(title, what);
  await volumeShown(result.ok.volume, what);
  const longest = await page.eval(() => Math.max(0, ...window.longTasks));
  console.log(`${what}: volume ${result.ok.volume} mm³ drawn; longest page task ${longest} ms`);
  check(longest < 250, `${what} blocked the page for ${longest} ms`);
  await close(what);
  return result.ok.volume;
}

await click(INSPECT);
await page.wait(500);
const plate = await measure(ROW.first, 'Boss-Extrude1', 1, 'The sample part\'s plate');
check(plate === 108000, `The 120 x 90 x 10 mm plate measured ${plate} mm³`);
check(await page.eval(() => window.kernelWorkers.length) === 1, 'Mass properties did not start one kernel worker');

// A worker from a different application revision reports the mismatch and closes.
// The page keeps running and can start another worker.
await page.eval(() => {
  window.kernelWorkers[0].postMessage({ init: { base: document.baseURI, commit: 'different-revision' } });
  return true;
});
const fatal = await answered(2, 'the failed worker');
check(fatal.fatal?.includes('Reload the page'), `The worker did not report the revision mismatch: ${JSON.stringify(fatal)}`);
check(await page.eval(() => document.getElementById('status').classList.contains('hidden')), 'The page stopped with its worker');
const flanged = await measure(ROW.second, 'Boss-Extrude2', 3, 'The plate and flange, after the worker stopped');
check(await page.eval(() => window.kernelWorkers.length) === 2, 'The next request did not start a new kernel worker');
check(flanged > plate, `The plate and flange (${flanged}) are not larger than the plate (${plate})`);

// File > New part, waiting for the blank replacement to open. On a slow frame the
// menu can still be undrawn when New part is clicked, so close the menu and try again.
async function newPart(what) {
  for (let attempt = 0; attempt < 3; attempt++) {
    await click(FILE);
    await page.wait(300 + 700 * attempt);
    await click(NEW_PART);
    for (let i = 0; i < 100; i++) {
      if (await page.eval(() => window.featureCount) === 0) return;
      await page.wait(100);
    }
    await page.press('Escape');
  }
  throw new Error(`${what}: File > New part opened no new part in three tries`);
}

// Hold the old part's job at the worker boundary until the new part is open.
// The measured part then gets a real worker answer regardless of how fast it builds.
await select(ROW.third, 'Boss-Extrude3', 'The whole part');
await page.eval(() => { window.holdKernelJob = true; return true; });
await click(MASS);
await sent(3, 'The whole part');
check(await page.eval(() => window.releaseKernelJob !== null), 'Expected a held job for the whole part');
await newPart('New part');
const replaced = await page.eval(() => {
  const at = performance.now();
  window.releaseKernelJob();
  window.releaseKernelJob = null;
  return at;
});
const late = await answered(4, 'the replaced part');
check('answer' in late && JSON.parse(late.answer).ok?.volume > 0 && late.at > replaced,
  'The whole part did not receive a successful worker answer after replacement');

// A new part with one 60 x 40 x 20 mm block, selected, with Details open. A new part's
// kernel numbers its bodies from the start, so two such blocks have the same body id.
async function block(what) {
  await click(PART);
  await page.wait(500);
  // E opens Extrude while the pointer is over the viewport, as browser-documents.mjs does.
  // Synthetic clicks move no keyboard focus, so the canvas takes it before the keys.
  await page.eval(() => {
    const canvas = document.getElementById('soliddraft');
    canvas.focus();
    const at = { clientX: 900, clientY: 450, bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true };
    canvas.dispatchEvent(new PointerEvent('pointermove', at));
    return document.activeElement === canvas;
  });
  await page.press('e');
  await page.wait(1500);
  await click(RECTANGLE);
  await page.press('Enter');
  await features(1, `${what}: Extrude`);
  await page.wait(1000);
  await click(INSPECT);
  await page.wait(500);
  await click([TREE, ROW.first]);
  await openDetails('Boss-Extrude1', what);
  for (let i = 0; i < 100 && (await shown())?.title !== 'Boss-Extrude1'; i++) await page.wait(100);
  const rows = await details();
  check(rows['Profile'] === 'Rectangle 60 × 40 on Top plane', `${what} is not the block: ${JSON.stringify(rows)}`);
  check(!('Volume' in rows), `${what} shows a volume it never measured: ${rows.Volume}`);
  await close(what);
}

await block('The part that replaced the sample');
const first = await measure(ROW.first, 'Boss-Extrude1', 5, 'The first new part\'s block');
check(first === 48000, `The 60 x 40 x 20 mm block measured ${first} mm³`);
await newPart('A second new part');
await page.wait(1500);
await block('A second new part with the same body ids');
console.log('Kernel worker check passed');
