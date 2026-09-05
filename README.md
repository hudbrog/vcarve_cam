# V-carve CAM

A Rust project for combined endmill and V-bit carving toward a shared target: sloped walls, flat floors in broad regions, and shallower narrow details.

M0–M5 implement SVG jobs, endmill clearing, V-bit finishing/rest machining, combined stock previews, and bounded continuous stock verification. M6 adds LinuxCNC output with explicit machine profiles and numeric readback; actual controller validation remains pending.

Engine 0.7.2 adds spatial indexing, batched stock unions, and compact plan files for larger artwork. The [scalability report](docs/flat-v-carve/scalability-report.md) records the real flower import and 1×/10×/100× measurements, with full-pipeline limits tracked separately.

The Rust workspace lives in [`flat-v-carve/`](flat-v-carve/README.md). Import and inspect a bundled Inkscape export with the pinned Rust toolchain:

```sh
cd flat-v-carve
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json --output artifacts/m2/preview.svg
```

See the [workspace README](flat-v-carve/README.md) for validation and development commands, [Windows setup](flat-v-carve/README.md#windows-setup) for the native MSVC toolchain and PowerShell commands, the [architecture](docs/flat-v-carve/architecture.md) for scope, and the [implementation plan](docs/flat-v-carve/implementation-plan.md) for progress. The [M6 capability report](docs/flat-v-carve/m6-capability-report.md) records machine contracts, output verification, and remaining controller validation.
