# GPU viewport acceptance

These manual gates exercise the shipping viewport, not a second renderer. Run them on a physical Mac or on native Windows with an interactive desktop. Do not run acceptance on a Linux build host, under WSL, or with a headless browser. A gate passes only with a hardware adapter and a matching picture of the original canonical P1 bracket. These are display checks, not mesh certificates or timing tests.

Install Python 3 with Pillow in an isolated environment. Build the native application with `cargo build --locked -p soliddraft-app` and the shipping site with `trunk build --release --locked` from `crates/soliddraft-web`. Set `--out` to a new directory outside the checkout for each invocation.

| Gate | Command arguments after `python ci/gpu-acceptance/run.py` | Adapter required |
| --- | --- | --- |
| macOS desktop | `native --app target/debug/soliddraft-app --out /tmp/native` | Metal |
| macOS browser | `safari --dist crates/soliddraft-web/dist --fixture /tmp/native --out /tmp/safari` | WebGPU in Safari 26+ |
| Windows desktop | `native --app target/debug/soliddraft-app.exe --out C:/Temp/native` | The shipping default hardware selection (D3D12 or Vulkan); no `WGPU_BACKEND` override |
| Windows browser | `chrome --dist crates/soliddraft-web/dist --fixture C:/Temp/native --out C:/Temp/chrome` | WebGPU in headed Chrome |

SafariDriver requires **Develop → Developer Settings → Allow remote automation** in the logged-in Safari user session. Chrome uses `chrome-devtools-axi` on `PATH` and starts a separate headed browser. Keep hardware acceleration enabled. Neither browser gate reuses a personal profile. The loopback server adds a test probe to the shipping HTML without changing its Wasm or JavaScript modules. It records the adapter of the device configured on the actual SolidDraft canvas. Being able to request a hardware adapter separately is not enough.

The native gate replays the checked-in [canonical P1 scenario](../scenarios/canonical-p1.json) through the desktop app to its `original` checkpoint and saves a scratch part. The browsers open that part through the existing file-drop path. Each gate retains `result.json`, the empty and P1 frames, and the extracted P1 silhouette used for comparison. The comparison omits window chrome, subtracts the empty frame, and allows differences in display scale and edge antialiasing. It requires at least 0.84 intersection-over-union with `p1-reference.pgm`. Do not regenerate the reference to clear a regression; inspect the screenshots first.

The adapter-policy tests run without a GPU: `python -m unittest discover -s ci/gpu-acceptance`. For a real negative control on the Mac, run the Chrome command with `--force-software` and fresh output. It requests a fallback adapter and enables SwiftShader in the isolated browser. Expect exit 1 and an adapter record naming the forced fallback. Missing SwiftShader, a browser launch failure, or a timeout is not evidence that the software adapter was rejected.
