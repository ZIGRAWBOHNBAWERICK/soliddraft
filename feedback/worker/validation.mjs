export const MAX_BYTES = 16_384;
const object = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const text = (v, max, pattern) => typeof v === "string" && v.length <= max && (!pattern || pattern.test(v));
const integer = (v, max) => Number.isInteger(v) && v >= 0 && v <= max;
const keys = (v, allowed) => object(v) && Object.keys(v).every((k) => allowed.includes(k));
const safeName = /^[a-zA-Z0-9 ._()+-]*$/;

export function validContext(c) {
  if (c === null) return true;
  if (!keys(c, ["browser", "os", "screen", "sessionSeconds", "actions", "diagnostics", "document"])) return false;
  if (!text(c.browser, 80, safeName) || !text(c.os, 80, safeName)) return false;
  if (!Array.isArray(c.screen) || c.screen.length !== 2 || !c.screen.every((n) => integer(n, 100_000))) return false;
  if (!integer(c.sessionSeconds, 31_536_000)) return false;
  if (!Array.isArray(c.actions) || c.actions.length > 20 || !c.actions.every((a) =>
    keys(a, ["command", "secondsAgo"]) && text(a.command, 80, /^[a-z][a-z0-9_.-]*$/) && integer(a.secondsAgo, 31_536_000))) return false;
  if (!Array.isArray(c.diagnostics) || c.diagnostics.length > 10 || !c.diagnostics.every((d) =>
    keys(d, ["level", "code", "secondsAgo"]) && ["error", "warning"].includes(d.level) &&
    ["app-error", "app-warning", "browser-error", "unhandled-rejection", "crash", "startup-error"].includes(d.code) && integer(d.secondsAgo, 31_536_000))) return false;
  return keys(c.document, ["extension", "sizeClass"]) &&
    ["none", ".dwprt", ".dwdrw", ".dwasm"].includes(c.document.extension) &&
    ["empty", "small", "medium", "large", "unknown"].includes(c.document.sizeClass);
}

export function validate(value) {
  if (!keys(value, ["source", "version", "commit", "kind", "message", "email", "context"])) throw new Error("Unexpected fields.");
  if (!["site", "app"].includes(value.source)) throw new Error("Choose a feedback source.");
  if (!text(value.version, 40, /^[a-zA-Z0-9][a-zA-Z0-9.+_-]*$/) || !text(value.commit, 40, /^(?:[a-f0-9]{7,40}|unknown)$/)) throw new Error("Invalid build identity.");
  if (!["bug", "idea", "other"].includes(value.kind)) throw new Error("Choose bug, idea or other.");
  if (!text(value.message, 4000) || value.message.trim().length < 5) throw new Error("Write between 5 and 4000 characters.");
  if (value.email !== undefined && (!text(value.email, 254) || !/^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/.test(value.email))) throw new Error("Check your email address or leave it blank.");
  if (!validContext(value.context)) throw new Error("Invalid diagnostic context.");
  return value;
}

export async function readLimited(request) {
  if (!request.body) throw new Error("Empty request.");
  const reader = request.body.getReader();
  let bytes = 0;
  const chunks = [];
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.length;
      if (bytes > MAX_BYTES) throw new RangeError("Message is too large.");
      chunks.push(value);
    }
  } finally {
    await reader.cancel();
  }
  const body = new Uint8Array(bytes);
  let offset = 0;
  for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.length; }
  return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body));
}
