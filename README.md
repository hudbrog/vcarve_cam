# V-carve CAM

A Rust project for combined endmill and V-bit carving toward a shared target: sloped walls, flat floors in broad regions, and shallower narrow details.

M0 and M1 implement the geometry foundation, validated cutter models, target geometry, tool-center access, and headless previews. SVG import, cutting paths, stock verification, and LinuxCNC output are planned in subsequent milestones.

The Rust workspace lives in [`flat-v-carve/`](flat-v-carve/README.md). Run the eight procedural previews with the pinned Rust toolchain:

```sh
cd flat-v-carve
cargo run --release --locked -p cam-app -- target-demo --output artifacts/m1
```

See the [workspace README](flat-v-carve/README.md) for validation and development commands, the [architecture](docs/flat-v-carve/architecture.md) for scope, and the [implementation plan](docs/flat-v-carve/implementation-plan.md) for progress. The [M1 capability report](docs/flat-v-carve/m1-capability-report.md) records test evidence and numerical limits.
