# Canonical target parts in the application

`ci/scenarios/canonical-p1.json` through `canonical-p5.json` describe the five public Catallax target parts through SolidDraft's document commands. The dimensions follow Catallax's `parts/P1-bracket.json` through `parts/P5-flanged-housing.json`. The existing bracket scenario is a different part and remains separate.

| Part | Build | Parameter edit |
| --- | --- | --- |
| P1 | 60 x 40 x 5 plate, flush 60 x 5 x 25 flange, radius-8 boss of height 10, radius-4 through bore, four radius-2 bolt holes, eight radius-3 cuts and the slot bridge | Boss radius 8 to 9 |
| P2 | Full-turn collar, radii 6 and 12, height 12, both outer rims chamfered 1 mm | Chamfer 1 to 0.75 |
| P3 | 40 x 30 x 10 block, radius-8 boss of height 12, junction radius 2, four top edges rounded at radius 1 | Junction radius 2 to 2.5 |
| P4 | Centred 40 x 30 x 12 tray, top open, wall 2, intersected with a radius-23 cylinder | Wall 2 to 1.5 |
| P5 | Revolved housing with inner radius 8, outer radius 12, flange radius 22, four diameter-4 through holes with diameter-8 countersinks at 90 degrees, junction radius 2 | Junction radius 2 to 2.5 |

P1 retains all thirteen circular subtractions and the final slot bridge from the canonical recipe. It does not replace the chain with one slot. P4 subtracts the complement of the rounding disk inside a 60 x 60 enclosing rectangle. For a tray A contained in rectangle R, `A - (R - C) = A intersect C`. Both tools span z = -1 to 13. This gives the exact canonical intersection with an editable shell, using the application's existing Cut command.

The `canonical_parts` acceptance test runs the commands, captures persistent face and edge references before the edit, and resolves those same references after the edit, undo, redo and reopening from saved bytes without the previous document. It checks independent volume expectations and requires every displayed face to retain its generating feature. Split P5 blend patches retain their shared name; `p5_blend_reference` separately checks the stored appearance reference across those patches.

Refused edits must preserve the accepted document and its revision. P5 also tries a field lattice over the original radius-2 housing and records the source bounding-box refusal.

Each checkpoint exports STEP and renders the real application in the internal Edit Suite layout. P1 to P4 must also export a closed STL. P5 may return the kernel's named `mesh.precision-or-budget` refusal. That refusal must preserve both an existing destination and the accepted part; the test retains the reason and removes the sentinel file. A successful P5 STL still has to pass every mesh check. STEP must report one solid within its requested tolerance. STL must report a closed, nonempty mesh and a finite chord-height bound. The test keeps the files, reports and PNGs under `ribbon-acceptance/canonical-parts` in the Cargo target directory.

Run on a machine with a Vulkan adapter, including software Vulkan:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=2 \
  cargo test --locked -p soliddraft-app \
  --test canonical_parts --test p5_blend_reference --test release_geometry \
  -- --nocapture --test-threads=1
```

The scenario files also use the existing [application-loop runner](application-loop.md), including fresh-process reopening, independent STEP read-back and kernel replay. Their entries in `ci/scenario-inventory.json` select them for CI.

MCP limits each kernel call to 20 million work units. A CLI export checks its whole request, including opening and rebuilding, against one 20-million-unit allowance. The acceptance test uses the MCP diagnostics for editing and public exports, and the desktop mass worker's existing 200-million-unit allowance for its independent mass oracle. The CLI evidence capture and desktop worker share that mass allowance. The scenario runner separately checks whole CLI export requests, whose limit stays at 20 million units. Cold rebuild and STEP or STL export of the full P1 chain now fit within that allowance. Its CLI probes require successful certified exports at 20M and the named `numbers.operation-budget` refusal with an unchanged destination at a lowered 10M allowance. The reopened STEP is checked independently for validity, face count and volume. The live-application exports remain independently checked at every checkpoint. Passing P4 here does not close its separate one-million-unit kernel scoreboard budget. P5's B-rep result does not establish field support at radius 2. The application's lattice attempt refuses its source bounding box with `measure.unsupported-surface` on this pin, before field compilation. Freeform Boolean coverage is also a separate kernel limit. These limits do not justify exporting a prefix or treating a refused build as a completed part.
