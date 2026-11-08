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
| Distance | The relation between two planar faces' normals, and one translation |
| Coincident | The same as a distance of zero: two planar faces on one supporting plane |
| Parallel | The relation between two planar faces' normals, and no translation: two turns |
| Perpendicular | One turn: two planar faces' normals at right angles |
| Width | A tab's two parallel faces between a groove's two parallel faces: two turns and the translation across the groove, or with `free` the two turns only, the tab kept between the groove's faces |
| Lock | Every motion of one component relative to another: its relative placement is stored |
| Concentric | Two axes on one line (two turns, two translations), a point on an axis (two translations) or two points on one (three translations). With Lock rotation, the turn about the axis too |
| Tangent | A cylinder lying against a plane (one turn, one translation) or a sphere touching it (one translation), one radius away |
| Hinge | A Concentric between two axes and a Coincident between two planes, under one name: everything but the turn about the axis |
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
Front and Right planes, or a plane feature built on one of them, with its offsets. Faces and
planes must be axis-aligned with rational offsets, on parts in the default configuration.
The commands below name the root's own part components; a mate read from a file may also
name a part inside a sub-assembly by its path, as forming one leaves it (see
[subassemblies.md](subassemblies.md)). A mate may turn a part nothing else holds onto the
axes, as it turns any part held by one face. A face that stays off the world axes is
refused: one measured where it is placed, or one a frame fixes there. A placement is on the
axes only when its stored quaternion, read as the rationals its binary64 parts are, takes the
normal exactly onto an axis. A turn within 1e-12 of a quarter turn is stored as that quarter
turn's own quaternion whenever a placement is made, composed or read from a file, and that
form is exact, since its nonzero parts share one magnitude. A spin about the face's own
normal is exact too. A turn that lands only near an axis is not on it: far from the origin a
tilt of 2^-42 is a real gap. It is refused when measured, and a solve turns the part exactly
onto the axes instead of keeping the tilt. A later general rotation needs a bounded transform contract and does not
inherit this one.

An axis is read from a cylindrical or conical face, a circular edge or a straight edge, and a
point from a spherical face's centre or a vertex. Each is read from the kernel's own definition
(see [Proofs and Update](#proofs-and-update)): its direction must lie on a world axis and its
coordinates must be rational. A hole copied by a circular pattern at anything but quarter turns
has an algebraic centre, so a mate on it is refused as `unsupported_reference`, never rounded.
A part whose axis or point a mate places takes quarter-turn rotations only, so the coordinates
it reads stay rational: the solve keeps its authored turn when that is one, and otherwise turns
it onto the nearest. A part held by planes or by an axis's direction alone keeps its own spin,
as before.

A Lock's relative rotation must be a quarter-turn rotation, one that takes axes to axes. A
Lock between parts turned 30 degrees apart is refused as `unsupported_placement`, never
rounded, and so is one whose first part is turned off the quarter turns, because its offset
would no longer be an exact difference along the world axes.

A profile is a planar face's outline or a circular edge. The outline is the face's outer loop;
holes inside it are ignored. It is a circle when its edges are one circle, whole or in arcs, and
its centre is the circle's. It is a polygon when its edges are straight: a rectangle when it has
four corners and each side is exactly square to the next, a regular polygon when its sides, and
its corners' distances from their average, agree to 1e-9 of their size. A sketched hexagon's
corners are rounded, so regularity is read to that tolerance; the centre is still exact, the
average of the corners as the part stores them, in rationals. An outline that mixes lines and
arcs, such as a rounded or chamfered rectangle, or one with straight sides that is neither, is
refused as `unsupported_reference`. A straight edge does not say which face it bounds, so it
is refused; mate the face.

A slot is found from a wall or a round end. The round ends' axes give its depth; walking from
the picked face across straight edges along that depth gives the faces round it. They must be
two parallel planar walls and two half-cylinders of one radius, with the walls exactly two radii
apart and each end's axis exactly on their mid-plane. The ends' centres, ordered along the
slot's run, are its first and second end. Arc slots, whose walls are round, slots off the world
axes, and any other ring of faces are refused as `unsupported_reference`.

A file may carry a configuration on an endpoint or on the reference a mate reads. It is kept as
read and refused as `unsupported_configuration`: nothing solves on the default instead, and an
unmated reference is left alone.

Anything else is refused by name. Nothing falls back to a mesh, a bounding box or a displayed
centroid.

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
  must be parallel planes, or the mate is refused as `not_parallel`. `alignment` relates
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
  the integer matrix of a quarter-turn rotation, by rows, taken where the solve leaves it
  when Lock rotation is turned on. It is allowed between two axes only. Planes are refused.
- **Tangent** reads `a` as a plane and `b` as a cylinder or a sphere. The radius is the
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
  Distance between two axes or two points fixes a length across them, which this version does
  not solve exactly, so it is refused as `unsupported_reference`.
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
  Both parts must be on exact quarter turns. The rotation is the product of their integer
  matrices, and the offset is taken through the first one's, so no rounding enters the
  relation.
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

Plane normals are exact signed axes, which splits a solve in two:

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
   always gives the same placements. A bound on a coordinate a relation moves by a ratio other
   than one is not a difference and is refused as `unsupported_reference`.

The two steps are solved together. Which axis a distance's planes face decides which
coordinate it moves, so a turn that keeps every normal right can still leave a distance
that cannot hold, while another turn holds it exactly. The search therefore accepts turns
only when the translations they leave also hold, checking each mate's translation as soon
as its parts' turns are chosen, and backtracks otherwise. Turns that take a group's mated
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
| `assembly_mate_tangent` | Lays a cylinder or sphere against a plane, `aligned` or `opposed` |
| `assembly_mate_hinge` | Puts two axes on one line and two planes on one plane, under one name |
| `assembly_mate_symmetric` | Mirrors `a` onto `b` about `plane`; `alignment` between planes or axes |
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
| `unsupported_reference` | The kernel will not prove this axis, point, profile or slot (off the world axes, not rational, or not a rectangle, regular polygon, circle or straight slot), the family does not take this pair, such as a Distance between two points, or a bound falls on a coordinate a relation scales |
| `unsupported_placement` | A placed normal is off the world axes, or a Lock's or Lock rotation's turn is off the quarter turns |
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
three: the face, its mirror image, then the symmetry plane. Width takes four: the groove's two,
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
rebuilt from that revision's bytes on the kernel, through `Kernel::entity_support`, which reads
the native definition exactly and costs no numerical work: a plane's equation, a cylinder's
axis origin, direction and squared radius, a cone's apex and direction, a sphere's centre and
radius, a circle's centre, normal and squared radius, a line's origin and direction, or a
vertex's explicit point. A profile or a slot is read from the part's topology, its faces,
edges and vertices, each proved the same way. A value that is not structurally rational, a
direction off the world axes, or a vertex defined only where surfaces meet is refused with
`unsupported_reference`, and the message names the part and the face, edge or vertex: a
hole copied 60 degrees round an axis reads `tabs.dwprt, CirPattern1 · Copy 1 face 7: ... the
axis of ... is not structurally rational`. Gaps are computed in rationals from those offsets and the stored
placements. `measurement.rational` holds the exact value;
`lo` and `hi` are rounded outward. A Coincident is measured the same way, and reads `"0"` while
it holds.

Editing or saving a part moves nothing. The accepted pin, pose and proof stand, and a
measurement whose source moved on says `retained`. Update is the only way forward: it loads
every source's proposed revision, proves the mates on them, solves, and commits pins, display
and pose together. A face whose feature is gone refuses Update even when a new feature made an
identical box with the same label, because the reference is to the feature, not the shape.

A proof needs the pinned revision's bytes. They come from the file while it is at that
revision, otherwise from the session's revision store. A file opened after its part moved on
still shows, solves nothing and reports `proof_unavailable` per mate until Update.

## Limits for later batches

Angle and Limit Angle, hinge limits and the mechanical families; Tangent between two curved
faces, and Distance or Limit Distance between two axes or two points; Width between faces
that are not parallel; the misaligned Concentric option, which needs an explicit tolerance;
Parallel or Perpendicular between a plane and an axis; arc slots, slots off the world axes and
a slot in a slot; rounded or chamfered profiles; a bound on a coordinate a Symmetric or Linear
Coupler moves by a ratio; picking edges and vertices in the Mate panel; picking a face inside a
sub-assembly for a new mate; mates on pattern members; configurations; dragging under mates;
general rotations, including a Lock off the quarter turns, every family on parts placed off
the world axes, and axes, points or profiles read off the world axes or at algebraic
coordinates; torus and spline faces. See M6 in the plan.
