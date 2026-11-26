#!/usr/bin/env python3
"""Reads the manufacturing information of a STEP file with OpenCASCADE and checks it.

usage: check-step-pmi.py <file.step> <expected.json>
       check-step-pmi.py <file.step> --print

OpenCASCADE's XDE reader lists the file's datums, dimensions, geometric tolerances and
part notes, each with the faces it is on. A face is described by its surface, a radius
for a cylinder and its area centroid, so the check knows which face of the part an item
sits on without knowing the file's entity numbers. The check passes when what was read
equals <expected.json>, whatever the order. `--print` writes what was read, in the
form the check compares, to make or review an expectation.

OpenCASCADE has no reader for the AP242 surface texture entity. It reads a surface
finish through the finish's graphical callout instead: a dimension of type
`DimensionPresentation` on the finish's faces, with the callout's name and its drawing.
An expectation names what OpenCASCADE does not read under `not_read` with the reason;
the check does not look for them.

This checker belongs to CI. The application never runs OpenCASCADE.
"""

import json
import sys

from OCP.BRepAdaptor import BRepAdaptor_Surface
from OCP.BRepGProp import BRepGProp
from OCP.GeomAbs import GeomAbs_Cylinder, GeomAbs_Plane
from OCP.GProp import GProp_GProps
from OCP.IFSelect import IFSelect_RetDone
from OCP.Interface import Interface_Static
from OCP.STEPCAFControl import STEPCAFControl_Reader
from OCP.TCollection import TCollection_ExtendedString
from OCP.TDF import TDF_AttributeIterator, TDF_ChildIterator
from OCP.TDocStd import TDocStd_Document
from OCP.TopAbs import TopAbs_FACE
from OCP.TopoDS import TopoDS
from OCP.XCAFApp import XCAFApp_Application
from OCP.XCAFDoc import XCAFDoc_DimTolTool, XCAFDoc_DocumentTool

try:
    from OCP.TDF import TDF_LabelSequence as Labels
except ImportError:  # The 8.0 bindings name the sequence by its element.
    from OCP.collections import Sequence_TDF_Label as Labels

NOTES = ("general tolerance", "material") + tuple(f"note {n}" for n in range(1, 10))


def static(owner, name):
    """A static method under either binding's spelling."""
    return getattr(owner, name + "_s", None) or getattr(owner, name)


def kind(value, prefix):
    text = str(value).split(".")[-1]
    return text.split(prefix, 1)[-1] if prefix in text else text


def number(value):
    """A length as the check compares it: micrometre steps, and no negative zero."""
    return round(value, 3) + 0.0


def face(shape):
    if shape.ShapeType() != TopAbs_FACE:
        return {"surface": "not a face"}
    surface = BRepAdaptor_Surface(static(TopoDS, "Face")(shape))
    props = GProp_GProps()
    static(BRepGProp, "SurfaceProperties")(static(TopoDS, "Face")(shape), props)
    centre = props.CentreOfMass()
    out = {"centroid": [number(centre.X()), number(centre.Y()), number(centre.Z())]}
    if surface.GetType() == GeomAbs_Plane:
        out["surface"] = "plane"
    elif surface.GetType() == GeomAbs_Cylinder:
        out["surface"] = "cylinder"
        out["radius"] = number(surface.Cylinder().Radius())
    else:
        out["surface"] = "other"
    return out


def attribute(label, type_name):
    it = TDF_AttributeIterator(label)
    while it.More():
        if it.Value().DynamicType().Name() == type_name:
            return it.Value()
        it.Next()
    return None


def ordered(items):
    return sorted(items, key=lambda item: json.dumps(item, sort_keys=True))


def read(path):
    app = static(XCAFApp_Application, "GetApplication")()
    doc = TDocStd_Document(TCollection_ExtendedString("XmlXCAF"))
    app.InitDocument(doc)
    static(Interface_Static, "SetIVal")("read.stepcaf.subshapes.name", 1)
    reader = STEPCAFControl_Reader()
    for setter in ("SetGDTMode", "SetPropsMode", "SetMatMode", "SetNameMode", "SetMetaMode"):
        if hasattr(reader, setter):
            getattr(reader, setter)(True)
    if reader.ReadFile(path) != IFSelect_RetDone:
        raise SystemExit(f"OpenCASCADE could not read {path}")
    if not reader.Transfer(doc):
        raise SystemExit(f"OpenCASCADE could not transfer {path}")
    shapes = static(XCAFDoc_DocumentTool, "ShapeTool")(doc.Main())
    dimtol = static(XCAFDoc_DocumentTool, "DimTolTool")(doc.Main())

    def faces(labels):
        return ordered(face(static(shapes, "GetShape")(labels.Value(i))) for i in range(1, labels.Length() + 1))

    def targets(label):
        first, second = Labels(), Labels()
        static(XCAFDoc_DimTolTool, "GetRefShapeLabel")(label, first, second)
        return faces(first), faces(second)

    datums = {}
    labels = Labels()
    dimtol.GetDatumLabels(labels)
    for i in range(1, labels.Length() + 1):
        label = labels.Value(i)
        obj = attribute(label, "XCAFDoc_Datum").GetObject()
        letter = obj.GetName().ToCString() if obj.GetName() is not None else ""
        on, _ = targets(label)
        # XDE keeps one label for the datum feature and one for each reference to it.
        if on:
            datums[letter] = on

    dimensions = []
    labels = Labels()
    dimtol.GetDimensionLabels(labels)
    for i in range(1, labels.Length() + 1):
        label = labels.Value(i)
        obj = attribute(label, "XCAFDoc_Dimension").GetObject()
        first, second = targets(label)
        item = {"type": kind(obj.GetType(), "DimensionType_"), "value": number(obj.GetValue()), "faces": first}
        if second:
            item["to_faces"] = second
        if obj.IsDimWithPlusMinusTolerance():
            item["upper"] = number(obj.GetUpperTolValue())
            item["lower"] = number(obj.GetLowerTolValue())
        if obj.IsDimWithClassOfTolerance():
            item["class"] = True
        if item["type"] == "DimensionPresentation":
            # A presentation only: the item is known by its callout's name and drawing.
            name = obj.GetPresentationName()
            item["presentation"] = name.ToCString() if name is not None else ""
            item["drawn"] = not obj.GetPresentation().IsNull()
        notes = [obj.GetDescription(j).ToCString() for j in range(obj.NbDescriptions())]
        if "theoretical" in notes:
            item["basic"] = True
        dimensions.append(item)

    tolerances = []
    labels = Labels()
    dimtol.GetGeomToleranceLabels(labels)
    for i in range(1, labels.Length() + 1):
        label = labels.Value(i)
        obj = attribute(label, "XCAFDoc_GeomTolerance").GetObject()
        on, _ = targets(label)
        references = Labels()
        static(XCAFDoc_DimTolTool, "GetDatumWithObjectOfTolerLabels")(label, references)
        letters = {}
        for j in range(1, references.Length() + 1):
            datum = attribute(references.Value(j), "XCAFDoc_Datum").GetObject()
            letters[datum.GetPosition()] = datum.GetName().ToCString()
        item = {"type": kind(obj.GetType(), "GeomToleranceType_"), "value": number(obj.GetValue()), "faces": on}
        if "Diameter" in str(obj.GetTypeOfValue()):
            item["diameter_zone"] = True
        if letters:
            item["datums"] = [letters[place] for place in sorted(letters)]
        tolerances.append(item)

    notes = {}

    def walk(label):
        data = attribute(label, "TDataStd_NamedData")
        if data is not None and data.HasStrings() and shapes.IsTopLevel(label):
            for name in NOTES:
                key = TCollection_ExtendedString(name)
                if data.HasString(key):
                    notes[name] = data.GetString(key).ToExtString()
        child = TDF_ChildIterator(label, False)
        while child.More():
            walk(child.Value())
            child.Next()

    free = Labels()
    shapes.GetFreeShapes(free)
    for i in range(1, free.Length() + 1):
        walk(free.Value(i))
    return {
        "datums": dict(sorted(datums.items())),
        "dimensions": ordered(dimensions),
        "tolerances": ordered(tolerances),
        "notes": notes,
    }


def main(arguments):
    if len(arguments) != 2:
        raise SystemExit(__doc__)
    found = read(arguments[0])
    if arguments[1] == "--print":
        json.dump(found, sys.stdout, indent=1, sort_keys=True, ensure_ascii=False)
        print()
        return
    with open(arguments[1], encoding="utf-8") as file:
        expected = json.load(file)
    failures = []
    for part in ("datums", "dimensions", "tolerances", "notes"):
        want = expected.get(part, {} if part in ("datums", "notes") else [])
        if part in ("dimensions", "tolerances"):
            want = ordered(want)
        if found[part] != want:
            failures.append(part)
            print(f"{part}: OpenCASCADE read", json.dumps(found[part], sort_keys=True, ensure_ascii=False))
            print(f"{part}: expected       ", json.dumps(want, sort_keys=True, ensure_ascii=False))
    if failures:
        raise SystemExit(f"{arguments[0]}: {', '.join(failures)} differ from {arguments[1]}")
    counts = ", ".join(f"{len(found[part])} {part}" for part in ("datums", "dimensions", "tolerances", "notes"))
    print(f"{arguments[0]}: OpenCASCADE read {counts}, as expected")
    for item in expected.get("not_read", []):
        print(f"not read by OpenCASCADE: {item}")


if __name__ == "__main__":
    main(sys.argv[1:])
