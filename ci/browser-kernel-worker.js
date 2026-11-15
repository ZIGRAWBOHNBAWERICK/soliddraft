// Run through chrome-devtools-axi run after opening the packaged application at /?demo,
// on an origin with no earlier sessions: a second load would leave the first session's
// crash journal window over the Inspect strip.
// Mass properties run in the browser's kernel worker, not on the interface thread:
// the page asks, the worker rebuilds the part on its own kernel and answers, and no
// task on the page's thread runs long meanwhile. A worker that fails reports its reason,
// the page keeps its part, and the next request starts a new worker.
//
// The clicks go to fixed points of a 1400 x 813 page: the Inspect strip button, the
// feature tree's rows and the Mass props button.
function check(value, message) { if (!value) throw new Error(message); }

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
  // Every worker the page starts, and what each one answers.
  window.kernelWorkers = [];
  window.kernelAnswers = [];
  const Original = window.Worker;
  window.Worker = class extends Original {
    constructor(...args) {
      super(...args);
      window.kernelWorkers.push(this);
      this.addEventListener('message', event => window.kernelAnswers.push(JSON.parse(JSON.stringify(event.data))));
    }
  };
  window.longTasks = [];
  new PerformanceObserver(list => {
    for (const entry of list.getEntries()) window.longTasks.push(Math.round(entry.duration));
  }).observe({ type: 'longtask' });
  return true;
});

async function answered(count, what) {
  for (let i = 0; i < 600; i++) {
    const answers = await page.eval(() => window.kernelAnswers.filter(answer => 'answer' in answer || 'fatal' in answer));
    if (answers.length >= count) return answers[count - 1];
    await page.wait(100);
  }
  throw new Error(`No kernel worker answer for ${what}`);
}

async function measure(row, count, what) {
  await page.eval(() => { window.longTasks = []; return true; });
  await page.eval(`sdClick(380, ${row})`);
  await page.wait(300);
  await page.eval(() => sdClick(220, 112));
  const answer = await answered(count, what);
  const longest = await page.eval(() => Math.max(0, ...window.longTasks));
  console.log(`${what}: ${JSON.stringify(answer).slice(0, 200)}; longest page task ${longest} ms`);
  check('answer' in answer, `${what} failed in the worker: ${JSON.stringify(answer)}`);
  const result = JSON.parse(answer.answer);
  check(result.ok && result.ok.volume > 0 && result.ok.area > 0, `${what} answered no properties: ${answer.answer}`);
  check(longest < 250, `${what} blocked the page for ${longest} ms`);
  return result.ok;
}

// The Inspect strip, then Boss-Extrude3: the whole part.
await page.eval(() => sdClick(19, 241));
await page.wait(500);
const whole = await measure(255, 1, 'Mass properties of the whole part');
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

// Boss-Extrude1 alone: a second worker answers, and the plate is smaller than the part.
const plate = await measure(203, 3, 'Mass properties after the worker stopped');
check(await page.eval(() => window.kernelWorkers.length) === 2, 'The next request did not start a new kernel worker');
check(plate.volume < whole.volume, `The plate (${plate.volume}) is not smaller than the part (${whole.volume})`);
console.log('Kernel worker check passed');
