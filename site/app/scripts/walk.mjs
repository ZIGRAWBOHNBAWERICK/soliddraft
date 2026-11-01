#!/usr/bin/env node
// Walk the whole page top to bottom in headless Chromium, screenshot every step, and report
// console errors, frame rate per step and JS heap. The reference site froze at its project
// section; this is how we check ours does not.
//   node scripts/walk.mjs <url> <outdir> [--steps 24] [--w 1440] [--h 900] [--wait 1400] [--gpu]
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire("/home/bohn/.local/share/mise/installs/npm-playwright/latest/node_modules/");
const { chromium } = require("playwright");

const [url, outdir, ...rest] = process.argv.slice(2);
if (!url || !outdir) {
  console.error("usage: walk.mjs <url> <outdir> [--steps 24] [--w 1440] [--h 900] [--wait 1400] [--gpu]");
  process.exit(2);
}
const opt = (name, fallback) => {
  const i = rest.indexOf(`--${name}`);
  return i >= 0 ? rest[i + 1] : fallback;
};
const steps = Number(opt("steps", 24));
const width = Number(opt("w", 1440));
const height = Number(opt("h", 900));
const wait = Number(opt("wait", 1400));
mkdirSync(outdir, { recursive: true });

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
page.on("pageerror", (e) => errors.push(`pageerror: ${e}`));
page.on("console", (m) => {
  if (m.type() === "error") errors.push(`console: ${m.text()}`);
});
await page.goto(url, { waitUntil: "networkidle", timeout: 60000 });
await page.waitForFunction(() => document.documentElement.classList.contains("is-live"), null, { timeout: 30000 });
await page.waitForTimeout(1500);

const total = await page.evaluate(() => document.documentElement.scrollHeight - window.innerHeight);
const rows = [];
for (let i = 0; i <= steps; i++) {
  const y = Math.round((total * i) / steps);
  await page.evaluate((y) => window.scrollTo(0, y), y);
  await page.waitForTimeout(wait);
  const info = await page.evaluate(
    () =>
      new Promise((resolve) => {
        let n = 0;
        const t0 = performance.now();
        const tick = () => {
          n++;
          if (performance.now() - t0 < 700) requestAnimationFrame(tick);
          else {
            const mem = performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1048576) : null;
            const section = [...document.querySelectorAll("section[data-section]")].find((s) => {
              const r = s.getBoundingClientRect();
              return r.top <= 1 && r.bottom > 1;
            });
            const caption = [...document.querySelectorAll(".caption.is-on")].map((c) =>
              c.querySelector("h1,h2")?.textContent?.trim(),
            );
            resolve({
              fps: Math.round(n / 0.7),
              heapMB: mem,
              section: section?.id ?? null,
              light: document.documentElement.classList.contains("is-light"),
              veiled: document.documentElement.classList.contains("is-veiled"),
              caption,
              accent: getComputedStyle(document.documentElement).getPropertyValue("--accent").trim(),
            });
          }
        };
        requestAnimationFrame(tick);
      }),
  );
  const file = `${outdir}/step-${String(i).padStart(2, "0")}.png`;
  await page.screenshot({ path: file });
  rows.push({ step: i, y, ...info, file });
  console.log(JSON.stringify({ step: i, y, ...info }));
}
console.log(JSON.stringify({ total, errors: errors.slice(0, 20), errorCount: errors.length }, null, 2));
await browser.close();
