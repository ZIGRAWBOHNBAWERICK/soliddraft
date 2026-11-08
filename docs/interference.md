# Interference and clearance

Interference Detection asks whether two placed parts overlap. Clearance Verification asks
whether they stay a given distance apart. Both compare every pair of parts in scope and
change nothing in the assembly. The code is `crates/soliddraft-assembly/src/interference/`;
the commands and the Interference panel are in `crates/soliddraft-app/src/interference/`.

## What this version supports

This is the certified-mesh tier. It needs no exact two-body query from the kernel, so it
answers less than an exact tier would, and it says so instead of guessing.

| Check | What a pair can be |
|---|---|
| Interference | `clear`, `interferes`, or `touching_or_undecided` |
| Clearance | `met`, `below` (at or under the clearance, overlaps included), or `undecided` |

The scope is the whole assembly or the parts at or under named occurrences: parts,
sub-assemblies and pattern members. Suppressed parts never take part; hidden parts take part
only when asked. Parts must be in the default configuration. At most 128 parts take part in
one check.

## How a pair is proven

Each part is rebuilt from the bytes of the revision its assembly pins, as mate proofs are,
and the kernel tessellates it with a requested chord bound, 0.01 mm unless the request names
another between 0.001 mm and 1 mm. The display copy and the screen's preview mesh never take
part. A mesh the kernel marks unproved, or one whose chord bound it could not certify, is
refused.

Catallax proves the bound per triangle over the whole patch of face the triangle covers:
each point of the patch and the matching point of the triangle are at most the bound apart.
The adapter keeps its f32 rounding inside the same bound. So a part and its mesh agree on
what is inside everywhere farther than the bound from either surface. Three rules follow:

- **Clear.** Two meshes farther apart than the sum of their bounds, neither inside the
  other, hold two parts that do not meet. A mesh may have several separate pieces, such as
  the copies of a pattern or the inner wall of a cavity. Since the two surfaces do not meet,
  each piece lies wholly inside the other mesh or wholly outside it, so one point of every
  piece of both meshes is tested. The pair's proven distance is the meshes' lower
  bound less that sum, up to the meshes' upper bound plus it.
- **Interferes.** A point inside both meshes, farther than each mesh's bound from it, is
  inside both parts. The result carries that point as `witness`.
- **Touching or undecided.** Anything else: the meshes come within the margin and no such
  point was found. Two parts that touch, overlap by less than the margin or stand apart by
  less than it all land here, and the result says which of those it cannot tell apart.

The meshes' distance is kept as two numbers, a proven lower bound and a proven upper bound.
The search reaches every triangle pair that could be nearest, and each pair's distance is
itself an enclosure: two triangles are nearest at a corner of one over the other's face, at
a corner against an edge, between two edges' inner points, or zero apart where an edge
passes through the other. Which of these holds is a sign, never a rounded guess, and each
candidate is a square of a signed quantity over a positive one, so it stays accurate near
zero. A clearance is met only when the lower bound less the margin is more than the
clearance.

No distance or containment test trusts a rounded number. Each is computed first in binary64
with a running bound on the error of every operation. That decides nearly every sign and
bounds nearly every distance to a few units in the last place. A sign the bound leaves open,
or a distance enclosure wider than a thousandth of the rounding allowance, is computed again
in exact arithmetic: mesh coordinates are binary floats, so they are exact rationals, and
every quantity the tests form from them is a polynomial or a ratio of two. Two cases need
care in binary64 and are handled without that fallback where possible. A long thin
triangle's corners are far apart, so where a point projects onto it is decided by signs, not
by solving for barycentric weights. Two edges a hair off parallel have no well-conditioned
nearest points on their lines, so their lower bound is the distance from the part of one
alongside the other to the other's line. A containment ray's crossings are exact orientation
signs: a ray through an edge, a corner or a triangle's plane is a graze, and the next
direction is tried.

Every margin also carries an allowance of 1e-9 of the largest coordinate placed, reported as
`rounding_mm`, for placing each mesh in the root's frame in binary64. Turning and moving a
point loses a few units in the last place, about 1e-16 of the coordinate, so the allowance
has seven orders of magnitude in hand.

A pair whose meshes' boxes are farther apart than the margin is clear from the boxes alone
and is not measured further; its distance has a lower bound only. The witness search tries
up to 20,000 points per pair, walking in from where the two meshes meet, from a triangle of
every piece of each, and from spread-out triangles. Not finding one proves nothing, so the
pair stays undecided rather than clear.

## Exact touching from mates

SolidWorks reports coincident faces as a sliver of interference. Here an exact mate settles
them. When a root Coincident mate, or a Distance mate that measures zero, holds two opposed
faces on one plane, and the mate's proof on the accepted placements reads exactly `0`, those
faces lie on one plane. The proof holds only when each placement takes its face's normal
exactly onto the same world axis (see [mates.md](mates.md)), and both placed meshes must lie
across that axis. If the two faces' meshes overlap in that plane by more than their
bounds, the faces share a point, and the pair's result lists the contact with the mate, both
faces and the shared point. Its distance is then exactly zero, so a clearance check reports
it `below` any clearance.

The outcome stays `touching_or_undecided`: the mate proves the faces touch, not that the
parts overlap nowhere else. Proving that needs the exact tier. A pair the meshes prove to
interfere still interferes, whatever its mates say. Aligned faces on one plane and mates
inside sub-assemblies add nothing yet.

## What it refuses

A whole check is refused, and nothing compared, for:

| Code | Why |
|---|---|
| `invalid` | A negative clearance, a chord bound out of range, or an occurrence the assembly does not have |
| `too_few_parts` | Fewer than two parts in scope |
| `too_large` | More than 128 parts in scope |

A part that cannot be proven is left out by name in `skipped`, the rest are compared, and
`complete` is false:

| Code | Why |
|---|---|
| `mesh_reference` | A mesh reference body has no exact solid |
| `unsupported_body` | The kernel will not give the body a certified mesh, such as a lattice |
| `unsupported_configuration` | The part is in a configuration other than the default |
| `proof_unavailable` | The pinned revision's bytes, a kernel or a body are not available, or a sub-assembly's structure cannot be read |

A sub-assembly whose structure cannot be read at its pinned revision is listed in `skipped`
too, by its path and the reason, and its parts are left out. That covers one with no copy at
all, since nobody knows which parts it holds, and one shown from another revision of its
file, since that revision may place its parts elsewhere than the pin does. It is listed
whether it is in the scope or above an occurrence the request names. Update repins it. A check is `complete` only when
every part in scope is known and compared. A suppressed sub-assembly is out of scope, and so
is a hidden one unless `include_hidden` asks for hidden parts. When a check is not complete,
the Interference panel says only that the pairs of the checked parts are clear.

Every report also lists what this tier does not report, in `refused`: overlap volumes, and
exact touching without a mate. Both need the exact tier.

## Commands

| Command | What it does |
|---|---|
| `assembly_check_interference` | Proves each pair clear, interfering, or touching or undecided |
| `assembly_check_clearance` | Proves each pair to meet `clearance`, such as `0.5 mm`, to be at or under it, or undecided |

Both take `paths` (occurrence paths such as `"4"` or `"6/2"`; every part when left out),
`include_hidden` and `chord_mm`. The report lists the parts with the revisions they were
proven on, the pair counts, and every pair not proven clear (for a clearance check, every
pair not proven to meet it), interfering pairs first. Each result names its parts, its
outcome, the proven distance bounds, the faces whose meshes come nearest, any exact contact
and a sentence saying what was proven.

## In the app

**Interference** on the Assembly tab's Component group opens the Interference panel in the
property panel's place. It checks interference or a clearance, over the whole assembly or the
parts chosen in the tree, and lists the results. Clicking a result chooses its two parts. A
check made before the assembly changed is marked out of date.

## Hole alignment

The same panel also offers Hole alignment. This check reads internal cylindrical axes at
pinned revisions and uses exact rational comparisons, independently of the mesh tier.
See [mates.md](mates.md#static-assembly-checks) for its scope, command and limits.

## The exact tier

An exact tier would report overlap volumes, touching without a mate, and distances closer than
the meshes' bounds. It needs a Catallax query over two bodies under exact rigid motions; see
[needs-catallax.md](needs-catallax.md#two-body-intersection-and-distance).
