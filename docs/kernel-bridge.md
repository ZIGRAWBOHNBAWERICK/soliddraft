# The Catallax application entry

SolidDraft runs only on Catallax. `soliddraft-kernel-catallax` implements `soliddraft-kernel-api::Kernel` through Catallax's `catallax-app` entry. The application, CLI and tests use that implementation. Desktop and browser builds compile it in process with Rust. There is no C++ bridge, separate WebAssembly kernel or server fallback. Desktop edit construction and exact proof run on separate application workers; the browser currently runs both on its interface thread. The browser's [kernel worker](web.md) runs Mass props on a second instance of the module.

The adapter pins its Catallax revision in `Cargo.toml` and `CATALLAX_REVISION`. No Catallax source is copied here. See [the remaining gaps](needs-catallax.md) and [the roadmap](../ROADMAP.md).

## Operations and body ownership

Profiles are deferred definitions rather than sheet geometry. Recipe replay reports their loop count, closure and tags without asking for sheet topology, mass or validity. Extrusion and revolution consume those definitions to build solids. Each solid owns an immutable Catallax application build. Bodies have arena handles; operations leave their inputs unchanged and callers release handles they no longer need.

Topology reuses certified face areas across revisions when the retained checkpoint still answers the proof's dependencies and the directed trim is unchanged. Straight rational planar faces can also reuse an area across independent rebuilds. Limited requests still run the original proof; display-only fallback values are never reused as certificates.

Extrudes, cuts, booleans and rational-frame revolves use Catallax recipe steps. Holes, patterns, mirrors, fillets and chamfers between planes and cylinders, and extrusions up to the next boundary or a planar face use Catallax operations, as do drafts of planar faces and pull-parallel cylinders. `soliddraft-recipe/1` cannot spell these, so the adapter stores them as native Catallax steps and names their faces and edges by Catallax origins. A refused construction remains an error with Catallax's stable refusal code. Shell accepts convex planar trays with separated through bores. Closed oblique plane/cylinder fillets support measurement and STEP export for whole bands. Unfinished ribbon commands are disabled and show Coming soon. Existing saved features rebuild with an error and pass the preceding body through unchanged.

Mass props requests run off the interface thread with a 200 million work allowance: on a desktop worker thread, and in the browser's kernel worker, which rebuilds the part from its saved bytes first. An explicit mass-request allowance governs the query instead of the diagnostic default. Measurement keeps `Accuracy::default()`. At Catallax `f335a6c`, the radius-5 rim cut by `z = 8 - 0.75x` gave these local dev-profile measurements on a shared machine:

| Fillet radius, mm | Work units | Query time, seconds |
|---|---:|---:|
| 1 | 96,504,859 | 109.734 |
| 0.5 | 97,333,170 | 103.307 |

The rendered R1 test used the same 96,504,859 units in 214.918 seconds while processing a camera command before the result arrived. The 200 million allowance leaves roughly twice the measured peak. Worker refusals reach the status bar and property panel; an obsolete revision cannot publish them.

## Display and drawings

Catallax supplies shared-edge display meshes with chord bounds, face handles and edge polylines. The adapter reserves part of the chord bound for conversion to float display positions and refuses a request that cannot survive that conversion. It forwards the surface angle, edge length and aspect ratio as Catallax controls. A maximum edge length is enforced on the returned float triangle edges and edge polylines, or the request is refused. Catallax bounds facet normals rather than the turn along an edge, so a requested curve angle, and any control Catallax could not certify on a face, appears in `Mesh::unavailable_controls`.

The screen does not use those certified meshes. A proved body's viewport and feature-editor preview call `Kernel::preview`, `preview_levels` and `preview_edges`, which the adapter answers with Catallax's preview mesh. It samples toward the same chord and angle targets, keeps face and edge handles, claims no bound and is marked `Mesh::unproved`. Drawing previews can request it with `ViewSpec::preview`. The drawing editor creates and rebuilds saved line work with Catallax drawing queries. Display normals are averaged from the facets in `f64`.

Mesh exports and the mesh quality report use `tessellate`. A backend without a preview answers the preview calls with `tessellate`. See the [drawing document guide](documents.md#pinned-revisions-and-rebuild) for export eligibility and redraw guidance.

A drawing context requires the current proved model body with its matching definition hash. Commands that use model geometry on an existing drawing also check the part against the drawing's model reference. During a dimension drag, pending proof or a failed edit, automatic following and Rebuild keep the last views and explain why they wait.

Topology contains exact definitions and incidence with Catallax's proven measures: face areas, centroids and boxes, edge lengths and half-arc-length points, and the body's box. Mass properties include the proven centroidal inertia.

A preview section (`ViewSpec::preview`) cuts the preview mesh with `drawing::section::by_mesh_slice`. The saved section path subtracts a half-space with `by_half_space` and reads the cut body's view and face loops. Only a section view's own inclined cut can retry on the preview mesh when the saved section query fails. A successful retry logs the original error. See the [drawing query mapping](needs-catallax.md) and [export rules](documents.md#pinned-revisions-and-rebuild).

Saved curve accuracy distinguishes `exact` geometry, `polygon` paths such as sampled splines, and `approximate` geometry or placement. Export counts use the written paths. A cubic sheet path counts as approximate even if its source curve is exact.

On the full GPU tier, the part viewport clips the current proved display mesh and fills the section plane using stencil while the offset is dragged. Releasing the offset submits the same temporary half-space subtraction that WebGL2 uses throughout a drag. The live cap stays visible until the matching cut is ready. If the kernel refuses, the viewport keeps the cap and marks it display-only. It provides no pick IDs or measurement points. The worker accepts results only for the requested document, bodies, display version and plane. Section geometry never enters the model or exports.

## Request work

The Catallax bridge selects the allowance at the numerical request boundary in `request.rs`. Cold STEP admission, exact builds and checked displays keep their existing defaults. Diagnostic runs can supply a measured allowance and mark calls over 20 million units. Explicit bounded requests supply their own. Drawing section views have no application work limit of their own. Nested calls use the active allowance and report only their own charge. An exhausted STEP admission is not cached. Exports keep their separate ceiling and publish no incomplete output.

## Construction before exact proof

Scalar edits have a separate entry, `Kernel::construction_edit(body, edits, params)`. `ScalarEdit` identifies an existing operation by tag and changes only its extrusion distance, fillet radius or equal-distance chamfer distance. The adapter derives the requested recipe from the retained build and asks Catallax's checked field plan for a construction mesh without evaluating the new exact boundary first. Unsupported or unresolved construction remains a typed display refusal; it is not an approximate solid accepted as valid.

`ConstructionEdit` contains two distinct results: `display`, a `ConstructionMesh` or its refusal, and a single-use `ProofWork`. Construction faces have display-local ids and semantic origins, not reusable exact handles. The mesh is unproved and has no exportable body or certificate. On the desktop the construction worker can publish it while the exact worker is still occupied by an earlier request. The desktop starts each request's exact proof with `reserve_proof_work` as soon as the edit is applied, beside its construction display, so a slow construction never delays the proof; the construction request's own `ProofWork` is dropped unused. Each worker answer wakes the interface, which draws no frames while it only waits for one.

Construction reserves 2 million work units. Without a caller allowance, exact proof gives each kernel build the same per-build allowance as the synchronous rebuild. Inside a caller allowance, the bridge reserves 100 million units up front, split into 2 million for construction and 98 million for proof. The proof worker consumes its single-use share without charging the caller twice. If the edit has no construction display, `reserve_proof_work` prepares proof alone. `ProofWork::run` wraps the ordinary rebuild; successful proof is adopted without evaluating it again.

The application keeps the requested document revision, displayed revision and last proved revision separate. Each scheduled edit also has a request token. Only a matching revision and request may publish success or failure. A committed request temporarily hidden by a transient drag retains its own receipt and allowance so cancellation can resume it. Undo and redo use the model's deferred command history, not another history stack.

Picking is bound to the displayed document, mesh version and body. A construction face can be selected by its origins, but exact references, edges, vertices and measurements wait for proof. Only an unambiguous origin match in that same request restores a selected exact face. Failure restores the whole last proved part, keeps the failed definition and reason in the red feature row and controls, and refuses part export. Pending definitions can be saved; their construction mesh or proof receipt is never serialized as a proved body.

The model summary exposes derived `edit_state`: `document`, `requested_revision`, `displayed_revision`, `proved_revision`, the corresponding `requested_edit`, `displayed_edit` and `proved_edit` request tokens, `mesh_version`, `display_ready`, `transient`, `pending` and `failed`. A transient drag reports its outstanding request even before a construction mesh arrives. Request tokens are scoped scheduling identities, not saved document revisions. `display_ready` means a CPU display is available; neither it nor `mesh_version` certifies that a GPU frame has been presented. `current_proved_model()` requires matching requested/proved identities with no pending, failed or transient input. `current_proved_display()` additionally requires that current revision's adopted display. Diagnostic summaries omit stale cached body values during a transient edit; they do not run a measurement to populate the state.

## Export

`Kernel::export_step` writes a solid as STEP AP242 through Catallax's exporter, `catallax-step`, from the body's exact build. Its report gives Catallax's proven bound on the distance between the written and the exact geometry, rounded up to a decimal and to the next `f64`, with the counts of solids, faces, edges and vertices written and of fitted intersection curves. A bound over the requested tolerance is reported with Catallax's warning, not hidden. STL and 3MF come from `tessellate` meshes. See [part export](exchange.md).

### Manufacturing information

`Kernel::export_step_with_pmi` writes the same solid with its manufacturing information as AP242 semantic PMI, through `catallax_step::write_step_with_pmi`. `Kernel::pmi_nominals` measures every dimension exactly on the body, and `Kernel::read_step_pmi` reads a file's information through `catallax_import::read_step_pmi`, keyed by the face names its import reports. The adapter converts values to exact rationals and back, and turns a slot, one item in the app, into the three dimensions a file holds and back. A body with approximate faces is not supported yet: the kernel answers `Unsupported` and the app writes the file without the items and names each as left out. See [manufacturing information](manufacturing-information.md).

## Interface work that follows removal

The SMLib removal kept the existing operation and recipe interfaces. Construction previews now add the scoped edit entries above. Several parts still reflect SMLib's design and belong to the next roadmap step:

- `f64` geometry with a tolerance;
- history represented as tag propagation;
- `FaceRole::Other`;
- "modified" as a geometric-change guess;
- `validate()` as a guard against false successes.

The recipe format also stays unchanged. Faces carry Catallax's origins as `FaceInfo::origins`. The adapter maps each origin's recipe step back to the document's operation tag and leaves out roles the interface cannot express. Saved references resolve by those origins, not by history (see [modelling.md](modelling.md)). Legacy history records remain for drawings, the recipe runner and edge labels.

## Checks

Catallax acceptance tests exercise repeated recipe replay, input preservation, display labels, selection and edits. Waiting-feature tests check Coming soon messages and unchanged bodies after failure and reopening. Catallax view, SVG and DXF goldens remain in `recipes/golden/`; SMLib-only goldens have been removed. See [continuous integration](ci.md) for when Hosted CI builds the browser module, drives the interactive Catallax bracket and runs platform checks.

`Kernel::export_iges` writes supported exact surfaces as IGES 5.3 through `catallax-iges` on the caller's existing work allowance. Its separate `IgesReport` retains the whole-boundary rational similarity certificate and upward-rounded displacement bounds. Unlike STEP's over-tolerance warning, IGES refuses bounds above tolerance and unsupported definitions before returning any bytes. See [part export](exchange.md).
