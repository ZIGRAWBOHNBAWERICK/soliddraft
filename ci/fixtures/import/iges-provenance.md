The `occt-box.igs` and `occt-sphere.igs` fixtures are unchanged copies from
`crates/catallax-import/tests/fixtures/iges/` at Catallax commit
`9aed0777184353a5f52d1b660f9d2411b97f0fc9`.

OpenCASCADE 8.0 wrote these files in solid B-rep mode. The box measures
2 by 3 by 5 mm, with volume 30 mm³. The sphere's transform contains rounded
sine and cosine values. Catallax refuses it rather than moving its geometry.
