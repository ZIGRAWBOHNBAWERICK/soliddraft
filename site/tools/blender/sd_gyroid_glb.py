# Decimate the certified gyroid STL (parts/gyroid-block.txt, exported with soliddraft run) into the
# site's GLB. SC points at the folder holding parts/gyroid-block.stl; OUT is the GLB to write.
import bpy, os
SC = os.environ['SC']; OUT = os.environ['OUT']
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.wm.stl_import(filepath=f'{SC}/parts/gyroid-block.stl', forward_axis='Y', up_axis='Z')
ob = [o for o in bpy.context.scene.objects if o.type == 'MESH'][0]
ob.name = 'gyroid'
print('raw tris', len(ob.data.polygons), 'dims', tuple(round(v, 2) for v in ob.dimensions))
m = ob.modifiers.new('dec', 'DECIMATE'); m.ratio = 30000 / len(ob.data.polygons); m.use_collapse_triangulate = True
dg = bpy.context.evaluated_depsgraph_get(); me = bpy.data.meshes.new_from_object(ob.evaluated_get(dg), depsgraph=dg)
ob.modifiers.clear(); old = ob.data; ob.data = me; bpy.data.meshes.remove(old)
for p in ob.data.polygons: p.use_smooth = True
ob.select_set(True); bpy.context.view_layer.objects.active = ob
bpy.ops.export_scene.gltf(filepath=OUT, export_format='GLB', use_selection=True, export_apply=True, export_yup=True, export_materials='NONE', export_texcoords=False, export_normals=True)
print('GLB tris', len(ob.data.polygons), 'bytes', os.path.getsize(OUT))
