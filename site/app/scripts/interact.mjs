#!/usr/bin/env node
// Interaction checks in headless Chromium: rail click, skip intro, sound button, reduced motion
// fallback. Prints one JSON line per check plus any console errors.
//   node scripts/interact.mjs <url> <outdir> [--gpu]
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire("/home/bohn/.local/share/mise/installs/npm-playwright/latest/node_modules/");
const { chromium } = require("playwright");
const [url, outdir, ...rest] = process.argv.slice(2);
mkdirSync(outdir, { recursive: true });
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
const errors = [];
const attach = (page) => {
  page.on("pageerror", (e) => errors.push(`pageerror: ${e}`));
  page.on("console", (m) => m.type() === "error" && errors.push(`console: ${m.text()}`));
};
const state = (page) =>
  page.evaluate(() => ({
    scrollY: Math.round(scrollY),
    cls: document.documentElement.className,
    rail: document.querySelector(".rail a.is-active")?.textContent ?? null,
    caption: [...document.querySelectorAll(".caption.is-on h1, .caption.is-on h2")].map((h) => h.textContent),
    sound: document.querySelector("[data-sound]")?.getAttribute("aria-pressed"),
  }));

// Live page.
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
attach(page);
await page.goto(url, { waitUntil: "networkidle" });
await page.waitForFunction(() => document.documentElement.classList.contains("is-live"));
await page.waitForTimeout(1200);
console.log(JSON.stringify({ check: "load", ...(await state(page)) }));

await page.click('.rail a[data-section="cad"]');
await page.waitForTimeout(2600);
console.log(JSON.stringify({ check: "rail click CAD", ...(await state(page)) }));
await page.screenshot({ path: `${outdir}/rail-cad.png` });

await page.evaluate(() => window.scrollTo(0, 0));
await page.waitForTimeout(1500);
await page.click("[data-skip]");
await page.waitForTimeout(2600);
console.log(JSON.stringify({ check: "skip intro", ...(await state(page)) }));

await page.click("[data-sound]");
await page.waitForTimeout(600);
console.log(JSON.stringify({ check: "sound on", ...(await state(page)) }));
await page.click("[data-sound]");
await page.waitForTimeout(300);
console.log(JSON.stringify({ check: "sound off", ...(await state(page)) }));

await page.click('.rail a[data-section="contact"]');
await page.waitForTimeout(3200);
console.log(JSON.stringify({ check: "rail click Contact", ...(await state(page)) }));
await page.screenshot({ path: `${outdir}/rail-contact.png` });
await page.close();

// Reduced motion: no canvas, everything readable in flow.
const quiet = await browser.newPage({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
attach(quiet);
await quiet.goto(url, { waitUntil: "networkidle" });
await quiet.waitForTimeout(1500);
const q = await quiet.evaluate(() => ({
  cls: document.documentElement.className,
  canvasShown: getComputedStyle(document.querySelector("canvas.stage")).display,
  height: document.documentElement.scrollHeight,
  headings: [...document.querySelectorAll("h1, h2")].filter((h) => h.getBoundingClientRect().height > 0).length,
}));
console.log(JSON.stringify({ check: "reduced motion", ...q }));
await quiet.screenshot({ path: `${outdir}/static-full.png`, fullPage: true });
await quiet.close();

console.log(JSON.stringify({ errors: errors.slice(0, 20), errorCount: errors.length }));
await browser.close();
