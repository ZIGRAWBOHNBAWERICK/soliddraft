# Parts

The parts the site shows. Each `<name>.txt` is a `soliddraft run` script: one command per line,
the same commands the ribbon, the assistant and the MCP server use. Running it rebuilds the part
on Catallax, saves the `.dwprt` and exports the mesh beside it:

    soliddraft run --script bracket-plain.txt

`soliddraft export <part.dwprt> --out part.step --out part.obj` writes STEP AP242 with its proven
deviation, or OBJ, STL and 3MF from Catallax's certified mesh. `bracket.dwdrw` and
`bracket-drawing.pdf` come from `soliddraft draw bracket-plain.dwprt --template iso-a3`.
`fillet.obj` is the `recipes/boss-fillet.json` result (`fillet.recipe.json`). `lattice.dwprt`
and `gyroid-block.dwprt` export certified field STLs of 231,000 and 350,000 triangles; they are
not kept here because of their size. `tools/blender/sd_gyroid_glb.py` decimates the gyroid STL
into the site's GLB.

Made with SolidDraft 0.1.0 on Catallax bbabeef49f811a3781103000f240731aa3b159fc.
