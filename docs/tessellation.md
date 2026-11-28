# Tessellation quality and the display cache

How finely SolidDraft tessellates bodies for the screen, how that adapts to what is on
screen, and how a million placements share one definition's meshes. The code is the
`soliddraft-tessellation` crate; the kernel calls it uses are in
[kernel-bridge.md](kernel-bridge.md). The policy is automatic by default, with a quality slider saved in each
document and a per-component override in assemblies.

## The automatic policy

`Levels::new(quality, diagonal)` turns a quality setting and a body's bounding diagonal
into the kernel parameters of three levels:

| Level | Chord height | Used for |
|---|---|---|
| Coarse | 4 x standard | far away, lightweight opening |
| Standard | 0.2 percent of the diagonal under Auto | the setting itself |
| Fine | 1/4 x standard | close up |

- The chord height follows the body's size, so a washer and an engine block look equally
  smooth at the size they fill the screen. 0.2 percent is FreeCAD's default.
- Curve and surface angles stay at 15 degrees on every level. At SMLib's own 25,
  triangles beside circular fillets can wind against their face's normal.
- No chord height goes below 0.0001 mm, so a degenerate body never asks the kernel for
  an unbounded mesh.
- The chord height is the deviation the app reports: the mesh is within that distance of
  the exact surface.

Small bodies whose curves the 15 degree angle bounds before the chord does can come out
the same at two levels. The display cache keeps one mesh for both.

## The setting

`Quality` is what a document saves:

| Mode | JSON | Standard chord height |
|---|---|---|
| Auto | `{"mode": "auto"}` | 0.2 percent of the diagonal |
| Slider | `{"mode": "slider", "value": 7}` | Auto times sqrt(2) to the power (5 - value); 1 is 0.8 percent, 10 is about 0.035 percent |
| Custom | `{"mode": "custom", "chord_height": 0.05, "angle_deg": 15, "max_edge_length": 0, "max_aspect_ratio": 0}` | the raw values; levels scale the chord height and edge length |

Slider position 5 gives what Auto gives. `"auto"`, `"7"` and `"chord=0.05,angle=10"`
parse into the three modes for command-line tools.

An assembly can show every part at one quality, and a component with everything under
it at its own (`Settings::overrides`); the nearest override wins. A part shown at
another quality than its own becomes a definition of its own, so it gets its own mesh
while other placements keep theirs. See [assemblies.md](assemblies.md#display-quality).

## Choosing a level per occurrence

`Thresholds::update` moves one occurrence's `Choice` (its level and how long it has
been quiet, eight bytes) on by a frame:

- **Promote at once** when the current level's error on screen (deviation times pixels
  per millimetre at the near side of the occurrence's bounding sphere) covers more than
  1 pixel, to the coarsest level that is within it.
- **Demote one level** after the error has stayed under 0.1 pixel for 2 seconds.
- Levels are a factor of four apart and the thresholds a factor of ten, so a demoted
  occurrence lands at 0.4 pixel or less and does not promote straight back.
- **The ceiling** from the device budget caps the level: an occurrence above it drops
  at once.

`Thresholds::initial` gives a newly shown occurrence the coarsest sharp level, so an
assembly opened from coarse display copies only asks for finer meshes where the camera
needs them.

The assembly viewport makes the same choice on the GPU, for every placement at once, in
its cull pass; see "Rendering and picking" in [assemblies.md](assemblies.md).

## Choosing a level per face

A single part can choose per face. `Levels::faces()` gives its `FaceLevels`: the kernel
plans the part once, at the parameters of the fine level, and nests three levels of
every face in that plan.

| Face level | Chord height | Mesh |
|---|---|---|
| Fine | the plan's chord height | the part's mesh at the planned level |
| Standard | 2 times that | fewer points inside the face |
| Coarse | 10 times that | fewer still |

Every level of every face keeps the plan's edge points. Two neighbouring faces at
different levels therefore meet on the same points, and a finer level of a face keeps
the points of its coarser ones.

The kernel only nests levels where it refines a face in rounds. With the pinned
Catallax that is spline faces, such as the wall of a loft between circles off one axis.
A plane, cylinder, cone, sphere or torus is meshed in one pass and is the same mesh at
all three levels, so a part of such faces changes its points only with the level it is
planned at (`Levels::faces_at`).

- `View` is the camera of a frame and the space it shows: a `Projection` and the clip
  planes of the viewport. `View::perspective` and `View::orthographic` build one.
- A face is in view while its bounding box, as the body's topology gives it, has a
  corner inside every clip plane (`View::shows_box`).
- `Thresholds::initial_faces` and `Thresholds::update_faces` keep one `Choice` per
  face and move each by the rule above, at the scale of the sphere around the face's
  box.
- A face with no part in view is never promoted. It counts as quiet and steps down
  after the wait.

So zooming in on one end of a part moves the faces on screen to finer levels and leaves
the rest where they were. The viewport does not draw per-face levels yet; it still
shows one level for the whole part.

## The device budget

The app owns `DeviceBudget`, not the document. Desktop and browser use one quarter of
the active device's maximum buffer size, capped at 2 GiB, and allow one triangle per
64 budgeted bytes. This is a pressure limit, not a measure of free GPU memory.
Below 80 percent of both limits the ceiling is fine; below both limits it is standard;
at or above either limit it is coarse. The budget changes only which level
is drawn, not its parameters or mesh bytes. The render tier also checks the enabled
compute limit and keeps OpenGL on the basic path.

### Budgeting a part before it is meshed

For per-face levels the budget acts before any mesh exists. `Kernel::face_level_points`
gives the points each face has at each level, counted from the plan with no vertex or
index buffer built. `Usage::of_points` turns a count into the most display data it can
take: 24 bytes of position and normal per point and at most two triangles per point,
64 bytes in all.

`DeviceBudget::grant` takes each face's counts, the level the camera asks for and the
pixels a millimetre covers there, and returns the levels that fit in what is left of
the budget:

- Every face gets what it asks for when the counted total fits.
- Otherwise faces step down, the step that costs the least error on screen per point
  saved first. Faces out of view go first.
- A face never steps to a level that saves it no points, as with a planar face.
- When even the coarsest levels do not fit, the grant says so.

`DeviceBudget::mesh_faces` does it in one kernel call: the kernel plans the part,
`grant` settles the levels on the counts, and the kernel emits the mesh from the same
plan. The mesh has exactly the counted points. When the grant does not fit, the kernel
is told so before it builds anything and no mesh is made.

`DeviceBudget::mesh_part` then tries the part's next coarser level: it plans at fine,
standard and coarse in turn and meshes the first plan that fits. A part over the budget
even at its coarse level is meshed there, so it can still be drawn, and says it is over.
A plan that is counted and turned down costs the kernel its planning work and no
display memory.

A larger adapter is granted finer levels and a smaller one coarser; the bytes of a face
at a level are the same on both.

## The display cache

`DisplayCache<K, M>` holds meshes by definition and parameters. `K` is whatever the
caller keys a body by: an assembly's definition key (document, configuration, revision)
or an open part's geometry hash. `M` is any mesh that can say how many triangles and
bytes it holds.

- Every placement of a definition draws from the same entries.
- `best(definition, levels, wanted)` returns the wanted level, or while that one loads
  the nearest held level, coarser first.
- Levels whose meshes are equal share one mesh and count once.
- `trim()` drops the least recently used standard and fine meshes until usage fits the
  budget. Coarse meshes stay, so every loaded definition can be drawn, and nothing used
  since the last `tick()` is dropped.

## Tessellation off the interface thread

`Tessellator<K>` answers requests on a worker thread, one `tessellate_levels` call per
body for all its levels. The interface polls once a frame and never waits on the kernel.
A new request for a key supersedes the old one: the worker skips it if it has not
started, and its answer is dropped if it has. `cancel(key)` withdraws a request. The
caller keeps the body alive until the answer arrives; a body released early answers
with `UnknownBody`. A WebAssembly build has no threads, so it answers in place and the
answer waits for the next poll.

## The four tolerances

Four tolerances decide how closely SolidDraft follows exact geometry. Each is its own
setting, and changing one never redoes another's work.

| Tolerance | Controls | Lives in | Changing it redoes |
|---|---|---|---|
| Viewport quality | display meshes | the document's `settings.json` | display meshes only |
| Drawing projection | how closely drawing views follow curves | the drawing, which caches its views | that drawing's views |
| Export | meshes written to mesh formats (0.01 mm, 10 degrees by default) | the document's `settings.json` | the next export |
| Modelling | when the kernel treats two points as one | the kernel | the geometry and everything made from it |

`settings.json` (`soliddraft-settings/1`) sits outside both document hashes, so changing
a setting never gives the document a new revision: assemblies and drawings that pin it
see no change, and no cache keyed by geometry goes stale. It also holds the "save display
data" switch. The modelling tolerance is the other way round: it changes geometry, so it
belongs in the part's truth and its hashes. SMLib fixes its own and takes none through
SM_API, so no document carries one yet.

The drawing tolerance (`DrawingTolerance`, auto or a distance in millimetres, sent as the
view request's `tolerance`) belongs to the drawing, because the drawing caches its views
against the revisions they came from. A tolerance kept in the model would change what a
view shows without the drawing knowing. Drawings use auto until the drawing file stores
one.

## Reporting

`report(kernel, body, quality)` tessellates a body at every level in one call and gives
each level's deviation in millimetres, its triangle count and the parameters used. The
quality slider shows these next to the setting.

`soliddraft_model::Document` exposes the part's side: `settings()`, `set_settings(s)`
(checked, not an undo step, no new revision) and `quality_report()`.

## The part view

The app's part document (`soliddraft-app`, `display_quality.rs`) shows one of the part's
three levels:

- A rebuild tessellates the level the camera last asked for at once, so the viewport
  never waits for the worker to show a new shape. The worker then makes the other two.
- A rebuild that leaves the shown body unchanged, including a new datum sketch or an
  edit to an unconsumed sketch, keeps its meshes, names, picks and GPU upload. Only
  changed appearance assignments update shading. The last four shown bodies keep their
  complete display geometry while the part keeps them, so undo and redo restore it.
- Each frame, `Document::update_display(projection, dt)` moves the level choice on with
  the viewport's pixels per millimetre (`ViewportState::projection`) and swaps in a finer
  or coarser mesh when it is ready. It returns true while the worker still owes a level,
  so the app keeps repainting until it arrives.
- `Document::set_viewport_quality(q)` and `Document::set_settings(s)` save the setting in
  the part and ask the worker for every level. The old mesh stays on screen until the
  new one arrives, and nothing rebuilds.
- `Document::display_quality()` says which level is shown and wanted, and each level's
  parameters, for the quality slider.

The Catallax adapter keeps five exact chord tolerances per immutable body in LRU order.
The certified display and viewport levels can coexist. A file save must not evict the
viewport mesh just to ask for its coarse and standard levels.
Each tolerance has its own build lock. Background refinement cannot hold the cache
lock while a query reads an already available mesh at another tolerance. A refused
mesh request is not cached as a permanent failure.

A zero chord control leaves Catallax's certified default unchanged and reuses that
body display. Only an explicitly positive chord bound is halved to reserve room for
conversion to the interface's `f32` positions. Disabled controls must not trigger an
unrequested finer remesh.

## Display data in part files

A part file keeps its coarse and standard levels in `cache/mesh/coarse.bin` and
`cache/mesh/standard.bin` when its settings save display data (the default) and it has a
body (`soliddraft_model::display`). Assemblies, drawing previews and the browser can show
the part from them without rebuilding it.

- Each is a `DisplayMesh` (`soliddraft_tessellation::mesh`, the same format as an
  assembly's `display.bin` copies): faces named by the part's persistent references,
  never kernel ids, and edges sampled along their curves.
- Its header records the kernel and its version, the level, the exact parameters and
  the part's geometry hash. `display::read` returns a mesh only when that geometry hash
  is the file's, so an edited part never shows an old shape. A mesh from another kernel
  or quality is still a true picture of the same geometry; the reader decides.
- Meshes are saved in canonical order (`DisplayMesh::canonical`): faces by name,
  triangles sorted, vertices numbered by first use. SMLib lists faces in an order that
  can change from run to run, and without this a reopened part would save to different
  bytes.
- The section is optional and outside both hashes. Turning "save display data" off
  leaves it out; the part's revision is the same either way.

For assemblies, `FileSource` shows parts from this data with no kernel at all, and
`KernelSource` reads it before rebuilding anything. `KernelSource::with_quality` shows
every part at one quality instead of each part's own, rebuilding them.

## Command line

`soliddraft quality <part>` prints the part's settings and each level's deviation and
triangle count; `--json` prints the same as one JSON object. `--set auto`, `--set 7` or
`--set chord=0.05,angle=10` changes the viewport quality, `--export 0.01,10` the export
accuracy and `--display-data on|off` the switch, and the part is saved in place. The
printed `revision` is the same before and after.
