# SolidDraft

Free, open-source parametric CAD for mechanical parts. It runs on your desktop and in your browser, on [Catallax](https://kernel.zigrawbohnbawerick.org/), an exact geometry kernel written in Rust.

We support free markets and reject intellectual property. Copying a program does not deprive anyone of their copy. Geometry is mathematics, and a CAD tool should not need permission from a closed kernel's owner to exist. SolidDraft and Catallax are one project in two repositories, free to use, change and share.

[Open in the browser](https://zigrawbohnbawerick.github.io/soliddraft/app/) · [Download for the desktop](https://github.com/ZIGRAWBOHNBAWERICK/soliddraft/releases) · [Website](https://zigrawbohnbawerick.github.io/soliddraft/)

## What works today

SolidDraft is alpha software. Keep backups of your documents. On main, targeting 0.5.0, these features work:

- The Edit Suite frame, with workspaces in the title bar, tools on the left, an options bar, a 3D viewport and feature history above the command line.
- Sketches with relations and dimensions. Extrude, cut, revolve, combine several bodies, drill holes, pattern and mirror. Edit features later, undo and redo, save and reopen parts. Supported sweeps, shells and drafts also work. See [modelling](docs/modelling.md).
- Ruled and smooth lofts through ordered closed sketches or supported planar faces. Offset grows or shrinks supported bodies by a signed distance; Thicken makes a solid from selected faces on the front or back side. Planar cases include shrinking a block and thickening its end face. Geometry and calculation limits still apply. See [lofts](docs/modelling.md#loft) and [operation acceptance tests](crates/soliddraft-app/tests/distance_pattern.rs).
- Fillets and chamfers on supported edges, including plane and cylinder combinations. Some blends and later operations still refuse unsupported geometry or exceed the calculation limit. See [current kernel limits](docs/needs-catallax.md).
- STEP and IGES import with repair review and the original source retained. Files with several solids create a separate body and import feature for each solid, in one undo step. Unsupported files leave the part unchanged. Export B-rep parts as STEP AP242, supported IGES 5.3, STL, 3MF and OBJ. See [import and export](docs/exchange.md).
- Desktop [assemblies](docs/assemblies.md#in-the-app). Insert parts and sub-assemblies, place them by position and rotation, save and reopen. Supported [mates](docs/mates.md) constrain components; [interference and clearance checks](docs/interference.md) report their proofs and refusals.
- ISO and ANSI drawings with aligned views, sections, details, dimensions and annotations. Export SVG, PDF and DXF. Hidden-line removal uses meshes; exported curves identify exact lines and circles or mesh polylines. Exact silhouettes remain in design.
- Lattices and gyroids with an unproved viewport preview and certified STL export. See [field modelling](docs/modelling.md#terminal-lattice) and [export limits](docs/exchange.md#terminal-fields).
- A desktop [assistant and MCP interface](docs/assistant.md), plus a [headless command line](docs/development.md#command-line) for recipes, parts, drawings and exports. The public browser app has no AI assistant or API key.

Broader modelling domains remain in design; a working command can still refuse unsupported inputs. See the [roadmap](ROADMAP.md) for what works and what is still planned. A failed or pending feature cannot export an unproved whole part.

## Get SolidDraft

[Open the browser app](https://zigrawbohnbawerick.github.io/soliddraft/app/). The kernel runs inside your tab. Files open by upload and save by download. Nothing to install. Assemblies are desktop-only, and browser geometry calculations run on the interface thread. See [the browser build](docs/web.md) for local builds and optional self-hosted assistant support.

[Desktop downloads](https://github.com/ZIGRAWBOHNBAWERICK/soliddraft/releases) include Linux AppImage, Debian package and portable archive, and a macOS disk image. Builds are unsigned for now, with checksums and build attestations. Windows downloads are deferred.

Start with a sketch on a plane or planar face, add dimensions with `D`, then extrude or cut. Press `?` in the app for keyboard shortcuts.

## Build from source

Install Rust 1.95 or later, then run these commands from the repository root. There is no C++ kernel, submodule or separate kernel build.

```sh
cargo build --release --locked
cargo run --release -p soliddraft-app -- --demo
```

Without `--demo`, the app opens its start page. Pass a part, assembly or drawing file to open it. See [development](docs/development.md) for command-line examples, tests and contribution checks, and [application scenarios](docs/application-loop.md) for agent-driven verification.

## Licence, support and thanks

Started by Zigrawbohnbawerick Pty Ltd. Written with AI coding agents, with human direction and review. The [MIT licence](LICENSE) grants permission to use, change and share the code while retaining its notice. Copyright attaches by default; we use the licence to give those rights away. See [NOTICE](NOTICE) for credits and the bundled Inter font's SIL Open Font License 1.1.

Support SolidDraft and Catallax through [Ko-fi](https://ko-fi.com/pedropaiva56920) or [GitHub Sponsors](https://github.com/sponsors/BohnBawerick). Both fund the same project. Thank you to everyone who contributes code, reports bugs or funds the work.

https://zigrawbohnbawerick.org/
