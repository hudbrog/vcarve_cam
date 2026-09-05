# V-carve CAM

A Rust project for combined endmill and V-bit carving toward a shared target: sloped walls, flat floors in broad regions, and shallower narrow details.

M0–M2 implement the geometry foundation, cutter/target models, SVG import, portable editable jobs, region selection, and headless previews. Cutting paths, stock verification, and LinuxCNC output are planned in subsequent milestones.

The Rust workspace lives in [`flat-v-carve/`](flat-v-carve/README.md). Import and inspect a bundled Inkscape export with the pinned Rust toolchain:

```sh
cd flat-v-carve
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json --output artifacts/m2/preview.svg
```

See the [workspace README](flat-v-carve/README.md) for validation and development commands, the [architecture](docs/flat-v-carve/architecture.md) for scope, and the [implementation plan](docs/flat-v-carve/implementation-plan.md) for progress. The [M2 capability report](docs/flat-v-carve/m2-capability-report.md) records test evidence and import limits.
