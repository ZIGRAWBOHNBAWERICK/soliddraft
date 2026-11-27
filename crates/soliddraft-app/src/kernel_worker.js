// The browser's kernel worker. The page starts it with the address of its wasm-bindgen
// glue and its compiled WebAssembly module (kernel_worker.rs), so the worker runs the
// same application code with its own memory and its own Catallax kernel.
//
// Jobs arrive as { id, slot, header, part }. A checked build posts { id, early } with its
// rebuilt part before its answer, so the page can mesh a GPU preview meanwhile. A newer
// job for the same slot replaces one that has not started: the worker answers the
// replaced job { id, superseded: true }.
// A job that traps the module ends the worker. It answers { id, fatal } first, and the
// page starts a new worker for its next job.

let glue = null;
let queue = [];
let draining = false;
let dead = false;

// A browser check that follows its jobs asks for these notes with { trace: true }.
function trace(job, stage) {
  if (job.trace) self.postMessage({ trace: { stage, id: job.id, at: Date.now() } });
}

self.onmessage = async (event) => {
  const message = event.data;
  if (message.init) {
    try {
      glue = await import(message.init.glue);
      glue.initSync({ module: message.init.module });
      self.postMessage({ ready: true });
    } catch (error) {
      dead = true;
      self.postMessage({ fatal: `the kernel worker could not start: ${error}` });
      self.close();
      return;
    }
    schedule();
    return;
  }
  trace(message, `worker received; ${glue ? 'ready' : 'starting'}, ${draining ? 'busy' : 'idle'}, ${dead ? 'dead' : 'alive'}, ${queue.length} queued`);
  queue.push(message);
  schedule();
};

function schedule() {
  if (glue && !draining && !dead && queue.length) {
    draining = true;
    // Let every message already posted arrive first, so a superseded job never starts.
    setTimeout(drain, 0);
  }
}

// Runs one job, then yields so messages posted meanwhile can replace the rest.
function drain() {
  const newest = new Map();
  for (const job of queue) newest.set(job.slot, job.id);
  const kept = [];
  for (const job of queue) {
    if (newest.get(job.slot) === job.id) kept.push(job);
    else self.postMessage({ id: job.id, superseded: true });
  }
  const job = kept.shift();
  queue = kept;
  trace(job, 'worker started');
  let answer;
  try {
    answer = glue.kernel_worker_run(job.header, job.part, job.id);
  } catch (error) {
    dead = true;
    self.postMessage({ id: job.id, fatal: String(error) });
    for (const waiting of queue) self.postMessage({ id: waiting.id, fatal: 'the kernel worker stopped' });
    self.close();
    return;
  }
  trace(job, 'worker replied');
  // The builds made for this job travel with its answer, for the page's own kernel.
  let hand_over = null;
  try {
    hand_over = glue.kernel_worker_hand_over();
  } catch (error) {
    hand_over = null;
  }
  if (hand_over && hand_over.length) {
    self.postMessage({ id: job.id, answer, hand_over }, [hand_over.buffer]);
  } else {
    self.postMessage({ id: job.id, answer });
  }
  draining = false;
  schedule();
}
