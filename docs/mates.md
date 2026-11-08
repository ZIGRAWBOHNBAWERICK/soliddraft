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
| Width | A tab's two parallel faces between a groove's two parallel faces: two turns and the translation across the groove |
| Lock | Every motion of one component relative to another: its relative placement is stored |

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

A Lock's relative rotation must be a quarter-turn rotation, one that takes axes to axes. A
Lock between parts turned 30 degrees apart is refused as `unsupported_placement`, never
rounded, and so is one whose first part is turned off the quarter turns, because its offset
would no longer be an exact difference along the world axes.

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
  `centered`, which puts the tab's mid-plane on the groove's; `dimension`, the gap from
  `groove[0]` to the tab face nearest it; or `percent`, that gap as a share of the play, the
  groove's width less the tab's, so 0 is against `groove[0]`, 50 is centred and 100 is
  against `groove[1]`. With `flipped` the gap and the share are measured from `groove[1]`.
  A dimension is an expression and can be driven like a distance. The mid-plane of the
  groove and of the tab are averages of exact offsets, so a width is one exact difference.
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

1. **Rotations.** Parts joined by Locks turn as one rigid group, so the unknowns are the
   groups' turns. An Origin mate with axes fixes a group's turn. Each mate between planes
   relates two normals: equal or opposite for a Distance, Coincident, Parallel or Width, at
   right angles for a Perpendicular. Every turn the exact solver can prove takes the normals
   it reads onto axes, so each group's candidates are the 24 quarter-turn rotations. One
   unlocked part held along a single axis may also keep its own spin about that axis. All
   groups are solved together, as a search over those candidates. Each group tries its
   candidates nearest its authored turn first, in a fixed order among equals, so an
   authored pose that already holds is kept and one that does not moves by the smallest
   turn that works. The answer does not depend on the order the mates were added. When no
   assignment exists, the refusal names a smallest set of mates that still cannot hold
   together. The search refuses as `too_large` past 200,000 steps.
2. **Translations.** Each distance, width and lock becomes an exact difference of one world
   coordinate, so each axis is a graph of differences. A lock adds one on each axis. Walking
   the graph places everything connected to the origin, or to the group's first component.
   An edge that disagrees by more than 1e-9 mm is a conflict.

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
| `assembly_edit_mate` | Changes a literal distance, an alignment, `align_axes`, a width's `percent` or `flipped` |
| `assembly_delete_mate` | Removes a mate; components stay where they are |
| `assembly_set_value` | Adds or changes a value |
| `assembly_rename_value` | Renames a value |
| `assembly_drive_mate` | Binds a distance to values |
| `assembly_update` | Accepts the current source revisions and solves on them |

A face is named by its label, or by `feature`, `generator` and, for a side, `normal`. A
reference plane is named by `plane`: "Top plane", "Front plane", "Right plane" (or the name
the part gave its datum) or a plane feature's name. Each must find exactly one face or plane.
The stored result is the `FaceRef` or the plane's reference.

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
| `unsupported_placement` | A placed normal is off the world axes, or a Lock's turn is off the quarter turns |
| `not_parallel` | The two planes are not parallel, or a Width's groove or tab faces are not |
| `proof_unavailable` | The pinned revision's bytes or a kernel are not available |
| `unknown_mate` | The assembly holds a mate this version cannot solve |
| `driven_input` | A literal edit of a driven distance |
| `already_driven` | A second driver, without `replace` |
| `unit_mismatch`, `value_cycle`, `invalid_value` | The expression cannot be used |
| `too_large` | More than 128 mated components, or turns that take too long to decide |
| `invalid` | A request that names nothing usable, such as a distance edit of a mate without one |

## The Mate panel

The panel offers every family above except Origin, which has its own button for the selected
component. Coincident, Distance, Parallel and Perpendicular take two picked faces. Width takes
four: the groove's two, then the tab's. Lock takes one face on each component. Alignment shows
for the families that have one.

The Conflicts list shows the last refusal that named mates or freedoms, from a mate command or
from Update. It says what kind of refusal it was, lists each named mate with its family, and
offers Delete for it and Flip when it has an alignment. A refusal that left freedom lists the
free motions. The list reads only what the refusal already carries; it solves nothing of its
own. A success clears it.

## Proofs and Update

A proof belongs to a document, a revision and a face. It comes from the part rebuilt from that
revision's bytes on the kernel, through `Kernel::face_support_plane`, which reads the native
plane definition exactly and costs no numerical work. Gaps are computed in rationals from
those offsets and the stored placements. `measurement.rational` holds the exact value;
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

Concentric, Angle, Tangent, Symmetric, the Width `free` mode, limit mates and the mechanical
families; mates on axes, edges and points; picking a face inside a sub-assembly for a new mate;
mates on pattern members; configurations; dragging; general rotations, including a Lock off
the quarter turns; and curved faces. See M6 in the plan.
