import { access, cp, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const root = resolve(import.meta.dirname, "..");
const clientDir = resolve(root, "dist/client");
const pagesDir = resolve(root, "dist/pages");
const serverUrl = pathToFileURL(resolve(root, "dist/server/server.js")).href;
// The page is served under this path on GitHub Pages; vite.config.ts uses the same default.
const base = process.env.SITE_BASE ?? "/soliddraft/";

await rm(pagesDir, { force: true, recursive: true });
await mkdir(pagesDir, { recursive: true });
await cp(clientDir, pagesDir, {
  filter(source) {
    const relativeSource = source.slice(clientDir.length).replaceAll("\\", "/");
    return !["/presets", "/assets/landing"].some(
      (excluded) => relativeSource === excluded || relativeSource.startsWith(`${excluded}/`),
    );
  },
  recursive: true,
});

async function assertAbsent(pathname) {
  try {
    await access(resolve(pagesDir, pathname));
  } catch (error) {
    if (error?.code === "ENOENT") return;
    throw error;
  }
  throw new Error(`${pathname} must not be present in the Pages artifact`);
}

await assertAbsent("assets/landing/template-preview.svg");

const server = (await import(serverUrl)).default;

async function render(pathname, output, expectedStatus) {
  const response = await server.fetch(
    new Request(`https://soliddraft.invalid${pathname}`),
    {},
    {},
  );
  if (response.status !== expectedStatus) {
    throw new Error(`${pathname} returned ${response.status}, expected ${expectedStatus}`);
  }
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.includes("text/html")) {
    throw new Error(`${pathname} returned ${contentType}, expected HTML`);
  }
  await writeFile(resolve(pagesDir, output), await response.text());
}

await render(base, "index.html", 200);
await render(`${base}page-that-does-not-exist`, "404.html", 404);
// The browser app is added under `app/` by the Pages workflow from the release zip; the page
// itself must not claim that path.
const appResponse = await server.fetch(new Request(`https://soliddraft.invalid${base}app`), {}, {});
if (appResponse.status !== 404) {
  throw new Error(`${base}app returned ${appResponse.status}, expected 404`);
}

const forbiddenTemplatePatterns = [
  /["']\/app(?:\?|["'])/,
  /label:"Completed",value:"128"/,
  /label:"In progress",value:"8"/,
  /label:"Collections",value:"24"/,
];

async function assertNoTemplateMetrics(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      await assertNoTemplateMetrics(path);
    } else if (/\.(?:html|js)$/.test(entry.name)) {
      const content = await readFile(path, "utf8");
      if (forbiddenTemplatePatterns.some((pattern) => pattern.test(content))) {
        throw new Error(`Template metrics found in ${path}`);
      }
    }
  }
}

await assertNoTemplateMetrics(pagesDir);

console.log(`Static Pages output: ${pagesDir}`);
process.exit(0);
