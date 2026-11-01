#!/usr/bin/env python3
"""Read exact IGES surfaces in OpenCASCADE without healing and check the certificate.

usage: check-iges.py <file.iges> <report.json>
The report contains the exported report, exact body counts and independent surface kinds.
Volume is a separate numerical diagnostic, not a consequence of the displacement bound.
"""

from collections import Counter
from fractions import Fraction
import json
import math
import sys


def certificate(report):
    """Check exact rational composition and both upward-rounded public bounds."""
    scale = Fraction(report["scale"])
    trim = Fraction(report["trim_deviation_mm"])
    coordinates = Fraction(report["coordinate_bound_mm"])
    bound = Fraction(report["boundary_bound_mm"])
    tolerance = Fraction(report["tolerance_mm"])
    assert scale > 0 and trim >= 0 and coordinates >= 0
    assert bound == abs(scale - 1) * coordinates + scale * trim
    assert 0 <= bound <= tolerance <= Fraction(1, 10_000_000)
    assert report["within_tolerance"] is True and report["units"] == "mm"
    decimal = Fraction(report["max_deviation"])
    floating = report["max_deviation_mm"]
    assert decimal >= bound
    assert type(floating) in (int, float) and math.isfinite(floating)
    assert Fraction(floating) >= bound
    return scale, bound


def check(path, evidence):
    import OCP
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.IGESControl import IGESControl_Reader
    from OCP.IGESToBRep import (IGESToBRep, IGESToBRep_AlgoContainer,
                               IGESToBRep_BRepEntity, IGESToBRep_ToolContainer)
    from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_SOLID, TopAbs_VERTEX
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS
    from OCP.Transfer import Transfer_TransientProcess

    report, exact = evidence["report"], evidence["body"]
    scale, bound = certificate(report)
    reader = IGESControl_Reader()
    assert reader.ReadFile(str(path)) == IFSelect_RetDone
    model = reader.IGESModel()
    assert model.GlobalSection().UnitFlag() == 2
    assert model.GlobalSection().UnitName().ToCString() == "MM"
    # The base boundary tool avoids IGESControl's internal ShapeFix path.
    algo = IGESToBRep_AlgoContainer()
    algo.SetToolContainer(IGESToBRep_ToolContainer())
    IGESToBRep.SetAlgoContainer_s(algo)
    assert algo.ToolContainer().IGESBoundary().DynamicType().Name() == "IGESToBRep_IGESBoundary"
    transfer = IGESToBRep_BRepEntity()
    transfer.SetModel(model)
    transfer.SetModeApprox(False)
    transfer.SetTransferProcess(Transfer_TransientProcess(model.NbEntities()))
    roots = [model.Entity(i) for i in range(1, model.NbEntities() + 1)
             if model.Entity(i).TypeNumber() == 186]
    assert len(roots) == report["solids"] == 1
    shape = transfer.TransferManifoldSolid(roots[0])
    assert not shape.IsNull() and BRepCheck_Analyzer(shape).IsValid()

    def unique(kind):
        found = []
        explorer = TopExp_Explorer(shape, kind)
        while explorer.More():
            current = explorer.Current()
            if not any(current.IsSame(previous) for previous in found):
                found.append(current)
            explorer.Next()
        return found

    assert len(unique(TopAbs_SOLID)) == 1
    faces = unique(TopAbs_FACE)
    assert len(faces) == report["faces"] == exact["faces"]
    for added in ("added_edges", "added_vertices"):
        assert type(report[added]) is int and report[added] >= 0
    assert len(unique(TopAbs_EDGE)) == report["edges"] == exact["edges"] + report["added_edges"]
    assert len(unique(TopAbs_VERTEX)) == report["vertices"]
    kinds = Counter(BRepAdaptor_Surface(TopoDS.Face(f)).GetType().name.removeprefix("GeomAbs_") for f in faces)
    assert kinds == evidence["surface_kinds"], (kinds, evidence["surface_kinds"])
    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props)
    expected = exact["volume"] * float(scale) ** 3
    assert math.isclose(props.Mass(), expected, rel_tol=2e-10, abs_tol=1e-10), (props.Mass(), expected)
    return {"binding": OCP.__version__, "transfer": "base IGESToBRep, no healing", "valid": True,
            "surface_kinds": dict(kinds), "faces": len(faces), "max_deviation_mm": str(bound),
            "volume_diagnostic_mm3": props.Mass()}


def main(argv):
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[2]) as source:
        evidence = json.load(source)
    print(json.dumps(check(argv[1], evidence), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
