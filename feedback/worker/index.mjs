import { readLimited, validate } from "./validation.mjs";

export default {
  async fetch(request, env) {
    const origin = request.headers.get("Origin");
    const allowed = env.ALLOWED_ORIGINS.split(",");
    const headers = {
      "Vary": "Origin",
      "Cache-Control": "no-store",
      "Content-Type": "application/json",
      "X-Content-Type-Options": "nosniff",
    };
    const reply = (status, body) => new Response(JSON.stringify(body), { status, headers });
    if (!allowed.includes(origin)) return reply(403, { error: "This origin cannot send feedback." });
    headers["Access-Control-Allow-Origin"] = origin;
    if (new URL(request.url).pathname !== "/feedback") return reply(404, { error: "Not found." });
    if (request.method === "OPTIONS") {
      headers["Access-Control-Allow-Methods"] = "POST";
      headers["Access-Control-Allow-Headers"] = "Content-Type, X-Turnstile-Token";
      headers["Access-Control-Max-Age"] = "600";
      return new Response(null, { status: 204, headers });
    }
    if (request.method !== "POST") return reply(405, { error: "Use POST." });
    const ip = request.headers.get("CF-Connecting-IP");
    if (!ip) return reply(503, { error: "Feedback is unavailable. Please email support@zigrawbohnbawerick.org." });
    if (!(await env.IP_LIMIT.limit({ key: ip })).success) {
      headers["Retry-After"] = "60";
      return reply(429, { error: "Too many attempts. Wait a minute and try again." });
    }
    if (request.headers.get("Content-Type")?.split(";")[0].trim() !== "application/json") return reply(415, { error: "Use JSON." });
    let feedback;
    try {
      feedback = validate(await readLimited(request));
    } catch (error) {
      return reply(error instanceof RangeError ? 413 : 400, { error: error instanceof SyntaxError ? "Invalid JSON." : error.message });
    }
    const token = request.headers.get("X-Turnstile-Token");
    if (!token || token.length > 2048) return reply(400, { error: "Complete the spam check, then try again." });
    try {
      const verification = await fetch("https://challenges.cloudflare.com/turnstile/v0/siteverify", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ secret: env.TURNSTILE_SECRET, response: token }),
        signal: AbortSignal.timeout(10_000),
      });
      const result = await verification.json();
      if (!verification.ok || !result.success || result.action !== "feedback" || result.hostname !== new URL(origin).hostname) {
        return reply(400, { error: "The spam check expired or failed. Try it again." });
      }
      const id = crypto.randomUUID();
      await env.DB.prepare("INSERT INTO feedback (id, created_at, payload) VALUES (?, ?, ?)")
        .bind(id, new Date().toISOString(), JSON.stringify(feedback)).run();
      return reply(201, { id });
    } catch {
      return reply(503, { error: "Feedback could not be saved. Try again or email support@zigrawbohnbawerick.org." });
    }
  },
};
