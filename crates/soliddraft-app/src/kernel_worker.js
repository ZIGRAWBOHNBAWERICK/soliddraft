// The browser's kernel worker. The page starts it with the address of its wasm-bindgen
// glue and its compiled WebAssembly module (kernel_worker.rs), so the worker runs the
// same application code with its own memory and its own Catallax kernel.
//
// Jobs arrive as { id, slot, header, part }. A newer job for the same slot replaces one
// that has not started: the worker answers the replaced job { id, superseded: true }.
// A job that traps the module ends the worker. It answers { id, fatal } first, and the
// page starts a new worker for its next job.

let glue = null;
let queue = [];
let draining = false;
let dead = false;

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
  let answer;
  try {
    answer = glue.kernel_worker_run(job.header, job.part);
  } catch (error) {
    dead = true;
    self.postMessage({ id: job.id, fatal: String(error) });
    for (const waiting of queue) self.postMessage({ id: waiting.id, fatal: 'the kernel worker stopped' });
    self.close();
    return;
  }
  self.postMessage({ id: job.id, answer });
  draining = false;
  schedule();
}
