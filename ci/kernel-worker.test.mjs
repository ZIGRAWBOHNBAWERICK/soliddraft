// The browser kernel worker's message protocol, driven in Node against a stand-in kernel
// module: the worker loads the module named by its init message, passes each job its id,
// relays a checked build's early hand-over before the answer, answers every job with its
// id and builds, and answers a replaced job as superseded.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';

const WORKER = new URL('../crates/soliddraft-app/src/kernel_worker.js', import.meta.url);

const KERNEL = `
export default async function init() {}
export function kernel_worker_commit() { return 'test-commit'; }
export function kernel_worker_run(header, part, id) {
  const job = JSON.parse(header);
  globalThis.runs.push({ job: job.job, id, part: [...part] });
  if (job.checked) self.postMessage({ id, early: new Uint8Array([7]) });
  return JSON.stringify({ ok: { job: job.job } });
}
export function kernel_worker_hand_over() { return new Uint8Array([9, 9]); }
`;

function settle(sent, count) {
  return new Promise((resolve, reject) => {
    const until = Date.now() + 5000;
    const look = () => {
      if (sent.length >= count) resolve();
      else if (Date.now() > until) reject(new Error(`only ${sent.length} of ${count} messages: ${JSON.stringify(sent)}`));
      else setTimeout(look, 5);
    };
    look();
  });
}

test('worker messages reach their handlers with their job ids', async () => {
  const base = mkdtempSync(join(tmpdir(), 'kernel-worker-'));
  writeFileSync(join(base, 'soliddraft-kernel.js'), KERNEL);
  const sent = [];
  globalThis.runs = [];
  globalThis.self = { postMessage: (message) => sent.push(message), close: () => sent.push({ closed: true }) };
  await import(WORKER.href);
  const post = (data) => self.onmessage({ data });
  const job = (id, slot, header) => post({ id, slot, header: JSON.stringify(header), part: new Uint8Array([id]) });

  post({ init: { base: pathToFileURL(base + '/').href, commit: 'test-commit', handOver: true } });
  await settle(sent, 1);
  assert.deepEqual(sent.shift(), { ready: true });

  // A checked build: the early hand-over comes first, then the answer with its builds.
  job(7, 'build', { job: 'build', checked: true });
  await settle(sent, 2);
  assert.deepEqual(sent[0], { id: 7, early: new Uint8Array([7]) });
  assert.equal(sent[1].id, 7);
  assert.deepEqual(JSON.parse(sent[1].answer), { ok: { job: 'build' } });
  assert.deepEqual(sent[1].hand_over, new Uint8Array([9, 9]));
  assert.deepEqual(globalThis.runs.shift(), { job: 'build', id: 7, part: [7] });
  sent.length = 0;

  // A newer job of the same slot replaces one that has not started; other slots run.
  job(8, 'build', { job: 'build' });
  job(9, 'build', { job: 'build' });
  job(10, 'part_proof', { job: 'part_proof', estimate: true });
  await settle(sent, 3);
  assert.deepEqual(sent[0], { id: 8, superseded: true });
  assert.deepEqual(sent.slice(1).map((message) => message.id).sort(), [10, 9].sort());
  assert.ok(sent.slice(1).every((message) => !('early' in message)), 'only a checked build hands over early');
  assert.deepEqual(globalThis.runs.map((run) => [run.job, run.id]).sort(), [['build', 9], ['part_proof', 10]].sort());
});
