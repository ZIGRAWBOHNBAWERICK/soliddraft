import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

function assetCacheDirectives(): Map<string, string | true> {
  const blocks = readFileSync(new URL("../public/_headers", import.meta.url), "utf8")
    .trim()
    .split(/\n\s*\n/);
  const assetBlock = blocks.find((block) => block.startsWith("/assets/*\n"));
  if (assetBlock == null) throw new Error("Missing /assets/* header block");

  const cacheControl = assetBlock
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.toLowerCase().startsWith("cache-control:"));
  if (cacheControl == null) throw new Error("Missing asset Cache-Control header");

  return new Map(
    cacheControl
      .slice(cacheControl.indexOf(":") + 1)
      .split(",")
      .map((directive) => directive.trim().split("=", 2))
      .map(([name, value]) => [name!.toLowerCase(), value ?? true]),
  );
}

test("stable asset names require revalidation", () => {
  const directives = assetCacheDirectives();

  expect(directives.get("max-age")).toBe("0");
  expect(directives.has("must-revalidate")).toBe(true);
  expect(directives.has("immutable")).toBe(false);
});
