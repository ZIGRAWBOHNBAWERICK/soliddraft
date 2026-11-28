# GPU viewport acceptance

These manual gates exercise the shipping viewport. Run them on a physical Mac or on native Windows. Do not run acceptance on a Linux build host, under WSL, or with a headless browser. A gate passes only with a hardware adapter and a matching picture of the original canonical P1 bracket. These are display checks, not mesh certificates or timing tests.

Install Python 3 with Pillow in an isolated environment. Build the native application with `cargo build --locked -p soliddraft-app` and the shipping site with `trunk build --release --locked` from `crates/soliddraft-web`. Set `--out` to a new directory outside the checkout for each invocation.

On the Mac, also run `cargo test --locked -p soliddraft-app --test gpu_viewport_acceptance --no-run`. Pass the test executable printed by Cargo as `--metal-harness`. It uses the app's wgpu renderer and reads back a Metal texture without depending on a visible window. An on-screen capture would need an unlocked, awake desktop; offscreen readback does not certify window presentation.

| Gate | Command arguments after `python ci/gpu-acceptance/run.py` | Adapter required |
| --- | --- | --- |
| macOS desktop | `native --app target/debug/soliddraft-app --metal-harness <test executable> --out /tmp/native` | Offscreen Metal |
| macOS browser | `safari --dist crates/soliddraft-web/dist --fixture /tmp/native --out /tmp/safari` | WebGPU in Safari 26+ |
| Windows desktop | `native --app target/debug/soliddraft-app.exe --out C:/Temp/native` | The shipping default hardware selection (D3D12 or Vulkan); no `WGPU_BACKEND` override |
| Windows browser | `chrome --dist crates/soliddraft-web/dist --fixture C:/Temp/native --out C:/Temp/chrome` | WebGPU in headed Chrome |

SafariDriver requires **Develop → Developer Settings → Allow remote automation** in the logged-in Safari user session. Browsers draw nothing for a hidden page, and in a locked macOS session every Safari window is hidden: the app starts, but it never paints or configures its canvas. The Safari gate therefore refuses a locked session and keeps the display on while it runs. If the page is hidden when a wait times out, the error says so. Chrome uses `chrome-devtools-axi` on `PATH` and starts a separate headed browser. Keep hardware acceleration enabled. Neither browser gate reuses a personal profile. The loopback server adds a test probe to the shipping HTML without changing its Wasm or JavaScript modules. It records the adapter of the device configured on the actual SolidDraft canvas. Being able to request a hardware adapter separately is not enough.

The native gate replays the checked-in [canonical P1 scenario](../scenarios/canonical-p1.json) through the desktop app to its `original` checkpoint and saves a scratch part. The browsers open that part through the existing file-drop path. Each gate retains `result.json`, the empty and P1 frames, and the extracted P1 silhouette used for comparison. The comparison omits window chrome, subtracts the empty frame, and allows differences in display scale and edge antialiasing. It requires at least 0.84 intersection-over-union with `p1-reference.pgm`. Do not regenerate the reference to clear a regression; inspect the screenshots first.

## GPU preview

`preview.py` checks that the 18 scenarios of the phase 5 speed set show their preview from the GPU mesher. Run it on the Mac. Build the release CLI with `cargo build --release --locked -p soliddraft-cli`. Build `catallax-verify` from the pinned Catallax checkout. The shipping site comes from the dist build above.

| Gate | Command arguments after `python ci/gpu-acceptance/preview.py` |
| --- | --- |
| macOS desktop | `native --soliddraft target/release/soliddraft --catallax-verify <verifier> --metal-harness <test executable> --out /tmp/preview-native` |
| macOS desktop control, not a gate | the same with `--gpu-off` and a fresh `--out` |
| macOS browser | `safari --dist crates/soliddraft-web/dist --fixtures /tmp/preview-native/fixtures --out /tmp/preview-safari` |

`--metal-harness` is the `gpu_viewport_acceptance` test executable, as for the P1 gate. The native gate replays each scenario with `ci/run-scenarios.py`. Each displayed step must run on a hardware Metal adapter with the GPU preview on. The scenario must show GPU previews. The viewport must ask the kernel for no CPU preview. Faces the mesher refuses are drawn by the CPU preview and are counted apart, by refusal reason. A scenario is strict when its step kernel logs hold no CPU preview call at all; saved display data and field bodies keep theirs, so few are. The gate sums the checked-mesh calls of each revision from the kernel logs. It saves each scenario's last part for the browser gate.

The gate then copies every part file checked into the repository beside those parts, and the Metal harness opens each one in the app on its own Metal device. The app picks the adapter and installs the GPU preview itself. The harness reads the preview back from the mesher's buffers. Every face of the body must be drawn by the GPU or, when the mesher refuses it, by the CPU preview. No GPU edge may be cracked: an edge only one GPU triangle uses passes only where it meets a CPU-drawn face within the tolerance, or on a sheet. Every GPU vertex must lie within the preview tolerance of its own face in the checked mesh. That tolerance is the chord height the display asked for, or the description's 0.01 mm default when it asked for none. The gate lists, without failing on them, edges shared by faces wound the same way, edges used by more than two triangles, triangles drawn twice or back to back with a reversed copy, vertices without a unit normal, and a checked mesh that never replaces the preview. A part with no body, and a lattice or gyroid, which the field display draws, are listed as outside the corpus. Each part keeps `record.json`, the preview frame `preview.png` and the checked frame `checked.png` under `readback/<part>/`. `result.json` lists each refused face with its reason, the deviation, the open-edge counts, and the frame and upload times. The times are information, not a gate: the upload time is the first frame of a part less a second frame of the same meshes.

The browser gate opens each part in a fresh page, the scenarios' and the repository's, and reads the page's `sdGpuPreview` counts. Set `SOLIDDRAFT_GPU_PREVIEW=off` (or `?gpu-preview=off` in the browser) to keep the CPU preview on a machine with a GPU. CI runs the scenarios that way. `--gpu-off` runs the native gate so, as a control: it passes when every part shows the CPU preview and none the GPU preview, and it never counts as the gate.

The adapter-policy and browser-session tests run without a GPU, and `ci/run lint` runs them: `python -m unittest discover -s ci/gpu-acceptance`. For a real negative control on the Mac, run the Chrome command with `--force-software` and fresh output. It requests a fallback adapter and enables SwiftShader in the isolated browser. Expect exit 1 and an adapter record naming the forced fallback. Missing SwiftShader, a browser launch failure, or a timeout is not evidence that the software adapter was rejected.

The Windows gates need an interactive native desktop, a usable Python/Pillow environment, a native SolidDraft executable and shipping Wasm distribution, and headed Chrome with `chrome-devtools-axi`. If the binaries are not provided, install Rust with the native and `wasm32-unknown-unknown` targets and Trunk on the build host. Windows interop alone does not establish access to a hardware-accelerated desktop.
