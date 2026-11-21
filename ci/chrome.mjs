// Headless Chrome for the browser checks: started in a process group of its own, and
// stopped together with the helpers it starts.
//
// Chromium's helpers write into the profile until they exit, and killing the browser
// process alone leaves them running: one was still writing there while a check deleted
// the profile, which failed with ENOTEMPTY after every check had passed. So a check
// closes the browser through the DevTools protocol, which ends its helpers, and then
// kills whatever is left of its process group before the profile goes.

import { spawn } from 'node:child_process';

export function startChrome(chrome, args) {
  return spawn(chrome, args, { stdio: ['ignore', 'ignore', 'pipe'], detached: true });
}

const within = ms => new Promise(resolve => setTimeout(resolve, ms));
const running = child => child.exitCode === null && child.signalCode === null;

// Kills the browser and every helper at once, as a crash would.
export function killGroup(child) {
  try {
    process.kill(-child.pid, 'SIGKILL');
  } catch (error) {
    if (error.code !== 'ESRCH') throw error;
  }
}

// `close`, when given, asks the browser to close itself, as DevTools' Browser.close does.
export async function stopChrome(child, close) {
  if (running(child)) {
    const exited = new Promise(resolve => child.once('exit', resolve));
    if (close) await Promise.race([close().catch(() => {}), within(5000)]);
    await Promise.race([exited, within(10000)]);
  }
  killGroup(child);
}
