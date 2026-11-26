# Manufacturing information

A part carries its manufacturing information itself: datums, dimensions with their tolerances, geometric tolerances, surface finishes and the notes of the whole part. Each item sits on faces of the part. The information travels in the part file and in STEP, and a drawing shows what the part states.

## In the app

Open Inspect > Manufacturing information. The panel has a form, the part's notes and the list of items.

To add an item, choose its kind, pick its faces in the viewport and fill in the values. A picked face is listed in the form; Remove takes one out and Clear empties the form.

| Kind | Pick | Values |
|---|---|---|
| Datum | The face that is the datum | A letter; the next free one when left empty |
| Diameter, Radius | The round face, or each round face of a pattern | A tolerance: a class (`H7`), a symmetric deviation (`±0.1`) or two deviations (`+0.1/0`). Through hole, for a diameter. A thread's designation, such as `M6x1-6H`, for a threaded hole's diameter, which then takes no tolerance |
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

## Threads

A threaded hole states its thread itself (see [threaded holes](modelling.md#threaded-holes)): each hole feature with a thread lists one item, the diameter of its walls with the designation and how deep the thread goes from where the hole starts now, such as `2× M6x1-6H DEEP 12` or `M8x1-6H THRU`, named after the hole (`Hole1 thread`). Holes of one feature that a later cut left different threads list an item each (`Hole1 thread 2`). The panel says which hole states it and offers no Edit or Delete: change the thread in the hole. The diameter it measures is the drill's.

A diameter added in the panel can carry a designation too, for a threaded hole the part did not drill, such as one of an imported part.

## In STEP

Exporting a part with manufacturing information as STEP writes the items into the file as AP242 semantic PMI, on the faces they apply to. The export reports how many items it wrote and names every item it left out with the reason. An item that does not apply is left out. So is one a STEP file cannot hold as it stands, such as a finish that is not a parameter and a value.

In the file a slot is three dimensions (length, width, end radius), a hole that goes through carries an attribute saying so, a threaded hole's diameter carries its designation as an attribute (AP242 edition 1 has no thread entity), and each locating distance of a position tolerance is a basic dimension. The depth of a thread that stops short is not in the file. A finish is also drawn on its face as a callout, a leader to the face with the finish symbol and the requirement, for readers that have no surface texture entity. The file carries the AP242 validation properties too: how many items of each kind the part has, and the text of each datum and distance, so a reader can check what it read.

Importing a STEP file reads its manufacturing information onto the imported faces, as one undo step after the import. A threaded diameter keeps its designation. The Manufacturing information panel lists what was not read, by name, with the reason. Nothing is guessed: an item the reader cannot represent in full is left out. The locating distances a file of ours holds are not read as items, since the part measures its own. The file's notes fill only what the part does not say itself.

Replacing an imported file keeps the part's own items: they follow the faces the replacement matched.

## An imported part: recognition and the interview

A STEP file from elsewhere often has geometry and no requirements. Two things help.

**Recognition** reads the part's features off its faces: round holes, groups of equal holes, round bosses and round-ended slots. For each it gives the faces that are the feature, whether a hole or a slot goes through, the faces a hole opens onto and the ones that close it, and its sizes. It is geometry and nothing else: the same on a modelled part and on an imported one, whatever the part states. Not read yet: countersinks and counterbores as one feature, pockets, steps, fillets and chamfers.

**The interview** lists the decisions the part has not taken, one at a time, each with a proposal and the reason for it. Tick Interview in the panel. Accept takes a proposal as it stands; to answer another way, add the item with the form, and the question goes away. A decision the part has already taken is not asked. A question about a hole, boss or slot names it by its faces, so two features of one size are asked and answered apart; its id ends with those faces' names, such as `hole Ø8 size: Import1 · Face 7`. Each hole of a pattern counts on its own: a size or position on some of its holes answers for those holes, and the question asks about the rest.

| Question | Proposal | Rule |
|---|---|---|
| Material | None | No rule can tell a material from a shape |
| General tolerance | ISO 2768-mK | The medium class for lengths and class K for form |
| Datums A, B, C | Three flat faces | The largest flat face, the largest square to it, the largest square to both |
| What a lone hole through a boss is for | H7, and Ra 1.6 | A fitted hole; H7 is the first-choice hole of ISO 286, and reaming leaves about Ra 1.6 |
| What equal holes in a pattern are for | H13 | Fastener holes; H13 is the clearance class of ISO 273, medium series |
| How closely a fastener hole is located | Position, the hole less the largest screw it clears | The floating fastener rule, with the screw from the ISO 273 table |
| How closely a fitted hole is located | Position Ø0.05 | A starting value, to set from what mates with the hole |
| A boss's size and height, a slot's size and place | The measured values, no tolerance of their own | A drawing needs them; the general tolerance covers them |

The proposals come from these rules and tables only. Nothing is inferred by a language model.

## On a drawing

A drawing of a part that states manufacturing information shows it. Make drawing brings every item in, once, and places it:

| Item | Where it goes |
|---|---|
| Diameter, radius | On the round edge, in a view that looks along its axis, with the count of a pattern before it and THRU after a through hole |
| Thread | The same way, as a callout of the designation followed by THRU or the thread's depth: `4× M6x1-6H THRU`, `M8x1-6H DEEP 12` |
| Distance | Between its two ends in a view that sees both edge on, stacked outside the view |
| Slot | One callout on a round end, where the slot is seen in true shape |
| Datum, tolerance or finish of a flat face | On the line the face shows as, in the view with the fewest annotations that sees it edge on |
| Tolerance of a feature with a size | Under that size |
| The basic distances of a position tolerance | Framed dimensions from the datum's line to each feature's centre, one for each distinct distance, on the outer side of the view |

The drawing also gets a section through the hole the part holds tightest: the one with a fit, or else the narrowest tolerance. The title block names the part's material and general tolerance, and the part's other notes go in the notes block in place of the template's.

The drawing places everything itself. Stacked dimensions settle first, nearest their views; callouts and symbols then move along their leaders, or straight across or up, until no text lies over a line, another text or the frame.

Each annotation made this way belongs to its item in the part. A thread's callout belongs to the hole that states it, so renaming the hole keeps it in place. To change its tolerance, count or text, edit the item in the part and rebuild the drawing. Rebuild refreshes imported items and keeps the saved items on views that refused to redraw. An item edited so that the drawing shows it another way, such as a diameter given a thread's designation or a designation taken off, is drawn again the new way.

An item no view can show is not drawn somewhere it does not belong. It is listed by name with the reason, such as a height on a drawing that has only the top view.

## Drawing and files

Inspect > Drawing and files writes three files in one action: the STEP file with the part's manufacturing information, and a drawing made from that information as PDF and DXF beside it. The drawing has the front, top and side views and the section, and stays open in the app.

The action reports what it wrote: how many items are in the file, which are on the drawing, which the drawing could not show, any text that lies over something, and anything on the drawing that the file does not state. The last three lists are empty for a part whose items all apply. The app summary counts approximate curves once for the sheet shared by PDF and DXF.

## Scripted commands

MCP clients and `soliddraft run` get every tool below. The assistant gets all but `pmi_remove` and `pmi_notes`; its `pmi_answer`, `pmi_add` and `manufacturing_package` make a card the user applies (see [the assistant](assistant.md#manufacturing-information)).

| Tool | What it does |
|---|---|
| `pmi_add` | Adds one item, or replaces the item named in `replaces` |
| `pmi_remove` | Removes the named item |
| `pmi_notes` | Sets the general tolerance, the material note and the other notes |
| `pmi_list` | Lists every item with its callout, its faces, its measured values, its locating distances and any problem, and the part's notes |
| `pmi_recognise` | Lists the features the part's faces show, each with its faces, whether it goes through, what it opens onto and its sizes |
| `pmi_interview` | Lists the open questions with their proposals and reasons |
| `pmi_answer` | Takes the proposals named in `accept`, or all of them, and sets the material when given, as one undo step |
| `manufacturing_package` | Writes `path` (.step) and a drawing of the part's manufacturing information beside it as .pdf and .dxf. Reports what the drawing shows and what it could not, with accuracy counts in `drawing.accuracy`. |

```json
{"kind": "diameter", "name": "Bore diameter", "faces": ["Cut-Extrude1 · Wall of circle 1"], "tolerance": "H7"}
```

`export_step` reports `manufacturing_information` with the items written and left out; `import_step` reports what was read.

## What is checked

`crates/soliddraft-app/tests/pmi_bracket.rs` builds the P1 bracket, adds its fourteen items, exports it, imports the file into a new part, saves, reopens and exports again, and edits the bore from 8 to 10 mm. `crates/soliddraft-app/tests/ux_pmi_panel.rs` adds the same fourteen items by hand in the app window. `crates/soliddraft-app/tests/pmi_interview.rs` imports the bracket's bare STEP file, compares what recognition finds face by face with the bracket as built, runs the interview and draws the answered part; it also checks that a pattern with items on some of its holes is asked about the others, and that a through hole and a blind hole of one diameter are answered on their own faces, by id and with Accept all. `crates/soliddraft-app/tests/pmi_assistant.rs` has the assistant, on a scripted provider, recognise the sample bracket's features, run the interview, take its proposals and the user's material from a card, add a finish the user stated and write the package from a card; the items it leaves are the ones the same answer over MCP leaves. `crates/soliddraft-app/tests/pmi_drawing.rs` writes the three files in one action and checks the drawing: four views, every item once, the six basic distances, no overlap, nothing that the file does not state, and the title block. Hosted CI reads the exported files in OpenCASCADE through `ci/check-step-pmi.py`. It reads all fourteen items. OpenCASCADE has no reader for the surface texture entity, so it reads the finish through its callout, a presentation named "Bore finish" on the bore. It has no reader for an attribute of a feature either, so it does not read the word "through". Hosted CI also reads both files in the NIST STEP File Analyzer through `ci/check-step-sfa.py`. It reads all fourteen items, the finish and "through" included, and the finish's callout and the validation properties, with no syntax error or warning.
