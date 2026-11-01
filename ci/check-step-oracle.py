#!/usr/bin/env python3
"""Checks a STEP export against a scenario's independent expectations, read twice.

usage: check-step-oracle.py <catallax checkout> <pinned revision> <file.step> <oracle.json>

The direct no-healing read and the ordinary read of the file must each give exactly one
valid solid, the expected surface kinds, the volume and every material point. Volume is
integrated at Gauss-Kronrod epsilons 1e-6, 1e-7 and 1e-8. The last two must agree within
1e-10 relative; a failed or nonfinite integration fails. The allowance on the reference is
the oracle's `fixed` plus the file's area times its 1e-7 mm bound plus the observed spread,
and at least its `floor`. `inside` and `outside` are points strictly away from the boundary.
Each of the optional `contacts` must be one shared vertex with at least `contact_edges` edges:
equal coordinates on separate vertices fail. Each of the optional `rings` names two surface
kinds and the length of the edges their faces share, within 1e-6 mm, and the optional
`holed_planes` counts the planar faces with more than one wire.

Runs under the kernel gate's OCP 8 binding, with the pinned checkout's reader.
"""

import importlib.metadata
import json
import math
import pathlib
import subprocess
import sys

FILE_BOUND = 1e-7
EPSILONS = (1e-6, 1e-7, 1e-8)
CONVERGENCE = 1e-10
SURFACE = {"GeomAbs_Plane": "plane", "GeomAbs_Cylinder": "cylinder", "GeomAbs_Cone": "cone",
           "GeomAbs_Sphere": "sphere", "GeomAbs_Torus": "torus", "GeomAbs_BSplineSurface": "spline"}


def solids(shape):
    from OCP.TopAbs import TopAbs_SOLID
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS

    found = []
    explorer = TopExp_Explorer(shape, TopAbs_SOLID)
    while explorer.More():
        found.append(TopoDS.Solid(explorer.Current()))
        explorer.Next()
    return found


def surfaces(solid):
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.TopAbs import TopAbs_FACE
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS

    kinds = {}
    explorer = TopExp_Explorer(solid, TopAbs_FACE)
    while explorer.More():
        kind = BRepAdaptor_Surface(TopoDS.Face(explorer.Current())).GetType().name
        kinds[SURFACE.get(kind, kind)] = kinds.get(SURFACE.get(kind, kind), 0) + 1
        explorer.Next()
    return kinds


def volumes(solid):
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps

    found = []
    for epsilon in EPSILONS:
        properties = GProp_GProps()
        error = BRepGProp.VolumePropertiesGK_s(solid, properties, epsilon, True, False, False, False, False)
        if error < 0 or not math.isfinite(properties.Mass()):
            raise ValueError(f"volume integration failed at {epsilon}")
        found.append(properties.Mass())
    return found


def area(solid):
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps

    properties = GProp_GProps()
    BRepGProp.SurfaceProperties_s(solid, properties, False, False)
    return properties.Mass()


def vertex_edges(solid, point):
    """The edge count of each distinct vertex within the file's bound of `point`."""
    from OCP.BRep import BRep_Tool
    from OCP.gp import gp_Pnt
    from OCP.TopAbs import TopAbs_EDGE, TopAbs_VERTEX
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS

    target = gp_Pnt(*point)
    edges, vertices = [], []
    walk = TopExp_Explorer(solid, TopAbs_EDGE)
    while walk.More():
        edge = walk.Current()
        walk.Next()
        if any(edge.IsSame(seen) for seen in edges):
            continue
        edges.append(edge)
        ends = TopExp_Explorer(edge, TopAbs_VERTEX)
        here = []
        while ends.More():
            vertex = ends.Current()
            ends.Next()
            near = BRep_Tool.Pnt_s(TopoDS.Vertex(vertex)).Distance(target) <= FILE_BOUND
            if near and not any(vertex.IsSame(end) for end in here):
                here.append(vertex)
        for vertex in here:
            for entry in vertices:
                if vertex.IsSame(entry[0]):
                    entry[1] += 1
                    break
            else:
                vertices.append([vertex, 1])
    return [count for _, count in vertices]


def faces_of(solid, kind):
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.TopAbs import TopAbs_FACE
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS

    found = []
    explorer = TopExp_Explorer(solid, TopAbs_FACE)
    while explorer.More():
        face = TopoDS.Face(explorer.Current())
        explorer.Next()
        if SURFACE.get(BRepAdaptor_Surface(face).GetType().name) == kind:
            found.append(face)
    return found


def subshapes(shape, kind):
    from OCP.TopExp import TopExp_Explorer

    found = []
    explorer = TopExp_Explorer(shape, kind)
    while explorer.More():
        found.append(explorer.Current())
        explorer.Next()
    return found


def ring_length(solid, kinds):
    """The length of the edges faces of both kinds use, once per physical edge."""
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps
    from OCP.TopAbs import TopAbs_EDGE

    first, second = ([edge for face in faces_of(solid, kind) for edge in subshapes(face, TopAbs_EDGE)]
                     for kind in kinds)
    shared = []
    for edge in first:
        if any(edge.IsSame(other) for other in second) and not any(edge.IsSame(seen) for seen in shared):
            shared.append(edge)
    properties = GProp_GProps()
    for edge in shared:
        BRepGProp.LinearProperties_s(edge, properties)
    return properties.Mass()


def holed_planes(solid):
    from OCP.TopAbs import TopAbs_WIRE

    return sum(len(subshapes(face, TopAbs_WIRE)) > 1 for face in faces_of(solid, "plane"))


def state(solid, point):
    from OCP.BRepClass3d import BRepClass3d_SolidClassifier
    from OCP.gp import gp_Pnt
    from OCP.TopAbs import TopAbs_IN, TopAbs_OUT

    classifier = BRepClass3d_SolidClassifier(solid)
    classifier.Perform(gp_Pnt(*point), FILE_BOUND)
    return {TopAbs_IN: "inside", TopAbs_OUT: "outside"}.get(classifier.State(), "boundary")


def failures(label, solid, oracle):
    from OCP.BRepCheck import BRepCheck_Analyzer

    found = []
    if not BRepCheck_Analyzer(solid).IsValid():
        found.append(f"{label}: invalid solid")
    kinds = surfaces(solid)
    if kinds != oracle["surfaces"]:
        found.append(f"{label}: surfaces {kinds}, expected {oracle['surfaces']}")
    values = volumes(solid)
    spread = max(values) - min(values)
    if abs(values[-1] - values[-2]) > CONVERGENCE * max(1, abs(values[-1])):
        found.append(f"{label}: volume did not converge: {values}")
    allowance = max(oracle.get("floor", 0.0), oracle.get("fixed", 0.0) + area(solid) * FILE_BOUND + spread)
    if abs(values[-1] - oracle["volume"]) > allowance:
        found.append(f"{label}: volume {values[-1]}, expected {oracle['volume']} within {allowance}")
    for expected in ("inside", "outside"):
        for point in oracle.get(expected, []):
            actual = state(solid, point)
            if actual != expected:
                found.append(f"{label}: {point} is {actual}, expected {expected}")
    contacts = {str(point): vertex_edges(solid, point) for point in oracle.get("contacts", [])}
    for point, edges in contacts.items():
        if len(edges) != 1 or edges[0] < oracle["contact_edges"]:
            found.append(f"{label}: contact {point} has vertices with edge counts {edges}")
    rings = []
    for ring in oracle.get("rings", []):
        rings.append(ring_length(solid, ring["between"]))
        if abs(rings[-1] - ring["length"]) > 1e-6:
            found.append(f"{label}: {ring['between']} share {rings[-1]} mm, expected {ring['length']}")
    if "holed_planes" in oracle and holed_planes(solid) != oracle["holed_planes"]:
        found.append(f"{label}: {holed_planes(solid)} planes with holes, expected {oracle['holed_planes']}")
    print(json.dumps({"read": label, "surfaces": kinds, "volumes": values, "allowance": allowance,
                      "contacts": contacts, "rings": rings}))
    return found


def main():
    checkout, revision, step, oracle = sys.argv[1:]
    assert importlib.metadata.version("cadquery-ocp") == "8.0.1.1.0", "the oracle requires OCP 8.0.1.1.0"
    actual = subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"], text=True).strip()
    assert actual == revision, "the reader's checkout differs from the adapter pin"
    sys.path.insert(0, str(pathlib.Path(checkout) / "tools/checks"))
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader
    from raw_step import raw_shapes

    oracle = json.loads(pathlib.Path(oracle).read_text())
    if not oracle.get("inside") or not oracle.get("outside"):
        raise ValueError("an oracle needs inside and outside points")
    raw = raw_shapes(step)
    reader = STEPControl_Reader()
    if reader.ReadFile(str(step)) != IFSelect_RetDone:
        raise ValueError("the ordinary reader refused the file")
    reader.TransferRoots()
    ordinary = solids(reader.OneShape())
    errors = []
    for label, found in (("raw", raw), ("ordinary", ordinary)):
        if len(found) != 1:
            errors.append(f"{label}: {len(found)} solids, expected one")
        else:
            errors += failures(label, found[0], oracle)
    for error in errors:
        print(f"FAIL: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
