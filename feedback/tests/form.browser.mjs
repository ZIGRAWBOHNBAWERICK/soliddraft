import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)("playwright");

const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROMIUM_PATH
    ? { executablePath: process.env.CHROMIUM_PATH }
    : {}),
});
try {
  const page = await browser.newPage({ viewport: { width: 320, height: 800 } });
  let attempts = 0;
  let preview;
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.hostname === "challenges.cloudflare.com") {
      return route.fulfill({
        contentType: "text/javascript",
        body: `
        let options;
        window.turnstile = {
          render(el, opts) { options = opts; window.widgetSize = opts.size; queueMicrotask(() => opts.callback("test-token")); return 1; },
          reset() { queueMicrotask(() => options.callback("test-token")); },
          remove() {},
        };`,
      });
    }
    if (url.hostname === "soliddraft-feedback.sdc-feedback-inbox.workers.dev") {
      assert.deepEqual(route.request().postDataJSON(), preview);
      attempts++;
      return route.fulfill({
        status: attempts === 1 ? 503 : 201,
        contentType: "application/json",
        headers: { "Access-Control-Allow-Origin": "https://feedback.test" },
        body: JSON.stringify(
          attempts === 1
            ? { error: "Temporary failure." }
            : { id: "test-reference" },
        ),
      });
    }
    if (url.hostname !== "feedback.test")
      throw new Error("Unexpected external request");
    if (url.pathname === "/") {
      return route.fulfill({
        contentType: "text/html",
        body: `
        <link rel="stylesheet" href="/form.css">
        <div id="feedback" style="width:260px"></div>
        <script type="module">
          import { mountFeedback } from '/form.mjs';
          mountFeedback(document.getElementById('feedback'), {
            source: 'app', version: '0.1.0', commit: 'abcdef0',
            context: () => ({ browser: 'Test browser' }),
          });
        </script>`,
      });
    }
    assert.ok(["/form.mjs", "/form.css", "/config.mjs"].includes(url.pathname));
    return route.fulfill({
      contentType: url.pathname.endsWith(".css")
        ? "text/css"
        : "text/javascript",
      body: await readFile(
        new URL(`../client${url.pathname}`, import.meta.url),
      ),
    });
  });
  await page.goto("https://feedback.test/");
  const message = "The message must survive a failed submission.";
  await page.getByRole("textbox", { name: "Your message" }).fill(message);
  await page.getByRole("button", { name: "Review feedback" }).click();
  await page.getByRole("checkbox").check();
  preview = JSON.parse(await page.locator("pre").textContent());
  assert.equal(preview.context, null);
  assert.equal(await page.evaluate(() => window.widgetSize), "compact");
  await page
    .getByRole("button", { name: "Send feedback", exact: true })
    .click();
  await page.waitForFunction(() =>
    document
      .querySelector("[role=status]")
      .textContent.includes("Your message is still here"),
  );
  // A new spam token must not erase the failure or the original message.
  assert.match(
    await page.getByRole("status").textContent(),
    /Temporary failure/,
  );
  assert.equal(
    await page.getByRole("textbox", { name: "Your message" }).inputValue(),
    message,
  );
  assert.deepEqual(
    JSON.parse(await page.locator("pre").textContent()),
    preview,
  );
  assert.ok(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  );
  await page
    .getByRole("button", { name: "Send feedback", exact: true })
    .click();
  await page.waitForFunction(() =>
    document
      .querySelector("[role=status]")
      .textContent.includes("Feedback saved."),
  );
  assert.equal(
    await page.getByRole("textbox", { name: "Your message" }).inputValue(),
    "",
  );
  assert.equal(attempts, 2);
  console.log(
    "Feedback form: context opt-out, compact widget, failure retention and retry passed.",
  );
} finally {
  await browser.close();
}
