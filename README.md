# V-carve CAM

A Rust project for combined endmill and V-bit carving toward a shared target: sloped walls, flat floors in broad regions, and shallower narrow details.

The [portable Windows build](flat-v-carve/README.md#portable-windows-application)
packages the CLI, local browser service, and web assets into one `cam.exe`.
From `flat-v-carve`, run `./scripts/build-portable.ps1`, then
`./artifacts/portable/cam.exe serve --open` to use the integrated workspace.

M0–M5 implement SVG jobs, endmill clearing, V-bit finishing/rest machining, combined stock previews, and bounded continuous stock verification. M6 adds LinuxCNC output with explicit machine profiles and numeric readback; actual controller validation remains pending.

Engine 0.7.2 adds spatial indexing, batched stock unions, and compact plan files for larger artwork. The [scalability report](docs/flat-v-carve/scalability-report.md) records the real flower import and 1×/10×/100× measurements, with full-pipeline limits tracked separately.

Engine 0.7.3 completes the unchanged saved flower job in 52–54 seconds for combined CLI planning and about 7 seconds for endmill alone on the measured Windows machine. The [CLI performance report](docs/flat-v-carve/flower-performance.md) documents the bottlenecks and reproducible profiling commands. Regenerate older plans from their saved jobs.

Live browser planning now keeps complete plans in temporary files and sends only
bounded previews to the UI. Verification and export reopen those files directly;
plan size no longer controls worker-message size. See the
[plan storage report](docs/flat-v-carve/web-ui/u7-plan-artifacts.md) for lifecycle,
remaining limits, and real-artwork checks. Rebuild and restart the portable
application to use the updated service and UI together.

The Rust workspace lives in [`flat-v-carve/`](flat-v-carve/README.md). Import and inspect a bundled Inkscape export with the pinned Rust toolchain:

```sh
cd flat-v-carve
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json --output artifacts/m2/preview.svg
```

See the [workspace README](flat-v-carve/README.md) for validation and development commands, [Windows setup](flat-v-carve/README.md#windows-setup) for the native MSVC toolchain and PowerShell commands, the [architecture](docs/flat-v-carve/architecture.md) for scope, and the [implementation plan](docs/flat-v-carve/implementation-plan.md) for progress. The [M6 capability report](docs/flat-v-carve/m6-capability-report.md) records machine contracts, output verification, and remaining controller validation.
