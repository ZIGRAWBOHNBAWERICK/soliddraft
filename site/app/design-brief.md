# SolidDraft site design brief (night build, 1 November 2026)

The page is the public face of SolidDraft, the free, open-source CAD app in this repository.
It keeps the night build's style (type, colour, motion, layout, one WebGL stage) and changes
the subject completely: every word, image and clip is about SolidDraft. The company that
started it gets one footer line.

## What the page must say

- What SolidDraft is: a free, open-source parametric CAD app for mechanical parts that runs on
  the desktop and in the browser, on Catallax, an exact geometry kernel written in Rust. MIT.
- What works today, as `README.md` and `docs/needs-catallax.md` say it. General fillets, sweeps,
  lofts, STEP import and mates are not built: they appear only with the "In design" label.
- Headlines state a concrete fact a first-time visitor understands. No puns, no abstract lines.
- One clear button opens the app. Its address is one setting, `site.links.app` in
  `src/content.ts`: the released browser build served under `app/` beside this page on GitHub
  Pages. Downloads go to the GitHub releases page, source to the repository.
- Credit: "Made by Zigrawbohnbawerick Pty Ltd. MIT licence." The site names only the company:
  no personal name. One plain link to the company's other products.
- Donations: a clear Ko-fi button and a GitHub Sponsors button in the about section, at their
  existing addresses. They stay.

## Media rules

- Every part shown is a real SolidDraft export. The sources are in `../parts/` (run scripts and
  saved parts); meshes and renders come from them through `../tools/blender/`.
- Every picture of the app is a real screenshot of the app (`public/assets/app/`), taken with
  `soliddraft-app --screenshot` on the desktop and headless Chromium on the browser build.
- The drawing sheet is the real ISO A3 drawing `soliddraft draw` made of the bracket.
- Higgsfield is for atmosphere and motion around those real parts, never for a part or an
  interface. Each generated file is logged in `../refs/higgsfield-manifest.jsonl`.

## Page order and scroll spans

| Section id | Chapter | Span | Beats |
| --- | --- | --- | --- |
| intro | hero | 170vh | Night terrain, drifting lights, pointer parallax. Title, line and three buttons: open, download, source. |
| kernel | kernel | 300vh | The rounded boss in dark metal, oxide B-rep edges glow (0 to 0.44). Dissolves to particles, re-forms as the lattice block, emerald (0.56 to 1). |
| model | cad | 300vh | The bracket's sketch draws on the floor, extrude rises through a clipping plane, holes drill, edges sweep (0 to 0.5). Part lifts and turns free, ice (0.62 to 1). |
| drawings | draw | 280vh | The real A3 sheet is revealed by a scan line while the part turns to its front view (0 to 0.5). The part drifts off, the sheet tilts and fans into SVG, PDF and DXF (0.6 to 1). Violet. |
| anywhere | screen | 300vh | A monitor lowers and lands at 0.46: camera shake, floor ripple, dust, thud. Its screen is the desktop screenshot; it crossfades to the browser screenshot at 0.62 to 0.74. Amber. |
| statement | (veil) | 110vh | Text only over the dimmed monitor: "Free parametric CAD on an exact kernel." Button to the app. |
| features | light | 260vh | Page goes bone white. Eight real parts drift as a field. The works-today list, with one "In design" row. |
| get | smoke | flow | Slow smoke. Four cards: browser, desktop downloads, source, kernel. Real screenshots and Blender turntables with `preload="none"`. |
| about | smoke | flow | Licence and credit, six links, one footer line. Nothing after it. |

Captions (title in caps, one line under, bottom left) live in `src/content.ts`. Chapters do not
render text.

## Visual rules

- Ground `#07080a`. Ink `#f2f0ea`. Mute `#8b8f96`. Light section: bone `#f2f0ea` ground, ink text.
- One accent per chapter: oxide `#ff5b1f` (kernel), emerald `#35c27c` (field), ice `#5bc8f5`
  (model), violet `#7660d6` (drawings), amber `#f2b134` (desktop and browser). Chapters call
  `ctx.setAccent`.
- The shared dot floor is tinted with the chapter accent. The hero hides it.
- Framing: captions sit bottom left, up to 42rem wide and about a third of the height. Keep the
  subject in the upper two thirds, centred or a little right. Never behind the caption, and never
  white paper behind white text.
- Satoshi 400 caps with wide tracking for titles, JetBrains Mono for labels. No italic.
- Motion never fully stops: idle drift in every scene. Pointer moves the camera a few degrees on
  desktop; touch devices auto orbit.

## Performance rules

- One WebGL canvas. Chapters are hidden groups, not separate renderers.
- Under 150k triangles visible, under 60k particles, textures 2048 or smaller, no per-frame
  allocations, everything disposed in `dispose()`.
- `ctx.quality` 0 halves counts for phones. Clips in the get section use `preload="none"`.
- Test the whole scroll, not only the hero.

## Fallbacks

No WebGL or reduced motion: `html.is-static`, the canvas is gone, captions and sections read in
normal flow on the dark page.
