# Verification

Verified on 1 November 2026, branch `fm/sd-site`, static build under `/soliddraft/`.

## Build and code checks

Run from `app/`.

| Check | Command | Result |
| --- | --- | --- |
| Lint | `bun run lint` | Passed. |
| Tests | `bun run test` | Passed, 5 tests, 40 assertions. The landing contract checks the SolidDraft sections, the app, releases, repository, Ko-fi and GitHub Sponsors links, the company credit line, no personal name, the "In design" label and no em dashes. |
| Static build | `bun run build:pages` | Passed. Output in `app/dist/pages`, 7.5 MB with all media. Includes the UI contract (no colour literals in route files) and the TypeScript check. |

The static export has no Pages Functions, database, forms, authentication, analytics, cookies
or third-party requests. Fonts, models, images and clips are all same-origin. The browser app
is added under `app/` by `.github/workflows/pages.yml` from the release zip; the page itself
returns 404 for that path, which the export checks.

## Browser checks

Headless Chromium through Playwright, using the machine's GPU (`--gpu`), against the static
build served under `/soliddraft/` (a folder with `soliddraft -> app/dist/pages` and
`python3 -m http.server`).

| Check | Command | Result |
| --- | --- | --- |
| Whole scroll, desktop 1440 by 900 | `node scripts/walk.mjs http://127.0.0.1:4311/soliddraft/ out --steps 22 --gpu` | 23 stops, zero console errors, JS heap 27 to 45 MB, 34 to 61 fps. Every section, caption, accent and theme switch landed where `src/content.ts` says. |
| Each chapter alone | `node scripts/shot.mjs "http://127.0.0.1:4310/soliddraft/lab?c=<id>&p=<p>&clean=1" out.png --gpu` | kernel (boss, lattice), cad (sketch, extrude through the clipping plane, drills, free part), draw (sheet reveal, fan of exports), screen (monitor landing, desktop then browser screenshot), light (parts field): all render with no errors and keep the subject clear of the caption. |
| Phone 390 by 844 | `shot.mjs ... --w 390 --h 844` | Hero title, line and three buttons fit; the works-today list and cards stack. |
| Reduced motion | Playwright context with `reducedMotion: "reduce"` | `html.is-static`: canvas gone, 13 headings readable in normal flow. |
| Statement clip | `shot.mjs ... --scroll 12300` | The Higgsfield drift plays behind the statement, masked so the text side stays dark. |

Headless frame rates are a floor: the browser renders to an offscreen buffer and reads it back.

## Assets and their origin

- Every part is a real SolidDraft export. `parts/` holds the `soliddraft run` scripts and the
  saved `.dwprt` files; `tools/blender/sd_glb.py` and `sd_gyroid_glb.py` turn the OBJ and STL
  exports into the GLBs in `app/public/assets/3d/` (millimetres, Y up, original coordinates).
- Every picture of the app is a real screenshot: `soliddraft-app --screenshot` on the desktop
  (dark and light, bracket, plate, boss, collar, the drawing workspace) and headless Chromium on
  the live browser build with the bracket opened through its file chooser. Commands in
  `tools/blender/sd_shots.sh` and `tools/web/`.
- The drawing is the ISO A3 sheet `soliddraft draw` made of the bracket, as SVG, PNG and WebP.
- Hero poster, social card, 404 image, card posters and turntable clips are Cycles renders of
  those exports (`tools/blender/sd_stills.py`, `sd_turntable.py`).
- One Higgsfield clip, `assets/world/bracket-drift.mp4`, is an image-to-video drift from the
  Blender bracket render (Kling 3.0 Turbo, 7.5 credits, logged in `refs/higgsfield-manifest.jsonl`).
  The part in it is the real export; nothing in it is an interface.

## Not done

- No deploy. `pages.yml` uploads the payload on every run and deploys only when the repository
  variable `SOLIDDRAFT_PAGES_ENABLED` is `true`; the variable is not set and Pages is not enabled.
- No run in a real desktop browser with a mouse; pointer parallax was driven from code.
- No Lighthouse run. Sound was checked for errors and state, not listened to.
