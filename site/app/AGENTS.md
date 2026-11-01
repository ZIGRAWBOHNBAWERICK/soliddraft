# SolidDraft site (app/)

One page, server-rendered by TanStack Start and exported as a static artifact. The design
contract is `design-brief.md`; read it before changing anything visual. This is the night
build: a dark page, one WebGL canvas, scenes that swap as you scroll. The subject is
SolidDraft, the CAD app in the repository this folder lives in.

## Where things live

| File | What it holds |
| --- | --- |
| `src/content.ts` | All copy and links: hero, captions with their progress windows, sections and scroll spans, the works-today list, the get cards, the about block. The public address and the app's address (`site.publicUrl`, `site.links.app`) live here, as does `asset()`, which puts the deploy base in front of a site-relative path. Change words here. |
| `src/routes/index.tsx` | Page markup only: canvas, top bar, rail, HUD captions, controls, sections. Boots `night/boot` in an effect. No colour literals: `scripts/check-ui.mjs` fails the build on them. |
| `src/routes/lab.tsx` | `/lab?c=<chapter>&p=<progress>` renders one chapter with a slider, for building and screenshots. Not linked from the page. |
| `src/routes/__root.tsx` | Head metadata, fonts, 404 and error pages. |
| `src/site.css` | Tokens, type, every component, the light theme (`html.is-light`), the veil, the static fallback. |
| `src/night/types.ts` | The chapter contract. Read it before touching a scene. |
| `src/night/stage.ts` | Renderer, camera, frame loop, chapter switching, camera shake, background easing, preload. |
| `src/night/floor.ts` | The shared dot floor shader with tint and ripple. |
| `src/night/scroll.ts` | Lenis + GSAP ScrollTrigger. Sections with `data-chapter` drive the stage; captions, rail, light theme and veil follow. |
| `src/night/sound.ts` | Procedural WebAudio ambience and cues. Off until the visitor presses the button. |
| `src/night/boot.ts` | Wires stage, scroll, sound and controls. Adds `html.is-live`, or `html.is-static` without WebGL or with reduced motion. |
| `src/night/chapters/*.ts` | One scene per file: hero, kernel, cad, draw, screen, light, smoke. |

Three.js, GSAP and Lenis are imported only inside effects, so the server bundle stays
browser-free.

## Base path and version

The page deploys under `/soliddraft/` on GitHub Pages. `vite.config.ts` sets that base (override
with `SITE_BASE`), `src/router.tsx` gives the router the same `basepath`, and every asset URL goes
through `asset()` or the chapters' asset loader, which add `import.meta.env.BASE_URL`. The dev
server therefore serves the page at `/soliddraft/` too. The version on the page is
`VITE_SOLIDDRAFT_VERSION`, set by the Pages workflow from the release tag and otherwise read from
the workspace `Cargo.toml`.

## Assets

- `public/assets/3d/`: GLBs of real SolidDraft exports (sources in `../parts/`, converter in
  `../tools/blender/sd_glb.py`). Millimetres, Y up: glTF Y is the part's Z.
- `public/assets/app/`: real screenshots of the desktop app and the browser build.
- `public/assets/drawing/`: the ISO A3 drawing `soliddraft draw` made of the bracket, as SVG, PNG and WebP.
- `public/assets/work/`: card posters and Blender turntable loops (`preload="none"`).
- `public/assets/world/`, `brand/`: hero poster, social card, 404 image, all Blender renders of real parts.
- `public/assets/fonts/`: self-hosted Satoshi and JetBrains Mono. No third-party requests anywhere.

## Checks

```bash
bun run lint
bun run test
bun run build:pages   # includes check:ui and tsc, writes dist/pages
PORT=4273 node scripts/serve-static.mjs
node scripts/shot.mjs "http://127.0.0.1:4273/lab?c=cad&p=0.4" out.png --gpu   # one chapter still
node scripts/walk.mjs http://127.0.0.1:4273/ out/walk --steps 24 --gpu        # whole scroll, fps and errors per step
```

Screenshots with WebGL work in headless Chromium with `--use-angle=swiftshader --enable-unsafe-swiftshader`;
the frame rate there is software rendering and says nothing about a real GPU.

## Scaffold boundaries

`packages/` and the unused `src/components/*`, `src/layouts/*` and `src/landing-content.ts`
are vendored Higgsfield scaffold. Leave them untouched; the build depends on the Quanta
Tailwind entry in `src/styles.css`.
