# Building a chapter

A chapter is one 3D scene of the scroll. Read, in this order: `../../design-brief.md`,
`types.ts` (the contract), `stage.ts` (what calls you and when), `floor.ts`, `assets.ts`,
`chapters/placeholder.ts` (a tiny complete example). The copy and caption timings you must
leave room for are in `../content.ts`.

## Rules

- Own exactly one file, `chapters/<id>.ts`, exporting the factory already named there
  (`createHero`, `createKernel`, ...). Do not edit any other source file. New assets go under
  `public/assets/<id>/`.
- Everything you create lives under `this.group`. Add your own lights to the group. The scene
  already has `scene.environment` (a room PMREM) at 0.6 intensity for metals.
- Set `ctx.camera.position` and `lookAt` every frame in `update()`. The stage adds shake after.
- Call `ctx.setBackground(...)`, `ctx.floor.set(...)` and `ctx.setAccent(...)` in `enter()`.
  Chapters change the floor tint and the accent; the previous chapter leaves them as it likes.
- Fade in over the first 0.06 of progress and out over the last 0.06. Progress arrives already
  eased, and it jumps to your section's value when you become active.
- Idle motion always: a slow rotation, drift or breathing, driven by `t`, so the scene never
  looks paused when the visitor stops scrolling.
- Pointer: `ctx.pointer.x/y` are -1..1 and smoothed. Move the camera a few degrees with it.
  When `ctx.pointer.coarse` (touch) auto orbit instead.
- Quality: `ctx.quality` 0 (phones), 1, 2. Halve particle and segment counts at 0.
- Budgets: under 150k triangles visible, under 60k particles, textures 2048 or smaller. No
  allocations in `update()` (reuse vectors). Dispose geometries, materials and textures in
  `dispose()`.
- Parts: `ctx.assets.gltf("/assets/3d/<name>.glb")` then `fitPart(gltf, size)` from
  `../assets.ts` gives a centred mesh scaled to `size` units on its longest side. Parts:
  bracket, collar, boss, tray, housing, lattice. They were modelled in Blender from the Catallax
  recipes (`../../../tools/blender/parts.py`), units mm, Y up after export.
- Units: 1 unit is about 1 m on the floor. The camera default sits at (0, 1.6, 6). The dot floor
  is at y = 0 with 1 unit spacing. Put your subject at roughly y = 0.6 to 1.6, in the upper
  two thirds of the frame, centred or a little right. The caption occupies the bottom left.
- No text in the 3D scene unless the brief asks for it.

## See it

The dev server is already running on `http://127.0.0.1:4300`. Do not start another one.
Vite reloads your file on save; the lab loads only your chapter's module, so a broken sibling
never blocks you, and a broken file of yours shows a Vite error overlay in the screenshot.

```bash
# one still at progress 0.42, pointer a little right and up
node scripts/shot.mjs "http://127.0.0.1:4300/lab?c=kernel&p=0.42" out/kernel-042.png --pointer 0.3,-0.2
# four frames 400 ms apart, to prove idle motion
node scripts/shot.mjs "http://127.0.0.1:4300/lab?c=kernel&p=0.42" out/kernel-motion.png --frames 4
```

Run it from `app/`. Write screenshots to the scratch folder given in your task. The script
prints the frame rate (software rendering, so 8 to 15 fps is normal here and says nothing about
a real GPU) and any console errors. Look at every screenshot with the Read tool before you
judge it. The lab shows the real captions for your chapter, so you can see the framing.

Mobile check: `--w 390 --h 844`.

## Done means

1. `bun run typecheck` and `bun run lint` pass (run from `app/`). Errors in other chapters'
   files are not yours; say so in the report.
2. Screenshots at progress 0.05, 0.25, 0.5, 0.75 and 0.95 at 1440x900, plus one at 390x844,
   plus one `--frames 4` set showing idle motion, all viewed and judged by you.
3. No console errors in the shot output.
4. `renderer.info.render.triangles` under budget: check with
   `node -e` or read it from the lab by evaluating `window.__lab` in your head; simplest is to
   log `ctx.renderer.info.render.triangles` once from `update()` while developing and remove it.
