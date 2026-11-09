# Sketch feature inventory

This is a planning inventory, not a parity claim. It compares public SOLIDWORKS Help with SolidDraft at `ea9d2b7b649553f29190d577df53a4537fe38c6b`. Descriptions are original summaries. No vendor prose, icons, screenshots or private material are included.

## Implementation progress

The tables below retain their original source-audit baseline. Later work has added:

- Orders 1-2: the full supported Relations menu and Show all relations, with glyph painting and picking disabled together. Coincidence now has a glyph. See [the ribbon](../crates/soliddraft-app/src/ribbon.rs) and [sketcher](../crates/soliddraft-app/src/sketcher.rs).
- Order 3: relation selection highlights owner geometry and referenced points without selecting them for deletion. The existing-relations list now covers selected points and remains available while inspecting a relation. Conflict and definition colours remain visible. See [the sketcher](../crates/soliddraft-app/src/sketcher.rs), [properties](../crates/soliddraft-app/src/properties.rs) and [pointer tests](../crates/soliddraft-app/tests/sketch_relation_inspection.rs).
- Orders 4-6: circle trim-to-closest, continuous power trim between pointer samples, natural-path extension of a grabbed line or arc, a reversible per-cut trail, cancellation, and one document undo step per drag. See [the shared trim operations](../crates/soliddraft-sketch/src/tools.rs) and [pointer acceptance tests](../crates/soliddraft-app/tests/sketch_trim.rs).
- Order 7: Corner mode in Trim joins two picked lines or circular arcs, with a shared preview/headless operation, persistent coincidence, atomic refusal, and one undo step. Existing shared endpoints keep their exact coordinates. See [tool logic](../crates/soliddraft-sketch/src/tools.rs), [pointer tests](../crates/soliddraft-app/tests/sketch_corner.rs) and [kernel profile tests](../crates/soliddraft-model/tests/sketch_corner.rs).

- Orders 8-9: Inside/Outside modes use two selected bounding curves and ignore unrelated crossings. Shared analytic previews and headless operations trim open lines/arcs with exactly one crossing per bound. Repeated targets, mode changes, reset, cancellation, undo/redo and reopen have [pointer coverage](../crates/soliddraft-app/tests/sketch_trim_bounds.rs). [Kernel profile tests](../crates/soliddraft-model/tests/sketch_trim_bounds.rs) cover lines and wrapping arcs. Targets with zero/multiple crossings, tangencies and closed circles are explicit refusals. Nonintersecting-target region classification from the vendor workflow remains unsupported.

- Orders 10-11: session-local keep-as-construction and ignore-construction-target options apply to closest/power, corner and inside/outside trimming. Guide curves still bound regular targets. Whole conversions preserve identity and relations; partial edits retain the existing stale-relation policy. [Shared operations](../crates/soliddraft-sketch/src/tools.rs) support strict optional headless settings with unchanged defaults. [Pointer/raster tests](../crates/soliddraft-app/tests/sketch_trim.rs) cover dashed previews, protected guides, retracing, live Undo, cancellation and reopen. [Kernel profile tests](../crates/soliddraft-model/tests/sketch_trim_bounds.rs) verify retained guides stay out of extrusion profiles.

- Orders 12 and 49: Existing relations now offers Selected, All, Conflicting and Dangling scopes with no geometry selection required. Inspect conflicts opens the solver-reported set. Missing entity and point references can be found and removed after reopen. Row selection retains owner highlights and safe deletion. [Native inspection tests](../crates/soliddraft-app/tests/sketch_relation_inspection.rs) cover filters, compact empty states, hidden glyphs, popup Escape, undo/redo and persistence. External-reference filtering and automatic conflict repair remain unsupported.

- Orders 13-14: existing rendered definition/status coverage remains a retention check, not another implementation.
- Order 15: read-only Sketch diagnostics locates open ends using the profile join tolerance, analytic overlaps, endpoint/interior branches and invalid curves. The [shared report](../crates/soliddraft-sketch/src/profile/diagnostics.rs) also identifies missing and wrong-kind relation references without rewriting them. [Native tests](../crates/soliddraft-app/tests/sketch_diagnostics.rs) drive navigation, markers, camera locate, stale refresh, safe explicit relation deletion, cancellation and reopen. [Model tests](../crates/soliddraft-sketch/tests/diagnostics.rs) cover topology, curve families, reference kinds and nonmutation. Configurable vendor-style gap thresholds, automatic merging/deletion and kernel validation remain outside this group.

- Order 16: the existing [diagnostics inspector](../crates/soliddraft-app/src/sketch_diagnostics.rs) now offers Boss extrude and Cut extrude preflight. It reuses closed-profile conversion and checks that Cut has a current part body, without creating a feature. [Native tests](../crates/soliddraft-app/tests/sketch_diagnostics.rs) cover target switching, warnings on unused curves, refresh after undo/redo, session reset, compact feedback and a real Cut editor/Catallax handoff. This is not full feature validation. Depth, direction, end conditions, intersection and kernel checks remain in preview; other feature types and vendor thin/open-profile cases remain unsupported.

- Order 17: [reference dimensions](../crates/soliddraft-sketch/src/dimension.rs) measure current distance, projected separation, point-to-line distance, angle, radius or diameter without a solver equation. Smart dimension can create them directly; selected properties switch mode atomically and show values read-only. [Native tests](../crates/soliddraft-app/tests/sketch_driven.rs) cover creation, driving edits, label dragging, compact properties, conversion, deletion, undo/redo and reopen. [Model tests](../crates/soliddraft-model/tests/sketch_reference.rs) protect expression bindings and stable D numbers, and prove profile changes through Catallax. Reference values as expression sources, automatic overdefinition conversion and text overrides remain unsupported.

- Follow-up to orders 15–16: [profile nesting](../crates/soliddraft-sketch/src/profile.rs) now tests analytic curves rather than display polygons. Near-boundary circular holes retain their identity under rotation, including inside arc-bounded regions. [Classification tests](../crates/soliddraft-sketch/tests/profile_nesting.rs) cover winding, joins, extrema, concave regions and nested islands. [Catallax tests](../crates/soliddraft-model/tests/sketch_nesting.rs) prove Boss/Cut volumes, an inner side face, undo/redo and reopen. Native feature checks retain one region and one hole without document edits. This does not add intersecting/touching-loop support or change display sampling.

- Orders 18–25: [native dimension acceptance](../crates/soliddraft-app/tests/sketch_driven.rs) retains single-line and two-point aligned/projected placement, both point orders, all-quadrant angular defaults, circle diameter and arc radius without changing geometry on placement. Existing perpendicular-distance, live-value, value-edit and label-drag controls remain. [The Values-tab workflow](../crates/soliddraft-app/tests/ux_values_tab.rs) checks globals, formulas, configurations and reopen. A reproduced precision-loss bug is fixed: accepting unchanged compact value text no longer rounds its stored target, changes geometry or adds history. All three sketch editor entry points share the same opening state; actual edits retain the existing validation.

- Order 26: [Link values](../crates/soliddraft-app/src/sketch_values.rs) is an actionable property-panel/palette command for driving dimension labels. It creates or joins named globals, edits literal shared values from a member and unlinks while retaining current numbers. The inline existing-name list supports keyboard selection and list-only Escape. Existing formulas are not silently replaced, and each operation writes the active configuration as one undo step. Named rows receive local overrides; Default link, edit and unlink changes follow existing sparse inheritance. [Document tests](../crates/soliddraft-app/tests/sketch_link_values.rs) cover units, cycles, formulas, atomic refusal, named-row overrides, Default inheritance and real Catallax volumes. [Native tests](../crates/soliddraft-app/tests/sketch_driven.rs) cover creation, joining, member edits, cancellation, unlink, undo/redo and saved-file reopen. A reproduced Values-tab negative-diameter fault is fixed at [parameter writes](../crates/soliddraft-model/src/params.rs); [model controls](../crates/soliddraft-model/tests/sketch_values.rs) protect every supported length family while preserving signed angles. Cross-part linking, automatic deletion of unused globals and numeric replacement of shared formulas in the link form remain unsupported.

- Orders 32-34: [shared analytic inference](../crates/soliddraft-sketch/src/inference.rs) offers bounded line/arc/circle intersections, and parallel, perpendicular and circular tangency cues while drawing lines. New point placements retain two point-on relations for an intersection; line endpoints retain the shown directional relation. The viewport's Quick snap control restricts one placement, including an explicit no-snap choice; it resets on placement, Escape or tool change. Explicit choices also work with Auto relations off. [Unit tests](../crates/soliddraft-sketch/tests/inference.rs) cover bounds, tangent contacts, filters and atomic refusal. [Native tests](../crates/soliddraft-app/tests/sketch_snaps.rs) cover preview, placement, cancellation, undo/redo and reopen. [The application scenario](../ci/scenarios/sketch-snaps.json) builds an intersection-snapped profile through Catallax and reopens it. Ellipse/spline tangency and persistent per-class preferences remain outside this group.
- Ellipse profile batch: full and counterclockwise partial ellipses have centre and perpendicular semi-axis handles, pointer previews, cancellation, history and persistence. Smart dimension measures either semi-axis; point coincidence, point-on-ellipse, centre coincidence and equal-axis relations use the solver. [Solver tests](../crates/soliddraft-sketch/tests/ellipse.rs), [pointer tests](../crates/soliddraft-app/tests/sketch_ellipse.rs) and [model volume tests](../crates/soliddraft-model/tests/sketch_ellipse.rs) cover the new entities. `sketch-ellipse.json` and `sketch-partial-ellipse.json` build the profiles through the application loop. Normal Boss/Cut extrusion uses exact Catallax ellipses. One profile may combine lines with ellipses having parallel first axes and equal aspect ratios. Circular mixtures, other ellipse frames, oblique extrusion, revolve, sweep, first-hit extrusion limits and field operations refuse explicitly. Ellipse tangent relations, trim, extend and offset remain unsupported; mirror and independent patterns preserve ellipse geometry. Diagnostic overlap/intersection checks still cover lines and circular curves only.

- Orders 44-48: whole-entity Fix holds line/arc coordinates and circle radii, and releases through the existing relation inspector. Collinear, coradial and intersection choices add their equivalent solver relations as one undo step. Symmetric pairs points or matching lines, arcs and circles about the last selected line; stored endpoint pairing follows later edits. See [shared relation requests](../crates/soliddraft-sketch/src/relations.rs), [native selection tests](../crates/soliddraft-app/tests/sketch_relation_sets.rs) and [the application-loop scenario](../ci/scenarios/sketch-relations.json).

- Orders 50-52: Sketch fillet, Sketch chamfer and Split entities are in Modify and the property panel, with shared analytic previews, Escape cancellation and one undo step. Fillets join lines or circular arcs with stored tangent and radius relations. Chamfers use two setbacks or a setback and angle between lines. Split uses one point for a line/arc and two points for a circle; endpoint references follow the resulting pieces. Changed corner anchors and whole-line equal/midpoint relations that cannot describe the result are released with a count in the tool message. See [tool operations](../crates/soliddraft-sketch/src/tools/finishing.rs), [native tests](../crates/soliddraft-app/tests/sketch_finishing.rs) and [the Catallax scenario](../ci/scenarios/sketch-fillet-chamfer-split.json).
- Spline profiles: fit-point splines use natural cubic interpolation. Control-point splines use a clamped uniform B-spline of degree up to three. The sketcher supports point placement, Enter/double-click completion, closed loops, editable points and control polygons, point dimensions, whole-entity Fix, point-on-curve and joined-end tangency. Linked mirrors retain every fit/control point and its stored symmetry pairing. Dynamic mirror previews and Enter/double-click completion create both splines in one undo step. Retained linear and circular patterns transform every fit/control point and preserve source links through edits and reopening. Normal Boss extrusion and contained block cuts retain polynomial Bézier controls in Catallax. [Pointer acceptance](../crates/soliddraft-app/tests/sketch_spline.rs), [solver tests](../crates/soliddraft-sketch/src/solve/spline.rs), [model volume tests](../crates/soliddraft-model/tests/sketch_spline.rs) and the fit/control application-loop scenarios cover these paths. Trim, extend, corner edits and offset refuse spline targets by name. General intersecting profiles, circular/elliptical mixtures, oblique extrusion, revolve, sweep, first-hit limits, field operations and STEP export remain refused. Independent tangent/curvature handles and curvature combs remain open.

- Orders 54-57: [linked analytic offsets](../crates/soliddraft-sketch/src/tools/offset.rs) retain signed line/arc/circle distance relations. The property panel can follow an unbranched chain from one seed, reverse direction, offset both sides, add straight or round caps to an open chain and keep the originals as construction guides. Selecting an offset exposes a distance editor that preserves target ids and respects other constraints. Internal straight joins stay on their source partitions; those anchors release when a driving source edit creates a corner and return when it becomes straight again. [Model tests](../crates/soliddraft-sketch/tests/offset_relations.rs) cover source edits, caps, signed distances, reversed members, stable ids and atomic refusals. [Native tests](../crates/soliddraft-app/tests/sketch_offsets.rs) cover previews, controls, Escape, undo/redo and reopen. [The protocol scenario](../ci/scenarios/sketch-linked-offsets.json) extrudes an edited annulus and capped strip through Catallax and reopens them. Branching chains, self-intersecting offsets and consumed radii are refused. Caps use the original boundary on one side, so a one-sided capped profile keeps its originals as profile geometry.

- Orders 58-59: Mirror entities retains an existing axis line or creates a fixed construction axis from two points. Keep symmetry stores point pairing and equal circle radii; shared axis endpoints remain joined. Dynamic mirror draws paired points, lines, circles, arcs, rectangles, slots and polygons with reflected previews, one undo step per placement and Escape to stop. [Model tests](../crates/soliddraft-sketch/tests/mirror_relations.rs), [native tests](../crates/soliddraft-app/tests/sketch_mirror.rs) and [the application scenario](../ci/scenarios/sketch-linked-mirror.json) cover axis edits, profiles, history and reopen. Ellipses retain the existing independent mirror path when Keep symmetry is off.

- Orders 60-64: [retained sketch patterns](../crates/soliddraft-sketch/src/tools/patterns.rs) link generated geometry to its sources. Linear patterns support two direction vectors and counts. Circular patterns use an angle per step or a total span; full turns omit the repeated endpoint. The property panel previews creation and edits, lists skipped instances and exposes saved definitions even when every generated instance is skipped. Surviving entity ids stay fixed during edits. Escape cancels and Apply creates one undo step. [Model tests](../crates/soliddraft-sketch/tests/patterns.rs), [native tests](../crates/soliddraft-app/tests/sketch_patterns.rs) and [the protocol scenario](../ci/scenarios/sketch-linked-patterns.json) cover source edits, skips, retained definitions, preview, history and reopening. Direct deletion of a source, copy or linking relation dissolves that definition and leaves its other geometry independent. Profiles still build through Catallax.

- Orders 65–68: dedicated Move entities, Copy entities, Rotate entities and Scale entities commands in the Sketch tab's Transform group. They share previews, numeric input, pointer base/centre placement, cancellation and one-step undo/redo with the existing Modify tools. Move and Copy also place a displacement with two clicks. Rotate can edit in place or create independent copies; Scale accepts a positive uniform factor. In-place transforms retain entity IDs, construction state and all relations/dimensions, and refuse atomically if solving would change the requested result or move unselected geometry. Driving dimensions are never silently rescaled. Copies preserve construction state and join matching copied endpoints, without inheriting driving relations. See [solver acceptance tests](../crates/soliddraft-sketch/tests/transforms.rs), [rendered interaction tests](../crates/soliddraft-app/tests/ux_sketch_transforms.rs) and [kernel profile tests](../crates/soliddraft-model/tests/sketch_transforms.rs).

- Orders 69–70: retained construction geometry and added a dedicated Centreline command in Draw, the property tools and the command palette. It shares line chaining and inference, always places construction geometry and preserves the ordinary Draw construction preference. Dashed previews, cancellation, per-segment history, `X`, profile exclusion, retained solver relations/dimensions and reopen have [native acceptance coverage](../crates/soliddraft-app/tests/sketch_construction.rs). The existing construction model, selected-entity toggle and profile conversion remain unchanged.

- Order 71: retained the existing Line chain with [native pointer coverage](../crates/soliddraft-app/tests/sketch_line_chains.rs) for uncommitted starts, coincident continuation with Auto relations off, right-click/double-click termination while staying in Line, closure onto an existing point, independent restarts, cancellation, per-segment undo/redo and serialized joins. The viewport hint distinguishes ending a chain from Esc returning to Select; see [the sketch workflow](DESIGN.md#phase-2-the-sketcher).

- Orders 72–75: retained the opposite-corner Rectangle command and added Centre rectangle, 3-point corner rectangle, 3-point centre rectangle and Parallelogram in Draw, the property tools and the command palette. Placement uses successive clicks: centre/corner, first edge/perpendicular height, centre/side midpoint/half-width, or three consecutive parallelogram corners. Axis-aligned rectangles retain horizontal/vertical relations; rotated rectangles retain opposite parallel sides and a perpendicular adjacent pair; parallelograms retain only opposite-side parallel relations. Centre tools retain a construction guide and centre point through midpoint relations. [Shared preview/placement tests](../crates/soliddraft-app/src/sketcher/rectangle.rs) and [native acceptance](../crates/soliddraft-app/tests/sketch_rectangles.rs) cover signed sides, degenerate picks and retries, incompatible snaps, previews, cancellation, construction, solver edits, undo/redo and reopen. The corner shortcut remains `R`. Vendor drag gestures, Shift-to-square and configurable construction-line arrangements are outside this group.

- Orders 81–83: retained the original Straight slot command and added Centrepoint slot, Three-point arc slot and Centrepoint arc slot to Draw, the property tools and the palette. Preview and placement share ordinary line/arc geometry, construction guides and retained coincidence, tangent and equal-radius relations; circular sides stay concentric. See [slot workflows and limits](sketch-slots.md), [placement](../crates/soliddraft-app/src/sketcher/slot.rs) and [native acceptance tests](../crates/soliddraft-app/tests/sketch_slots.rs).

- Order 84: centre-based **Polygon** placement now supports Inscribed and Circumscribed modes and retains its construction circle, joins, equal-length sides and circle relations. Selecting a side or guide exposes side count, circle radius and mode with one-step Apply. A driving guide radius/diameter also resizes through the solver. [Model tests](../crates/soliddraft-sketch/tests/polygon.rs) cover both modes, 3–1000 sides, degenerate/non-finite refusals, stable surviving IDs, centre snaps, construction profiles and serialization. [Native tests](../crates/soliddraft-app/tests/sketch_polygon.rs) cover placement, cancellation, parameter drafts, atomic Apply, undo/redo and reopen. External edge relations and conflicting guide dimensions require explicit release before parameter edits; deleting a defining member or relation releases retained editing. Existing independent polygons and mirror/pattern copies keep their ordinary geometry behavior. See [the sketch contract](DESIGN.md) for these boundaries.

## How to read the list

- **SolidDraft** means an end-user command exists, not merely that the solver could express it. `yes` means the listed operation is implemented; `partial` names its limit; `no` names the inspected code boundary where support is absent. These are source-audit findings, not fresh interactive acceptance results.
- **Makes sense** is a recommendation. `yes` rows are ordered by value for the current 2D sketch workflow, with relations first and trim second as requested. Already complete rows are retention checks, not requests to rebuild them. `later` needs another capability or a separate product decision. `no` is outside this work's scope, not a permanent rejection.
- **Size** estimates the remaining change, including validation: `0` retain and regression-check; `S` local interaction or display change; `M` tool logic plus UI and tests; `L` solver/model or persistent-reference work across layers; `XL` a new geometry family or reusable-sketch model. These are relative scope, not time estimates. A group can contain several sizes.
- Code links below point to the current implementation or, for missing features, the relevant enum, dispatcher or model. A missing command does not necessarily mean a new solver is needed.
- Source labels link to public vendor help. Indexed editions differ, so each link keeps its actual edition. This is not an exhaustive certification of one SOLIDWORKS release. Geometry support in SolidDraft's sketch editor also does not imply that every downstream kernel operation accepts it.

## Recommended work, in value order

| Order | Capability and public source | What it does | SolidDraft, with code location | Makes sense | Size |
|---|---|---|---|---|---|
| 1 | Relations command [SW-rel] | Offers a discoverable list of relation types and applies a compatible choice to the selected geometry. | partial; the ribbon tile has no action in [commands]; selection-specific buttons already use `applicable_relations` in [relation-ui]. | yes | M |
| 2 | Show/hide all relations [SW-rel-display] | Hides or reveals relation glyphs without removing constraints or hiding dimensions. | partial; [glyphs] already draws and picks glyphs, but [session] has no relation-visibility switch. | yes | S |
| 3 | Existing relation inspection and deletion [SW-rel-manager] | Lists relations on selected geometry, highlights their owners and removes a chosen relation. | partial; [properties] selects and deletes constraints, but [sketch-paint] does not highlight their owning entities when a relation is selected. | yes | M |
| 4 | Trim to closest [SW-trim-closest] | Removes the picked interval bounded by the closest intersections. | partial; [tool-model] `Trim` and `trim_preview` cover lines/arcs, with click input in [sketch-input]; circle targets and reported failures still need investigation. | yes | M |
| 5 | Power trim [SW-power-trim] | Trims successive crossed entities while the user drags a pointer trail. | no; [sketch-input] handles trim only on a primary click, not a swept drag. | yes | L |
| 6 | Extend while trimming [SW-power-extend] | Lengthens an entity along its natural path without leaving the trim workflow. | partial; [tool-model] has a separate `Extend` operation and [modify-ui] exposes a separate tool; no power-trim gesture. | yes | M |
| 7 | Trim corner [SW-trim-corner] | Trims or extends two entities to their intersection. | no; [tool-model] has no paired corner operation. | yes | M |
| 8 | Trim away inside [SW-trim-inside] | Removes portions between two chosen bounding entities. | no; [tool-model] has only a single picked interval. | yes | M |
| 9 | Trim away outside [SW-trim-tools] | Removes portions outside two chosen bounding entities. | no; [tool-model] has no inside/outside boundary selection mode. | yes | M |
| 10 | Keep trimmed pieces as construction [SW-trim-tools] | Retains removed portions as non-profile guides. | no; `Trim` in [tool-model] has no keep-construction option. | yes | M |
| 11 | Ignore construction when trimming [SW-trim-tools] | Protects guide entities from removal during trim. | no; `Trim` in [tool-model] has no construction filter option. | yes | S |
| 12 | Conflicting-relation feedback [SW-xpert] | Identifies relations or dimensions that cannot be satisfied together. | partial; [solver] records conflicting IDs, [entity-color] marks affected entities, and [properties] names conflicts; there is no dedicated conflict filter. | yes | M |
| 13 | Fully/under-defined colouring [SW-defined] | Shows which entities still have freedom and which are constrained. | yes; [solver] computes free entities and [entity-color] uses theme colours for free, constrained and conflicting geometry. It does not copy vendor colours. | yes | 0 |
| 14 | Fully defined/under-defined status [SW-defined] | Reports the sketch's constraint state while editing. | yes; `SolveStatus` in [sketch-model] and [properties] report the state. | yes | 0 |
| 15 | Repair sketch diagnostics [SW-repair] | Locates gaps, overlaps and branching junctions before a sketch is used. | partial; [profiles] rejects open/branched profiles, but no repair inspector locates or steps through faults. | yes | L |
| 16 | Check sketch for intended feature [SW-tools] | Tests whether geometry meets the requirements of the planned operation. | partial; [profiles] extracts closed regions and [properties] disables Extrude/Cut without a profile; no selectable feature-specific checker. | yes | M |
| 17 | Driven/reference dimensions [SW-driven] | Measures geometry without adding a solver equation, and can switch to driving when valid. | no; numeric variants in [constraints] are driving only; `Constraint` stores no driven flag. | yes | L |
| 18 | Smart dimension [SW-dim] | Chooses a dimension from the picked entities and label placement. | yes; `dimension_click` and `dimension_kind` in [dimensions] implement this for supported entities. | yes | 0 |
| 19 | Horizontal and vertical dimensions [SW-dim] | Controls projected point separation along either sketch axis. | yes; [constraints] has `DistanceKind` and [dimensions] chooses the axis from label placement. | yes | 0 |
| 20 | Aligned length and point-to-point dimensions [SW-dim] | Controls true distance between two points or a line's endpoints. | yes; `Distance` in [constraints] and [dimensions]. | yes | 0 |
| 21 | Point-to-line and parallel-line distance [SW-dim] | Controls perpendicular separation from a point to a line. | yes; [dimensions] emits `PointLineDistance`, which [solver] solves. Edge cases still belong in acceptance tests. | yes | 0 |
| 22 | Angular dimensions [SW-dim] | Controls the angle between two lines. | yes; `Angle` in [constraints], [dimensions] and [solver]. | yes | 0 |
| 23 | Radius and diameter dimensions [SW-dim] | Controls arc radius or circle diameter. | yes; [dimensions] chooses radius for arcs and diameter for circles; [constraints] stores the diameter flag. | yes | 0 |
| 24 | Edit and reposition dimension labels [SW-dim] | Changes a numeric value and moves its annotation without moving geometry unnecessarily. | yes; [selection-input] drags labels and [dimension-editor] edits values on double-click. | yes | 0 |
| 25 | Equations and global values for sketch dimensions [SW-values] | Drives dimensions from named expressions shared with the document. | yes; `Input::Dimension` and bindings in [values], with sketch re-solve in [params] and editing in [values-ui]. | yes | 0 |
| 26 | Linked dimension values [SW-linked] | Gives several dimensions the same shared value. | partial; [values] can bind dimensions to one named expression; there is no dedicated Link values command in [commands]. | yes | S |
| 27 | Automatic relations switch [SW-snaps] | Enables or disables inferred relations while drawing. | yes; [guides] exposes Auto relations and suppresses inferred snaps when off. This is distinct from glyph visibility. | yes | 0 |
| 28 | Endpoint, centre and origin snaps [SW-snaps] | Places new points on existing entity points. | yes; `infer` in [inference] scans entity points, including centres and the fixed origin, and `apply_snaps` adds coincidence. | yes | 0 |
| 29 | Midpoint snap [SW-snaps] | Places a point at the middle of a line and keeps that relation. | yes; [inference] detects line midpoints and adds `Midpoint`. | yes | 0 |
| 30 | On-entity snap [SW-snaps] | Places a new point on the nearest line, arc or circle. | yes; [inference] projects to curves and adds `PointOn`. | yes | 0 |
| 31 | Horizontal/vertical inference [SW-snaps] | Aligns a new segment with the previous point and adds the corresponding relation. | yes; [inference] offers horizontal/vertical snaps and `apply_snaps` adds them for a new line. | yes | 0 |
| 32 | Intersection snap [SW-snaps] | Offers the crossing of two curves as a snap target. | no; [inference] has no pairwise intersection candidates, although [tool-model] computes intersections for trim. | yes | M |
| 33 | Tangent/perpendicular/parallel inference [SW-snaps] | Offers design-intent cues relative to existing geometry while drawing. | no; [inference] offers point, midpoint, axis alignment and on-entity snaps only. Manual relations already exist. | yes | M |
| 34 | Quick snap filters [SW-quick-snaps] | Restricts the next placement to a chosen snap class. | no; [session] has a single auto-relations boolean, not per-class or one-shot snap choices. | yes | M |
| 35 | Horizontal and vertical relations [SW-rel] | Aligns one line or a pair of points with a sketch axis. | yes; `Horizontal`/`Vertical` in [constraints] and [relation-ui]. | yes | 0 |
| 36 | Parallel relation [SW-rel] | Keeps two lines parallel. | yes; `Parallel` in [constraints], [relation-ui] and [solver]. | yes | 0 |
| 37 | Perpendicular relation [SW-rel] | Keeps two lines at a right angle. | yes; `Perpendicular` in [constraints], [relation-ui] and [solver]. | yes | 0 |
| 38 | Coincident / merge points [SW-rel] | Makes two entity points share a location. | yes; `Coincident` in [constraints] and [relation-ui]. Points retain their entity ownership rather than sharing one stored vertex. | yes | 0 |
| 39 | Point on line or curve [SW-rel] | Keeps a point on the supporting line or circular curve. | yes; `PointOn` in [constraints], exposed as On line / On curve in [relation-ui]. | yes | 0 |
| 40 | Tangent relation [SW-rel] | Keeps a line and circular curve, or two circular curves, touching tangentially. | yes; [relation-ui] offers supported pairs and [solver] handles both shared-end and general tangency. Joined spline endpoints also support line, arc and spline tangency. | yes | 0 |
| 41 | Equal length / equal radius [SW-rel] | Matches two line lengths or two circular radii. | yes; `Equal` in [constraints] and [relation-ui]; [solver] distinguishes lengths from radii. | yes | 0 |
| 42 | Concentric relation [SW-rel] | Makes circular entities share a centre. | yes; `Concentric` in [constraints] and [relation-ui]. | yes | 0 |
| 43 | Midpoint relation [SW-rel] | Keeps a point halfway along a line. | yes; `Midpoint` in [constraints] and [relation-ui]. | yes | 0 |
| 44 | Fix and release geometry [SW-rel] | Holds selected geometry in place until its fixing relation is removed. | partial; [relation-ui] fixes one selected point and [properties] can delete the relation; no whole-entity fix command. | yes | M |
| 45 | Collinear relation [SW-rel] | Keeps two lines on the same infinite line. | no; [constraints] has no collinear kind, and [relation-ui] does not compose an equivalent set. | yes | M |
| 46 | Symmetric relation [SW-rel] | Keeps paired geometry symmetric about a selected line. | no; [constraints] has no symmetry kind; mirror copies in [tool-model] are independent. | yes | L |
| 47 | Coradial relation [SW-rel] | Gives circular curves both the same centre and the same radius. | partial; [constraints] can express `Concentric` plus `Equal`, but [relation-ui] has no single coradial choice. | yes | S |
| 48 | Intersection relation [SW-rel] | Keeps a point at the intersection of two selected lines. | partial; two `PointOn` constraints can represent it in [constraints]; [relation-ui] has no three-item shortcut. | yes | S |
| 49 | Relation list filters [SW-rel-manager] | Narrows relation inspection to selected, conflicting, dangling or external references. | partial; [properties] lists selected entities' relations and conflict names separately, without status filters or a whole-sketch list. | yes | M |
| 50 | Sketch fillet [SW-fillet] | Inserts a tangent radius between sketch entities, adjusting their ends. | no; [tool-model] has no sketch fillet. Solid-body fillets are a different operation and are not included here. | yes | L |
| 51 | Sketch chamfer [SW-chamfer] | Replaces a corner with a straight segment set by distances or distance/angle. | no; [tool-model] has no sketch chamfer. | yes | M |
| 52 | Split entities [SW-tools] | Divides a curve at a selected point without trimming away an interval. | no; [tool-model] has no split operation exposed to users. | yes | M |
| 53 | Extend entities [SW-tools] | Lengthens a chosen line or arc end to a boundary. | yes; [tool-model] `Extend`/`extend_boundary`, [modify-ui] and [sketch-input]. | yes | 0 |
| 54 | Offset entities [SW-offset] | Makes a parallel curve or chain at a signed distance. | partial; [tool-model] accepts an ordered connected line/arc chain or a lone circle, producing independent copies rather than an associative offset. | yes | L |
| 55 | Offset chain selection [SW-offset] | Finds connected curves from a seed so the user need not pick them in walking order. | no; [modify-ui] explicitly requires ordered selection. | yes | M |
| 56 | Reverse / bidirectional offset [SW-offset] | Chooses the side or offsets both sides of a seed. | partial; [modify-ui] accepts signed distance, but [tool-model] has no both-sides operation. | yes | M |
| 57 | Offset end caps [SW-offset] | Joins the ends of open offset chains with lines or arcs. | no; [tool-model] `Offset` has no cap setting. | yes | M |
| 58 | Mirror existing entities [SW-mirror] | Copies a selection across a line while preserving symmetric design intent. | partial; [tool-model] reflects copies about two axis points, but stores no symmetry relation or link to a picked axis entity. | yes | L |
| 59 | Dynamic mirror [SW-mirror] | Mirrors newly drawn entities as the user sketches. | no; [session] and [sketch-input] have no dynamic-mirror mode. Requires persistent symmetry, not just duplicate geometry. | yes | L |
| 60 | Linear sketch pattern [SW-linear-pattern] | Repeats sketch entities with controlled spacing and count. | partial; [tool-model] creates independent translations in one direction; [modify-ui] supplies step and count. | yes | L |
| 61 | Two-direction linear pattern [SW-linear-pattern] | Adds a second spacing/count direction to make an array. | no; `LinearPattern` in [tool-model] stores one step and one count. | yes | M |
| 62 | Circular sketch pattern [SW-circular-pattern] | Repeats entities about a centre with angular spacing and count. | partial; [tool-model] creates independent rotations; [modify-ui] sets centre, angle per step and count, not a persistent pattern definition. | yes | L |
| 63 | Pattern total-angle/equal-spacing mode [SW-circular-pattern] | Distributes a count of instances across a chosen angular span. | partial; [modify-ui] accepts an angle per step only; the user can calculate it, but there is no total-span control. | yes | S |
| 64 | Skip sketch-pattern instances [SW-circular-pattern] | Omits chosen instances without destroying the pattern definition. | no; pattern operations in [tool-model] have no skipped-instance list or editable definition. | yes | L |
| 65 | Move selected geometry [SW-move] | Translates a selection while respecting its constraints. | partial; [selection-input] supports entity/point dragging, not a numeric multi-entity Move command. | yes | M |
| 66 | Copy selected geometry [SW-move] | Duplicates a selection using a base point and displacement. | partial; a two-instance linear pattern in [tool-model] produces a translated copy; no dedicated Copy interaction. | yes | M |
| 67 | Rotate selected geometry [SW-move] | Rotates or copies a selection about a chosen point. | partial; a two-instance circular pattern in [tool-model] can copy a rotation; no in-place Rotate command. | yes | M |
| 68 | Scale selected geometry [SW-scale] | Resizes selected entities about a base point. | no; [tool-model] has no scale operation. Must define how driving dimensions constrain or reject the transform. | yes | M |
| 69 | Construction geometry [SW-tools] | Excludes guides from profiles while retaining their constraints. | yes; `Entity::construction` in [sketch-model], dashed rendering in [sketch-paint], `X` in [commands], exclusion in [profiles]. | yes | 0 |
| 70 | Dedicated centreline command [SW-tools] | Starts a construction line without changing ordinary line drawing mode. | partial; construction lines are possible through [commands] and [session], but no dedicated centreline tool. | yes | S |
| 71 | Line chains [SW-entities-overview] | Draws successive connected segments and ends a chain without leaving the tool. | yes; `Tool::Line` in [entity-input] connects segments and ends on double-click or an existing point. | yes | 0 |
| 72 | Corner rectangle [SW-rect] | Makes an axis-aligned four-line rectangle from opposite corners. | yes; `Tool::Rectangle` in [entity-input] adds coincidence and horizontal/vertical relations. | yes | 0 |
| 73 | Centre rectangle [SW-rect] | Makes a rectangle around a picked centre. | no; [entity-input] has only the opposite-corner rectangle path. | yes | M |
| 74 | Three-point corner/centre rectangles [SW-rect] | Places a rotated rectangle from a side or centre reference. | no; [entity-input] has no rotated rectangle variant. | yes | M |
| 75 | Parallelogram [SW-rect] | Draws a skew four-sided figure with opposite sides parallel. | no; [entity-input] has no parallelogram tool. | yes | M |
| 76 | Centrepoint circle [SW-circles] | Creates a circle from centre and radius. | yes; `Tool::Circle` in [entity-input]. | yes | 0 |
| 77 | Perimeter circle [SW-circles] | Fits a circle through three picked points. | no; [entity-input] creates circles only from centre and radius. | yes | M |
| 78 | Centrepoint arc [SW-arcs] | Creates an arc from centre, start and end. | partial; [entity-input] uses those picks but chooses the shorter sweep, so it is not a general major-arc interaction. | yes | M |
| 79 | Three-point arc [SW-arcs] | Creates an arc through start, end and an intermediate point. | no; [entity-input] has only a centre-first arc path. | yes | M |
| 80 | Tangent arc [SW-arcs] | Continues a line or arc with a tangent arc. | no; [entity-input] has no tangent-arc creation mode; a manual tangent relation can be added later. | yes | M |
| 81 | Straight slot [SW-slots] | Creates two parallel sides and equal-radius tangent ends. | yes; `Tool::Slot` in [entity-input] creates lines, arcs, a construction axis and their relations. | yes | 0 |
| 82 | Centrepoint straight slot [SW-slots] | Defines a straight slot symmetrically from its midpoint. | yes; midpoint, end centre and width create the existing straight-slot geometry with a retained midpoint relation. See [slot workflows](sketch-slots.md). | yes | M |
| 83 | Three-point / centrepoint arc slots [SW-slots] | Makes a constant-width slot along a circular path. | yes; three-point and counterclockwise centrepoint paths create concentric side arcs, tangent equal-radius caps and a construction path. See [slot workflows and limits](sketch-slots.md). | yes | L |
| 84 | Regular polygon [SW-polygon] | Creates a chosen number of sides about a centre. | partial; [tool-model] and [modify-ui] create a regular polygon from a vertex, but do not retain a parametric polygon object or inscribed/circumscribed mode. | yes | M |
| 85 | Sketch point [SW-entities-overview] | Adds a reference or construction point for relations and dimensions. | yes; `Tool::Point` in [entity-input], with point relations in [relation-ui]. | yes | 0 |
| 86 | Closed-contour shading [SW-shading] | Fills closed regions so missing closure is easier to notice. | partial; `paint_regions` in [sketch-paint] fills valid regions, but [session] has no shading toggle. | yes | S |
| 87 | Show/hide sketch dimensions [SW-dim] | Reduces annotation clutter without deleting driving values. | partial; [sketch-input] always paints active-sketch dimensions; closed-sketch display has a separate path in [closed-dimensions]. | yes | S |
| 88 | Show/hide sketches outside edit mode [SW-tools] | Controls reference geometry visibility independently of editing. | yes; [tree-visibility] handles sketch visibility and [passive-paint] draws passive geometry. | yes | 0 |
| 89 | Grid display and settings [SW-snaps] | Shows a spacing reference in the sketch workspace. | yes; [grid-settings] persists grid style and [grid-paint] draws the viewport grid. This does not imply snap-to-grid. | yes | 0 |
| 90 | Grid snapping [SW-snaps] | Places geometry at grid increments rather than only using the grid as a visual aid. | no; [inference] does not consult grid spacing. | yes | M |

## Useful later, with dependencies

These rows are deliberately outside the first relations and trim groups. Their order is not a delivery commitment.

| ID | Capability and public source | What it does | SolidDraft, with code location | Makes sense | Size |
|---|---|---|---|---|---|
| L1 | Fully define sketch automatically [SW-fully-define] | Adds dimensions and relations to remove remaining freedoms. | no; [solver] analyses freedom but does not author a defining constraint set. Requires predictable dimension placement and conflict handling. | later | L |
| L2 | SketchXpert-style solution choices [SW-xpert] | Proposes alternative conflicting relations to remove rather than deciding silently. | partial; [solver] supplies unsatisfied IDs and [properties] supports manual deletion, not solution-set search. | later | L |
| L3 | Automatic repair [SW-repair] | Merges overlaps or closes small gaps within a user-chosen tolerance. | no; [profiles] validates but does not offer a previewable repair command. Diagnostics should come first; never silently change geometry. | later | L |
| L4 | Suppress relations by configuration [SW-rel-manager] | Disables selected relations in specific configurations without deleting them. | no; `Constraint` in [constraints] has no suppression state. Document configurations are not per-constraint suppression. | later | L |
| L5 | Equal slots [SW-rel] | Keeps slot lengths and widths equal as slot geometry changes. | partial; [entity-input] makes a slot from ordinary entities, but [sketch-model] stores no slot identity or equal-slot relation. | later | L |
| L6 | Doubled-distance relation [SW-rel] | Expresses symmetric spacing relative to a centreline. | no; [constraints] lacks this kind. Needs a clear selection contract and dimension semantics. | later | L |
| L7 | Arc-length dimension [SW-arc-dim] | Controls length along a circular arc rather than its chord or radius. | no; [constraints] has no arc-length or sweep-angle dimension. | later | L |
| L8 | Path-length dimension [SW-path-dim] | Controls total length along a connected curve chain. | no; [constraints] has no aggregate-chain dimension. Requires persistent chain references. | later | L |
| L9 | Ordinate sketch dimensions [SW-dim] | Dimensions several points from a shared zero reference. | no; [constraints] stores pairwise distances, not an ordinate set; drawing dimensions are separate. | later | L |
| L10 | Stretch entities [SW-stretch] | Moves selected portions of geometry while leaving other portions anchored. | partial; point dragging in [selection-input] allows some edits, but [tool-model] has no window-based stretch operation. | later | M |
| L11 | Convert entities [SW-convert] | Projects selected model edges or contours into a sketch with source associations. | no; [tool-model] and [sketch-model] have no associative projected-curve entity. A face-attached sketch plane is not converted geometry. | later | L |
| L12 | On edge external relation [SW-convert] | Updates converted geometry when its source model edge changes. | no; [constraints] has only intra-sketch entity IDs, not external edge references. | later | L |
| L13 | Intersection curve [SW-intersection] | Builds sketch curves where a plane or surface crosses model geometry. | no; [tool-model] has no body/plane intersection command. Needs supported kernel curves, outside this implementation scope. | later | XL |
| L14 | Silhouette entities [SW-silhouette] | Projects a body's outline onto the sketch plane. | no; [tool-model] has no silhouette-to-sketch command. Drawing silhouettes do not supply an associative sketch. | later | XL |
| L15 | Derived sketch and underive [SW-derived] | Reuses a linked parent sketch on another plane, with an explicit way to break the link. | no; [feature-model] `SketchFeature` stores its own sketch, not a parent-sketch derivation. | later | XL |
| L16 | Make/edit sketch block [SW-blocks] | Groups reusable entities under one local definition and coordinate frame. | no; [sketch-model] stores flat entities and constraints, not block definitions. | later | XL |
| L17 | Insert/save linked blocks [SW-blocks] | Places scaled/rotated instances of a stored definition and updates them together. | no; [sketch-model] has no block instances or definition links. Needs file/version and external-reference rules. | later | XL |
| L18 | Explode block [SW-blocks] | Turns one instance into ordinary editable entities. | no; [tool-model] has no blocks to explode. Preserve unrelated instances and constraints when added. | later | M |
| L19 | Ellipse and partial ellipse [SW-entities-overview] | Creates full or partial elliptical curves. | yes; full and partial ellipse tools, solver dimensions/relations and exact normal Boss/Cut extrusion have the bounded support described above. | yes | 0 |
| L20 | Parabola and conic [SW-conic] | Creates non-circular conics with endpoint/tangency or shape controls. | no; [entities] has no conic kind. | later | XL |
| L21 | Interpolating spline [SW-splines] | Fits a smooth curve through picked points with end controls. | yes; natural cubic fit points, point dimensions, point-on-curve and joined-end tangency; see the bounded profile support above. | yes | 0 |
| L22 | Style spline [SW-style-spline] | Shapes a spline using a control polygon rather than interpolation points. | yes; clamped uniform control-point splines of degree up to three, editable control polygon and the bounded profile support above. | yes | 0 |
| L23 | Equal curvature relation [SW-rel] | Matches curvature and tangent direction at a curve join. | no; [constraints] has no equal-curvature relation. | later | L |
| L24 | Spline handles and curvature display [SW-splines] | Edits tangent/curvature handles and shows curvature variation. | partial; authored fit/control points are editable, but independent tangent/curvature handles and curvature combs remain open. | later | L |
| L25 | Fit spline [SW-fit-spline] | Replaces a selected chain with one spline within a chosen tolerance. | no; [tool-model] and [entities] have no spline fitting or spline output. | later | XL |
| L26 | Equation-driven curve [SW-equation-curve] | Creates a curve from explicit or parametric equations over a chosen interval. | no; [entities] has no equation-curve representation; scalar expressions in [values] are not curves. | later | XL |
| L27 | Sketch text [SW-text] | Makes font outlines usable as sketch profiles, including text along a path. | Straight text: editable content, bundled Inter Regular/Medium (SIL OFL 1.1), capital height, baseline position and angle. Glyph lines and Bézier curves retain counters in single-region Boss profiles. Extrusion currently requires one connected region; disconnected glyph regions show guidance before kernel evaluation. Text along a path, shaping/kerning, custom fonts and linked text patterns remain unsupported. | partial | XL |
| L28 | Sketch picture underlay [SW-picture] | Places a scaled, positioned image behind sketch geometry. | no; [session] and [sketch-paint] have no sketch image object or underlay. | later | L |
| L29 | Raster autotrace [SW-picture] | Converts image boundaries into editable curves for review. | no; [tool-model] has no tracing operation. Requires picture calibration and supported output curves first. | later | XL |

## Recorded but outside this work

| ID | Capability and public source | What it does | SolidDraft, with code location | Makes sense | Size |
|---|---|---|---|---|---|
| N1 | 3D sketch entities and planes [SW-tools] | Places sketch geometry in space rather than in one planar coordinate system. | no; [entities] uses `P2` and [feature-model] attaches each sketch to one plane. | no; separate 3D sketch project | XL |
| N2 | Along X/Y/Z, parallel YZ/ZX and normal relations [SW-rel] | Constrains directions relative to 3D axes or planes. | no; [constraints] supports two in-plane axes only. | no; 3D relations | XL |
| N3 | Pierce / on surface / on plane relations [SW-rel] | Connects sketch points to out-of-plane curves or supporting geometry. | no; [constraints] has no external spatial curve/surface targets. | no; spatial reference work | XL |
| N4 | Spline on surface [SW-surface-spline] | Keeps a spline attached to a selected model surface. | no; [entities] has neither spatial splines nor surface-bound points. | no; kernel and 3D work | XL |
| N5 | 3D sketch dimensions [SW-dim] | Drives distances and angles for spatial sketch entities. | no; [constraints] and [dimensions] operate in a 2D sketch plane. | no; explicitly excluded | XL |

## Delivery groups and acceptance evidence

1. **Relations, orders 1-3.** Reuse `ConstraintKind` from `soliddraft-sketch`, `applicable_relations`, the property-panel mutation path and existing glyph painters. The requested ribbon button must list every currently supported non-dimensional relation type, keep incompatible choices visible with a reason, and apply a valid choice to the selection as one undo step. Horizontal/vertical, parallel, perpendicular, tangent, equal, concentric, coincident, point-on, midpoint and fix already have representations. Collinear and symmetry are backlog items, not solver capabilities to pretend exist. Show/hide must affect both painting and picking, leave dimension labels alone, retain constraints, and remain stable across selection changes and redraws. Keep the existing ribbon/panel style.
2. **Trim, orders 4-11.** Start by driving the real pointer workflow, recording the reported failures as concrete inputs. Neither this audit nor the existing tests proves those failures reproduced. For each observed failure, add a failing regression before changing the shared operation. Compare power-trim and trim-to-closest behavior against the public descriptions. Cover lines/arcs/circles, zero/one/two crossings, tangent and coincident boundaries, endpoint clicks, arc wrap, construction geometry, stale constraints, cancellation, undo/redo and save/reopen. Power-trim should detect entities crossed between pointer samples, not just those underneath a sampled pointer position. A drag should be one undo step. A refusal must leave the sketch unchanged.
3. **Remaining yes rows.** Work in value order, skip already complete capabilities, and keep each coherent group in its own PR from accepted main. Diagnostics precede automatic repair; persistent symmetry precedes dynamic mirror; persistent pattern definitions precede skipped-instance editing. Keep CI green before proceeding. Do not fold kernel work, 3D dimensions or a panel redesign into these groups.

Existing evidence to extend, not a substitute for new acceptance runs:

- [Sketch tests] cover relation families, dimension side retention, free/fixed points, inconsistent constraints, profile closure and serialization.
- [Tool tests] cover line/arc interval trim, bounded extension, offset, reflection, patterns, rejected operations and serialization. Trim tests currently demonstrate picked-interval behavior, not power trim.
- [Modify UI tests] drive polygon, trim, extend, preview, undo and reopen through the app.
- [Copy UI tests] drive offset, mirror and patterns through the app.
- [Sketch pattern model tests] check model-level pattern behavior.

This first change is documentation only. It does not reproduce or fix trim, enable the Relations tile, or certify interactive parity.

## Public references

Only the public help pages below inform the SOLIDWORKS side. Search-index excerpts were used where the publisher blocked direct HTML retrieval. Edition-specific details and unsupported selection combinations must be checked again when each tool is implemented.

[SW-rel]: https://help.solidworks.com/2018/english/Solidworks/sldworks/c_Description_of_Sketch_Relations.htm
[SW-rel-display]: https://help.solidworks.com/2027/english/SolidWorks/sldworks/c_areas_reliant_sketch_relations.htm
[SW-rel-manager]: https://help.solidworks.com/2013/english/Solidworks/sldworks/HIDD_DVE_SK_EDIT_RELATIONS.htm
[SW-tools]: https://help.solidworks.com/2021/english/solidworks/sldworks/c_sketch_tools_top.htm
[SW-entities-overview]: https://help.solidworks.com/2027/english/SolidWorks/acadhelp/t_moving2d3d_sketching_overview.htm
[SW-circles]: https://help.solidworks.com/2026/English/SolidWorks/sldworks/c_Circles.htm
[SW-arcs]: https://help.solidworks.com/2024/English/SolidWorks/sldworks/c_Arcs.htm
[SW-slots]: https://help.solidworks.com/2020/english/SWConnected/swdotworks/c_slots.htm?id=23.14.19
[SW-polygon]: https://help.solidworks.com/2021/english/solidworks/sldworks/t_Sketching_Polygons.htm
[SW-splines]: https://help.solidworks.com/2026/english/solidworks/sldworks/c_splines.htm
[SW-equation-curve]: https://help.solidworks.com/2022/english/solidworks/sldworks/c_equation_driven_curves.htm
[SW-text]: https://help.solidworks.com/2016/english/solidworks/sldworks/hidd_dve_sketch_text.htm
[SW-power-trim]: https://help.solidworks.com/2025/english/solidworks/sldworks/t_Trimming_with_Power_Trim.htm
[SW-power-extend]: https://help.solidworks.com/2026/english/solidworks/sldworks/t_Extending_with_Power_Trim.htm
[SW-trim-closest]: https://help.solidworks.com/2023/English/solidworks/sldworks/t_Trimming_with_Trim_to_Closest.htm
[SW-trim-corner]: https://help.solidworks.com/2022/english/SolidWorks/Sldworks/t_Trimming_with_Corner.htm
[SW-trim-inside]: https://help.solidworks.com/2026/english/solidworks/sldworks/t_Trimming_with_Trim_Away_Inside.htm
[SW-trim-tools]: https://help.solidworks.com/2026/English/api/swconst/ToolsSketchToolsTrim.htm
[SW-fillet]: https://help.solidworks.com/2025/English/api/sldworksapi/SOLIDWORKS.Interop.sldworks~SOLIDWORKS.Interop.sldworks.ISketchManager~CreateFillet.html
[SW-chamfer]: https://help.solidworks.com/2024/English/SolidWorks/sldworks/c_Sketch_Chamfers.htm
[SW-offset]: https://help.solidworks.com/2025/english/solidworks/sldworks/c_Offset_Entities.htm
[SW-mirror]: https://help.solidworks.com/2021/english/solidworks/sldworks/c_Mirror_Entities.htm
[SW-linear-pattern]: https://help.solidworks.com/2023/English/SolidWorks/sldworks/hidd_dve_sketch_pattern_linear.htm
[SW-circular-pattern]: https://help.solidworks.com/2014/english/solidworks/Sldworks/HIDD_DVE_SKETCH_PATTERN_CIRCULAR.htm
[SW-move]: https://help.solidworks.com/2024/English/SolidWorks/sldworks/t_Moving_or_Copying_Sketch_Entities.htm
[SW-scale]: https://help.solidworks.com/2023/english/SolidWorks/sldworks/r_Scale_PropertyManager.htm
[SW-stretch]: https://help.solidworks.com/2013/english/solidworks/sldworks/r_Stretch_PropertyManager.htm
[SW-snaps]: https://help.solidworks.com/2019/english/SolidWorks/sldworks/c_sketch_snaps.htm
[SW-quick-snaps]: https://help.solidworks.com/2026/English/SolidWorks/sldworks/c_Quick_Snaps.htm
[SW-dim]: https://help.solidworks.com/2026/english/SolidWorks/sldworks/t_Dimensioning_a_2D_Sketch.htm
[SW-driven]: https://help.solidworks.com/2014/english/solidworks/Sldworks/HIDD_MAKE_DRIVEN.htm
[SW-arc-dim]: https://help.solidworks.com/2026/english/solidworks/sldworks/t_creating_arc_dimensions.htm
[SW-path-dim]: https://help.solidworks.com/2020/English/solidworks/sldworks/t_create_path_length_dimensions.htm
[SW-values]: https://help.solidworks.com/2019/english/SolidWorks/sldworks/t_link_dimensions_use_global_variables.htm
[SW-linked]: https://help.solidworks.com/2027/english/SolidWorks/sldworks/c_shared_values.htm
[SW-defined]: https://help.solidworks.com/2024/English/SolidWorks/sldworks/c_Fully_Defined_Sketches.htm
[SW-fully-define]: https://help.solidworks.com/2023/English/solidworks/sldworks/HIDD_FULLY_DEFINE_SKETCH.htm
[SW-repair]: https://help.solidworks.com/2021/english/solidworks/sldworks/HIDD_SKETCH_GEOM_DIAGNOSTICS_impexp.htm
[SW-xpert]: https://help.solidworks.com/2025/english/solidworks/sldworks/HIDD_DVE_SKETCH_XPERT.htm
[SW-rect]: https://help.solidworks.com/2023/English/solidworks/sldworks/c_Rectangles.htm
[SW-shading]: https://help.solidworks.com/2024/English/SolidWorks/sldworks/c_Shaded_Sketch_Contour.htm
[SW-convert]: https://help.solidworks.com/2022/english/solidworks/sldworks/c_Convert_Entities.htm
[SW-intersection]: https://help.solidworks.com/2018/english/Solidworks/sldworks/c_Intersection_Curves.htm
[SW-silhouette]: https://help.solidworks.com/2025/english/solidworks/sldworks/c_silhouette_entities.htm
[SW-derived]: https://help.solidworks.com/2018/english/solidworks/sldworks/c_derived_sketches.htm
[SW-blocks]: https://help.solidworks.com/2025/english/solidworks/sldworks/c_Blocks_in_Parts_and_Assemblies.htm
[SW-conic]: https://help.solidworks.com/2025/english/solidworks/sldworks/t_Sketching_Conics.htm
[SW-style-spline]: https://help.solidworks.com/2021/english/SolidWorks/sldworks/hidd_styled_curves.htm
[SW-fit-spline]: https://help.solidworks.com/2019/english/SolidWorks/sldworks/c_fit_splines.htm
[SW-surface-spline]: https://help.solidworks.com/2025/english/solidworks/sldworks/c_Splines_on_Surfaces.htm
[SW-picture]: https://help.solidworks.com/2025/english/solidworks/sldworks/r_sketch_picture_pm.htm

## SolidDraft code and test references

[commands]: ../crates/soliddraft-app/src/commands.rs#L1465-L1581
[session]: ../crates/soliddraft-app/src/state.rs#L538-L618
[relation-ui]: ../crates/soliddraft-app/src/sketcher.rs#L1840-L1895
[properties]: ../crates/soliddraft-app/src/properties.rs#L1030-L1335
[constraints]: ../crates/soliddraft-sketch/src/lib.rs#L373-L526
[entities]: ../crates/soliddraft-sketch/src/lib.rs#L131-L160
[sketch-model]: ../crates/soliddraft-sketch/src/lib.rs
[solver]: ../crates/soliddraft-sketch/src/solve.rs
[glyphs]: ../crates/soliddraft-app/src/sketcher.rs#L1096-L1220
[sketch-input]: ../crates/soliddraft-app/src/sketcher.rs#L249-L372
[selection-input]: ../crates/soliddraft-app/src/sketcher.rs#L492-L590
[entity-input]: ../crates/soliddraft-app/src/sketcher.rs#L635-L822
[inference]: ../crates/soliddraft-app/src/sketcher.rs#L137-L241
[dimensions]: ../crates/soliddraft-app/src/sketcher.rs#L825-L950
[closed-dimensions]: ../crates/soliddraft-app/src/sketcher.rs#L1436-L1468
[dimension-editor]: ../crates/soliddraft-app/src/sketcher.rs#L1673-L1752
[entity-color]: ../crates/soliddraft-app/src/sketcher.rs#L953-L968
[sketch-paint]: ../crates/soliddraft-app/src/sketcher.rs#L970-L1094
[passive-paint]: ../crates/soliddraft-app/src/sketcher.rs#L1813-L1837
[tool-model]: ../crates/soliddraft-sketch/src/tools.rs
[modify-ui]: ../crates/soliddraft-app/src/sketch_tools.rs#L121-L193
[profiles]: ../crates/soliddraft-sketch/src/profile.rs
[guides]: ../crates/soliddraft-app/src/measure/guides.rs
[values]: ../crates/soliddraft-model/src/values.rs
[params]: ../crates/soliddraft-model/src/params.rs#L181-L191
[values-ui]: ../crates/soliddraft-app/src/values_tab.rs
[feature-model]: ../crates/soliddraft-model/src/feature.rs
[tree-visibility]: ../crates/soliddraft-app/src/tree_visibility.rs
[grid-settings]: ../crates/soliddraft-app/src/settings/mod.rs
[grid-paint]: ../crates/soliddraft-app/src/viewport/grid.rs
[Sketch tests]: ../crates/soliddraft-sketch/src/tests.rs
[Tool tests]: ../crates/soliddraft-sketch/src/tools_tests.rs
[Modify UI tests]: ../crates/soliddraft-app/tests/ux_sketch_modify.rs
[Copy UI tests]: ../crates/soliddraft-app/tests/ux_sketch_copy.rs
[Sketch pattern model tests]: ../crates/soliddraft-model/tests/sketch_pattern.rs
