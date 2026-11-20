# Manufacturing information

A part carries its manufacturing information itself: datums, dimensions with their tolerances, geometric tolerances, surface finishes and the notes of the whole part. Each item sits on faces of the part. The information travels in the part file and in STEP, and a drawing shows what the part states.

## In the app

Open Inspect > Manufacturing information. The panel has a form, the part's notes and the list of items.

To add an item, choose its kind, pick its faces in the viewport and fill in the values. A picked face is listed in the form; Remove takes one out and Clear empties the form.

| Kind | Pick | Values |
|---|---|---|
| Datum | The face that is the datum | A letter; the next free one when left empty |
| Diameter, Radius | The round face, or each round face of a pattern | A tolerance: a class (`H7`), a symmetric deviation (`±0.1`) or two deviations (`+0.1/0`). Through hole, for a diameter |
| Distance | The faces it starts on, then From, then the faces it ends on | Where each end is taken: the face or axis, the middle of two faces, the near or the far side of a round face. A tolerance |
| Slot | The two flat sides and the two round ends | A tolerance for the length and the width |
| Geometric tolerance | The face, or each face of a pattern | The characteristic, the value, a cylindrical zone, a material condition and the datums, primary first |
| Surface finish | The face | The requirement, such as `Ra 1.6`, and what the process may do |

A dimension takes no size. The panel shows the size the part has, measured exactly on the body, so `Ø8 H7` becomes `Ø10 H7` when the bore is edited. A slot is one item: `20×6 +0.1/0 R3`.

A feature under a position tolerance is located from the datums of that tolerance. The part measures a basic distance from each datum plane the feature's axis runs along and lists them under the tolerance, such as `25 from B, 30 from C`. Nobody types these.

Edit fills the form from an item; Change puts the edited item in its place. Delete removes one. A datum that a tolerance refers to stays until that tolerance lets go of it. Changing a datum's letter changes it in the tolerances that refer to it. Each change is one undo step.

Part notes hold the general tolerance, such as `ISO 2768-m`, the material as a drawing names it, and any other notes, one a line. A part with an assigned material and no material note uses the material's name.

## Items follow their faces

Items name faces the way features do, so they survive edits upstream. An item whose face is gone, or whose faces no longer fit it, stays in the part and says why: a diameter on a face that became flat, a tolerance whose datum does not apply. It applies again when the face comes back. Nothing is moved to another face.

Manufacturing information is part of the part's revision and not of its shape: changing it marks drawings out of date and leaves the geometry hash alone.

## In STEP

Exporting a part with manufacturing information as STEP writes the items into the file as AP242 semantic PMI, on the faces they apply to. The export reports how many items it wrote and names every item it left out with the reason. An item that does not apply is left out. So is one a STEP file cannot hold as it stands, such as a finish that is not a parameter and a value.

In the file a slot is three dimensions (length, width, end radius), a hole that goes through carries an attribute saying so, and each locating distance of a position tolerance is a basic dimension.

Importing a STEP file reads its manufacturing information onto the imported faces, as one undo step after the import. The Manufacturing information panel lists what was not read, by name, with the reason. Nothing is guessed: an item the reader cannot represent in full is left out. The locating distances a file of ours holds are not read as items, since the part measures its own. The file's notes fill only what the part does not say itself.

Replacing an imported file keeps the part's own items: they follow the faces the replacement matched.

## Scripted commands

| Tool | What it does |
|---|---|
| `pmi_add` | Adds one item, or replaces the item named in `replaces` |
| `pmi_remove` | Removes the named item |
| `pmi_notes` | Sets the general tolerance, the material note and the other notes |
| `pmi_list` | Lists every item with its callout, its faces, its measured values, its locating distances and any problem, and the part's notes |

```json
{"kind": "diameter", "name": "Bore diameter", "faces": ["Cut-Extrude1 · Wall of circle 1"], "tolerance": "H7"}
```

`export_step` reports `manufacturing_information` with the items written and left out; `import_step` reports what was read.

## What is checked

`crates/soliddraft-app/tests/pmi_bracket.rs` builds the P1 bracket, adds its fourteen items, exports it, imports the file into a new part, saves, reopens and exports again, and edits the bore from 8 to 10 mm. `crates/soliddraft-app/tests/ux_pmi_panel.rs` adds the same fourteen items by hand in the app window. Hosted CI reads the exported files in OpenCASCADE through `ci/check-step-pmi.py`. OpenCASCADE has no reader for the surface texture entity or for an attribute of a feature, so it does not read a finish or the word "through".
