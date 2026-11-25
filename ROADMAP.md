# Catallax and SolidDraft roadmap

Catallax is the geometry kernel. SolidDraft is the CAD application. This is their shared plan after the v0.5.0 kernel and v0.8.0 app release. Edit the source copy in Catallax and copy it byte for byte to SolidDraft. The [previous roadmap and gap board](https://github.com/BohnBawerick/catallax/blob/main/docs/archive/ROADMAP-before-checkpoint-3.md) are historical records.

## Words we use

| Word | Meaning | Example |
| --- | --- | --- |
| Phase | Work that ends in one joint release after a checkpoint. | Phase 5 |
| Step | A numbered goal within a phase, with a done-when line. Measure it at the end. Independent steps can run in parallel. | 5.1 Profile everything |
| Task | One worker, one branch, one pull request. The worker carrying a task is its lane. | Cache display meshes per face |

A **checkpoint** is the measured report and the owner's review at the end of a phase, before release. A **coverage line** is one of 61 capabilities scored against OpenCASCADE and SMLib. Neither is a unit of work. Use phase, step and task instead of "stage", "item N", "job" or "milestone" for roadmap work. G numbers refer only to the historical gap board.

## Before phase 5

**0.1 Finish release housekeeping.** Land the SolidDraft work for in-context editing, Extrude From on curved faces, parameters and configurations, the D1 drawing shell, the feature panel and the hole wizard. Close work already on main without merging it again. Keep unfinished display work as input to 5.1. Fix stalled Rust toolchain setup in both CI configurations and make the merge path reliable.

Done when: the finished work is on main, the display notes are ready for profiling, both CI fixes and the merge-path fix are in place, and public stats match Catallax v0.5.0.

**0.2 Publish one roadmap and clean both sets of docs.** Audit current docs, label and keep useful history, and fix references. Review both codebases for maintainability separately. The owner picks any follow-up code changes.

Done when: both docs PRs have owner approval, both roadmap copies match, SolidDraft CI rejects a differing copy, and the separate architecture reports are ready for review. Do not start phase 5 implementation before the docs and roadmap approvals.

## Phase 5: speed and manufacturing information

Profile first. Reduce the work before adding cores. GPU display and the manufacturing information tools can proceed alongside independent speed work. Release once, after the checkpoint.

| Step | Work | Done when |
| --- | --- | --- |
| 5.1 Profile everything | Measure the five slow operations, six slow clicks, 13 over-budget scenarios and frozen display corpus. Include spline previews, which use the checked path. | Results cover 1 and 24 threads on a timing system and a low-end system. Both kernels rerun the STEP census under the rules below. The GPU mesher test gives a go or no-go decision, and CI checks display answers on the frozen corpus. |
| 5.2 Skip repeated work | Rebuild from the edited feature, cache per face and feature, and reuse proofs. Use one display path for every face type, including saddles. | Point prediction matches the resulting count, independent dense checks confirm the bound, and before-and-after runs show which work the changes avoid. |
| 5.3 Use more cores | Split import by body and meshing by face. Parallelize export where the work is independent. | The timing report shows the gains and the mesh bytes match at every tested worker count on both systems. |
| 5.4 Move display work to the GPU | Keep the checked mesh on the CPU. After 5.2 provides closed meshes and per-face detail, move section caps, silhouettes, single-part culling and detail selection to the GPU. | The checked CPU mesh still passes the display gates, the GPU handles these viewer operations, and the detail budget scales with the hardware. Use a different browser path if WebGPU cannot meet the need. |
| 5.5 Decide whether to build a GPU mesher | Default to no GPU mesher. Build fast preview only if the 5.1 test supports it. Detail on zoom can come later. | The checkpoint records why we skipped it, or measured fast preview works on the tested parts, including spline bodies. Preview never replaces checked geometry. |
| 5.6 Drawings and PMI | Close the drawing, product and manufacturing information (PMI), and model-based definition gaps below. Start the assistant tools alongside the speed work. | The bracket round trip passes the NIST analyzer in CI. The agreed drawing and PMI cases work, and the built-in assistant can call all six tools while rules still determine every PMI value. |
| 5.7 Proof on demand | Add a low-priority proof button for one part. It does not hold the release. | A request runs a whole-edge proof and reports proof or "could not prove". It stores no proof request and runs no proof service. |

### Drawings and PMI build order

1. Close the C1 semantic PMI round trip for the P1 bracket. Keep all 14 items and their attachments through exchange, save and reopen. Run the NIST analyzer in CI.
2. Add missing kinds: compound holes, blind and thread depth, chamfers, edge breaks, widths, composite position, projected zones, basic and angular dimensions, datum targets, knurl callouts and notes on faces.
3. Check geometric dimensioning and tolerancing (GD&T) validity and tolerance status per face.
4. Show PMI on the model, save 3D views, then read and write graphical PMI. Preserve unrelated file content when editing imported PMI.
5. Recognize features on bare imports and extend the rules-based interview to ask for missing information.
6. Produce inspection characteristics, CSV and an AS9102 first-article workbook.
7. Fill drawing gaps such as offset, aligned, half and break views, larger sheets and title block editing. Native DXF dimensions already work.
8. Add revision comparison and markup, tolerance stack-up, 3D PDF and QIF. Leave low-value gaps out unless the owner adds them.

Give the built-in assistant access to `pmi_recognise`, `pmi_interview`, `pmi_answer`, `pmi_add`, `pmi_list` and `manufacturing_package`. Keep `soliddraft run` for headless use and `soliddraft mcp` for external clients. No separate Python API or PMI CLI is planned. A dedicated PMI assistant is deferred.

### Limits and measurement

- File open, edits and display have no app work limit. Keep cancel for file open; do not add a progress bar. Tests and speed runs flag edits and display over 20 million work units. This is a flag, not a refusal or a time target. Keep measurement in tests only if it noticeably slows the shipped app.
- Give both Catallax and OpenCASCADE the same 3,000 STEP files, no work limit and 10 minutes per file. Record time and work per file; report an unavailable work count as such. Rerun the OpenCASCADE reference under this protocol. Do not compare the new result with the old one-minute OpenCASCADE run or the old Catallax run capped at 20 million work units per file. Track checked solid exports separately from partial or surface-only results.
- Use a three-minute cutoff per speed case, not a fixed target for every Boolean or feature. Record work and instruction counts on every speed PR. At each step end, measure wall time on the timing system and run the measured cases on both systems. Do not run these timings nightly. Compare total time with the prior run. If a step saves less than about 10%, the owner decides whether to continue.

## Phase 6: parity that helps users

Ship the kernel capability and the SolidDraft command in the same step. Some groups only need an app command because the kernel already does the work. The public 61-line score remains a comparison, not a plan to hit 100%. The review found one full line, 16 method-only lines where users already get the result another way, 40 real gaps and four unclear lines.

| Step | Gap group | Done when |
| --- | --- | --- |
| 6.1 STEP import parity | Start from the post-phase-5 census. Address sewing, torus, offset and extrusion surfaces, voids and repair. | The comparable census meets the agreed checked-solid target and reports remaining refusals. |
| 6.2 Freeform cuts, joins and fillets | Handle spline intersections, Booleans and fillets on modeled and imported parts. | The three phase-2 target edit chains build and export as they do in both reference kernels. |
| 6.3 Edge and wall features | General shell, draft, chamfer and fillet. | The agreed imported and modeled parts take these edits, including spline faces and mixed corners. |
| 6.4 Features that meet at an angle | General cone and torus pairs and off-axis countersinks. | The checked operations build on the angled acceptance parts. |
| 6.5 Pipes, springs and threads | Sweep curved profiles along 3D curves, with guide, twist and helix controls in the app. | The sweeps work through SolidDraft and the P8 spring and threaded boss build. |
| 6.6 Measure and check | Curved-pair distance, clash, curvature and inertia for fitted blends. | The agreed cases give checked measurements and no clash pair stays unresolved. |
| 6.7 Direct edits | Bring direct edits on imported solids to the app. | The commands delete, move, offset and replace the intended faces and heal the result where needed. |
| 6.8 Guided lofts | Add guide rails and resolve valid lofts that refuse. | The agreed valid lofts build with and without rails. |
| 6.9 More extrusion options | Extrude up to a curved face and taper any outline. | Both operations build on the agreed analytic and spline examples. |
| 6.10 Surface tools | Project, wrap, offset spline curves, fill N-sided gaps, fit points, imprint and split. | Each tool builds and edits the agreed surface examples. |
| 6.11 More exchange formats | Preserve face colours and support more assembly encodings and spline faces in IGES. | The exchange cases preserve the intended structure, colours and geometry. |
| 6.12 Drawings of any part | Extend the kernel views beyond the current part families. | The agreed analytic and freeform parts produce checked drawings. |
| 6.13 Rare tools | Variable fillets on curved edges, curvature-continuous blends and mesh Booleans. | Each works on the agreed cases through the app. |
| 6.14 Selected follow-ups | Add only the deferred feature ideas the owner selects at checkpoint 5. | Each selected feature meets the acceptance line set at checkpoint 5. |

Catallax has ported the spline intersection code, about 6 to 9% of SMLib. No known scoreboard failure comes from missing SMLib code: the six parts SMLib builds and Catallax refuses fail in Catallax's own code, and OpenCASCADE builds all six. Port four pieces into Rust where needed: least-squares point fitting in 6.1 and 6.10, fillet corners and the fillet executive in 6.3, spline offset and shell in 6.3, and fixes for the remaining tracer refusals in 6.2. Skip the rest unless a step proves a need.

## Phase 7: assemblies, scene parts and robotics

Build for machinists and suppliers, hobbyists and 3D printing, robotics and simulation, and scripted or CI workflows. Finish assembly depth before robot export, then add the simulation link.

| Step | Work | Done when |
| --- | --- | --- |
| 7.1 STEP assemblies | Import and export whole assemblies. | The assembly round trip preserves structure. |
| 7.2 Assembly mass | Compute assembly mass properties. | The result combines part properties in the correct frame. |
| 7.3 Subassemblies and mates | Support rigid and flexible subassemblies and more than 128 parts. | Both subassembly modes and mates work on the public 200-part test assembly. |
| 7.4 Configurations | Apply configurations in every assembly tool. | Each tool uses the selected configuration. |
| 7.5 Part operations | Replace, mirror and pattern parts. | The operations work through the app and its assistant. |
| 7.6 Bill of materials | Include subassembly levels, part numbers and CSV export. | The displayed and exported bills agree on the agreed assembly. |
| 7.7 Assembly cuts and holes | Cut across parts and add a hole series. | The three-plate stack shows the right result in each configuration and drawing. Parts shared by other assemblies keep their existing geometry by default. |
| 7.8 In-context references | Let part sketches reference other parts and assembly geometry. | References link by default, the app names mates that may break them, and the user can delink in one click. |
| 7.9 Assembly views | Show edges in assemblies and compute exact hidden lines in drawings. | The viewer and drawings show the checked edges on the test assemblies. |
| 7.10 Scene parts | Convert closed meshes to solids, fit editable faces, then add materials, textures and compressed glTF. Also read USD BrepArray solids despite their absence in a survey of 981 public USD files. | Scene solids support mechanical queries and edits, USD follows references and payloads and respects subdivision defaults, and import names everything it skips. |
| 7.11 Robot export | Export one model to URDF, MJCF, SDF and the USD SimReady profile. | The robot arm and gripper export with separate collision shapes per link and mass in the right frame. Checks catch startup overlap, bad inertia and unit errors. Closed loops use SDF or MJCF joints. |
| 7.12 Simulation round trip | Store simulator values on links and joints with stable IDs, and add a live link. | Values survive re-export and read back from edited URDF or MJCF. The live link uses those same IDs. |
| 7.13 Assembly speed, if needed | Use the phase 5 measurements to decide whether assemblies need their own speed step. | Either the measured assemblies do not need it, or a dedicated run shows the gain and preserves the checked answers. |

For 7.7, ask where the result goes when the user creates the feature. A plain cut defaults to this assembly configuration. A hole series defaults to real holes in each part, with an override. When another assembly uses a part, default to a new part configuration, name the other uses, and offer to write anyway or save a copy. Use ISO 273 clearance classes and 6H tapped tolerances; let the user adjust them. Do not insert bolts. Kernel cuts need stable new face names, and wider references need stable edge names.

An assembly interference command is out of scope unless the kernel clash work makes it cheap to add. Kernel clash and robot export overlap checks still matter.

## Phase 8: Windows and FEA

| Step | Work | Done when |
| --- | --- | --- |
| 8.1 Windows | Package SolidDraft for Windows and sign the downloads. | The signed packages install and run the app on Windows. |
| 8.2 Finite element analysis (FEA) | On hold until the owner gives the go-ahead. | If approved, the checkpoint demonstrates and measures the agreed FEA result on the same part the kernel exports. Set the acceptance details before starting. |

## Working rules

- Keep PRs large enough to deliver useful work and use the fewest tests that prove it. Speed work is in scope. Show before-and-after work units and instruction counts for the cases a speed PR touches.
- Do not change exact answers, loosen a limit or tolerance to pass a case, or produce different bytes at another thread count. CI fails a changed exact answer, a new refusal of a case that built before, or a thread-count difference. It flags a work or instruction increase over 10% on an untouched case for review rather than failing it.
- A speed PR may change display meshes. They must stay closed, keep the proved distance bound, share identical edge points between adjoining faces and produce the same bytes at every thread count. List the test parts whose meshes changed.
- Ship each kernel feature in SolidDraft on desktop and in the browser within the same step. Either may come first. A step owns refusals of possible geometry in its scope. There is no separate refusal queue.
- Use census and checkpoint runs to find refusals. Do not publish nightly generated-recipe counts. Port SMLib code only when a step needs it, test it in Catallax and ship it through SolidDraft.

## Open decisions

- Set the STEP parity target after the 5.1 census and before phase 6. The post-phase-5 run separates gains from removing the work limit from the remaining import gaps.
- Decide the four unclear coverage lines at the start of phase 6: bend and twist (C12), tangent contact (I4), curve intersections (I5) and non-manifold bodies (S7).
- At checkpoint 5, the owner picks which deferred feature ideas A through K belong in phase 6.
- Decide where the parts library lives when its files are available. FEA remains on hold until the owner approves it.
