# sd_stills.py - the site's still images from real SolidDraft parts, in the night studio.
#   MODE=hero|social|notfound|cover [PREVIEW=1] blender -b --python tools/blender/sd_stills.py
# hero:     app/public/assets/world/hero-poster.jpg 2560x1440, bottom-left third kept clear
# social:   app/public/assets/brand/social-card.jpg 1200x630, left 55% kept clear
# notfound: app/public/assets/brand/not-found.webp 1280x800, collars on a shelf, one fallen
# cover:    app/public/assets/brand/launch-cover.jpg 1600x1000, the bracket alone
import bpy, os, sys, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__))); import studio, sd_common as sd

MODE = os.environ.get('MODE', 'hero'); PREVIEW = os.environ.get('PREVIEW') == '1'
SIZES = {'hero': (2560, 1440, 128), 'social': (1200, 630, 96), 'notfound': (1280, 800, 96), 'cover': (1600, 1000, 128)}
W, H, SPP = SIZES[MODE]
if PREVIEW: W, H, SPP = W // 4, H // 4, 24
OUTS = {'hero': ('world/hero-poster.jpg', 'JPEG'), 'social': ('brand/social-card.jpg', 'JPEG'),
        'notfound': ('brand/not-found.png', 'PNG'), 'cover': ('brand/launch-cover.jpg', 'JPEG')}
rel, fmt = OUTS[MODE]
out = os.path.join(sd.SCRATCH, f'{MODE}_preview.png') if PREVIEW else os.path.join(os.environ.get('OUT', sd.ASSETS), rel)
os.makedirs(os.path.dirname(out), exist_ok=True)

scene = studio.reset(); studio.setup_cycles(scene, SPP, W, H); studio.studio(scene)
bone = studio.mat('bone', '#F2F0EA', 0.55); graphite = studio.mat('graphite', '#15181C', 0.35)
oxide = studio.mat('oxide', '#FF5B1F', 0.25, coat=0.5); ice = studio.mat('ice', '#5BC8F5', 0.3, coat=0.4)

def part(name, loc, size, rot, material):
    ob = sd.load(name, max_tris=120000 if name == 'lattice' else None)
    sd.fit(ob, size); studio.place(ob, loc, ob.scale[0], rot, material); return ob

if MODE == 'hero':
    # Upper two thirds and right half carry the parts; the lower left stays dark for the title.
    part('bracket', (1.0, 0.5, 1.2), 2.2, (40, 15, -35), bone)
    part('lattice', (3.7, 0.2, 0.3), 1.4, (20, 15, 30), oxide)
    part('collar', (4.4, 1.6, 2.0), 1.1, (60, 30, 0), graphite)
    part('plate', (3.9, -0.6, -1.3), 1.6, (55, 10, 20), bone)
    part('boss', (1.9, -1.0, -1.1), 1.2, (25, -35, 40), graphite)
    part('tray', (-0.9, 2.6, 2.3), 1.5, (-35, 10, 30), bone)
    part('fillet', (-2.7, 2.3, 1.6), 1.3, (30, -20, 40), bone)
    studio.camera(scene, (0, -8.5, 1.2), (0.9, 0, 0.1), 35)
elif MODE == 'social':
    part('bracket', (2.4, 0.6, 0.5), 1.9, (40, 15, -35), bone)
    part('lattice', (1.1, -0.6, -1.1), 1.0, (20, 15, 30), oxide)
    part('collar', (3.9, 1.0, -0.9), 0.9, (60, 30, 0), graphite)
    studio.camera(scene, (0, -7.6, 1.2), (0.4, 0, -0.2), 35)
elif MODE == 'cover':
    # Seen from the open corner so the plate, the boss and the holes all show.
    part('bracket', (0, 0, 0.2), 3.0, (0, 0, -20), bone)
    studio.area('accent', (-3.5, 2.5, 2.5), (0, 0, 0), 2.0, 220, color=studio.hex2lin('#5BC8F5'))
    studio.camera(scene, (4.6, -5.4, 3.4), (0, 0, 0.1), 45)
elif MODE == 'notfound':
    # A shelf of five collars standing on their rims; the last one has fallen flat.
    shelf = studio.cyc('#1A1D22', -0.6, 1.2, 8, 3, 0.2)
    for i in range(5):
        ob = sd.load('collar'); sd.fit(ob, 0.9)
        s = ob.scale[0]
        if i < 4: studio.place(ob, (-2.4 + i * 1.15, 0.2, -0.6 + 0.45), s, (90, 0, 8 * (i - 1.5)), bone if i != 2 else oxide)
        else: studio.place(ob, (2.6, -0.5, -0.6 + 0.24), s, (4, 0, 20), bone)
    studio.camera(scene, (0.6, -6.2, 0.9), (0.2, 0, -0.2), 40)

studio.render(scene, out, fmt)
if fmt == 'PNG' and not PREVIEW:
    webp = out[:-4] + '.webp'
    os.system(f'magick "{out}" -quality 84 "{webp}" && rm "{out}"'); print('WEBP', webp)
