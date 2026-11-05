import { test } from "node:test";
import assert from "node:assert/strict";
import { validate, readLimited, MAX_BYTES } from "../worker/validation.mjs";
import worker from "../worker/index.mjs";

const payload = () => ({ source: "site", version: "0.1.0", commit: "abcdef0", kind: "bug", message: "Something broke.", context: null });
const request = (body = payload(), headers = {}, method = "POST") => new Request("https://feedback.example/feedback", {
  method, headers: { Origin: "https://zigrawbohnbawerick.github.io", "Content-Type": "application/json", "CF-Connecting-IP": "192.0.2.1", "X-Turnstile-Token": "dummy-token", ...headers },
  ...(method === "POST" ? { body: JSON.stringify(body) } : {}),
});
const env = () => ({
  ALLOWED_ORIGINS: "https://zigrawbohnbawerick.github.io",
  TURNSTILE_SECRET: "dummy-secret",
  IP_LIMIT: { limit: async () => ({ success: true }) },
  DB: { prepare: () => ({ bind: (...args) => ({ run: async () => { assert.equal(args.length, 3); assert.equal(args[2], JSON.stringify(payload())); } }) }) },
});

test("valid messages and optional contact; unknown data and malformed fields fail closed", () => {
  assert.deepEqual(validate(payload()), payload());
  assert.equal(validate({ ...payload(), email: "someone@example.org" }).email, "someone@example.org");
  for (const patch of [
    { source: "elsewhere" }, { kind: "spam" }, { message: "  " }, { message: "x".repeat(4001) },
    { email: "not-email" }, { email: "a\nb@example.org" }, { ip: "192.0.2.1" }, { version: "../../file" },
    { commit: "unbuilt" }, { version: "v".repeat(41) }, { context: {} }, { context: { filename: "/private/part.dwprt" } },
  ]) assert.throws(() => validate({ ...payload(), ...patch }));
});

test("byte cap works without a Content-Length header and with multibyte input", async () => {
  await assert.rejects(readLimited(new Request("https://example.org", { method: "POST", body: "x".repeat(MAX_BYTES + 1) })), RangeError);
  await assert.rejects(readLimited(new Request("https://example.org", { method: "POST", body: "😀".repeat(MAX_BYTES / 3) })), RangeError);
  await assert.rejects(readLimited(new Request("https://example.org", { method: "POST", body: "{" })), SyntaxError);
  assert.deepEqual(await readLimited(request()), payload());
});

test("CORS is exact, methods and content types are limited, and IP attempts are throttled", async () => {
  assert.equal((await worker.fetch(request(undefined, { Origin: "https://evil.example" }), env())).status, 403);
  assert.equal((await worker.fetch(request(undefined, { Origin: "https://zigrawbohnbawerick.github.io.evil.example" }), env())).headers.get("Access-Control-Allow-Origin"), null);
  const cors = await worker.fetch(request(undefined, {}, "OPTIONS"), env());
  assert.equal(cors.status, 204);
  assert.equal(cors.headers.get("Access-Control-Allow-Origin"), "https://zigrawbohnbawerick.github.io");
  assert.equal((await worker.fetch(request(undefined, {}, "GET"), env())).status, 405);
  assert.equal((await worker.fetch(request(undefined, { "Content-Type": "text/plain" }), env())).status, 415);
  const limited = env();
  limited.IP_LIMIT.limit = async () => ({ success: false });
  const response = await worker.fetch(request(), limited);
  assert.equal(response.status, 429);
  assert.equal(response.headers.get("Retry-After"), "60");
});

test("Turnstile must pass with the correct action and hostname before a private row is stored", async () => {
  const original = globalThis.fetch;
  let writes = 0;
  const bindings = env();
  const prepare = bindings.DB.prepare;
  bindings.DB.prepare = (...args) => { writes++; return prepare(...args); };
  try {
    for (const verification of [
      { success: false }, { success: true, action: "other", hostname: "zigrawbohnbawerick.github.io" },
      { success: true, action: "feedback", hostname: "evil.example" },
    ]) {
      globalThis.fetch = async () => Response.json(verification);
      assert.equal((await worker.fetch(request(), bindings)).status, 400);
    }
    assert.equal(writes, 0);
    globalThis.fetch = async (url, options) => {
      assert.equal(url, "https://challenges.cloudflare.com/turnstile/v0/siteverify");
      assert.equal(JSON.parse(options.body).remoteip, undefined);
      return Response.json({ success: true, action: "feedback", hostname: "zigrawbohnbawerick.github.io" });
    };
    const response = await worker.fetch(request(), bindings);
    assert.equal(response.status, 201);
    assert.match((await response.json()).id, /^[a-f0-9-]{36}$/);
    assert.equal(writes, 1);
    bindings.DB.prepare = () => { throw new Error("internal details"); };
    const failed = await worker.fetch(request(), bindings);
    assert.equal(failed.status, 503);
    assert.ok(!(await failed.text()).includes("internal details"));
  } finally { globalThis.fetch = original; }
});
