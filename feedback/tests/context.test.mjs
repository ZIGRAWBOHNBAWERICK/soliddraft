import { test } from "node:test";
import assert from "node:assert/strict";
import { browserAndOS, buildContext, createRecorder, sizeClass } from "../client/context.mjs";
import { validate } from "../worker/validation.mjs";

test("browser and OS are coarse descriptions, never the raw user agent", () => {
  assert.deepEqual(browserAndOS("Mozilla/5.0 (X11; Linux x86_64) Chrome/130.0.1.2 Safari/537.36"), { browser: "Chrome 130.0.1.2", os: "Linux" });
  assert.deepEqual(browserAndOS("private/path user-supplied data"), { browser: "Unknown", os: "Unknown" });
});

test("context allows only bounded command IDs, diagnostic codes and document classes", () => {
  let now = 0;
  const recorder = createRecorder(() => now);
  for (let i = 0; i < 30; i++) { now += 1000; recorder.action("part.extrude"); recorder.diagnostic("app-error"); }
  recorder.action("/home/private/part.dwprt");
  recorder.diagnostic("file contents: secret");
  recorder.document(".dwprt", 51);
  const context = recorder.context({ ua: "Linux Chrome/130.0", width: 1920, height: 1080 });
  assert.equal(context.actions.length, 20);
  assert.equal(context.diagnostics.length, 10);
  assert.equal(context.sessionSeconds, 30);
  assert.equal(context.actions.at(-1).secondsAgo, 0);
  assert.deepEqual(context.document, { extension: ".dwprt", sizeClass: "medium" });
  assert.ok(!JSON.stringify(context).includes("private"));
  validate({ source: "app", kind: "bug", message: "Test message", version: "0.1.0", commit: "abcdef0", context });
});

test("builder strips unknown properties and invalid commands rather than redacting raw data later", () => {
  const c = buildContext({
    ua: "", width: 100, height: 50, now: 10_000, started: 0,
    actions: [{ command: "save", at: 0, path: "/private/model.dwprt", contents: "secret" }],
    diagnostics: [{ code: "crash", level: "error", at: 1000, message: "private name" }],
    document: { extension: ".dwdrw", sizeClass: "large", filename: "secret" },
  });
  assert.deepEqual(c.actions, [{ command: "save", secondsAgo: 10 }]);
  assert.deepEqual(c.diagnostics, [{ code: "crash", level: "error", secondsAgo: 9 }]);
  assert.ok(!JSON.stringify(c).includes("secret"));
  assert.deepEqual([0, 50, 51, 500, 501, -1].map(sizeClass), ["empty", "small", "medium", "medium", "large", "unknown"]);
});
