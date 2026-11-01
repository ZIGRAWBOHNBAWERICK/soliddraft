# sd_turntable.py - a short turntable clip and a poster of one real SolidDraft part.
#   PART=bracket [FRAMES=48] [W=960 H=600 SPP=12] [PREVIEW=1] blender -b --python tools/blender/sd_turntable.py
# Frames go to $SD_SCRATCH/turn/<part>/; the script then calls ffmpeg for
# app/public/assets/work/<part>.mp4 (960x600 by default, 24 fps, h264, 2 s loop) and renders the poster
# app/public/assets/work/<part>.jpg (1600x1000) from the first frame's camera.
import bpy, os, sys, math, subprocess
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__))); import studio, sd_common as sd

PART = os.environ.get('PART', 'bracket'); FRAMES = int(os.environ.get('FRAMES', '48'))
PREVIEW = os.environ.get('PREVIEW') == '1'
OUT = os.environ.get('OUT', os.path.join(sd.ASSETS, 'work')); os.makedirs(OUT, exist_ok=True)
FR = os.path.join(sd.SCRATCH, 'turn', PART); os.makedirs(FR, exist_ok=True)
LOOK = {  # material, tilt about x, elevation of the camera, lens
    'bracket': ('bone', 0, 2.6, 45), 'collar': ('graphite', 90, 1.4, 50), 'lattice': ('oxide', 0, 2.0, 45),
    'plate': ('bone', 0, 2.4, 45), 'boss': ('graphite', 0, 2.2, 45), 'tray': ('bone', 0, 2.6, 45),
    'fillet': ('bone', 0, 2.2, 45),
}
matname, tilt, elev, lens = LOOK.get(PART, ('bone', 0, 2.4, 45))

scene = studio.reset()
W, H, SPP = (int(os.environ.get('W', '960')), int(os.environ.get('H', '600')), int(os.environ.get('SPP', '12'))) if not PREVIEW else (320, 200, 8)
studio.setup_cycles(scene, SPP, W, H); studio.studio(scene); scene.cycles.max_bounces = 4
mats = {'bone': studio.mat('bone', '#F2F0EA', 0.55), 'graphite': studio.mat('graphite', '#1C2026', 0.35),
        'oxide': studio.mat('oxide', '#FF5B1F', 0.25, coat=0.5)}
ob = sd.load(PART, max_tris=120000 if PART == 'lattice' else None)
sd.fit(ob, 2.2); s = ob.scale[0]
rig = bpy.data.objects.new('rig', None); bpy.context.collection.objects.link(rig)
studio.place(ob, (0, 0, 0), s, (tilt, 0, 0), mats[matname]); ob.parent = rig
# Rest on the floor shadow line rather than hanging in the air.
rig.location = (0, 0, -0.3)
key = studio.area('accent', (3.5, 2.5, 2.5), (0, 0, 0), 2.0, 600, color=studio.hex2lin('#5BC8F5'))
studio.camera(scene, (0, -6.4, elev), (0, 0, -0.1), lens)
scene.frame_start = 1; scene.frame_end = FRAMES; scene.render.fps = 24
bpy.context.preferences.edit.keyframe_new_interpolation_type = 'LINEAR'
rig.rotation_euler = (0, 0, math.radians(35)); rig.keyframe_insert('rotation_euler', frame=1)
rig.rotation_euler = (0, 0, math.radians(35 + 360)); rig.keyframe_insert('rotation_euler', frame=FRAMES + 1)

# Poster first: one still at 1600x1000, more samples.
scene.frame_set(1)
if not PREVIEW:
    scene.render.resolution_x, scene.render.resolution_y, scene.cycles.samples = 1600, 1000, 96
    studio.render(scene, os.path.join(OUT, f'{PART}.jpg'), 'JPEG')
    scene.render.resolution_x, scene.render.resolution_y, scene.cycles.samples = W, H, SPP
scene.render.image_settings.file_format = 'PNG'; scene.render.image_settings.color_mode = 'RGB'
scene.render.filepath = os.path.join(FR, 'f_')
bpy.ops.render.render(animation=True)
mp4 = os.path.join(OUT, f'{PART}.mp4')
subprocess.run(['ffmpeg', '-y', '-loglevel', 'error', '-framerate', '24', '-i', os.path.join(FR, 'f_%04d.png'),
                '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '26', '-preset', 'slow', '-movflags', '+faststart', mp4], check=True)
print('MP4', mp4, os.path.getsize(mp4), flush=True)
