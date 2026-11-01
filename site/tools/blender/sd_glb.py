# sd_glb.py - every real SolidDraft part to a GLB for the site's Three.js chapters.
#   blender -b --python tools/blender/sd_glb.py            (all parts)
#   PARTS=bracket,collar blender -b --python ...            (some)
# Output: app/public/assets/3d/<name>.glb, Y up, millimetres, one mesh, flat normals.
import bpy, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__))); import sd_common as sd

OUT = os.environ.get('OUT', os.path.join(sd.ASSETS, '3d')); os.makedirs(OUT, exist_ok=True)
names = os.environ.get('PARTS', ','.join(sd.SOURCES)).split(',')
LIMIT = {'lattice': 36000, 'boss': 9000, 'fillet': 8000, 'plate': 9000, 'collar': 6000, 'bracket': 7000}
for name in names:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    ob = sd.load(name, max_tris=LIMIT.get(name), centre=False)
    for o in bpy.context.scene.objects: o.select_set(o is ob)
    bpy.context.view_layer.objects.active = ob
    path = os.path.join(OUT, f'{name}.glb')
    bpy.ops.export_scene.gltf(filepath=path, export_format='GLB', use_selection=True, export_apply=True,
                              export_yup=True, export_materials='NONE', export_texcoords=False, export_normals=True)
    print(f'GLB {name}: tris={sd.tris(ob)} verts={len(ob.data.vertices)} dims_mm={tuple(round(d, 2) for d in ob.dimensions)} bbox_mm={sd.bbox(ob)} bytes={os.path.getsize(path)}', flush=True)

# Check the mapping on one file: reimport the GLB (glTF Y up becomes Blender Z up again) and
# print its box. The bracket must come back as x 0..120, y 0..90, z 0..60 (part frame).
if 'bracket' in names:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=os.path.join(OUT, 'bracket.glb'))
    ob = [o for o in bpy.data.objects if o.type == 'MESH'][0]
    print('GLB-CHECK bracket reimported bbox_mm (Blender Z up):', sd.bbox(ob), flush=True)
