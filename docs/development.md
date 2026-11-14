# Development

SolidDraft uses Catallax as its only geometry backend. Start with the build and run commands in the [README](../README.md). For a local browser build, see [web.md](web.md).

## Running the desktop app

A part, assembly or drawing file on the command line opens it. Without a file, the app opens the start page with recent documents.

- `--demo` loads a sample part.
- `--bench` measures frame times on a dense body and exits.
- `--screenshot out.png` saves the window and exits.
- `--help` lists the remaining options.

Inspect > Section and the viewport navigation bar cut the display on XY, YZ, ZX or a planar face. Drag Offset to move the cut, Flip side to switch the retained half, and Close section to restore the view. The cap preserves openings. Saving and exporting keep the whole part.

Fillet and Chamfer accept picked edges or a picked face's boundary edges. A refused operation leaves the part unchanged and shows the kernel's reason. Desktop depth, fillet-radius and chamfer-distance drags show provisional construction before exact proof finishes. One drag is one undo step. A failed edit restores the last proved part and leaves the failed feature red with its reason. See [backend limits](needs-catallax.md).

## Command line

Run these examples from the repository root. The `part` example needs an existing saved part named `bracket.dwprt`.

```sh
cargo run --release -p soliddraft-cli -- replay --steps recipes/union.json
cargo run --release -p soliddraft-cli -- view recipes/bracket.json --dir front,top,right,iso --svg 'bracket-{view}.svg'
cargo run --release -p soliddraft-cli -- part bracket.dwprt
cargo run --release -p soliddraft-cli -- export recipes/bracket.json --out bracket.step --out bracket.3mf --out bracket.obj
cargo run --release -p soliddraft-cli -- draw recipes/bracket.json --template iso-a3
```

Replay prints each step's face, edge and vertex counts, volume, area, validity and geometric digest. Export chooses its format from each output filename and reports its deviation. View writes hidden-line views. Part rebuilds a saved part and prints each feature's state. Draw makes a drawing of the recipe's result. Each subcommand has `--help`.

See [assistant.md](assistant.md) for API keys, the desktop assistant, MCP and scripted commands. See [application-loop.md](application-loop.md) for agent scenarios, per-command evidence and windowless viewport captures.

## Tests and contributions

Pull requests are welcome. Agent-written code is fine and expected. Say so in the pull request and stay accountable for what it does.

Read [DESIGN.md](DESIGN.md) for the kernel interface and [modelling.md](modelling.md) for the document model before changing either. Keep document and viewport code behind the existing kernel interface. If a change needs a new kernel operation, describe it in an issue before changing the bridge.

Run these checks before opening a code pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The app's exit tests drive the real interface through `egui_kittest` on wgpu. They skip themselves when no GPU adapter is available. See [ci.md](ci.md) for the full checks.

## Project background

The free-market and anti-IP position in the README follows the argument that ideas are not scarce: copying code does not take it away from its author. See the [patents and copyrights section of Man, Economy, and State](https://mises.org/online-book/man-economy-and-state-power-and-market/10-monopoly-and-competition/7-patents-and-copyrights) and Against Intellectual Property, Journal of Libertarian Studies 15:2, 2001, section "Scarcity and Ideas". The first treats patents as state monopoly grants while retaining copyright as contract; the second rejects intellectual property more broadly. A public kernel lets small vendors, scripts, plug-ins and individual products compete without renting a closed kernel.

SolidDraft and Catallax also test whether AI coding agents, directed and reviewed by humans, can build and maintain a CAD application and geometry kernel. That remains an open question.

SolidDraft is not affiliated with, sponsored by or endorsed by Dassault Systèmes, Autodesk, Siemens or NVIDIA. SolidWorks, DraftSight, Parasolid and other product names belong to their owners.
