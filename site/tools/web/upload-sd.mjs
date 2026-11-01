// Try to open a real part in the live browser app through its File > Open upload.
import { createRequire } from "node:module";
const require = createRequire("/home/bohn/.local/share/mise/installs/npm-playwright/latest/node_modules/");
const { chromium } = require("playwright");
const [part, out] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: "/usr/bin/chromium", args: ["--headless=new", "--use-angle=gl-egl", "--enable-gpu", "--ignore-gpu-blocklist", "--enable-webgl", "--no-sandbox"] });
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 1 });
page.on("console", (m) => { if (m.type() === "error") console.log("console:", m.text().slice(0, 160)); });
await page.goto("https://sd.zigrawbohnbawerick.org/", { waitUntil: "load", timeout: 120000 });
await page.waitForTimeout(12000);
console.log("inputs:", await page.locator("input[type=file]").count());
const chooser = page.waitForEvent("filechooser", { timeout: 8000 }).catch(() => null);
// Ctrl+O is the app's open shortcut; the canvas needs focus first.
await page.mouse.click(800, 500);
await page.keyboard.press("Control+KeyO");
let fc = await chooser;
if (!fc) {
  console.log("no chooser from Ctrl+O; trying the File menu");
  const chooser2 = page.waitForEvent("filechooser", { timeout: 8000 }).catch(() => null);
  await page.mouse.click(122, 15); await page.waitForTimeout(600);
  await page.screenshot({ path: out.replace(".png", "-menu.png") });
  await page.mouse.click(160, 60);
  fc = await chooser2;
}
if (fc) { await fc.setFiles(part); console.log("file set"); await page.waitForTimeout(8000); }
else console.log("no file chooser appeared");
await page.screenshot({ path: out });
await browser.close();
