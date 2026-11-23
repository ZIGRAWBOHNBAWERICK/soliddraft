# Field preview fixtures

Parts for `ci/browser-field-preview.mjs`. Each `<name>.txt` is a `soliddraft run` script that
builds the part on Catallax and saves `<name>.dwprt`. The parts are saved without display data,
so the browser makes each first display itself:

    soliddraft run --script lattice.txt
    soliddraft quality lattice.dwprt --display-data off

`quality` saves the setting, then reports that a field part has no certified display levels.

- `lattice.dwprt`: a cubic lattice small enough that its certified STL exports in seconds.
- `gyroid.dwprt`: the network gyroid G0, shown but not exported in the browser check.
- `gyroid-refused-export.dwprt`: a network gyroid at level 1/2, 2 mm accuracy, in a box it leaves
  a separate corner piece in. It builds and shows its preview; Catallax refuses its certified STL.
- `gyroid-graded-refused.dwprt`: a gyroid level graded from 0 to 0.9, past the certified range
  [-1/4, 1/4], so the gyroid is refused.
