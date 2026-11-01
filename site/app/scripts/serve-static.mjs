import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, resolve } from "node:path";

const pagesDir = resolve(import.meta.dirname, "../dist/pages");
const port = Number(process.env.PORT ?? 4173);
const mimeTypes = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".ico", "image/x-icon"],
  [".jpg", "image/jpeg"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".mp4", "video/mp4"],
  [".png", "image/png"],
  [".svg", "image/svg+xml"],
  [".webmanifest", "application/manifest+json"],
  [".webp", "image/webp"],
  [".woff2", "font/woff2"],
  [".xml", "application/xml; charset=utf-8"],
]);

createServer(async (request, response) => {
  const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
  const candidate = pathname === "/" ? "index.html" : pathname.slice(1);
  let filePath = resolve(pagesDir, candidate);
  let status = 200;

  if (!filePath.startsWith(`${pagesDir}/`) && filePath !== pagesDir) {
    response.writeHead(400).end("Bad request");
    return;
  }

  try {
    const details = await stat(filePath);
    if (details.isDirectory()) filePath = resolve(filePath, "index.html");
    await stat(filePath);
  } catch {
    filePath = resolve(pagesDir, "404.html");
    status = 404;
  }

  response.writeHead(status, {
    "Content-Type": mimeTypes.get(extname(filePath)) ?? "application/octet-stream",
  });
  if (request.method === "HEAD") {
    response.end();
    return;
  }
  createReadStream(filePath).pipe(response);
}).listen(port, "127.0.0.1", () => {
  console.log(`Static preview: http://127.0.0.1:${port}`);
});
