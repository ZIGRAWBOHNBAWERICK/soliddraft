# Mates

A mate is an authored relationship between components. Solving it writes placements, and a
component's placement stays the truth of where it is. Mates, values and their expressions
are saved in `assembly.json`. Proofs, solver status and diagnostics are derived and never
saved. The code is `crates/soliddraft-assembly/src/mates/`; the commands and the Mate panel
are in `crates/soliddraft-app/src/mates/`.

## What this version supports

| Family | Fixes |
|---|---|
| Origin | A component's origin on the assembly's. With `align_axes` its axes match too |
| Distance | Plane-plane and plane-point/axis gaps; Euclidean point-point distance; perpendicular separation between axes with their directions kept parallel |
| Coincident | The same as a distance of zero: two planar faces on one supporting plane |
| Parallel | The relation between two planar faces' normals, and no translation: two turns |
| Perpendicular | One turn: two planar faces' normals at right angles |
| Width | Parallel faces support all modes. Nonparallel faces support centered bisectors only. For parallel faces: two turns and the translation across the groove, or with `free` the two turns only, the tab kept between the groove's faces |
| Lock | Every motion of one component relative to another: its relative placement is stored |
| Concentric | Two axes on one line (two turns, two translations), a point on an axis (two translations) or two points on one (three translations). With Lock rotation, the turn about the axis too |
| Tangent | Plane-cylinder, plane-sphere, cylinder-cylinder, cylinder-sphere and sphere-sphere contact |
| Angle | A signed angle between two projected directions about an explicit reference direction |
| Limit Angle | The same signed angle, free between inclusive angular stops |
| Hinge | A Concentric between two axes and a Coincident between two planes, under one name: everything but the turn about the axis, optionally between signed angular stops |
| Gear | Rotation of two existing Hinge or unlocked Concentric joints, in a ratio, opposite by default |
| Rack and Pinion | A pinion joint rotation coupled to translation along the rack's selected plane normal or axis |
| Screw | Rotation and axial translation of an unlocked Concentric joint, at a positive pitch |
| Symmetric | One part's plane, line or point the mirror image of another's about a plane: two turns and a translation for planes, two turns and two translations for lines, three translations for points |
| Linear Coupler | One translation: how far one part slides along its plane's normal or axis is a ratio of how far another slides |
| Limit Distance | The turns a Distance fixes; its translation stays free between a least and a greatest distance |
| Profile Center | Two rectangles, regular polygons or circles centred on each other, their planes a distance apart: everything for polygons, everything but the spin for circles unless Lock rotation holds it |
| Slot | A pin's axis in a straight slot: two turns and the translation across the slot, and along it a bound between the ends or a position |

Coincident and Distance also take a plane with an axis or a point: the axis lies parallel to
the plane, or the point sits on it or a distance in front of it. Coincident also takes two
axes, which is the same as a Concentric between them, an axis and a point, and two points.
Parallel and Perpendicular also take two axes.

A mate's planes are faces or reference planes. A reference plane is one of the part's Top,
Front and Right planes, or a plane feature built on one of them. Commands name the root's
own part components. Files can also name part occurrences inside sub-assemblies by path.
Configurations other than the default remain unsupported.

Axis-aligned groups keep the original exact solver, rational measurements, placements and
serialized bytes. The exact path retains its exact-or-refused rule. It never retries a
conflict as a numerical solve. Independent exact groups keep that path when another group
needs bounded motions.

A group with general rotations, an Angle or Limit Angle, a curved-curved Tangent, a
point-point or axis-axis Distance, a mechanical coupling, or enclosed native supports uses the bounded solver.
Every accepted mate in that group reports a `proof` with `certainty: "bounded"`,
`bound_mm: 1e-9`, `bound_angle: 1e-9`, and the certified residuals. The angular bound is in
radians. These are residual bounds at the stored pose, not an exact-coincidence claim or a
bound on distance to a unique solution. Free motions may leave many valid solutions.

The bounded solver reads Catallax's analytic definitions, never display centroids or mesh
vertices. Off-axis or algebraic plane normals, axis directions, explicit points and radii
come as rational enclosures. Its quaternion transform includes the stored binary64
components and their normalization. Outward-rounded arithmetic checks the final pose before
commit, including frame composition for nested occurrences. A successful numerical iteration
alone does not permit a commit.

Locks accept general relative turns. Rotation locks on Concentric, circular Profile Center
and Slot mates keep the legacy cube-matrix representation when possible, otherwise a
quaternion. General turns and offsets carry bounded claims. Existing cube locks keep their
saved representation.

## General motions

`assembly_mate_angle` takes `a`, `b`, `reference` and signed `degrees` in [-180, 180]. Each
endpoint is a plane normal or axis direction. The reference is the positive rotation axis
in its own part's frame. The solver projects the first two directions perpendicular to it
and measures from `a` to `b` by the right-hand rule. A direction parallel to the reference
has no projected angle and is refused by name.

`assembly_mate_limit_angle` takes the same endpoints and `min_degrees` and `max_degrees`.
The limits must be ordered within [-180, 180]. They may include either 180-degree endpoint.
A pose already inside the range stays there. An outside pose starts from its nearest stop.
Limits count the bounded turn as free, as Limit Distance counts its bounded slide.
`assembly_edit_mate` accepts `degrees` or `angle_limits: [min_degrees, max_degrees]`.

Point-point Distance fixes the Euclidean length between the selected points. Axis-axis
Distance keeps their directions parallel according to `alignment` and fixes the length
perpendicular to the first axis. Both use nonnegative lengths and report bounded proofs.
The point-on-axis and coincident-axis forms retain their existing meanings.
At exactly zero distance, point pairs use coincidence equations and parallel axes use
transverse equations, so the freedom count and strict admission keep all constraints.
Positive distances keep their length equations.

Curved Tangent accepts cylindrical and spherical faces with material sides. `alignment`
relates their outward normals at contact. External contact uses the sum of radii; internal
contact uses their absolute difference. Equal-radius internal contact is degenerate and is
refused in favor of Concentric. Parallel cylinders also keep their axes parallel; skew
cylinders use their transverse separation. Edges have no material side and cannot stand in
for a curved face.

For nonparallel Width, Centered aligns the selected pairs' bisector planes and puts them on
one plane. The other modes return `unsupported_reference` with the message
"Width between nonparallel faces supports Centered for now". Free, angular Dimension and
Percent for nonparallel selections are follow-up work. Parallel Width keeps Free, Centered,
Dimension and Percent unchanged for its existing rational local supports. Width selected
from enclosed native supports currently accepts Centered only, including parallel pairs.

The numerical solve uses a fixed iteration order, pure-Rust `libm` trigonometry, at most 120
damped iterations, and the existing 128-body cap. A certified authored pose stays byte for
byte unchanged. Failure to find or enclose a solution returns `uncertified`, naming the
mates. It does not claim the system is over-constrained. A Jacobian whose ranks at 1e-6 and
1e-9 disagree returns `uncertain_rank`. Every refusal leaves truth, placements and undo/redo
history unchanged.

## Move and rotate under mates

Drag the Placement fields to preview a component's translation or rotation while its mates
hold. Release commits one undo step. Escape cancels the preview. Typed placements use the
same solve on acceptance. A fixed component stays at its accepted pose.

`assembly_drag` takes an occurrence `path` and a `placement` in its parent's frame.
A placement has `translation: [x,y,z]` in millimetres and `rotation: [x,y,z,w]`, a unit
quaternion. With the default `phase: "move"` it solves and commits once. For a continuous
drag, send `phase: "begin"` with the path, then `phase: "update"` with each target placement,
then `phase: "commit"` on release or `phase: "cancel"` to restore the accepted pose.
Updates change the displayed occurrence table. They leave saved truth and undo/redo history
unchanged. A refused update retains the previous preview. Saving during a preview saves the
accepted assembly. An ordinary assembly edit cancels the preview before reading placements or capturing history.

Drag targets are wishes. Mates correct constrained motions, while free translations and
turns follow the target. Limit Distance, Limit Angle, free Width and straight Slot bounds
act as inclusive stops. Axis-aligned cases keep the exact solver and its refusals. General
rotations and Hinge stops use the bounded solver and the same certified residual bounds.
Collision while dragging and Mate Controller are outside this version.

Set a Hinge's stops with `assembly_edit_mate` and `angle_limits: [min,max]`, in signed degrees
within [-180,180]. The angle runs about the first selected axis by the right-hand rule.
Each component's zero direction is its local axis crossed with the local coordinate axis
least parallel to it, with X first on ties. This definition comes from the selected analytic
axis, independently of the pick position. Hinges without stops keep their existing bytes.

## Mechanical motion

`assembly_mate_gear` takes `joint_a` and `joint_b`, the names of two existing Hinge or unlocked
axis-axis Concentric mates. `ratio: [a,b]` means the first turns through `a` revolutions while
the second turns through `b`. The default direction is opposite. `reverse: true` makes their
rotations have the same sign. The joints must represent different pairs of components.

`assembly_mate_rack_and_pinion` takes a pinion `joint`, a `rack` endpoint and positive
`travel_per_turn` in millimetres. The rack endpoint is a plane or an axis whose local normal
or direction defines positive travel. `reference` optionally names the component whose origin
is the rack's reference; otherwise the holding assembly's origin is used. Positive travel
goes with positive pinion rotation, and `reverse` changes its sign. Use ordinary mates to
hold the rack's orientation and other translations.

`assembly_mate_screw` takes an unlocked axis-axis Concentric `joint` and positive `pitch` in
millimetres per revolution. Positive rotation about the joint's first axis advances the second
component along that axis. `reverse` reverses this travel. Screw cannot use a Hinge, which
already holds axial travel.

These relations capture the current pose as their initial phase. They reference the source
joint ids and analytic endpoints. Deleting a referenced source joint or enabling a source
Concentric's Lock rotation is refused atomically. `assembly_edit_mate` accepts a Gear's `ratio`,
any mechanical relation's `reverse`,
and a Rack or Screw's `pitch`. Editing these settings captures the current pose as the new
phase, as editing a Linear Coupler's ratio does.

The angle uses the same local transverse frames as Hinge stops. Each relation saves its last
accepted unwrapped angles, so continuous dragging follows multiple revolutions and resumes
that motion after reopen. For nested joints, the parent saves these angles in occurrence
overrides. Each occurrence keeps its own winding, and the child file stays unchanged.
Consecutive target orientations must differ by less than 180 degrees
to identify the intended turn. A quaternion alone cannot distinguish whole revolutions in
one update. A rotation drag can advance a screw axially; a translation drag can turn it.
Gear, Rack and Screw report bounded proofs under B5's residual contract. Their stored file
entry has `kind: "mechanical"` and `mechanism: "gear"`, `"rack_and_pinion"` or `"screw"`.

Linear Coupler remains exact on axis-aligned supports. Dragging either component drives its
partner. Scalar coefficients in bounds are divided out exactly, so a scaled slide can stop
at a Limit Distance. Bounds that cannot reduce to a coordinate or a difference after this
normalization remain refused by name. Exact drag wishes beyond a stop approach the nearest
feasible point along the requested translation; every accepted pose still uses the exact solve.

These are kinematic relations. They do not model teeth, thread contact, collision or dynamics.

## In assembly.json

```json
{
  "mates": [
    { "kind": "origin", "id": 5, "name": "O", "component": 2, "align_axes": true },
    {
      "kind": "distance", "id": 8, "name": "Z",
      "a": { "path": "2", "document": "3f0c...", "face": { "feature": 3, "generator": { "kind": "end_cap" } } },
      "b": { "path": "4", "document": "91ab...", "face": { "feature": 3, "generator": { "kind": "start_cap" } } },
      "alignment": "opposed",
      "distance": { "text": "gap", "tree": { "Ref": 9 } }
    },
    {
      "kind": "coincident", "id": 10, "name": "Cx",
      "a": { "path": "2", "document": "3f0c...", "face": { "feature": 3, "generator": { "kind": "side", "entity": 2 } } },
      "b": { "path": "4", "document": "91ab...", "plane": { "on": "datum", "datum": "right" } },
      "alignment": "aligned"
    },
    {
      "kind": "perpendicular", "id": 11, "name": "R",
      "a": { "path": "2", "document": "3f0c...", "plane": { "on": "feature", "feature": 6, "normal": [0, 0, 1] } },
      "b": { "path": "4", "document": "91ab...", "face": { "feature": 3, "generator": { "kind": "end_cap" } } }
    },
    {
      "kind": "width", "id": 12, "name": "W",
      "groove": [{ "path": "2", "...": "..." }, { "path": "2", "...": "..." }],
      "tab": [{ "path": "4", "...": "..." }, { "path": "4", "...": "..." }],
      "alignment": "aligned",
      "constraint": { "mode": "percent", "percent": 25.0, "flipped": true }
    },
    {
      "kind": "lock", "id": 13, "name": "L",
      "a": { "path": "2", "document": "3f0c..." },
      "b": { "path": "4", "document": "91ab..." },
      "relative": { "rotation": [0.0, 0.0, 0.7071067811865476, 0.7071067811865476], "translation": [10.0, 10.0, 30.0] }
    },
    {
      "kind": "concentric", "id": 14, "name": "C",
      "a": {
        "path": "2", "document": "3f0c...", "face": { "feature": 5, "generator": { "kind": "side", "entity": 1 } },
        "support": { "kind": "axis", "direction": [0, 0, 1], "through": [10.0, 5.0, 0.0], "radius": 4.0, "outward": false }
      },
      "b": {
        "path": "6", "document": "77de...", "face": { "feature": 3, "generator": { "kind": "side", "entity": 1 } },
        "support": { "kind": "axis", "direction": [0, 0, 1], "through": [0.0, 0.0, 0.0], "radius": 4.0, "outward": true }
      },
      "alignment": "aligned",
      "lock_rotation": [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
    },
    {
      "kind": "coincident", "id": 15, "name": "P",
      "a": { "path": "2", "document": "3f0c...", "vertex": { "faces": ["..."], "position": [20.0, 15.0, 10.0] },
             "support": { "kind": "point", "at": [20.0, 15.0, 10.0] } },
      "b": { "path": "4", "document": "91ab...", "edge": { "faces": ["...", "..."], "fingerprint": { "...": "..." } },
             "support": { "kind": "axis", "direction": [1, 0, 0], "through": [0.0, -5.0, 0.0] } },
      "alignment": "aligned"
    },
    {
      "kind": "tangent", "id": 16, "name": "T",
      "a": { "path": "2", "...": "a planar face" },
      "b": { "path": "6", "...": "a cylinder or sphere, with its support" },
      "alignment": "opposed"
    },
    {
      "kind": "hinge", "id": 17, "name": "H",
      "axes": [{ "path": "2", "...": "..." }, { "path": "6", "...": "..." }],
      "planes": [{ "path": "2", "...": "..." }, { "path": "6", "...": "..." }],
      "alignment": "aligned",
      "plane_alignment": "aligned"
    },
    {
      "kind": "symmetric", "id": 18, "name": "S",
      "a": { "path": "4", "...": "a plane, an axis or a point" },
      "b": { "path": "6", "...": "one of the same kind" },
      "plane": { "path": "2", "document": "3f0c...", "plane": { "on": "datum", "datum": "right" } },
      "alignment": "aligned"
    },
    {
      "kind": "linear_coupler", "id": 19, "name": "K",
      "a": { "path": "4", "...": "a plane or an axis" },
      "b": { "path": "6", "...": "a plane or an axis" },
      "reference_a": { "path": "2", "document": "3f0c..." },
      "ratio": [1.0, 2.0],
      "reverse": true,
      "offset": "-381/5"
    },
    {
      "kind": "limit_distance", "id": 20, "name": "L",
      "a": { "path": "2", "...": "a plane" },
      "b": { "path": "4", "...": "a plane, an axis or a point" },
      "alignment": "opposed",
      "min": { "text": "5 mm", "tree": { "Number": [5.0, 1, 0] } },
      "max": { "text": "1 cm", "tree": { "Number": [10.0, 1, 0] } }
    },
    {
      "kind": "profile_center", "id": 21, "name": "P",
      "a": { "path": "2", "document": "3f0c...", "face": { "...": "..." },
             "support": { "kind": "profile", "normal": [0, 0, 1], "centre": [0.0, 0.0, 10.0], "sides": 4 } },
      "b": { "path": "4", "document": "91ab...", "face": { "...": "..." },
             "support": { "kind": "profile", "normal": [0, 0, -1], "centre": [0.0, 0.0, 0.0], "sides": 4 } },
      "alignment": "opposed",
      "distance": { "text": "2 mm", "tree": { "Number": [2.0, 1, 0] } }
    },
    {
      "kind": "slot", "id": 22, "name": "SL",
      "slot": { "path": "2", "document": "3f0c...", "face": { "...": "a wall or a round end" },
                "support": { "kind": "slot", "depth": [0, 0, 1], "ends": [[-11.0, 0.0, 0.0], [11.0, 0.0, 0.0]], "radius": 4.0 } },
      "pin": { "path": "4", "...": "an axis" },
      "alignment": "aligned",
      "constraint": { "mode": "distance", "distance": { "text": "4 mm", "tree": { "Number": [4.0, 1, 0] } }, "flipped": true },
      "lock_rotation": [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
    }
  ],
  "values": [
    { "id": 9, "name": "gap", "expression": { "text": "1.2 cm", "tree": { "Number": [12.0, 1, 0] } } }
  ]
}
```

- **Endpoints** name a face by occurrence path, part document, optional configuration and
  the part's persistent `FaceRef`, which is a feature id and what that feature made. Labels
  and kernel handles are never saved as identities. The face reference's own label and
  fingerprint are kept as written; only a repair message reads them.
- **Edge and vertex endpoints** carry `edge` or `vertex` instead of `face`. An edge is named
  by the two faces it separates, as a fillet names one. A vertex is named by every face that
  meets at it, and resolves to the one vertex all of them touch; its `position` is kept for
  messages only.
- **Supports.** An endpoint read as an axis or a point keeps what its proof found when the
  mate was made: the axis's signed direction and a point on it, or the point, with a radius
  and its material side where there is one. They are the nearest doubles and only the
  solver's status reads them, so the status needs no part. A solve reads exact values from a
  new proof, and a proof whose kind or direction differs from the stored one is refused as
  `dangling_reference`. A profile keeps its plane's normal, its centre and a polygon's number
  of sides; a slot keeps its depth, its two ends' centres and its radius. A planar face keeps
  none.
- **Plane endpoints** carry `plane` instead of `face`: a datum by name, or a plane feature by
  id with the normal it had when it was mated. A plane feature that now faces another way is
  refused as `dangling_reference`, as a face whose normal changed is. A plane feature built on
  a face is refused as `unsupported_plane`; mate the face itself.
- **Alignment** is the vector relation between the two outward normals: `aligned` when they
  agree, `opposed` when they oppose. The window may call one of them flipped. The saved word
  is always one of these two.
- **Distance** is signed, measured from `a`'s plane toward `b`'s along `a`'s outward normal.
  These are supporting planes: the faces need not overlap, so this is not the minimum
  distance between two trimmed faces unless their projections overlap.
- **Coincident** puts `b`'s plane on `a`'s, with the normals as `alignment` says. It removes
  the two turns and the one translation a zero distance removes; it does not line up origins,
  hold the faces' in-plane positions or their spin about the normal. It stores no distance
  and has no expression, so it cannot be driven, and a distance edit on it is refused as
  `invalid`. The solver takes it as a distance of zero. A distance of zero stays a distance:
  nothing rewrites one family as the other.
- **Parallel** keeps `b`'s plane parallel to `a`'s, the normals as `alignment` says. It stores
  no distance. Its measurement is the signed gap, as a Distance's is.
- **Perpendicular** keeps the two normals at right angles. It has no alignment, because the
  normals are at right angles either way.
- **Width** names the groove's two faces, on one part, and the tab's two, on another. The four
  use all modes when each pair is parallel. Nonparallel selections use centered bisectors
  only, as described above. `alignment` relates
  `tab[0]`'s normal to `groove[0]`'s; flipping it turns the tab round. `constraint` is
  `free`, which keeps the tab's faces between the groove's and fixes no translation;
  `centered`, which puts the tab's mid-plane on the groove's; `dimension`, the gap from
  `groove[0]` to the tab face nearest it; or `percent`, that gap as a share of the play, the
  groove's width less the tab's, so 0 is against `groove[0]`, 50 is centred and 100 is
  against `groove[1]`. With `flipped` the gap and the share are measured from `groove[1]`.
  A dimension is an expression and can be driven like a distance. The mid-plane of the
  groove and of the tab are averages of exact offsets, so a width is one exact difference.
- **Concentric** reads each end as an axis or a point. Between two axes `alignment` relates
  their directions, as a cylinder's or circle's direction is the kernel's own; it is absent
  where a point is involved. `lock_rotation`, when present, is `b`'s turn in `a`'s frame as
  a cube matrix by rows or a bounded quaternion, taken where the solve leaves it
  when Lock rotation is turned on. It is allowed between two axes only. Planes are refused.
- **Tangent** reads a plane and a cylinder or sphere, or two curved faces as described above.
  On the exact plane-curved path, The radius is the
  kernel's square root of its squared radius, which must be a rational square, or the mate is
  refused as `unsupported_reference` with the square it found. `alignment` relates the
  curved face's outward normal at the contact to the plane's: `opposed` for a shaft lying on a
  table, `aligned` for one pressed against the plane from behind it. A bore's wall looks
  towards its axis, so the side flips for it.
- **Hinge** reads `axes` as a Concentric between two axes and `planes` as a Coincident, the
  first of each on one part and the second on the other. `alignment` relates the axes and
  `plane_alignment` the planes. Angle limits are not supported.
- **Coincident and Distance with an axis or a point** put the point, or the axis through its
  point, at the signed distance along the plane's outward normal, whichever end the plane is;
  an axis also lies parallel to the plane. Their `alignment` is kept but reads nothing. A
  Distance between two points fixes their Euclidean length. Between axes it keeps their
  directions parallel and fixes the transverse length. These two forms are bounded.
- **Symmetric** reads `a` and `b` as two planes, two axes or two points, of one kind, on two
  parts, and `plane` as a planar face or reference plane of any part. `b` is `a` mirrored
  about it. `alignment` relates `b`'s outward normal or direction to `a`'s mirrored: `aligned`
  for two faces that look away from each other across the mirror, as a mirrored part's do.
  It is absent for points. A plane or line square to the mirror is its own image, so `b`
  lies on `a`'s and the mirror may slide along its normal. Two cylinders or spheres must have
  equal radii.
- **Linear Coupler** reads `a` and `b` as planes or axes. Each part slides along its plane's
  outward normal or its axis's direction, measured from `reference_a` or `reference_b`'s
  origin when one is named, else from the origin of the assembly that holds the mate.
  `ratio[0]` of `a`'s slide goes with `ratio[1]` of `b`'s, the other way with `reverse`.
  `offset` is `ratio[0]` times how far `b` lies along its direction, less (plus, with
  `reverse`) `ratio[1]` times how far `a` does, taken where the parts are when it is made and
  written as an exact rational. Editing the ratio or `reverse` takes it again where the parts
  are then. The two directions may lie on different world axes.
- **Limit Distance** reads its ends as a Distance does: a plane, and a plane, an axis or a
  point. The signed distance from `a`'s plane may be anywhere from `min` to `max`, literal
  lengths with `min` no more than `max`. Planes are kept parallel as `alignment` says.
- **Profile Center** reads two profiles of one kind: two circles, or two polygons with as many
  sides. `b`'s centre goes on the line through `a`'s along `a`'s plane normal, and `b`'s plane
  `distance` from `a`'s, signed along `a`'s outward normal, the normals as `alignment` says.
  A polygon's edges line up with the other's: the solve takes the quarter turn about the
  normal nearest the authored one that does. Two circles keep their spin unless
  `lock_rotation` holds `b`'s turn in `a`'s frame, as a Concentric's does. The `distance` is
  an expression and can be driven.
- **Slot** reads `slot` as a straight slot and `pin` as an axis. The pin is parallel to the
  slot's round ends' axes, its direction as `alignment` says, and on the slot's mid-plane.
  Along the slot `constraint` is `free`, anywhere from the first end's centre to the second's;
  `center`, halfway; `distance`, that far from the first end's centre towards the second, or
  from the second with `flipped`; or `percent` of the run between them. `lock_rotation` holds
  the pin's turn in the slot part's frame. A distance is an expression and can be driven.
- **Lock** stores `b`'s placement in `a`'s frame, taken from where both are when it is added.
  Quarter turns use the existing exact integer-matrix and rational-offset path. General
  relative placements use the bounded path.
- **Values** use the part's expression grammar, units, decimal conversion and cycle checks
  (`soliddraft_model::ValueTable`). There is one evaluator. A value keeps its text as typed.
- **A driver** is a distance whose expression reads a value. It reads by stable id, so
  renaming the value changes the text and nothing else. A distance has one expression, so
  it has one driver; replacing it is an explicit request.
- **Mates of a later version** are kept exactly as read. While one is present, `solver.state`
  is `error` and nothing that would solve can commit, Update included.

The geometry hash leaves out `mates` and `values`, because the placements they decide are
hashed. An assembly with neither writes neither key, so old files keep their hashes.

## Solving

In the exact tier, plane normals are signed axes, which splits a solve in two:

1. **Rotations.** Parts joined by Locks, or by a Concentric with Lock rotation, turn as one
   rigid group, so the unknowns are the groups' turns. An Origin mate with axes fixes a
   group's turn. Each mate between planes relates two normals: equal or opposite for a
   Distance, Coincident, Parallel or Width, at right angles for a Perpendicular. Two axes
   relate their directions the same way, and an axis held in or at a distance from a plane,
   or a Tangent cylinder, lies at right angles to the plane's normal. A Symmetric relates
   three groups: `b`'s normal or direction is `a`'s mirrored across the mirror's normal, by
   its alignment. A Slot relates the pin's direction to the slot's depth, and a Profile
   Center between polygons asks that some edge of `b` lie along some edge of `a`. Every turn the exact solver can prove takes the normals
   it reads onto axes, so each group's candidates are the 24 quarter-turn rotations. One
   unlocked part held along a single axis may also keep its own spin about that axis, unless
   a mate places one of its axes or points: then it takes quarter turns only. A group no
   mate turns keeps its authored turn, unless a lock in it places parts by that turn; it
   then tries its authored turn and then every quarter turn. All
   groups are solved together, as a search over those candidates. Each group tries its
   candidates nearest its authored turn first, in a fixed order among equals, so an
   authored pose that already holds is kept and one that does not moves by the smallest
   turn that works. The answer does not depend on the order the mates were added. When no
   assignment exists, the refusal names a smallest set of mates that still cannot hold
   together. The search refuses as `too_large` past 200,000 steps.
2. **Translations.** Each distance, width and lock becomes an exact difference of one world
   coordinate, so each axis is a graph of differences. A lock adds one on each axis. Two axes
   on one line add one on each of the two world axes across their direction, a point on an
   axis does the same, two points on one add one on each world axis, and a point or axis at a
   distance from a plane adds one along the plane's normal. A Slot adds one across the slot
   and, with a position, one along it; a Profile Center adds a distance and a point on an axis;
   a Symmetric between points adds one on each world axis along the mirror. Each reads the
   quarter-turn matrix of its part's turn, so the difference stays rational. Walking the graph
   places everything connected to the origin, or to the group's first component. An edge that
   disagrees by more than 1e-9 mm is a conflict.

   Some relations are not differences. A Symmetric puts `a + b = 2 s` along the mirror's
   normal, three parts at once, and a Linear Coupler relates two parts by a ratio, across two
   world axes when their directions differ. These are solved over the graphs' roots by exact
   elimination in rationals, the unknowns ordered so that a later component's root moves
   before an earlier one's; whatever stays free keeps its authored place. A relation that
   reduces to a nonzero constant is a conflict, named with the mates it came from.

   Bounds come last: a Limit Distance, a free Width and a free Slot each ask `lo <= t_b - t_a
   <= hi` along one world axis. Over what elimination leaves free these are difference
   constraints, decided exactly by Bellman-Ford; a negative cycle is a conflict that names its
   mates. Each free coordinate then keeps its authored value where the bounds allow it and
   otherwise takes the nearest value they allow, earlier components first, so the same file
   always gives the same placements. Scalar coefficients are divided out exactly. A bound
   that still cannot reduce to a coordinate or a difference is refused as `unsupported_reference`.

The two steps are solved together. Which axis a distance's planes face decides which
coordinate it moves, so a turn that keeps every normal right can still leave a distance
that cannot hold, while another turn holds it exactly. The search therefore accepts turns
only when the translations they leave also hold, checking each mate's translation as soon
as its parts' turns are chosen, and backtracks otherwise. While some mates still wait for
their parts' turns, a bound that reads a ratio is left out of that check, because a mate not
yet in may make it a difference or a constant; it is refused only if it cannot reduce to a coordinate or a difference
once every mate is in. Turns that take a group's mated
normals onto the same axes decide the same, so only one of them is tried. An assembly is
over-constrained only when no supported turns work. The refusal then names a smallest set
of mates that cannot hold together under any of them, with what each mate asks in the pose
nearest the authored one.

An unconstrained coordinate keeps its authored value. A group with no Origin mate keeps its
first component where it is, because something has to stay put, and it still reports its
freedoms. Nothing is ever a hidden fixed flag.

The root solves its own mates and those of every sub-assembly in it, each with the
sub-assembly's path in front, over part occurrences at any depth. A sub-assembly's frame
stays where it is authored; a part inside it that a solve moves is kept as an override in
the root, never written to the sub-assembly's file.

A bound fixes no motion: a Limit Distance, a free Width or a free Slot counts the slide it
bounds as free.

Freedoms come from the rank of the rigid-body constraint Jacobian at the solved pose, so a
shared rotation about an anchored origin counts. Every part occurrence that moves on its own
has six, wherever it is in the tree; a pattern member follows its seed. A partial mate is valid: mates go on one at
a time. `require_fully_constrained` refuses a result with any freedom left.

## Commands

| Command | What it does |
|---|---|
| `insert_component` | Places a file, optionally at a placement |
| `assembly_measure_planes` | Reads the proven signed distance between two faces' planes |
| `assembly_mate_origin` | Adds an Origin mate |
| `assembly_mate_distance` | Adds a distance mate |
| `assembly_mate_coincident` | Puts two planar faces on one plane, `aligned` or `opposed` |
| `assembly_mate_parallel` | Keeps two planar faces parallel, `aligned` or `opposed` |
| `assembly_mate_perpendicular` | Keeps two planar faces at right angles |
| `assembly_mate_width` | Holds a tab's two faces between a groove's, `centered`, by `dimension` or by `percent` |
| `assembly_mate_lock` | Locks a component where it is relative to another |
| `assembly_mate_concentric` | Puts two axes on one line, a point on an axis or two points on one; `alignment` between axes, optional `lock_rotation` |
| `assembly_mate_tangent` | Plane-curved or curved-curved contact, `aligned` or `opposed` |
| `assembly_mate_angle` | Sets signed `degrees` about an explicit `reference` direction |
| `assembly_mate_limit_angle` | Bounds that angle by `min_degrees` and `max_degrees` |
| `assembly_mate_hinge` | Puts two axes on one line and two planes on one plane, under one name |
| `assembly_mate_symmetric` | Mirrors `a` onto `b` about `plane`; `alignment` between planes or axes |
| `assembly_mate_gear` | Couples two joint rotations by a ratio |
| `assembly_mate_rack_and_pinion` | Couples a pinion joint to rack translation in millimetres per revolution |
| `assembly_mate_screw` | Couples a Concentric joint's rotation and axial travel by pitch |
| `assembly_drag` | Begins, updates, commits or cancels a placement preview, or performs one complete move |
| `assembly_mate_linear_coupler` | Couples `b`'s slide to `a`'s by `ratio`, with `reverse` and optional `reference_a` and `reference_b` components |
| `assembly_mate_limit_distance` | Holds a distance between `min` and `max`, `aligned` or `opposed` |
| `assembly_mate_profile_center` | Centres two profiles `distance` apart; `lock_rotation` for circles |
| `assembly_mate_slot` | Puts `pin` in `slot`: `free`, `center`, `distance` or `percent`, with `flipped` and `lock_rotation` |
| `assembly_edit_mate` | Changes a literal distance, an alignment, `align_axes`, a width's or slot's `percent` or `flipped`, a hinge's `plane_alignment`, a `lock_rotation`, a limit's `min` or `max`, or a coupler's `ratio` or `reverse` |
| `assembly_delete_mate` | Removes a mate; components stay where they are |
| `assembly_set_value` | Adds or changes a value |
| `assembly_rename_value` | Renames a value |
| `assembly_drive_mate` | Binds a distance to values |
| `assembly_update` | Accepts the current source revisions and solves on them |

A face is named by its label, or by `feature`, `generator` and, for a side, `normal`. A
reference plane is named by `plane`: "Top plane", "Front plane", "Right plane" (or the name
the part gave its datum) or a plane feature's name. An edge is named by `edge`, the labels of
the two faces it separates, and a vertex by `vertex`, the labels of the faces that meet there.
Each must find exactly one face, plane, edge or vertex. The stored result is the `FaceRef`,
the plane's reference, the `EdgeRef` or the `VertexRef`, with the support its proof found.

Every change runs one transaction: copy the truth, change the copy, prove and solve it, then
publish truth, placements and display as one undo step. A refusal changes nothing and keeps
the redo stack. Refusals are JSON: `refused` holds the code, with `message`, `mates`,
`remaining_dof` and `free` where they apply.

| Code | Why |
|---|---|
| `under_constrained` | A strict request left freedom |
| `over_constrained` | The mates cannot all hold |
| `dangling_reference` | A stored face is gone at the proposed revision |
| `ambiguous_reference` | A selector or reference finds several faces |
| `no_such_face` | A selector finds nothing |
| `unsupported_configuration` | A mate or its source names a configuration other than the default |
| `unsupported_plane` | The kernel will not prove this face's plane |
| `unsupported_reference` | A support or pair is outside the family's contract, a profile or slot lacks a proof, a nonparallel Width uses a mode other than Centered, or an exact bound cannot reduce to a coordinate or a difference |
| `unsupported_placement` | The exact tier cannot prove the requested placement |
| `uncertified` | The bounded solver could not find or enclose an acceptable stored pose |
| `uncertain_rank` | The numerical constraint rank is uncertain near a singular pose |
| `unsupported_motion` | A mechanical relation references an unsupported or locked joint |
| `not_parallel` | The two planes are not parallel, or a Width's groove or tab faces are not |
| `proof_unavailable` | The pinned revision's bytes or a kernel are not available |
| `unknown_mate` | The assembly holds a mate this version cannot solve |
| `driven_input` | A literal edit of a driven distance |
| `already_driven` | A second driver, without `replace` |
| `unit_mismatch`, `value_cycle`, `invalid_value` | The expression cannot be used, a percentage is outside 0 to 100, a coupler's ratio is not positive, or a limit's least distance passes its greatest |
| `too_large` | More than 128 mated components, or turns that take too long to decide |
| `invalid` | A request that names nothing usable, such as a distance edit of a mate without one, or Lock rotation on a polygon's Profile Center |

## The Mate panel

The panel offers every family above except Origin, which has its own button for the selected
component. Coincident, Distance, Parallel, Perpendicular, Concentric, Linear Coupler, Limit
Distance and Profile Center take two picked faces, and Tangent takes the plane, then the
cylinder or sphere. Slot takes a wall or round end of the slot, then the pin. Symmetric takes
three: the face, its mirror image, then the symmetry plane. Angle and Limit Angle also take
three: two directions followed by the positive reference direction. Width takes four: the groove's two,
then the tab's. Hinge takes four too: the two axes' faces, then the two planes. Lock takes one
face on each component. Alignment shows for the families that have one; Hinge also shows its
planes' alignment, and Concentric, Profile Center and Slot a Lock rotation box. Width and Slot
choose where the tab or pin sits, Limit Distance takes its least and greatest distances, and
Linear Coupler its ratio and Reverse. The panel picks faces; edges, vertices and a coupler's
reference components are named through the MCP tools.

The Conflicts list shows the last refusal that named mates or freedoms, from a mate command or
from Update. It says what kind of refusal it was, lists each named mate with its family, and
offers Delete for it and Flip when it has an alignment. A refusal that left freedom lists the
free motions. The list reads only what the refusal already carries; it solves nothing of its
own. A success clears it.

## Proofs and Update

A proof belongs to a document, a revision and a face, edge or vertex. It comes from the part
rebuilt from that revision's bytes on the kernel. `Kernel::entity_support` reads supported
rational, axis-aligned definitions exactly, without numerical work. The bounded path uses
`Kernel::entity_support_enclosed` for off-axis or algebraic analytic definitions. Its rational
endpoints enclose each native coordinate to a requested width of 2^-60. A profile or slot
still needs its existing topology proof. Torus and spline supports and intersection-only
vertices remain refused by name.

Exact gaps use rational local offsets and stored placements. `measurement.rational` holds
the exact value and `lo` and `hi` round outward. A bounded plane-gap measurement has
`certainty: "bounded"`, an outward enclosure, and no `rational` claim. Mate residual proofs
are separate: `solver.bounded` lists every certified mate in a bounded connected group, and
MCP mate rows copy the corresponding entry to `proof`. The Mate panel shows its bounds.

Editing or saving a part moves nothing. The accepted pin, pose and proof stand, and a
measurement whose source moved on says `retained`. Update is the only way forward: it loads
every source's proposed revision, proves the mates on them, solves, and commits pins, display
and pose together. A face whose feature is gone refuses Update even when a new feature made an
identical box with the same label, because the reference is to the feature, not the shape.

A proof needs the pinned revision's bytes. They come from the file while it is at that
revision, otherwise from the session's revision store. A file opened after its part moved on
still shows, solves nothing and reports `proof_unavailable` per mate until Update.

## Limits for later batches

Path, Cam, arc Slot, Universal Joint and Physical Dynamics remain unsupported. Collision
while dragging and Mate Controller are deferred. Mechanical relations add kinematics without
contact simulation.

Nonparallel Width Free, angular Dimension and Percent; Distance limits across two points or
two axes; the misaligned Concentric option; Parallel or Perpendicular between a plane and an
axis; arc slots and a slot in a slot; profiles or slots whose local topology cannot yet be
proved; rounded or chamfered profiles; panel picking of edges and vertices; new mates picked
inside sub-assemblies; pattern members; configurations; torus and spline supports; and
vertices defined only as intersections remain follow-up work.

B5 scenarios cover signed Angle, Limit Angle, curved Tangent, Euclidean distances,
nonparallel centered Width, and the earlier mate families on rotated parts. The exact
regression scenarios continue to check axis-aligned placements and rational measurements.

## Static assembly checks

The Assembly tab's Interference button opens the check panel. Interference Detection and
Clearance Verification use certified meshes from the pinned part revisions. Each result
says clear, interferes, or touching or undecided; clearance results state met, below, or
undecided. Exact overlap volumes and exact touching without a mate remain follow-up Catallax
work. See [interference.md](interference.md) for the mesh bounds and refusals.

Choose Hole alignment in the same panel to inspect internal cylindrical axes, including
native cylinders in imported parts. Set the maximum lateral deviation and check the whole
assembly or chosen components. Results identify coaxial axes as aligned and parallel axes
with nonzero offsets within the maximum as misaligned. Clicking a row selects both parts.
The command is `assembly_check_hole_alignment`, with `max_deviation` as a length, optional
`paths`, and `include_hidden`. The check changes no placements, mates, pins or undo history.

Hole results use the `exact_axes` tier. Cylinder definitions and the stored placements are
read as rationals, so zero offset and the maximum-deviation boundary are exact comparisons.
Only axes exactly parallel to a world axis are supported. Nonparallel pairs and pairs beyond
the maximum are counted separately. Coaxial faces of one part share a single axis entry.
External cylinders are excluded. An unsupported cylinder, configuration or unavailable pin
marks its part unchecked and the report incomplete. Suppressed parts are excluded; hidden
parts are optional. One check takes at most 128 parts and 512 distinct bore axes.

This compares infinite bore axes, so separated holes on one line are aligned regardless of
their depth. It does not certify hole fit, diameter, fastener passage, angular deviation or
contact. Collision while dragging belongs to a later batch.
