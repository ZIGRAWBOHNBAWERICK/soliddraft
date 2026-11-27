# Part export

SolidDraft exports B-rep parts as STEP AP242, IGES 5.3, STL, 3MF or OBJ. The code is the `soliddraft-exchange` crate. The exact export calls are `Kernel::export_step` and `Kernel::export_iges`, described in [kernel-bridge.md](kernel-bridge.md). Every export returns the file and a report of how far the file may lie from the exact part.

## Formats

| Format | Made from | The report's bound |
|---|---|---|
| STEP AP242 (`.step`, `.stp`) | the exact B-rep, written by Catallax's exporter | Catallax's proven largest distance between written and exact geometry, rounded up |
| IGES 5.3 (`.iges`, `.igs`) | supported exact B-rep definitions through Catallax | proven whole-boundary displacement, including every face and trim |
| STL (`.stl`, binary) | Catallax's qualified print mesh, with its native certificate header | certified model, tessellation and file-rounding bound |
| 3MF (`.3mf`, core specification) | the same qualified closed mesh | the same certified bound |
| OBJ (`.obj`, Wavefront) | the same qualified mesh, retaining face groups and normals | the same certified bound |

STEP writes exact planes, cylinders, cones, spheres, tori, lines and circles. Intersection curves that have no exact STEP form are fitted splines, and the report counts them. A ring fillet between nonparallel cylinders writes one fitted band under a certificate, and the report's `fit_mm` holds its fit to the exact definition. The status line adds that fit to the file's own bound. The certified route refuses a file over the requested tolerance, and a ring with no certificate, such as a patterned copy, writes nothing. The requested tolerance is 0.0000001 mm. If decimal rounding or spline fitting cannot meet it, the CLI refuses the whole export batch before writing any file. The app still writes the file, reports the bound over tolerance with Catallax's warning, and shows the message as an error.

IGES writes Catallax's supported exact planes, cylinders and cones with shared B-rep topology in millimetres. It refuses unsupported definitions, marked models without a whole-definition certificate and a proven bound above 0.0000001 mm before returning any file. There is no fitted-surface or mesh fallback. Coverage includes planar line/circle trims, full cylindrical/conical bands and supported cardinal axial sectors. Spheres, tori, general numeric frames, uncovered cone angles and other periodic trims remain refused. STEP remains available for its broader coverage.

The `iges` JSON report retains the rational similarity scale, coordinate bound, trim bound and composed boundary bound, plus the decimal and `f64` bounds rounded upward for display. It counts the solids, faces, shared edges, vertices and required export-only seams and ring vertices. MCP's `export_iges {"path":"bracket.igs"}` returns the same one-line deviation summary as `export_step`. As with other formats, a CLI batch prepares all exports before replacing any destination, so an IGES refusal leaves every destination unchanged.

Meshes use the export tolerance in the part's `settings.json`, 0.01 mm and 10 degrees by default; see [tessellation.md](tessellation.md#the-four-tolerances). The chord height is proven after rounding to the file's 32-bit coordinates. Catallax certifies the chord height only, so the report lists the angle as a control it could not guarantee.

## Meshes

The kernel's display mesh repeats a position on every face that meets there, so each face keeps its own normals. Catallax's meshes share edges exactly, so `mesh::weld` merges vertices whose 32-bit coordinates are bit for bit equal. That restores the shared mesh without a distance tolerance. A triangle left with a repeated vertex has no area and is dropped.

The mesh report says whether the welded mesh is closed: every edge joins exactly two triangles that run it in opposite directions. 3MF requires a closed mesh, so a 3MF export of an open mesh is refused. STL and OBJ have no such rule; their reports still say whether the mesh is closed.

Binary STL headers never start with `solid` in any case, because some readers then take the file for ASCII STL. 3MF packages carry a fixed timestamp, so the same mesh always gives the same bytes.

OBJ writes the part name in an `o` record and the source face IDs in `g face_<id>` groups, following Catallax's OBJ convention. It writes the existing display mesh through the same kernel boundary as STL and 3MF, without rebuilding a Catallax mesh just to call its writer. Per-face vertices and `vn` records preserve sharp-face normals. Coordinates round-trip to the display mesh's 32-bit values, and triangles retain their outward winding. A comment records the chord height in millimetres. OBJ has no standard unit field, so the header states millimetres. Control characters in the part name become spaces to keep the name on one line. As with STL and 3MF, triangles collapsed to repeated positions are dropped.

## Terminal fields

An active lattice or gyroid exports only as binary STL, through the native field stage. The stage certifies one closed, connected surface at the feature's accuracy and minimum detail settings. Its `catallax-print-stl/1` header records the field route, exact requested accuracy, caps and proven distance bound. The `field_mesh` JSON report names the label, marks the result approximate and includes the exact requested accuracy and retained minimum detail, the composed bound, the topology verdict (closed, free of self-intersection, connected, components), mesh counts, the cell and vertex caps, the deterministic work the export used and the file SHA-256. The viewport still draws a sample of the field. STEP, IGES, OBJ and 3MF are refused while the field is active. Returning before the field restores ordinary B-rep export.

A field's mass stand-in is a separate STEP output, written only when asked for: the Mass stand-in command, the `export_stand_in` tool or `soliddraft export part.dwprt --stand-in mass.step`. It is never the field's boundary and never stands in for a refused field export. The file holds the field's source region as one massless `SHELL_BASED_SURFACE_MODEL` envelope and one or two unit-density `MANIFOLD_SOLID_BREP` proxy blocks, with no density or material metadata. Catallax first proves the field's own moments to 0.01 mm³, 0.001 mm and 0.1 mm⁵; the proxy's exact moments, and then its moments after the file's coordinate rounding, must lie inside those enclosures, or the export is refused (`stand-in.properties-not-matched`, `stand-in.exchange-not-enclosed`) before any byte is written. A field read through a subtracted operand is refused with `field.stand-in-envelope-coverage`. The `stand_in` JSON report is labelled "mass stand-in — not field boundary geometry" and carries the field enclosures, the proxy's exact moments, their exchanged bounds, the inertia residual, the body counts, the file's rounding bound, the deterministic work and the file SHA-256. The proxy's file is the same, byte for byte, as Catallax's public `catallax-stand-in/1` request answers for the field's native recipe. A consumer that sums the mass of the solids alone gets the field's mass properties; the envelope carries none. `soliddraft export --max-work <units>` runs the whole request, opening and rebuilding the part included, on one deterministic allowance, refuses with `numbers.operation-budget` and writes nothing when it runs out, and adds the work used to each `--json` line.

## Print check and independent part check

Desktop mesh export opens Print check before writing a destination. It shows the selected
print route, requested and reached accuracy, and the model, tessellation and file-rounding
contributions. OBJ and 3MF write each binary32 position as its exact decimal value,
so text encoding adds no further coordinate error. The qualification checks closure, vertex connections, outward orientation,
and preservation of components and cavities. Field exports show their native field verdict,
minimum detail and distance bound.

An accuracy or refinement refusal offers a button labelled with a coarser accuracy. Each
choice runs a fresh check on the same representation and leaves saved settings unchanged.
Save export writes only the accepted bytes. Cancel and refusals leave an existing
file alone. Changing or reopening the part invalidates the report. MCP mesh exports accept
an optional `accuracy_mm` for the same explicit choice and return structured `evidence`
beside the summary. They never retry with a coarser accuracy automatically.
The CLI offers the same choice with `soliddraft export part.dwprt --out part.stl
--accuracy-mm 0.04`. It reports the newly certified accuracy, keeps saved settings
unchanged and leaves exact-geometry export tolerances unchanged. A refused mesh
request suggests the flag and writes none of the requested files.

Inspect > Check independently builds every solid-valued recipe prefix in the boundary
model and distance field. Its report counts confirmed solid steps, identifies reference
definitions separately, and lists each representation's refusals. Each prefix receives a
100,000,000-unit work allowance. This is construction coverage. It does not certify sampled
surface agreement, whole-boundary agreement or print accuracy. Reports belong to one open
revision and become stale after edits or undo. Qualified print geometry is cached per immutable body and export controls, separately
from display meshes. Repeated formats reuse that certificate. Changed geometry or
accuracy needs a new qualification. The desktop runs these checks off the interface thread. The browser runs them
synchronously with the same construction and per-prefix work limits.

CLI recipe export builds the result without collecting unused replay mass or display
summaries. It retains step build counters for STEP proof sidecars. Recipe validation
and expected-error input checks still run. Opening the
input, rebuilding it and preparing every output share the same 100,000,000-unit allowance.

## Refusals

A part with a feature that needs a repair, is blocked or failed is refused, so no file shows a body the part does not have. Suppressed and rolled-back features are fine; the export is the body the tree shows. A refused export writes no file. The CLI also refuses a batch containing any over-tolerance STEP report before creating or replacing any destination, including other formats in that batch.

## Where to export

- In the app, File > Export offers STEP AP242, IGES 5.3, STL, 3MF and OBJ. Mesh formats open Print check for review before saving. The status bar shows the saved export's report in one line.
- On the command line, `soliddraft export <part.dwprt | recipe.json> --out bracket.step --out bracket.iges --out bracket.3mf --out bracket.obj` writes each file; `--json` prints each export report. It does not measure the body for acceptance checks.

## Manufacturing information in STEP

A part with manufacturing information writes it into its STEP file as AP242 semantic PMI and reads it back on import. See [manufacturing information](manufacturing-information.md#in-step).

## The OpenCASCADE check

Hosted CI's STEP job exports the bracket and opens its STEP in OpenCASCADE through `ci/check-step.py`. The check passes when OpenCASCADE reads one valid solid with the face count of the report and of the exact body less the report's `removed_faces`, removed counts that are whole numbers from 0 to the exact body's face and edge counts, and a volume within the reported deviation of Catallax's exact volume. A surface moved by at most `d` changes the enclosed volume by at most `area * d` to first order, so the allowance is that product, plus OpenCASCADE's own integration error estimate and the exact volume's measurement width.

OpenCASCADE and the exact-body acceptance measurements are test tools only. The application never links or runs OpenCASCADE. To capture the bracket and its measurement report locally, run `SOLIDDRAFT_STEP_PROOF_DIR=/tmp/bracket-proof cargo test -p soliddraft-exchange --test bracket step_carries_the_proven_deviation_and_the_body_faces -- --exact`, then `python3 ci/check-step.py /tmp/bracket-proof/bracket.step /tmp/bracket-proof/step-report.json`.

The `iges-export` application scenario builds the bracket through MCP, exports `.igs`, saves and reopens the part, and exports `.iges` through the CLI. It requires identical bytes, checks the rational certificate and upward-rounded bounds, then uses `ci/check-iges.py` with the CI OCP 8 binding to transfer the manifold solid directly through the base `IGESToBRep` boundary tool without healing. The reader checks validity, topology counts, millimetre units and exact analytic surface kinds. Its signed volume comparison after the certified common scale is a separate numerical diagnostic, not a volume guarantee derived from the displacement bound. The scenario also checks wrong-extension refusal and an exhausted export request over an existing file.

## STEP import

Default STEP import keeps a source-certified result when Catallax can produce one, including certified source repairs. Otherwise it uses Catallax's default tolerant reader, including numerical bodies written by fillets. Certified is the higher label; bodies admitted through local fits are labelled Tolerant. A checked model of healed geometry does not by itself certify the original source. Local tolerances are not a whole-body deviation bound: a tolerant body's display, measurements and mass are estimates. A refused body keeps the kernel's reason in plain words.

File > Import STEP reads the source and shows its repair review. Accept imports the solid with automatic repairs. A certified source repair above the automatic cap, or with no known nearest feature, asks for an allowed movement of at least the measured gap and the approver's name. Accept stays off until both are given. Decline saves an unbuilt import with the declined answer. On desktop, checking the file and building an accepted import run off the interface thread. Cancel or close either window to stop the check and leave the part unchanged. In the browser, Import STEP uploads the file for the same repair review. Open still opens saved documents.

Default STEP imports keep each body Catallax admits independently and list refused bodies with their reasons in Import report. Admitted bodies become separate import features in one undo step. Each keeps the original bytes, their hash, admission evidence and its source selector, so reordering features cannot swap solids. Part-level limits, multi-body reviews with explicit repair answers, IGES imports and replacements remain all-or-nothing.

Desktop STEP admission has no application work cap. The browser keeps its existing allowance while it runs admission on the page thread. The desktop census test opens and cancels ABC file `00000025`, which exceeds the former 100-million-unit allowance. Catallax can admit solids from this file even if a sibling body is refused. Download [the ABC STEP fixture](https://concept-collection.github.io/abc-step-1000/step/00000025_ad34a3f60c4a4caa99646600_step_005.step.gz) and decompress it before running the test:

```sh
SOLIDDRAFT_CENSUS_STEP=/path/to/00000025.step cargo test -p soliddraft-app --lib \
  census_step_opens_as_solid_and_cancel_stops_its_running_admission -- --ignored --nocapture
```

Changed source bytes for a selected STEP import replace the whole file only when every solid has a verified partner; see [Replace an imported file](#replace-an-imported-file). An unmatched solid, or changed IGES source bytes for a selected solid, refuses the replacement. Import the changed file as new bodies instead.

The part file keeps the original source bytes, their SHA-256, repair decisions and the import record. Tolerant records also retain the reader's controls, body tiers and admission evidence. Reopening checks the digest and verifies the saved record against the source before rebuilding the body. An approved source repair is saved under its question as `accept:<tolerance-mm>:<approver>`. Repairs never overwrite the source file. STEP export reports whether the result is certified or tolerant; tolerant output carries no whole-body certificate.

Further features are available where Catallax supports the imported body. Certified imports support fillet, chamfer, extrude, cut, hole, shell, pattern, mirror and draft. Their faces keep their `Import1 · Face N` names through those features, save and reopen. A tolerant body is not cast to an exact solid to enable an unsupported feature. Lattice and Gyroid are available when Catallax can build a field from the imported geometry. The viewport draws a sample of the field. Export bounds the clipped field at the requested accuracy before writing STL. The `import-features` scenario replays the certified steps over MCP.

Selecting the import shows the record. Each gap has its answer, its approved limit and its vertex movement, and each repaired face has its attained distance from the source. These are separate numbers and are never added up: the limit is what was allowed, and the attained figure is what the repair proved over the whole face. They describe the import only, so later features are not covered. Output error belongs to each export, whose report is measured against the repaired body. No repair goes past a tenth of the nearest feature, even when approved.

MCP clients use `preview_import {"path":"source.step"}` to read the preview, then pass its token to `import_step {"path":"source.step","preview_token":"<token>","decline":["g0"]}` to decline a reviewed source repair. The token binds the source digest and the repair questions shown. A changed file or changed questions require another preview. An empty decline list uses default admission and automatic repairs. A source repair that asks for approval goes in `accept`, as `{"id":"g0","tolerance_mm":"0.02","approver":"Ana"}`. Certified cleanup records keep the source discrepancy, geometric deviation and direct source-to-result bound separately; tolerant admission does not claim those certificates.

The focused rollout check is `CARGO_BUILD_JOBS=3 cargo test -p soliddraft-app --test step_import_rollout -- --test-threads=1`. Set `SOLIDDRAFT_STEP_IMPORT_PROOF_DIR` to keep its source files, saved parts, display screenshots, measured f32 mesh bounds and checked export reports outside the checkout. The test uses the real import dialog and renderer. It also takes an approved seam through the dialog, renderer, save, reopen and export. It does not independently validate the export in another CAD application; the `import-repair` scenario does.

## Import report

File > Import report lists every STEP, IGES and mesh file imported into the part: what each opened as and why. Solid and Solid, repaired identify exact results; Tolerant identifies local fits within local tolerances. A surface body holds open faces with no volume or mass. A refusal is listed with its plain-words reason and no tree row. An unanswered review is not a refusal. Suppressed or rolled-back imports are not evaluated. Bodies from one file retain its repair evidence. A certified repaired face names its solid, so it counts for that body only; gap repairs belong to the file. Show in tree selects an import, and Replace file reviews a new source for it.

The report opens only when asked for. A tolerant or repaired import, or a refusal, produces a notice above the status bar with a link to the report. A certified import without repairs says only that it was imported. A refusal closes the import dialog, since answering it again cannot help. A new import is fitted in the view, however small.

## Replace an imported file

In the Edit Suite frame, select an imported feature and choose Replace file in its
properties or right-click menu. Review the replacement through the same import
dialog. Cancel or a refused replacement leaves the part unchanged. Acceptance
keeps the feature's identity and name, replaces its saved source and repair answers,
and rebuilds later features. Undo restores the previous source and references.

An unchanged source keeps its references. A changed STEP file is matched to the old
one by Catallax, by complete face definitions, never by entity numbers or order. Each
face of the new file with exactly one verified partner keeps that face's name, so every
reference to it follows: sketches on it, features that cut, fillet or chamfer its edges,
appearances, tracked measurements, mates and drawing views and dimensions, in this part
and in other documents. A file whose entity numbers alone changed therefore keeps every
reference. New faces get numbers after every face the import has had, so no new face
takes a number an old reference names.

A face that moved or changed size has no exact partner, and neither has any face whose
boundary touches it. Planes, and cylinders and spheres whose radius the kernel reads,
follow by shape instead. A candidate face of the new file has the same surface type; a
plane faces the same way, or a cylinder's axis lies parallel, within 1 degree; a
cylinder or sphere has the same radius and material side; every neighbour already paired
is still a neighbour; and its centroid lies within a quarter of the old body's diagonal.
A pair is made only when each face is the other's clear nearest among all faces eligible
for it: the only one, or less than half as far as the next. Two holes that could swap
places therefore stay unpaired, and their references are listed for repair. Pairs found
this way constrain the next round, until no more are found. Other surfaces follow only
by an exact match.

A reference to a face with no partner either way is not redirected. The review lists
each one by name, with the feature, appearance or measurement that holds it, before
Accept, and so does `preview_import` under `needs_repair`. After Accept those features
ask for repair, and everything else keeps building.

All imports of the old file move to the new file together, as one undo step. A STEP
file whose bodies select solids of one source file is replaced only when every solid has
a verified partner, which Catallax gives only when all of its faces are unchanged;
otherwise the replacement is refused and nothing changes. The match is made when the replacement is previewed; if the
part changes before Accept, Accept refuses and the replacement must be previewed again.

The match carries no repair answers yet. For an import of a single solid, a file that
cannot be matched, whose old import was answered or whose review answers a repair
question still replaces the source, and the review says why: changed source bytes then
give the import new reference names, so existing face and edge references ask for
repair. For imports that select solids of one file, the replacement is refused and
nothing changes. Exports remain unavailable while a feature needs repair.

MCP clients preview with `feature` naming the import, then call `replace_import`
with the same `feature`, `path`, `format`, preview token and reviewed answers.

## Closed mesh solids

In the Edit Suite frame, File > Import mesh solid accepts binary STL, ASCII STL and
OBJ. Coordinates are millimetres. The mesh must bound one solid, with at most
200,000 triangles. The kernel checks closed edges, consistent outward winding,
nonzero triangle areas and crossings without adding a tolerance or repairing the
mesh. Open, crossing, reversed or unreadable meshes receive plain messages before
the part changes.

Accept saves the source bytes and digest as an import feature and builds an exact
polyhedral solid with coplanar triangles merged into planar faces. The solid has
volume and mass properties and can be saved, reopened and exported. Import scene
continues to load reference meshes into assemblies.

MCP clients use `preview_import` with `format: "mesh"`, then `import_mesh` with
the same path and preview token. Mesh solids take no repair answers.

## IGES import

File > Import IGES reads `.igs` and `.iges` files into the same repair review as
STEP. Accept imports a supported closed solid and saves the unchanged source,
its format, digest, repair decisions and record. Cancel leaves the part unchanged.
The command is hidden while a sketch is open in both internal frame choices.

Reopening uses the saved format even if the source filename has a different
extension. Older import features without a format field keep using STEP.
Catallax refusals appear as plain messages. For example, an unsupported transform
explains that importing it would change the shape. Files that need repair records
the app cannot retain are refused before their certificates can be lost.

MCP clients call `preview_import {"path":"source.igs","format":"iges"}`, then
`import_iges {"path":"source.igs","preview_token":"<token>"}`. The token binds
the source format as well as its bytes and questions. The same `accept` and
`decline` arrays as STEP apply.

The `iges-import` scenario uses unchanged OpenCASCADE fixtures from Catallax's
corpus. It checks the box's 30 mm³ volume, saved repair record and fresh-process
reopen, and a sphere refused for its transform. The `iges_import` app test drives
Cancel and Accept, renders the imported body, reopens it, checks refusal leaves
the part unchanged, and checks the command is hidden while sketching in both
frames. This coverage does not promise all IGES surface and transform types.

## Scene import

File > Import scene opens an OpenUSD file from NVIDIA Omniverse or elsewhere (`.usd`, `.usda`, `.usdc`, `.usdz`), a glTF file (`.glb`, `.gltf`), a URDF robot, an OBJ file or an STL file as an assembly. This covers what [sand3d](https://github.com/Sandflow-ai/sand3d) exports. The import reads the file's tree, names, units, up axis, transforms, materials and joints where the format carries them. Every mesh arrives as a mesh reference body, never as an editable solid. Measure, mass properties, mates and drawing projection refuse a mesh reference body, because it has no exact solid. Turning a mesh into a solid, and USD NURBS or other exact geometry, wait for Catallax. The code is the `soliddraft-scene` crate; the app module is `crates/soliddraft-app/src/scene_import.rs`.

The import asks for the source file, then for the assembly to write. Beside that assembly it makes a folder of the same name ending in `.parts`, holding one `.dwmsh` mesh reference per distinct mesh and one sub-assembly per group in the file. Files of the same names are replaced. The app then opens the assembly, and the status bar says how many bodies and joints came in and in what units.

| Format | Tree | Units and up axis | Materials | Joints |
|---|---|---|---|---|
| OpenUSD | the composed stage under the default prim, with references, payloads, variants and instances resolved | the root layer's `metersPerUnit` and `upAxis`, or USD's fallbacks of centimetres and Y up | UsdPreviewSurface and OmniPBR constants | UsdPhysics revolute, prismatic, fixed, spherical and planar D6 joints |
| glTF | the default scene's nodes; a mesh of several primitives gives a child per primitive | metres, Y up | metallic-roughness factors | sand3d's `s3d_joint` node extras |
| URDF | the links nested by their joints, the root link merged into the robot, each visual a child of its link | metres, Z up | `<material>` colours, the robot's named ones included | every joint, with limits and mimics |
| OBJ | one child per object | a header comment such as sand3d's "metres, Y up" or SolidDraft's "millimetres", else millimetres and Y up | MTL diffuse colours | none |
| STL | one body | a header such as sand3d's "metres, Z up", else millimetres and Z up | none | none |

The import converts every frame, point, normal and joint to millimetres with Z up once, as it reads the file. Y up becomes Z up by turning +Y onto +Z and +Z onto -Y. When a file names no units or names them wrongly, `units` (m, cm, mm or in) and `up` (y or z) override what the file says; the import record then says the choice came from the caller.

Assembly placements are rigid. The import splits each node's frame in the root's into its rigid part, by polar decomposition, and the rest. The rigid part places the component. The rest, a scale, shear or mirror, is baked into the mesh, and a mirrored mesh keeps its outward winding. A tree of rigid frames keeps every frame exactly as the file has it. glTF files that scale metres down to millimetres at the root, as sand3d's do, arrive at true size. One mesh placed several times with the same baked residual is one mesh reference. OpenUSD and OBJ polygons are cut into triangles by ear clipping in each polygon's own plane, so a concave face keeps its notch and every triangle keeps the face's winding. A polygon that crosses itself or has no area has no right cut: it is fanned from its first corner, and the record's notes say so.

A group becomes a sub-assembly named after it, and a body a component named after its node. A node with both a mesh and children, such as a glTF node or URDF link, holds its own mesh as a component of the same name inside its sub-assembly. Scopes and transforms with no mesh beneath them, such as USD's `Looks` and `Joints`, are left out. A USD prim marked invisible arrives as a hidden component, which hides everything below it; an invisible default prim hides every component of the assembly, and USD guide geometry is left out. A material becomes the assembly's colour on the component (see [assemblies.md](assemblies.md#colours)). Texture maps are not read: a textured material keeps its constant factors, and the record marks it as textured.

USD is read at the stage's start time code, which is the default value wherever an attribute has no time samples; an animated stage says so in the record. The table says what the reader does with each UsdGeom attribute it meets. Whatever it cannot bring in faithfully, it leaves out or shows plainly, and says so in the record's notes.

| USD | Import |
|---|---|
| `points`, `faceVertexCounts`, `faceVertexIndices` | read; each polygon cut by ear clipping |
| `orientation` | `leftHanded` turns the winding; authored normals keep their direction |
| `normals`, `primvars:normals` | read, indexed or not, per vertex, face, corner or constant |
| `holeIndices` | hole faces are not drawn; a mesh that is all holes is left out |
| `subdivisionScheme` | `none` and `bilinear` are the polygons; `catmullClark` (USD's fallback) and `loop` show the control polygons, with a note |
| `GeomSubset` materials | not read, with a note; the mesh takes its own material |
| `primvars:displayColor`, `displayOpacity` | a constant colour is the body's colour when no material it can read is bound; other interpolations give a note |
| materials | UsdPreviewSurface and OmniPBR constants; any other surface gives a note |
| `visibility` | `invisible` hides the component and everything below it, the default prim's included |
| `purpose` | `default` and `render` come in; `guide` is left out, `proxy` too with a note |
| `xformOpOrder` and its ops, `!resetXformStack!` | composed by the `openusd` crate |
| `active`, `class`, undefined `over` | left out, as on the stage |
| `instanceable` | read through the prototype; instances share one mesh reference |
| `PointInstancer` | left out with its prototypes, with a note |
| `Cube`, `Sphere`, `Cylinder` | meshed from their size, radius, height and axis |
| `Cone`, `Capsule`, `Plane`, curves, points, NURBS, `TetMesh` | left out, with a note |
| `SkelRoot` | its meshes come in at their rest pose, with a note |
| cameras, lights, `PhysicsScene` | left out |
| `metersPerUnit`, `upAxis` | read from the root layer, else USD's fallbacks of centimetres and Y up |

The other readers follow the same rule.

| Format | Read | Left out, with a note | Refused |
|---|---|---|---|
| glTF | the default scene's nodes and their transforms; triangles, strips and fans; normals; morph targets at the node's weights, else the mesh's; metallic-roughness factors, with alpha only for `BLEND` materials (the constant opacity, not texture alpha); sparse accessors; sand3d's `s3d_joint` extras | `MASK` cut-outs (shown opaque) and `BLEND` texture alpha, naming each mesh; other scenes, points and lines, primitives with no positions, vertex colours, skins (shown at rest), animations, `EXT_mesh_gpu_instancing` instances | a file that requires an extension the reader does not know, such as Draco, meshopt or quantised geometry |
| OBJ | objects and groups, polygons by ear clipping, normals, the MTL file's `Kd`, `d` and diffuse texture flag | lines and points, free-form curves and surfaces, vertex colours, a missing or broken MTL file | - |
| STL | binary and ASCII, every solid of an ASCII file as its own body, a header's units and up axis | colours in a binary file's attribute bytes or a `COLOR=` header, facet normals that disagree with the winding (the winding is kept) | - |
| URDF | links nested by their joints, visual origins and scales, OBJ and STL meshes, box, cylinder and sphere, materials and their colours, joints with limits and mimics | collision geometry, capsules (shown as cylinders), a visual whose mesh is missing or in another format such as COLLADA | a xacro file, which must be expanded first |

Cameras, lights, textures beyond the flag, emissive and occlusion factors, URDF inertia, transmissions and simulator tags are not geometry and are not read.

The root assembly keeps the import record in `import.json`, outside both hashes. It holds the source file's name, format, size and SHA-256, the units and up axis and where each came from (the file, the format's rules, a header comment, a default, a fallback or the caller), the box around every body, and every node's name, path in the file and occurrence path. It also holds the materials, the meshes written and the notes on what was left out. Each joint is recorded in assembly terms: its kind, the occurrence paths of the parent and the child, its origin and rotation in the parent's assembly frame, its axis, and its limits in millimetres or degrees. A USD joint that names no `physics:body0` is anchored to the stage's world frame: its parent is the root, but the root's own placement does not move it. Joints are recorded, not mated: assembly mates need exact faces.

MCP clients call `import_scene {"path":"washer.glb","out":"washer.dwasm"}`, with `units` and `up` when needed. The reply holds the status line, the import record and the opened assembly's summary, whose occurrences carry their names and kind `mesh`.

The `scene-import` application scenario imports the sand3d bracket as `.usda`, `.glb`, `.urdf`, `.obj` and `.stl`, the sand3d washer as `.usda`, `.glb` and `.urdf`, and NVIDIA's SimReady cardboard box as binary `.usd`. It checks each tree, its names, scale and orientation against the source, and the transforms, colours and joints where the file has them. It saves and reopens the washer in a fresh process. The fixtures and their licences are in `ci/fixtures/scene-import`. `cargo test -p soliddraft-scene` runs the same files through the reader alone, and checks that the washer's three formats agree on every part's box and every joint's axis. `cargo test -p soliddraft-app --test scene_import_app` imports the washer from GLB and the box from USD in the rendered window; set `SOLIDDRAFT_SCENE_IMPORT_PROOF_DIR` to keep its frames.

## Scan references and fitted parts

File > Import scan reference reads STL and OBJ coordinates in millimetres. Review
shows the point and triangle counts, open edges, reversed edge pairs and
nonmanifold edges. Accept stores the unchanged source with the part as `Scan1`.
A reference has no inside, mass or volume. Holes, crossing or reversed triangles,
degenerate triangles and isolated OBJ points stay as supplied. The limits are
200,000 triangles and 1,000,000 points.

Open Scan workspace from File or the command palette. Fit scan to part runs in
the background and offers a review before it changes the part. It needs at least
100 points and an empty solid history. The supported family is block stock with
bosses, bores and convex rounds on stock edges. Unsupported regions and surfaces
are listed explicitly, including their sample counts. Free-form regions are reported as unexplained.

The review shows maximum, RMS, 95th percentile and sample count in both
directions. Scan to part measures the supplied samples against the fitted
boundary. Part to scan also exposes gaps in the scan. Switch between their
colour maps in the viewport; the legend is in millimetres. These are sampled
distances, not bounds on the whole scanned surface. The display mesh's chord
bound is shown separately. The viewport draws up to 30,000 evenly spaced map
points from the selected map. Both complete maps stay in the saved part.
Scan-to-part statistics also include triangle-interior probes. The scan map
contains the input fitting points and does not retain those additional probes.

Accept fitted part keeps the scan and its maps and rebuilds the saved native
recipe as a solid. Later solid features and checked exports use that solid.
Reopening replays the accepted definitions without fitting again. Undo removes
the fit and restores the reference. Cancel, a refused fit or a part changed
during fitting leaves the original part unchanged.

The `scan_workspace` application test drives reference review, cancellation,
acceptance, a fitting refusal, both maps, fit acceptance, save/reopen, downstream
editing and STEP export through the real application and renderer.

## STEP precision reports

Every STEP file written by the app or `soliddraft export` has two adjacent reports:
`part.step.precision.txt` for reading and `part.step.precision.json` for tools. The
suffix preserves the full filename, including `.stp`. Browser exports download all three
files. A report write failure prevents publication of the new STEP file. Earlier versions
named the reports `part.step.proof.txt` and `part.step.proof.json`;
`soliddraft_exchange::proof::read_report` still reads those.

The JSON format is `soliddraft-precision/1`; reports in the earlier
`soliddraft-proof/1` format still read. It records the STEP bytes' SHA-256, the
export deviation certificate, and each feature's name, id, model answer kind,
model bound, independent display bound, and work used. Suppressed features and
features below the rollback bar are inactive and contribute no geometry.

Work counts cover the original retained feature build, including its geometry
queries. They exclude rendering and STEP export. Cache reuse, undo and renaming
keep the count with that build. A null count means the backend or input did not
record it. Raw recipe exports list the contributing operation tags and their build work, excluding replay summary queries.

Pending, refused and model-bounded answers cannot export. Finish or cancel a live
feature, sketch or dimension preview first. A measured display bound does not
certify geometry. When the exact model has independent proof, STEP export uses
that model and includes the display evidence separately in the report.

Reports are written before the STEP file, with an atomic write for each desktop
file. A later write failure can leave reports for bytes that were not published;
compare their SHA-256 with the STEP file before relying on them.
