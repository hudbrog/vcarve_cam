# Flat V-carve CAM

An isolated Rust workspace for the combined endmill/V-bit planner described in the [project docs](../docs/flat-v-carve/architecture.md). M0–M2 implement geometry, cutter/target models, SVG import, editable jobs, region selection, and headless previews. Toolpaths, machine output, and the browser workflow belong to later milestones.

## Run

Install [Rust with rustup](https://www.rust-lang.org/tools/install). The workspace pins Rust **1.95.0**; rustup selects it when running Cargo here. The tested target is **x86_64-unknown-linux-gnu**, Ubuntu 24.04.4 under WSL2. Windows-native and WebAssembly builds have not been tested.

From this directory:

```sh
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg \
  --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json \
  --output artifacts/m2/preview.svg --report artifacts/m2/report.json
cargo run --locked -p cam-app -- validate-job artifacts/m2/job.json
```

`import` embeds the SVG in schema-versioned JSON, selects all visible supported regions, and leaves machining settings unset. The original SVG file is no longer needed. Edit job settings in JSON; `inspect` rebuilds geometry from the snapshot, and `validate-job` writes a JSON inspection to stdout. Missing machining settings are allowed while editing; supplied invalid values are rejected. No geometry cache in the job is trusted.

Select a component using an ID listed in the inspection report or preview:

```sh
cargo run --locked -p cam-app -- select artifacts/m2/job.json \
  --select letter-b::0 --output artifacts/m2/selected.json
cargo run --locked -p cam-app -- inspect artifacts/m2/selected.json \
  --output artifacts/m2/selected.svg
```

Repeat `--select` for multiple regions. `select` with no IDs saves an empty selection. `import` also accepts `--select` and `--tolerance <mm>` (default 0.001 mm). Component IDs use `source-id::index`, assigned before workpiece placement. The job's `import.placement` contains `origin_mm`, `scale`, and `rotation_deg`: workpiece XY is `scale * rotate(page_XY - origin_mm)`. Page XY has its origin at the lower left, with Y upward.

Supported SVG input includes explicit page dimensions, mm/cm/in/pt/pc/px and unitless CSS pixels, `viewBox`, affine transforms, all path commands including elliptical arcs, rectangles/rounded rectangles, circles, ellipses, polygons, compound fills, and inherited solid styles/visibility. Text and strokes must be converted to paths in Inkscape. CSS stylesheets, references/clones, gradients, clipping/masking/filter effects, animation, nested viewports, relative lengths, and artwork outside the page are rejected with diagnostics. The [M2 report](../docs/flat-v-carve/m2-capability-report.md) defines the supported subset and precision limits.

M2 exit codes are `0` for successful import/inspection or valid editable jobs, `1` for invalid SVG/jobs or unavailable planning, and `2` for argument/I/O errors. An invalid inspected job, including malformed job JSON, replaces the previous SVG/report with an error result. Import failures leave existing job files untouched, so callers must check the exit status. `cam plan job.json --output diagnostics.json` validates the job and reports `PLANNING_NOT_IMPLEMENTED`; cutting paths start in M3.

## M1 target and cutter previews

```sh
cargo run --release --locked -p cam-app -- target-demo --output artifacts/m1
```

The eight [M1 models](fixtures/m1/) cover wide/narrow channels, a finite-tip corner, an island, exact-fit lines and points, and mixed components. All dimensions are synthetic examples in millimeters. Each model produces `input.json`, `report.json`, and `preview.svg` in its own output subdirectory; `artifacts/m1/report.json` summarizes the run.

Plan views overlay nominal depth sections, V-bit centers, and endmill centers including wall allowance. Cross-sections show the nominal surface separately from bounded estimates of the best removal achievable by the modeled V-bit. Those estimates allow arbitrary feasible poses; they do not represent a planned cutting sequence or combined stock simulation.

Edit a copy of a model, validate its parameters, and regenerate its preview:

```sh
cp fixtures/m1/finite_tip_corner.json model.json
cargo run --locked -p cam-app -- validate-model --input model.json
cargo run --locked -p cam-app -- target-preview \
  --input model.json --output artifacts/edited
```

`validate-model` writes JSON to stdout. M1 commands return `0` for valid settings or a complete preview, `1` for rejected settings or an inconclusive preview, and `2` for command/JSON/I/O errors. A preview can be inconclusive when a center region is too small for the polygon grid or reachability exceeds its numerical/resource budget. Reports retain the diagnostics and available bounds. A parsed model with invalid settings replaces the previous preview with an error view.

The strict M1 input format is defined by [`ModelInput`](crates/cam-core/src/preview.rs) and illustrated by the fixtures. Set `ticks_per_mm` to `null` for automatic precision selection; `geometry_tolerance_mm` and `preview_depth_tolerance_mm` control different errors. Exact-fit lines/points have zero clearance margin and do not establish a usable entry. This remains a separate geometry experiment format; M2 SVG jobs use [`Job`](crates/cam-core/src/job.rs).

## M0 geometry checks

```sh
cargo run --release --locked -p cam-app -- geometry-spike --output artifacts/m0
```

The bundled [fixture suite](fixtures/m0.json) runs without an input artwork file. Each fixture checks analytic quantities, independently evaluated distances, topology, or a required rejection. Expected invalid-input diagnostics count as successful tests of rejection behavior.

Outputs:

- `artifacts/m0/report.json`: versioned aggregate results, build identity, precision settings, measurements, diagnostics, and limitations.
- `artifacts/m0/<fixture>.json`: individual result, including input and derived geometry.
- `artifacts/m0/<fixture>.svg`: input, polygon output, and finite Voronoi edges in millimeters.
- `artifacts/m0/repro/<fixture>.json`: standalone input for reproducing either a success or failure.

Replay one fixture:

```sh
cargo run --locked -p cam-app -- geometry-spike \
  --fixture artifacts/m0/repro/voronoi_concave.json \
  --output artifacts/replay
```

Exit codes: `0` for all expectations met, `1` for a failed capability check, `2` for command/input/I/O errors. Progress goes to stderr; artifacts go to the specified directory. Re-running replaces the same named artifact files. An individual reproducer is a fixture object; the bundled suite is an array of those objects.

## Develop

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

After dependencies have been fetched, these commands also accept `--offline` (except `cargo fmt`, which needs no network). `Cargo.lock` is part of the project. Both geometry crates have default features disabled, and all direct dependency versions are pinned.

`cam-core` contains in-memory geometry contracts, narrow dependency adapters, SVG normalization, portable jobs, cutter/target models, independent distance queries, and preview calculations. It has no filesystem or process access. `cam-app` handles command arguments, fixtures, JSON/SVG output, and build metadata. The debug SVGs visualize geometry; no toolpaths are generated yet.

See the [M2 capability report](../docs/flat-v-carve/m2-capability-report.md) for importer/job evidence and the [M1 report](../docs/flat-v-carve/m1-capability-report.md) for finite-tip and exact-fit geometry. The [M0 report](../docs/flat-v-carve/m0-capability-report.md) records the underlying geometry dependencies and precision policy.
