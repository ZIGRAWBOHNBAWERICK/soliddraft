# Scene import fixtures

Files the scene import tests and the `scene-import` application scenario open. None was
edited after it was written or downloaded.

## sand3d-bracket and sand3d-washer

Written by [sand3d](https://github.com/Sandflow-ai/sand3d) 1.6.3 (commit
84c980a523adcb0474420ed4e3990681d403dce3) on Blender's `bpy` 5.2, with `s3d.export`.
sand3d is under the Apache License 2.0. Its programs are in the repository's README
(`bracket`) and `programs/washing_machine.py` (`washer`).

```python
import sand3d as s3d

def bracket(w=80, d=40, t=6, hole_d=6.6, pitch=50):
    plate = s3d.extrude("plate", s3d.rect(w, d, r=4), t)
    s3d.hole(plate, hole_d, on=s3d.grid(2, 1, pitch, 0))
    s3d.fillet(plate, 1, edges="top")
    return plate

for ext in ("usda", "glb", "urdf", "obj", "stl"):
    s3d.export(bracket(), f"bracket.{ext}")
```

The washer is `washer()` from `programs/washing_machine.py`, exported to `.usda`, `.glb`
and `.urdf` the same way. The bracket is 80 x 40 x 6 mm, its plate on Z = 0; the washer
is a 600 x 600 x 850 mm cabinet with a drum, a door of three parts, a drawer and a dial,
and four joints.

## omniverse-cubebox

`cubebox_a02.usd` is the SimReady cardboard box from NVIDIA's
[LearnOpenUSD](https://github.com/NVIDIA-Omniverse/LearnOpenUSD) course,
`docs/exercise_content/foundations/cubebox_a02/cubebox_a02.usd` at commit
c047baa06ff3c6d4b14be0746ab5d5c36f6643c8, under the Apache License 2.0. It is binary
USD (crate format), in metres with Z up. Its textures are not included; the import
keeps only constant material factors.
