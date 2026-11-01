# Blender tools

Headless Blender (5.2, Cycles on OptiX) turns real SolidDraft exports into the site's media.
Every part comes from `site/parts/` (made with `soliddraft run --script`, see the README
there). Nothing is modelled by hand.

| Script | Makes | Run |
| --- | --- | --- |
| `sd_glb.py` | `app/public/assets/3d/*.glb`, Y up, millimetres, original coordinates | `blender -b --python sd_glb.py` |
| `sd_stills.py` | hero poster, social card, 404 image, cover | `MODE=hero blender -b --python sd_stills.py` |
| `sd_turntable.py` | card poster and 4 s hover clip of one part | `PART=bracket blender -b --python sd_turntable.py` |
| `sd_shots.sh` | real desktop and browser screenshots | `APP=... ./sd_shots.sh` |
| `studio.py`, `sd_common.py` | shared studio, lights, loader | imported by the others |

`PREVIEW=1` renders a quarter-size still with few samples. `SD_PARTS`, `SD_ASSETS`, `OUT` and
`SD_SCRATCH` override the default paths. `sd_glb.py` decimates the heavy exports (planar
dissolve, then collapse) so each GLB stays under 700 KB; it reimports the bracket and prints
its box to show the axis mapping: glTF Y is the part's Z.
