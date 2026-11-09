# Sketch slots

Choose a slot from Sketch → Draw, the sketch property tools or the command palette.
Each completed slot is one undo step. Escape or right-click cancels the pending
placement; changing tools also clears its unfinished picks. Draw construction
applies to all four boundary curves. The axis/path and helper points are always
construction geometry and do not contribute to an extrusion profile.

| Tool | Clicks, in order |
|---|---|
| Slot (`S`, existing straight slot) | First end centre, second end centre, width |
| Centrepoint slot | Midpoint, one end centre, width; the other end is reflected about the midpoint |
| Three-point arc slot | First end centre, second end centre, a point along the desired arc, width |
| Centrepoint arc slot | Arc centre, first end centre, counterclockwise second end centre, width |

Straight width is twice the perpendicular distance from the width pick to the
construction axis. The existing Slot command retains its minimum half-width of
5% of the distance between end centres. Centrepoint slot uses the actual picked
width and refuses zero width. Its midpoint is a construction point with a
Midpoint relation, so a snapped midpoint stays symmetric during later edits.

Arc width is twice the absolute difference between the width pick's distance
from the arc centre and the path radius. Picking inside or outside the path
therefore gives the same width. The centrepoint variant projects the second end
onto the starting radius and follows counterclockwise from start to end, including
sweeps above 180 degrees and across zero. To describe the opposite direction,
reverse the start and end picks. The three-point variant chooses the arc through
the third pick and can describe either direction and either a minor or major arc.
Its third point remains as a construction point on the path, retaining its snap.
A projected centrepoint end retains snaps only if it lands at the snapped point.
Width picks set initial geometry and do not retain snap relations.

All tools refuse coincident defining points. Three-point arcs also refuse
collinear picks, including near-collinear picks within projection tolerance.
Arc slots refuse zero width, an inner radius at or below zero,
closed paths and major-arc caps that touch or overlap. Refusals retain the valid
pending picks and leave document geometry and history unchanged, allowing another
pick or cancellation. New tools refuse conflicting snap relations atomically.

Slots remain ordinary editable entities, with no new persistent slot type.
Straight slots use two lines, two equal-radius tangent caps and a construction
axis, preserving the original command's coincidence and tangent relations.
Arc slots use two concentric side arcs, two equal-radius tangent caps and a
concentric construction path whose endpoints are the cap centres. These relations
retain the constant-width geometry as the sketch solves. Equal-slot identity,
slot-specific dimensions and automatic full definition are outside this group.

The click workflows follow the public SOLIDWORKS descriptions for
[straight slots](https://help.solidworks.com/2026/english/SolidWorks/sldworks/t_sketch_creating_straight_slot.htm),
[centrepoint straight slots](https://help.solidworks.com/2025/English/SolidWorks/sldworks/t_Sketch_Creating_Centerpoint_Straight_Slot.htm),
[three-point arc slots](https://help.solidworks.com/2024/english/SolidWorks/sldworks/t_Sketch_Creating_3_Point_Arc_Slot.htm)
and [centrepoint arc slots](https://help.solidworks.com/2024/english/solidworks/sldworks/t_Sketch_Creating_Centerpoint_Arc_Slot.htm).
The limits and direction rules above describe SolidDraft's implementation.
