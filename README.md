# V-carve CAM

A Rust project for combined endmill and V-bit carving toward a shared target: sloped walls, flat floors in broad regions, and shallower narrow details.

The [portable Windows build](flat-v-carve/README.md#portable-windows-application)
packages the CLI, local browser service, and web assets into one `cam.exe`.
From `flat-v-carve`, run `./scripts/build-portable.ps1`, then
`./artifacts/portable/cam.exe serve --open` to use the integrated workspace.
GitHub Actions builds and tests it on every push to `main`.

M0–M5 implement SVG jobs, endmill clearing, V-bit finishing/rest machining, combined stock previews, and bounded continuous stock verification. M6 adds LinuxCNC output with explicit machine profiles and numeric readback; actual controller validation remains pending. The M7 browser workflow is implemented: import-to-export runs in the local UI with background planning, stock slices, gated export, a local tool library, and a labeled 3D stock simulator. The same UI also builds statically with the engine compiled to WebAssembly for browser-only hosting.

Engine 0.7.7 completes the unchanged real flower job (`real_data/flower_box-svg.job-real.json`) in **2.71–2.73 seconds** across five fresh CLI runs, using spatial indexing, checked stay-down routing, contour-following V-bit cuts, bounded simplification, and grouped/parallel stock construction. Wood-finish preset jobs (`real_data/flower_box-wood-balanced.job.json`, `flower_box-wood-finish.job.json`) trade a little speed for a finer floor. See the [workspace README](flat-v-carve/README.md) for benchmark and profiling commands.

The Rust workspace lives in [`flat-v-carve/`](flat-v-carve/README.md). Import and inspect a bundled Inkscape export with the pinned Rust toolchain:

```sh
cd flat-v-carve
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json --output artifacts/m2/preview.svg
```

Documentation lives in [`docs/flat-v-carve/`](docs/flat-v-carve): the [architecture](docs/flat-v-carve/architecture.md) for scope and components, the [technical design](docs/flat-v-carve/technical-design.md) for geometry and data contracts, the [implementation plan](docs/flat-v-carve/implementation-plan.md) for milestone status and the remaining M6/M8 work, the [web UI plan](docs/flat-v-carve/web-ui.md) for the browser product and its remaining U4/U6 stages, the [tool library guide](docs/flat-v-carve/tool-library.md), and the [M6 capability report](docs/flat-v-carve/m6-capability-report.md) for machine contracts and the pending controller validation. The [workspace README](flat-v-carve/README.md) holds validation and development commands, and [Windows setup](flat-v-carve/README.md#windows-setup) covers the native MSVC toolchain.
