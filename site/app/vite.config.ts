import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import {
  higgsfieldDesignInspectorVitePlugin,
  higgsfieldDesignSourceBabelPlugin,
} from "./src/module/design-inspector/vite";
import svgr from "vite-plugin-svgr";
import { defaultServerConditions, defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The page is served under the repository path on GitHub Pages. The same base applies in
// development so asset paths are exercised the way they deploy.
const SITE_BASE = process.env.SITE_BASE ?? "/soliddraft/";

// The released version: the Pages workflow passes the tag's version; a local build reads the
// workspace Cargo.toml so the page never shows a made-up number.
if (!process.env.VITE_SOLIDDRAFT_VERSION) {
  const cargo = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
  const match = cargo.match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/);
  if (match) process.env.VITE_SOLIDDRAFT_VERSION = match[1];
}

// The vendored @higgsfield/quanta components import their glyphs from the private
// Nexus-only `@higgsfield-ai/icons`. Generated sites build on the PUBLIC npm
// registry, so we redirect every `@higgsfield-ai/icons/*` import to a lucide
// shim instead (see src/lib/quanta-icons.ts). tsconfig.json has
// the matching `paths` entry so type-checking resolves it too.
const QUANTA_ICONS_SHIM = fileURLToPath(new URL("./src/lib/quanta-icons.ts", import.meta.url));

export default defineConfig(({ command, mode }) => {
  const designInspectorEnabled = process.env.HF_DESIGN_INSPECTOR === "1" || mode === "design";

  return {
    base: SITE_BASE,
    // fsevents can miss edits under some setups (bun-launched dev, synced/virtual
    // dirs), leaving HMR dead so changes only appear after a manual restart.
    // Polling the watcher makes file changes reliably trigger HMR / SSR reload.
    server: {
      watch: { usePolling: true, interval: 150 },
    },
    resolve: {
      tsconfigPaths: true,
      alias: [{ find: /^@higgsfield-ai\/icons(\/.*)?$/, replacement: QUANTA_ICONS_SHIM }],
    },
    // The server bundle runs as a Cloudflare Worker — there is no node_modules
    // at runtime. Vite's default SSR build leaves npm deps as bare external
    // imports (h3, react, @tanstack/*, seroval, …), which resolve on a Node
    // server but throw "No such module" in a Worker. Bundle them all in.
    // (node: builtins stay external — nodejs_compat provides them.)
    // BUILD ONLY: `vite dev` SSR runs in Node where externalized deps are
    // correct — noExternal there makes the dev module runner evaluate CJS
    // deps (react) as ESM and crash with "module is not defined".
    ssr: {
      // BUILD ONLY: the SSR bundle runs on workerd (Cloudflare Workers), not
      // Node. Target a worker runtime and resolve bundled deps through the
      // edge export conditions (workerd/worker/browser) so packages that ship
      // both variants bundle their edge build (react-dom's web-streams server,
      // etc.) instead of the Node variant leaning on nodejs_compat shims.
      // `vite dev` SSR runs in Node, where default node resolution is correct.
      ...(command === "build"
        ? {
            target: "webworker" as const,
            resolve: {
              conditions: [
                "workerd",
                "worker",
                "browser",
                ...defaultServerConditions.filter((c) => c !== "node"),
              ],
            },
          }
        : {}),
      noExternal: command === "build" ? true : undefined,
      // `cloudflare:workers` is a workerd runtime built-in that exposes the Worker
      // env / bindings (D1 `DB`, R2 `STORAGE`). Like node: builtins it must NOT be
      // bundled; the runtime provides it. (`ssr.external` is typed string[].)
      external: ["cloudflare:workers"],
    },
    build: {
      // Keep `cloudflare:*` external in the SSR rollup pass too — `noExternal`
      // above would otherwise try to resolve+bundle it and fail.
      rollupOptions: { external: [/^cloudflare:/] },
    },
    plugins: [
      // Local SVG assets (e.g. the branded generate-button sparkle) import as
      // React components via `?react`. `icon: true` sizes them 1em; fill is
      // forced to currentColor so they color like text. Keep the viewBox so
      // CSS sizing scales the glyph.
      svgr({
        svgrOptions: {
          icon: true,
          svgProps: { fill: "currentColor" },
          svgoConfig: {
            plugins: [{ name: "preset-default", params: { overrides: { removeViewBox: false } } }],
          },
        },
      }),
      // TanStack Start plugin must run before React's plugin.
      //
      // SSR build: `vite build` emits a Workers-shaped server bundle
      // (dist/server/server.js — `export default { fetch }`) plus dist/client
      // (hashed static assets). The platform publishes that as a per-tenant
      // Worker on Workers for Platforms, served at <sub>.higgsfield.app/ (host
      // root, so Vite's default base "/" — no base-path juggling).
      //
      // Rendering happens on the server per request, so site code must be
      // SSR-safe: never touch browser-only globals (window, document,
      // localStorage, navigator) during render or at module top level — only
      // inside effects/handlers, or guarded with `typeof window !== "undefined"`.
      tanstackStart({
        server: { entry: "server" },
      }),
      higgsfieldDesignInspectorVitePlugin(designInspectorEnabled),
      react({
        babel: {
          plugins: designInspectorEnabled ? [higgsfieldDesignSourceBabelPlugin] : [],
        },
      }),
      tailwindcss(),
    ],
  };
});
