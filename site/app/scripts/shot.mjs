#!/usr/bin/env node
// Screenshot a page with WebGL in headless Chromium.
//   node scripts/shot.mjs <url> <out.png> [--w 1440] [--h 900] [--wait 1800] [--p 0.4] [--pointer 0.3,-0.2] [--scroll 2400] [--frames 1] [--gpu]
// With --p the page is a /lab URL and progress is set through window.__lab after it is ready.
// With --frames N it writes N numbered shots 400 ms apart to check that things move.
import { createRequire } from "node:module";

const require = createRequire("/home/bohn/.local/share/mise/installs/npm-playwright/latest/node_modules/");
const { chromium } = require("playwright");

const [url, out, ...rest] = process.argv.slice(2);
if (!url || !out) {
  console.error("usage: shot.mjs <url> <out.png> [--w 1440] [--h 900] [--wait ms] [--p 0..1] [--pointer x,y] [--scroll px] [--frames n]");
  process.exit(2);
}
const opt = (name, fallback) => {
  const i = rest.indexOf(`--${name}`);
  return i >= 0 ? rest[i + 1] : fallback;
};
const width = Number(opt("w", 1440));
const height = Number(opt("h", 900));
const wait = Number(opt("wait", 1800));
const progress = opt("p", null);
const pointer = opt("pointer", null);
const scroll = opt("scroll", null);
const frames = Number(opt("frames", 1));

// --gpu uses the machine's GPU through EGL (real frame rates); default is SwiftShader (works anywhere).
const gpu = rest.includes("--gpu");
const browser = await chromium.launch({
  executablePath: "/usr/bin/chromium",
  args: [
    ...(gpu
      ? ["--headless=new", "--use-angle=gl-egl", "--enable-gpu", "--ignore-gpu-blocklist"]
      : ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"]),
    "--enable-webgl",
    "--autoplay-policy=no-user-gesture-required",
    "--no-sandbox",
  ],
});
const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") errors.push(`${m.type()}: ${m.text()}`);
});
await page.goto(url, { waitUntil: "networkidle", timeout: 60000 });
await page.waitForFunction(() => document.documentElement.classList.contains("is-live"), null, { timeout: 30000 });
if (progress !== null) {
  await page.waitForFunction(() => Boolean(window.__lab), null, { timeout: 30000 });
  await page.evaluate(() => window.__lab.ready);
  await page.evaluate((p) => window.__lab.set(Number(p)), progress);
}
if (pointer) {
  const [x, y] = pointer.split(",").map(Number);
  if (progress !== null) await page.evaluate(([x, y]) => window.__lab.pointer(x, y), [x, y]);
  else await page.mouse.move(((x + 1) / 2) * width, ((y + 1) / 2) * height, { steps: 8 });
}
if (scroll !== null) {
  await page.evaluate((y) => window.scrollTo(0, Number(y)), scroll);
}
await page.waitForTimeout(wait);
for (let i = 0; i < frames; i++) {
  const path = frames > 1 ? out.replace(/\.png$/, `-${String(i + 1).padStart(2, "0")}.png`) : out;
  await page.screenshot({ path });
  if (frames > 1) await page.waitForTimeout(400);
}
const fps = await page.evaluate(
  () =>
    new Promise((resolve) => {
      let n = 0;
      const t0 = performance.now();
      const tick = () => {
        n++;
        if (performance.now() - t0 < 1000) requestAnimationFrame(tick);
        else resolve(n);
      };
      requestAnimationFrame(tick);
    }),
);
console.log(JSON.stringify({ out, fps, errors: errors.slice(0, 12) }, null, 2));
await browser.close();
