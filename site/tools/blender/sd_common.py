# sd_common.py - load real SolidDraft exports (OBJ or STL, millimetres) into Blender.
# Parts come from site/parts (SD_PARTS) and were made with `soliddraft run --script`.
import bpy, os, math
from mathutils import Vector

HERE = os.path.dirname(os.path.abspath(__file__))
SITE = os.path.abspath(os.path.join(HERE, '..', '..'))
PARTS = os.environ.get('SD_PARTS', os.path.join(SITE, 'parts'))
ASSETS = os.environ.get('SD_ASSETS', os.path.join(SITE, 'app', 'public', 'assets'))
SCRATCH = os.environ.get('SD_SCRATCH', '/tmp/sd-blender')

FALLBACK = os.environ.get('SD_PARTS_FALLBACK', '')

# Site name -> export file. SolidDraft writes OBJ and STL in the part's frame: Z up,
# the top plane is XY and extrudes go +Z. Blender is Z up too, so the import is the identity.
SOURCES = {
    'bracket': 'bracket-plain.obj', 'collar': 'collar.obj', 'boss': 'rounded-boss.obj',
    'tray': 'tray.obj', 'plate': 'csk-plate.obj', 'lattice': 'lattice.stl',
    'fillet': 'fillet.obj',
}

def source(name):
    p = os.path.join(PARTS, SOURCES[name])
    if not os.path.exists(p) and FALLBACK: p = os.path.join(FALLBACK, SOURCES[name])
    return p

def load(name, path=None, max_tris=None, centre=True):
    """Import one export, flat shaded, in the part's own millimetre coordinates.
    centre=True moves the bounding-box centre to the origin for renders; the GLBs keep the
    original coordinates (the site rescales them itself)."""
    path = path or source(name)
    before = set(bpy.data.objects)
    if path.endswith('.obj'):
        bpy.ops.wm.obj_import(filepath=path, forward_axis='Y', up_axis='Z')
    else:
        bpy.ops.wm.stl_import(filepath=path, forward_axis='Y', up_axis='Z')
    new = [o for o in bpy.data.objects if o not in before and o.type == 'MESH']
    ob = new[0]
    if len(new) > 1:
        for o in new: o.select_set(True)
        bpy.context.view_layer.objects.active = ob
        bpy.ops.object.join()
    ob.name = name; ob.data.name = name
    if max_tris and len(ob.data.polygons) > max_tris:
        # CAD exports spend most triangles on flat caps and fine cylinder chords. Merge the
        # coplanar ones first, then collapse what is still over the limit.
        bpy.context.view_layer.objects.active = ob
        m = ob.modifiers.new('planar', 'DECIMATE'); m.decimate_type = 'DISSOLVE'; m.angle_limit = math.radians(1.0)
        bpy.ops.object.modifier_apply(modifier='planar')
        bpy.ops.object.mode_set(mode='EDIT'); bpy.ops.mesh.select_all(action='SELECT')
        bpy.ops.mesh.quads_convert_to_tris(); bpy.ops.object.mode_set(mode='OBJECT')
        if len(ob.data.polygons) > max_tris:
            m = ob.modifiers.new('dec', 'DECIMATE'); m.ratio = max_tris / len(ob.data.polygons)
            bpy.ops.object.modifier_apply(modifier='dec')
    for p in ob.data.polygons: p.use_smooth = False
    if centre:
        lo = Vector([min(v.co[i] for v in ob.data.vertices) for i in range(3)])
        hi = Vector([max(v.co[i] for v in ob.data.vertices) for i in range(3)])
        c = (lo + hi) / 2
        for v in ob.data.vertices: v.co -= c
    ob.location = (0, 0, 0)
    return ob

def bbox(ob):
    lo = [min(v.co[i] for v in ob.data.vertices) for i in range(3)]
    hi = [max(v.co[i] for v in ob.data.vertices) for i in range(3)]
    return tuple(round(v, 3) for v in lo), tuple(round(v, 3) for v in hi)

def tris(ob):
    return sum(len(p.vertices) - 2 for p in ob.data.polygons)

def size(ob):
    return max(ob.dimensions)

def fit(ob, unit=1.0):
    """Scale so the longest side is `unit` scene units (studio.py suits ~1 unit parts)."""
    s = unit / size(ob); ob.scale = (s, s, s); return s
