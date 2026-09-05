# Flat V-carve CAM

An isolated Rust workspace for the combined endmill/V-bit planner described in the [project docs](../docs/flat-v-carve/architecture.md). M0–M5 implement geometry, SVG jobs, both planners, recorded-motion previews, bounded continuous stock verification, and rounded-coordinate checks. Machine output and the browser workflow follow in M6–M7.

## Run

Install [Rust with rustup](https://www.rust-lang.org/tools/install). The workspace pins Rust **1.95.0**; rustup selects it when running Cargo here. Tested native targets are **x86_64-pc-windows-msvc** on Windows and **x86_64-unknown-linux-gnu** on Ubuntu 24.04.4 under WSL2. See [Windows setup](#windows-setup) for prerequisites and PowerShell commands. WebAssembly builds have not been tested.

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

Exit codes are `0` for successful import/inspection, valid editable jobs, or a complete/empty endmill stage; `1` for invalid inputs or an incomplete/inconclusive stage; and `2` for argument/I/O errors. An invalid inspected job or stale plan replaces the previous SVG/report with an error result. Import failures leave existing job files untouched, so callers must check the exit status.

## M3 endmill planning and stock

```sh
cargo run --release --locked -p cam-app -- plan fixtures/m3/island.json \
  --output artifacts/m3/island/plan.json
cargo run --release --locked -p cam-app -- inspect artifacts/m3/island/plan.json \
  --output artifacts/m3/island/preview.svg --report artifacts/m3/island/report.json
cargo run --release --locked -p cam-app -- verify artifacts/m3/island/plan.json \
  --output artifacts/m3/island/verification.json
```

The [M3 fixtures](fixtures/m3/README.md) supply **synthetic test settings**, including feeds and spindle speed. Imported jobs still have no machining defaults. Planning requires stock thickness, depth, horizontal wall allowance, endmill dimensions/capability/feeds/spindle/stepdown/stepover, V-bit geometry to define the target angle, planning tolerances, and `endmill_planning` settings for the clearance plane, start XY, entry, strategy, and resource limits. V-bit cutting settings and finish-quality limits remain editable for M4.

`depth_dependent` clearing generates offset loops inside each layer's admissible center region; `deepest_region` uses the deepest region at every stepdown. Direct plunges require a plunge-capable endmill and explicit plunge feed. Ramps require `ramp_capable: true` and an explicit angle/feed. Every disconnected loop retracts and links at the configured clearance Z. M3 limits stepover to half the tool diameter; it does not optimize travel or calculate machine-specific cutting parameters.

Each saved plan embeds the job and records actual XYZ moves, identity fingerprints, generation issues, and stock reports. `inspect` and `verify` rebuild clearance and stock from the motions, ignoring cached analysis. Editing the job or motions invalidates the fingerprint and requires replanning. Job schema 1 migrates to schema 2 without inventing new settings; plan schema 1 is a separate artifact format tied to the generating engine version.

The preview shows layer paths, removed stock, and remaining target. Missing accessible floor is pink; possible overcut is purple. No-access stages are empty. Exact-fit contacts and insufficient numerical margin are inconclusive; unsupported entries and missed floors are reported explicitly. Partial plans remain inspectable and exit with status 1.

M3 checks whole-segment center clearance and compares actual endmill sweeps at planned depth slices. `complete` refers to this endmill-stage coverage within the declared XY tolerance. Remaining target includes slopes, wall allowance, and detail for M4. It does not establish combined finish quality, adaptive full-volume verification (M5), or readiness for machine output (M6). See the [M3 capability report](../docs/flat-v-carve/m3-capability-report.md) for formulas, evidence, and limits.

## M4 combined finishing and rest machining

```sh
cargo run --release --locked -p cam-app -- plan fixtures/m4/curved-medial.json \
  --output artifacts/m4/curved-medial/plan.json
cargo run --release --locked -p cam-app -- inspect artifacts/m4/curved-medial/plan.json \
  --output artifacts/m4/curved-medial/preview.svg --report artifacts/m4/curved-medial/report.json
cargo run --release --locked -p cam-app -- verify artifacts/m4/curved-medial/plan.json \
  --output artifacts/m4/curved-medial/verification.json
```

`plan` generates both stages when `vbit_planning` is configured; otherwise it retains endmill-only behavior. `--stage combined` explicitly requests both stages, and `--stage endmill` generates only M3 roughing, including from an M4 job. Both artifact kinds support `inspect` and `verify`.

M4 combines full-depth boundary contours, variable-depth medial-axis branches, and clipped floor lanes. Finite tip geometry is used throughout. Branches split at positive-cut and depth-cap transitions; curved branches are subdivided with XYZ error and continuous clearance checks. Exact-fit lines and points remain represented, using a small guarded depth reserve where necessary. All endmill work precedes the V-bit; the complete achievable boundary/rising-detail family runs last, after bounded cleanup.

Each V-bit depth pass begins from recorded endmill stock. Floor lanes are divided into thirds so an interior section can be omitted only when its **entire cutter sweep**, including its flank, fits inside a recorded endmill sweep. Other sections are retained. Final finishing is always retained. M4 uses direct plunge entries with explicit V-bit `plunge_capable: true` and plunge feed; it does not infer that capability from the cutter dimensions or use V-bit ramps.

Set the V-bit cutting/plunge feeds, spindle speed, stepdown, stepover, `max_floor_ridge_mm`, `max_detail_residual_mm`, and `vbit_planning` limits explicitly. Job schema 3 adds these planning controls and per-tool plunge capability; schemas 1 and 2 migrate without inventing values. The [M4 fixtures](fixtures/m4/README.md) provide **synthetic test settings**, not machining recommendations.

Pointed-bit floor lane spacing is constrained by the permitted ridge. A pointed V-bit with zero allowed ridge is rejected when residual floor area needs clearing; finite flat tips can support zero-ridge clearing with overlapping lanes. Cutter-limited detail uses independent reachability bounds and is reported separately from missed reachable material.

Saved combined plans bind both stages, tool-transition markers, path execution records, generation issues, and the engine/job identity. Reopening recomputes the actual sweeps and quality report. Changing cached analysis cannot create a successful result. The verifier also rejects omitted depth passes without stock evidence and an absent or incomplete final finish.

M4 `complete` means candidate-family completion, continuous segment clearance, accessible-floor slice coverage, and quality at the reported sample lattice/motion witnesses. Floor coverage is checked at `D - allowed_ridge - numerical_depth_budget`, where the explicitly reported numerical depth budget is half the verification tolerance. The report also retains XY coverage tolerance. Sampled residual maxima are **not global error bounds**; use M5 verification below for bounded continuous checks. See the [M4 capability report](../docs/flat-v-carve/m4-capability-report.md).

## M5 continuous verification and coordinate precision

From this workspace, using PowerShell or one-line shell commands:

```powershell
cargo run --release --locked -p cam-app -- plan fixtures/m4/curved-medial.json --output artifacts/m5/curved-medial/plan.json
cargo run --release --locked -p cam-app -- verify artifacts/m5/curved-medial/plan.json --output artifacts/m5/curved-medial/verification.json --decimal-places 6 --preview artifacts/m5/curved-medial/verification.svg
```

`verify` authenticates the combined plan and checks the entire normalized target and cutting-sweep domain, including islands and exterior material. Adaptive cells bound overcut, floor ridges, unreachable nominal detail, and other reachable residue. Depth bands and integrated volumes carry separate area/volume bounds. `inspect` continues to show M4 planning evidence; the M5 finding preview uses red for failures and amber for unresolved bounds.

Check `verification.status`: `passed`, `failed`, or `inconclusive`. Only `passed` exits 0; failed or inconclusive verification exits 1, and argument/I/O errors exit 2. The outer `valid` field means the artifact was readable/authenticated and is not a finish-quality result. Cached analysis is never trusted.

`--decimal-places 0..9` checks the actual formatted XYZ coordinates independently. Omitting it verifies original coordinates only. The report retains both results, their fingerprints, coordinate changes, located findings, maximum-error intervals, and uncertainty. No G-code or machine defaults are generated. M6 will require the actual machine profile and output precision.

Resource controls are `--max-cells` (default 1,000,000; total across refinement passes per coordinate set), `--max-depth` (24), `--reachability-cells` (4,096 per point query), and `--max-depth-bands` (512). Exhausted bounds remain inconclusive. Lowering limits cannot produce a coarse-grid pass. The geometric model is the rebuilt normalized polygon; source flattening/snap error is reported separately.

M5 enforces the explicit ridge and detail limits without adding M4's numerical allowance. Consequently, the M4 zero-ridge `contact-line` and `contact-point` examples do not pass M5: their guarded cap motions leave about 0.01 mm. The [M5 fixtures](fixtures/m5/README.md) record these expected failures alongside successful and resource-limited cases. Endmill-only `verify` retains its M3 stage contract; the new M5 options require a combined plan.

Engine **0.6.0** invalidates plans created by older engines. Regenerate plans from saved jobs; job schema 3 still accepts schemas 1 and 2. The [M5 capability report](../docs/flat-v-carve/m5-capability-report.md) records the methods, regression evidence, Windows performance measurements, and remaining limits. Run `scripts/benchmark-m5.ps1` from PowerShell 7 after a release build to reproduce the ten release cases and their JSON/SVG artifacts.

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

## Windows setup

Install the [MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html): Visual Studio Build Tools with the **Desktop development with C++** workload, including the x64/x86 compiler and a Windows SDK. This workspace was built with Visual Studio Build Tools 2019, MSVC **14.29.30133**, and Windows SDK **10.0.19041.0** on Windows build **26200.9168** (x64).

Install the x64 Windows version of [rustup](https://www.rust-lang.org/tools/install) using the MSVC host, then open a new PowerShell window so `%USERPROFILE%\.cargo\bin` is on `PATH`. From the repository root:

```powershell
Set-Location flat-v-carve
rustup toolchain install 1.95.0 --profile minimal --component clippy --component rustfmt
rustup show
cargo build --workspace --locked
cargo build --release --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

`rustup show` should report `1.95.0-x86_64-pc-windows-msvc` as active for this workspace. The executables are `target\debug\cam.exe` and `target\release\cam.exe`. Run the release CLI from PowerShell:

```powershell
.\target\release\cam.exe import fixtures/m2/inkscape-export.svg --output artifacts/windows/job.json
.\target\release\cam.exe inspect artifacts/windows/job.json --output artifacts/windows/preview.svg --report artifacts/windows/report.json
```

The multiline examples elsewhere in this README use POSIX shell `\` continuations; in PowerShell, put each command on one line as above. A Developer PowerShell session is not required for this workspace when the MSVC prerequisites are installed.

## Develop

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

After dependencies have been fetched, these commands also accept `--offline` (except `cargo fmt`, which needs no network). `Cargo.lock` is part of the project. Both geometry crates have default features disabled, and all direct dependency versions are pinned.

`cam-core` contains in-memory geometry contracts, narrow dependency adapters, SVG normalization, portable jobs, cutter/target models, independent distance queries, both planners, linear motions, stock analysis, and preview calculations. It has no filesystem or process access. `cam-app` handles command arguments, fixtures, JSON/SVG output, and build metadata. The debug SVGs visualize source geometry, recorded paths, combined stock slices, and sampled finish quality; no G-code is generated.

See the [M4 capability report](../docs/flat-v-carve/m4-capability-report.md) for combined planning, the [M3 report](../docs/flat-v-carve/m3-capability-report.md) for endmill evidence, the [M2 report](../docs/flat-v-carve/m2-capability-report.md) for importer/job evidence and the [M1 report](../docs/flat-v-carve/m1-capability-report.md) for finite-tip and exact-fit geometry. The [M0 report](../docs/flat-v-carve/m0-capability-report.md) records the underlying geometry dependencies and precision policy.
