"""Check the P1 QIF/CSV pair using an installed copy of the DMSC QIF 3.0 XSDs.

Generate the pair with SOLIDDRAFT_QIF_PROOF_DIR=<directory> cargo test --locked
-p soliddraft-model --lib p1_inspection_qif_exports_requirements_without_inventing_results.
Then run: python3 crates/soliddraft-exchange/tests/check-qif.py <directory> <QIFDocument.xsd>

Use the QIFApplications/QIFDocument.xsd entry point from the DMSC standard download:
https://qifstandards.org/download/. The QIF Community's pinned originals are also at
https://github.com/QualityInformationFramework/qif-community/tree/64268c175a3058d09db29c14869b94b781518709/bindings/CPP-Kramer/schema
(restore the .xsd names and QIFApplications/QIFLibrary layout specified by the includes).
Set XML_CATALOG_FILES if the schema imports the W3C XML Signature XSD by URL;
the catalog must resolve that URL to a local copy. Validation never uses the network.
"""

import argparse
import csv
from decimal import Decimal
import json
from pathlib import Path
import subprocess
import xml.etree.ElementTree as ET


NS = {"": "http://qifstandards.org/xsd/qif3"}
PRECEDENCE = ["PRIMARY", "SECONDARY", "TERTIARY"]
MATERIAL = {"regardless": "NONE", "maximum": "MAXIMUM", "least": "LEAST"}
KINDS = {
    "diameter": "Diameter",
    "radius": "Radius",
    "slot_radius": "Radius",
    "slot_width": "Width",
    "slot_length": "Length",
    "distance": "Length",
    "surface_finish": "SurfaceTexture",
    "position": "Position",
    "flatness": "Flatness",
    "perpendicularity": "Perpendicularity",
}


def children(node, path):
    return node.findall(path, NS)


def one(node, path):
    matches = children(node, path)
    assert len(matches) == 1, (node.tag, path, len(matches))
    return matches[0]


def text(node, path):
    return one(node, path).text


def attribute(node, name):
    return one(node, f"Attributes/AttributeStr[@name='{name}']").attrib["value"]


def ids(node, path):
    return [entry.text for entry in children(node, f"{path}/Id")]


def qif_tag(node):
    return node.tag.split("}", 1)[1]


def check(xml, csv_path):
    root = ET.parse(xml).getroot()
    with csv_path.open(encoding="utf-8", newline="") as file:
        rows = list(csv.DictReader(file))
    assert rows and all(not row["actual"] and not row["disposition"] for row in rows)
    assert root.tag == "{" + NS[""] + "}QIFDocument"
    assert not children(root, "Results") and not children(root, "Plan")
    indexed = {element.attrib["id"]: element for element in root.iter() if "id" in element.attrib}
    assert len(indexed) == sum("id" in element.attrib for element in root.iter())
    assert int(root.attrib["idMax"]) == max(map(int, indexed))
    for element in root.iter():
        if "n" in element.attrib:
            assert int(element.attrib["n"]) == len(element), element.tag

    part = indexed[text(root, "Product/RootPart/Id")]
    assert qif_tag(part) == "Part"
    assert text(part, "ModelNumber") == rows[0]["part"]
    assert text(part, "Version") == rows[0]["revision"]
    assert attribute(root, "SolidDraft.Material") == rows[0]["material"]
    assert attribute(root, "SolidDraft.GeneralTolerance") == rows[0]["general_tolerance"]
    formal = indexed[text(root, "Characteristics/FormalStandardId")]
    assert text(formal, "Organization/StandardsOrganizationEnum") == "ISO"
    assert (text(formal, "Designator"), text(formal, "Year")) == ("1101", "2017")
    linear = "FileUnits/PrimaryUnits/LinearUnit"
    primary = text(root, f"{linear}/UnitName")
    units = {
        text(node, "UnitName"): Decimal(text(node, "UnitConversion/Factor"))
        for node in children(root, linear) + children(root, "FileUnits/OtherUnits/LinearUnit")
        if text(node, "SIUnitName") == "meter"
    }
    assert primary == "mm" and units == {"mm": Decimal("0.001"), "µm": Decimal("0.000001")}

    face_ids = set(ids(part, "FeatureNominalIds"))
    assert face_ids == {f.attrib["id"] for f in one(root, "Features/FeatureNominals")}

    def faces(node):
        result = []
        for feature_id in ids(node, "FeatureNominalIds"):
            assert feature_id in face_ids
            feature = indexed[feature_id]
            assert qif_tag(feature) == "OtherShapeFeatureNominal"
            assert qif_tag(indexed[text(feature, "FeatureDefinitionId")]) == "OtherShapeFeatureDefinition"
            result.append(json.loads(attribute(feature, "SolidDraft.FaceRefs")))
        return result

    def value(node, path, expected, unit):
        scalar = one(node, path)
        assert scalar.attrib.get("linearUnit", primary) == unit
        assert Decimal(scalar.text) == Decimal(expected), (path, scalar.text, expected)

    datum_ids = ids(part, "DatumDefinitionIds")
    nominal_ids = ids(part, "CharacteristicNominalIds")
    frame_ids = set(ids(part, "DatumReferenceFrameIds"))
    assert set(nominal_ids) == {c.attrib["id"] for c in one(root, "Characteristics/CharacteristicNominals")}
    assert frame_ids == {f.attrib["id"] for f in one(root, "DatumReferenceFrames")}
    datums = {text(indexed[i], "DatumLabel"): indexed[i] for i in datum_ids}
    by_source = {attribute(indexed[i], "SolidDraft.CharacteristicId"): indexed[i] for i in nominal_ids + datum_ids}
    assert len(by_source) == len(nominal_ids) + len(datum_ids)
    assert set(by_source) == {row["characteristic_id"] for row in rows if row["kind"] != "note"}
    items = {text(item, "CharacteristicDesignator/Designator"): item for item in one(root, "Characteristics/CharacteristicItems")}
    assert set(items) == {row["characteristic_id"] for row in rows if row["applicability"] == "inspect"}
    assert len(items) == len(one(root, "Characteristics/CharacteristicItems"))

    positions = []
    for row in rows:
        kind = row["kind"]
        if kind == "note":
            continue
        key = row["characteristic_id"]
        nominal = by_source[key]
        expected_faces = json.loads(row["face_refs"])
        if row["locating_datum"]:
            label = row["locating_datum"]
            assert attribute(nominal, "SolidDraft.LocatingDatum") == label
            expected_faces += [face for face in faces(datums[label]) if face not in expected_faces]
        assert faces(nominal) == expected_faces, key
        if kind == "datum":
            assert qif_tag(nominal) == "DatumDefinition"
            continue
        target_tag = KINDS[kind] + "Characteristic"
        assert qif_tag(nominal) == target_tag + "Nominal"
        definition = indexed[text(nominal, "CharacteristicDefinitionId")]
        assert qif_tag(definition) == target_tag + "Definition"
        assert text(definition, "Description") == row["pmi_callout"]
        if key in items:
            item = items[key]
            assert qif_tag(item) == target_tag + "Item"
            assert indexed[text(item, "CharacteristicNominalId")] is nominal
            linked_features = [indexed[i] for i in ids(item, "FeatureItemIds")]
            assert all(qif_tag(f) == "OtherShapeFeatureItem" for f in linked_features)
            assert [text(f, "FeatureNominalId") for f in linked_features] == ids(nominal, "FeatureNominalIds")
        if row["nominal"]:
            value(nominal, "TargetValue", row["nominal"], row["unit"])
        else:
            assert not children(nominal, "TargetValue"), key
        if row["lower_limit"]:
            value(definition, "Tolerance/MinValue", row["lower_limit"], row["unit"])
            value(definition, "Tolerance/MaxValue", row["upper_limit"], row["unit"])
            assert text(definition, "Tolerance/DefinedAsLimit") == "true"
        elif kind == "surface_finish":
            value(definition, "RoughnessAverage", row["upper_limit"], row["unit"])
        elif kind in ("position", "flatness", "perpendicularity"):
            value(definition, "ToleranceValue", row["upper_limit"], row["unit"])
            if kind == "position":
                positions.append(row)
                one(definition, "ZoneShape/DiametricalZone")
            ordered = json.loads(row["datum_order"])
            if ordered:
                frame_id = text(definition, "DatumReferenceFrameId")
                assert frame_id in frame_ids
                frame = children(indexed[frame_id], "Datums/Datum")
                assert len(frame) == len(ordered)
                for precedence, datum, ref in zip(PRECEDENCE, frame, ordered):
                    resolved = indexed[text(datum, "SimpleDatum/DatumDefinitionId")]
                    assert resolved is datums[ref["letter"]]
                    assert text(datum, "Precedence/PrecedenceEnum") == precedence
                    assert text(datum, "SimpleDatum/MaterialModifier") == MATERIAL[ref.get("material", "regardless")]
            else:
                assert not children(definition, "DatumReferenceFrameId")
        else:
            assert not children(definition, "Tolerance"), key
        if row["fit_class"]:
            fit = one(definition, "LimitsAndFitsSpecification")
            assert fit.attrib["zoneVariance"] == "HOLE"
            assert text(fit, "FormVariance") + text(fit, "Grade") == row["fit_class"]
        if row["tolerance"] == "basic":
            assert text(definition, "DimensionType") == "BASIC_OR_TED" and key not in items

    holes = [row for row in positions if row["pmi_item"] == "Hole position"]
    assert {row["member"] for row in holes} == {"1", "2", "3", "4"} and len(holes) == 4
    assert len({tuple(ids(by_source[row["characteristic_id"]], "FeatureNominalIds")) for row in holes}) == 4
    assert [row["upper_limit"] for row in positions if row["pmi_item"] == "Bore position"] == ["0.05"]
    print(f"Resolved {len(by_source)} P1 CSV IDs, {len(items)} inspection items and {len(positions)} position controls")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("proof_dir", type=Path)
    parser.add_argument("schema", type=Path)
    args = parser.parse_args()
    qif = args.proof_dir / "p1.qif"
    subprocess.run(["xmllint", "--nonet", "--noout", "--schema", args.schema, qif], check=True)
    check(qif, args.proof_dir / "p1.csv")
