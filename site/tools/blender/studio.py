# studio.py - OptiX setup, graphite cyclorama, lights, camera, materials, output.
import bpy, bmesh, math, time
from mathutils import Vector

def hex2lin(h):
    h = h.lstrip('#'); c = [int(h[i:i+2], 16)/255 for i in (0, 2, 4)]
    return tuple((x/12.92) if x <= 0.04045 else ((x+0.055)/1.055)**2.4 for x in c)

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    return bpy.context.scene

def setup_cycles(scene, spp, w, h, denoise=True):
    prefs = bpy.context.preferences.addons['cycles'].preferences
    prefs.compute_device_type = 'OPTIX'; prefs.get_devices()
    for d in prefs.devices: d.use = (d.type == 'OPTIX')
    scene.render.engine = 'CYCLES'; scene.cycles.device = 'GPU'
    scene.cycles.samples = spp; scene.cycles.use_denoising = denoise; scene.cycles.denoiser = 'OPTIX'
    scene.cycles.max_bounces = 8; scene.cycles.caustics_reflective = False; scene.cycles.caustics_refractive = False
    scene.cycles.sample_clamp_indirect = 10
    scene.render.resolution_x = w; scene.render.resolution_y = h; scene.render.resolution_percentage = 100
    scene.view_settings.view_transform = 'AgX'
    for look in ('AgX - Medium High Contrast', 'Medium High Contrast'):
        try: scene.view_settings.look = look; break
        except TypeError: pass

def mat(name, hex, rough=0.5, metallic=0.0, coat=0.0):
    m = bpy.data.materials.new(name); m.use_nodes = True
    p = m.node_tree.nodes['Principled BSDF']
    p.inputs['Base Color'].default_value = (*hex2lin(hex), 1); p.inputs['Roughness'].default_value = rough
    p.inputs['Metallic'].default_value = metallic
    if coat: p.inputs['Coat Weight'].default_value = coat
    return m

def emission(name, hex, strength, obj_alpha=False):
    m = bpy.data.materials.new(name); m.use_nodes = True; nt = m.node_tree
    for n in list(nt.nodes): nt.nodes.remove(n)
    out = nt.nodes.new('ShaderNodeOutputMaterial'); em = nt.nodes.new('ShaderNodeEmission')
    em.inputs['Color'].default_value = (*hex2lin(hex), 1); em.inputs['Strength'].default_value = strength
    nt.links.new(em.outputs[0], out.inputs[0])
    if obj_alpha:  # object colour alpha scales the strength (distance falloff per copy)
        oi = nt.nodes.new('ShaderNodeObjectInfo'); mul = nt.nodes.new('ShaderNodeMath'); mul.operation = 'MULTIPLY'
        mul.inputs[0].default_value = strength
        nt.links.new(oi.outputs['Alpha'], mul.inputs[1]); nt.links.new(mul.outputs[0], em.inputs['Strength'])
    return m

def look_at(ob, target):
    d = Vector(target) - ob.location; ob.rotation_euler = d.to_track_quat('-Z', 'Y').to_euler()

def area(name, loc, target, size, power, color=(1, 1, 1)):
    L = bpy.data.lights.new(name, 'AREA'); L.energy = power; L.size = size; L.color = color
    ob = bpy.data.objects.new(name, L); bpy.context.collection.objects.link(ob)
    ob.location = loc; look_at(ob, target); return ob

def cyc(hex, floor_z, wall_y, half, height, bevel):
    bm = bmesh.new()
    v = [bm.verts.new(p) for p in [(-half, -half, floor_z), (half, -half, floor_z), (half, wall_y, floor_z),
                                   (-half, wall_y, floor_z), (half, wall_y, floor_z+height), (-half, wall_y, floor_z+height)]]
    bm.faces.new((v[0], v[1], v[2], v[3])); bm.faces.new((v[3], v[2], v[4], v[5]))
    bmesh.ops.bevel(bm, geom=[e for e in bm.edges if len(e.link_faces) == 2], offset=bevel, segments=12, profile=0.5, affect='EDGES')
    me = bpy.data.meshes.new('backdrop'); bm.to_mesh(me); bm.free()
    for p in me.polygons: p.use_smooth = True
    ob = bpy.data.objects.new('backdrop', me); bpy.context.collection.objects.link(ob)
    me.materials.append(mat('backdrop', hex, 0.75)); return ob

def world(scene, hex, strength):
    w = bpy.data.worlds.new('W'); scene.world = w; w.use_nodes = True
    bg = w.node_tree.nodes['Background']; bg.inputs[0].default_value = (*hex2lin(hex), 1); bg.inputs[1].default_value = strength

def studio(scene, s=1.0):
    """Graphite studio scaled by s (s=1 suits ~1 m parts seen from 7 m)."""
    cyc('#15181C', -2.2*s, 7*s, 60*s, 40*s, 4*s)
    area('key', (-5*s, -4*s, 7*s), (0, 0, 0), 6*s, 2400*s*s)
    area('rim', (4*s, 6*s, 4*s), (0, 0, 0), 3*s, 1500*s*s, color=(0.88, 0.93, 1.0))
    area('fill', (6*s, -6*s, 1*s), (0, 0, 0), 5*s, 250*s*s)
    world(scene, '#15181C', 0.25)

def camera(scene, loc, target, lens=35):
    cam = bpy.data.cameras.new('cam'); cam.lens = lens; cam.sensor_width = 36; cam.clip_end = 1000
    ob = bpy.data.objects.new('cam', cam); bpy.context.collection.objects.link(ob)
    ob.location = loc; look_at(ob, target); scene.camera = ob; return ob

def render(scene, path, fmt='JPEG', quality=88):
    scene.render.image_settings.file_format = fmt; scene.render.image_settings.color_mode = 'RGB'
    if fmt == 'JPEG': scene.render.image_settings.quality = quality
    scene.render.filepath = path; t = time.time()
    bpy.ops.render.render(write_still=True); print(f'RENDER {path} {time.time()-t:.1f}s', flush=True)

def place(ob, loc, scale, rot_deg, material=None):
    ob.location = loc; ob.scale = (scale,)*3; ob.rotation_euler = [math.radians(a) for a in rot_deg]
    if material is not None:
        ob.data.materials.clear(); ob.data.materials.append(material)
    return ob
