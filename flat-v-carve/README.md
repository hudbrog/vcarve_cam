# Flat V-carve CAM

An isolated Rust workspace for the combined endmill/V-bit planner described in the [project docs](../docs/flat-v-carve/architecture.md). M0–M5 implement geometry, SVG jobs, both planners, recorded-motion previews, and bounded continuous stock verification. M6 implements LinuxCNC output and numeric readback; actual controller validation remains pending. The M7 workflow is implemented in `cam-gui`: import-to-export with background planning, verification, gated output, a tool library, and a 3D stock simulator, running either as a native window or as a browser application with the in-browser WebAssembly engine.

## Desktop and browser GUI

[`crates/cam-gui`](crates/cam-gui/README.md) is the only GUI, built in the main
Cargo workspace with shared Rust state and wgpu rendering. Run
`cargo run -p cam-gui --release --locked`, or use
`./scripts/build-gui.ps1` and `./scripts/build-gui.ps1 -Target web` for review
artifacts. New GUI work belongs in this crate; `experiments/gui1` preserves the
completed framework experiment, and the former React workspace UI (`web/`) with
its `cam-wasm` engine crate has been removed.

## Portable Windows application

GitHub Actions builds and tests the portable app on pushes to `main`, pull
requests targeting `main`, and manual runs. Open [Build and test](https://github.com/hudbrog/vcarve_cam/actions/workflows/build.yml),
choose a successful run, and download the `flat-v-carve-windows-x64` artifact.
Extract `cam.exe` and run any CLI command; the GUI is built separately. Each
artifact includes `SHA256SUMS` and is retained for 30 days.

CI uses the pinned Rust toolchain and Node.js 24 (for the browser GUI build).
It checks formatting, Clippy, and Rust tests, then builds the native GUI, the
browser GUI, and the portable command-line EXE. The build runs on a fresh
Windows x64 runner.

The test step runs `scripts/run-tests-parallel.ps1`, which executes the compiled
test binaries with a worker pool instead of one after another: the workspace has
68 binaries whose durations sum to roughly 50s while the longest single binary is
about 12s, so the pool cuts the step from ~50s to ~20s on a 16-thread machine and
from ~70s to ~50s on a 4-core runner. `scripts/measure-validation.ps1` records
per-step wall time for the whole pipeline, and
`scripts/validation-timing-report.mjs` turns that record into a summary and
charts.

Build the portable command-line executable:

```powershell
./scripts/build-portable.ps1
# Add -Offline when Cargo dependencies are already cached.
.\artifacts\portable\cam.exe --help
```

Copy `artifacts/portable/cam.exe` to another directory or supported Windows x64
machine. It needs no Node.js, Rust, or separately installed Visual C++ runtime. It
contains the CLI and the static file server for the browser build. It no longer
embeds a UI, so `cam serve` needs `--ui-dir <directory>`; the native GUI ships
separately as `cam-gui.exe` from `./scripts/build-gui.ps1`. The EXE still uses
Windows system DLLs.

`cam serve` binds to `127.0.0.1:4848`; `--port 0` chooses an available port and
prints the URL, and `--port <number>` requires that port to be free. `--open`
opens the browser. It publishes the UI directory read-only and answers GET/HEAD
only: the browser build plans in its own WebAssembly worker, so there is no
server-side planning, verification, export or library API and no writable
server state.

For development, `cam serve --ui-dir <directory>` and the `cam-web` alias serve a
prebuilt UI directory; point it at `artifacts/gui/browser` after
`./scripts/build-gui.ps1 -Target web`. The Windows release script links the C
runtime statically.

## Run

Install [Rust with rustup](https://www.rust-lang.org/tools/install). The workspace pins Rust **1.95.0**; rustup selects it when running Cargo here. Tested native targets are **x86_64-pc-windows-msvc** on Windows and **x86_64-unknown-linux-gnu** on Ubuntu 24.04.4 under WSL2. See [Windows setup](#windows-setup) for prerequisites and PowerShell commands. The browser GUI is built from `crates/cam-gui` for **wasm32-unknown-unknown**; the former React workspace and its `cam-wasm` engine crate are gone.

From this directory:

```sh
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg \
  --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json \
  --output artifacts/m2/inspection.json
```

There is exactly one job document: **schema 5**. `import` writes the same
document an SVG import produces in the GUI — one artwork item holding the
embedded bytes, a page-sized stock, the planning tolerances and **no
operations** — and prints the path it wrote. It never invents a machining
step: add Face, Flat V-carve, Profile, Drag knife or Drill explicitly. The original
SVG file is no longer needed. `inspect` is the read-only view of a stored
document (artwork tree, geometry bounds, assignments, machine readout); it is
the same implementation as `cam collection inspect`.

An older document is refused by name, never converted: opening one exits 2
with `COLLECTION_SCHEMA_UNSUPPORTED` (or `CAM_JOB_SCHEMA_VERSION` in the
workspace) and writes nothing. See the
[job model](../docs/flat-v-carve/job-model.md).

Selection binds to an operation rather than to the document, so it is made
where the operation is edited in the workspace pickers. The CLI can inspect and
plan stored selections; an operation-scoped selection command remains in the
[backlog](../docs/flat-v-carve/backlog.md). `import` accepts `--tolerance <mm>`
(default 0.001 mm). Geometry
IDs are owner-qualified (`item:kind:local`), and an artwork item's `placement`
contains `origin_mm`, `scale`, and `rotation_deg`: workpiece XY is
`scale * rotate(page_XY - origin_mm)`. Page XY has its origin at the lower
left, with Y upward.

Supported SVG input includes physical units, viewBox, transforms, paths/arcs, basic shapes, compound fills and the documented CSS subset. Operations select filled regions, contours, centerlines or drill markers from the same import. Text and unsupported rendering effects require conversion or removal; external references/stylesheets are refused. See [SVG normalization](../docs/flat-v-carve/technical-design.md#4-svg-normalization) for supported readings and diagnostics.

Exit codes are `0` for a successful command, `1` for a completed
check/verification that failed or stayed inconclusive, and `2` for
argument/I/O errors and for a document or SVG the one model cannot read. Import
failures write nothing, so callers must check the exit status.

## Cutting profiles and the tool library

The library is a portable JSON file the GUI owns: **Carve & tools → Manage tool
library** imports, edits, exports and captures records, and there is no
server-side or command-line library backend any more (the `cam tool-library`
command and the HTTP library endpoints were deleted with the schema diet).
Library edits use revision checks; applying a tool or a cutting preset copies
its settings *into* the document, so later library edits never change a saved
job.

On the command line the same copy semantics are explicit inputs to
`cam collection`:

```sh
cargo run --locked -p cam-app -- collection apply-machine fixtures/gui2/flower.job.json \
  --profile fixtures/gui2/machine.json --name Workbench --output configured.json
cargo run --locked -p cam-app -- collection apply-profile fixtures/gui4/lettering.job.json \
  --library fixtures/gui5/library.json --library-id gui5-lettering-library \
  --operation carving --role endmill --tool endmill --preset rough \
  --output detailed.json
```

`apply-machine` copies a reusable configuration into the document's one applied
machine snapshot, and `resolve-profile` turns that snapshot back into the
validated profile export needs; the profile file itself is never needed again.
`--library` takes the library object, and also the catalog file the workspace
exports (`{schema, id, library}`), so the exported file works as it is.

See the [tool library guide](../docs/flat-v-carve/tool-library.md) for the data
model, validation and snapshot behavior.

## M3 endmill planning and stock

```sh
cargo run --release --locked -p cam-app -- collection plan \
  fixtures/gui2/flower.job.json --output artifacts/m3/summary.json
cargo run --release --locked -p cam-app -- collection export \
  fixtures/gui2/flower.job.json --output artifacts/m3/export
```

Planning and export are the `cam collection` surface: `plan` writes the
per-operation summary (motions, stages, checks), and `export` plans once and
writes the checked output with its manifest and report. The
[M3 fixtures](fixtures/m3/README.md) still supply **synthetic test settings**,
including feeds and spindle speed, but they are engine-level inputs for the
planner tests rather than documents. In a document an import supplies only the
page-sized stock and the tolerances, so tools and cutting values must still be
supplied: planning requires stock thickness, depth, horizontal wall allowance,
endmill dimensions/capability/feeds/spindle/stepdown/stepover, V-bit geometry
to define the target angle, planning tolerances, and the roughing resource
limits. V-bit cutting settings and finish-quality limits are M4 fields on the
same operation.

`depth_dependent` clearing generates offset loops inside each layer's admissible center region; `deepest_region` uses the deepest region at every stepdown. Direct plunges require a plunge-capable endmill and explicit plunge feed. Ramps require `ramp_capable: true` and an explicit angle/feed. Nearby plunge-entry contours use continuously checked cutting links, with contour retracing or entry between vertices when needed. Deeper links also require swept-stock clearance above the allowed fresh stepdown; unproved and disconnected connections retract to the configured clearance Z. Stepover remains limited to half the tool diameter. The planner does not calculate machine-specific cutting parameters.

Engine **0.7.6** also collapses bounded microscopic endmill edges and reconciles
nearby V-bit endpoints before recording path identities; deeper stay-down links
additionally require swept-stock clearance above the permitted fresh stepdown.
Restart with the rebuilt application and regenerate plans from older engines.

Engine **0.7.7** extends bounded endmill simplification and groups connected stock
sweeps on sufficiently fine grids. The current real flower job completes combined
CLI planning in **2.71–2.73 seconds**, with all original job tolerances retained.
Regenerate saved plans from earlier engine versions.

Planning recomputes from the submitted document on every call and records
actual XYZ moves, identity fingerprints, and generation issues; no cached
analysis is ever trusted. Editing the machining intent changes the machining
identity and requires replanning, while the retained runtime reuses a plan
whose identity still matches (display, provenance and output-only edits do
not). A plan is not a second stored document format: `cam collection export`
is the retained path from document to checked bytes.

The preview shows layer paths, removed stock, and remaining target. Missing accessible floor is pink; possible overcut is purple. No-access stages are empty. Exact-fit contacts and insufficient numerical margin are inconclusive; unsupported entries and missed floors are reported explicitly. Partial plans remain inspectable and exit with status 1.

M3 checks whole-segment center clearance and compares actual endmill sweeps at planned depth slices. `complete` refers to this endmill-stage coverage within the declared XY tolerance. Remaining target includes slopes, wall allowance, and detail for M4. It does not establish combined finish quality, adaptive full-volume verification (M5), or readiness for machine output (M6).

## M4 combined finishing and rest machining

```sh
cargo run --release --locked -p cam-app -- collection plan fixtures/gui2/flower.job.json \
  --output artifacts/m4/summary.json
```

`collection plan` generates both stages when the Flat V-carve operation's mode
is combined, and only the M3 roughing stage when it is endmill-only.
`--through <operation-id>` plans (and exports) the enabled prefix ending at
that operation, so a job can be worked one operation at a time.

M4 combines full-depth boundary contours, variable-depth medial-axis branches, and floor cleanup contours. Finite tip geometry is used throughout. Branches split at positive-cut and depth-cap transitions; curved branches are subdivided with XYZ error and continuous clearance checks. Exact-fit lines and points remain represented, using a small guarded depth reserve where necessary. All endmill work precedes the V-bit; the complete achievable boundary/rising-detail family runs last, after bounded cleanup.

Floor cleanup uses inward contours restricted to stock left by recorded endmill sweeps. The clipping region includes the permitted-ridge cutter footprint and a numerical guard so cuts centered beside residual stock can still finish its edge. Disconnected fragments get separate clearance links; a small local raster covers any final thin core. Each depth pass also retains the conservative whole-sweep air proof, including the cutter flank. Final finishing is always retained. M4 uses direct plunge entries with explicit V-bit `plunge_capable: true` and plunge feed; it does not infer that capability from the cutter dimensions or use V-bit ramps.

Set the V-bit cutting/plunge feeds, spindle speed, stepdown, stepover,
`max_floor_ridge_mm`, `max_detail_residual_mm`, and the finish limits
explicitly on the operation. The [M4 fixtures](fixtures/m4/README.md) provide
**synthetic test settings**, not machining recommendations, and are engine
inputs for the planner tests.

Floor contour spacing is at most 90% of the cutter radius at the permitted ridge height, capped by the configured stepover. This reserves coverage at converging corners, where the parallel-lane half-spacing formula is insufficient. A pointed V-bit with zero allowed ridge is rejected when residual floor area needs clearing; finite flat tips can support zero-ridge clearing with overlapping passes. Cutter-limited detail uses independent reachability bounds and is reported separately from missed reachable material. M5 independently verifies the resulting stock.

A combined plan binds both stages, tool-transition markers, execution records, generation issues and engine/document identity. Collection export checks the retained ordered execution and emitted numeric motions; detailed stock-quality analysis is separate.

M4 `complete` means candidate-family completion, continuous segment clearance, accessible-floor slice coverage, and quality at the reported sample lattice/motion witnesses. Floor coverage is checked at `D - allowed_ridge - numerical_depth_budget`, where the explicitly reported numerical depth budget is half the verification tolerance. The report also retains XY coverage tolerance. Sampled residual maxima are **not global error bounds**; use M5 verification below for bounded continuous checks.

## M5 continuous verification and coordinate precision

Exercise the detailed verifier directly through its regression suite:

```powershell
cargo test --locked -p cam-core --test verification
```

The M5 engine verifier checks the normalized target and cutting-sweep domain, including islands and exterior material, using independent adaptive bounds. It distinguishes overcut, reachable residue, permitted floor ridges and cutter-limited detail. Current collection export runs basic plan checks and numeric readback; it does not automatically invoke detailed M5 stock-quality analysis. The old standalone `cam verify` command is removed. See the [technical design](../docs/flat-v-carve/technical-design.md#7-stock-and-verification) for the M5 contract.

Coordinate precision is checked from the profile's `decimal_places`, which is
the **minimum** output precision: export raises it up to nine decimals when
needed to preserve every motion's direction and required travel, and the report
names the precision used. M6 export below applies the document's applied
machine configuration first.

The M5 refinement limits cover cells, depth, reachability and depth bands; exhausted bounds remain inconclusive, and lowering a
limit can never produce a coarse-grid pass. The geometric model is the rebuilt
normalized polygon; source flattening/snap error is reported separately.

M5 enforces the explicit ridge and detail limits without adding M4's numerical
allowance. Consequently, the M4 zero-ridge `contact-line` and `contact-point`
examples do not pass M5: their guarded cap motions leave about 0.01 mm. The
[M5 fixtures](fixtures/m5/README.md) record these expected failures alongside
successful and resource-limited cases. Endmill-only output keeps the M3 stage
contract.

Engine **0.7.3** invalidates plans created by older engines. The ten release
cases are recorded in [the M5 fixtures](fixtures/m5/README.md); their
reproduction script still drives the removed `cam plan`/`cam verify` commands
and has not been ported to `cam collection` (see
[remaining script work](../docs/flat-v-carve/backlog.md)).

## Real artwork and scalability

Successive engine releases reduced flower planning from an incomplete run
stopped after ~15 minutes (0.7.2) through 52–54 s (0.7.3), 29 s (0.7.5), and
4.2 s (0.7.6) to **2.71–2.73 seconds** on the unchanged real job
(`../real_data/flower_box-svg.job-real.json`) in engine **0.7.7**, using
spatial indexes, checked stay-down routing, contour-following V-bit cuts,
bounded contour simplification, and grouped/parallel stock construction. These are historical measurements, not a fresh benchmark of the current code. Set
`CAM_TIMINGS=1` for stage timings on stderr when running `cam collection plan`.
`scripts/benchmark-flower.ps1` still depends on the removed CLI/artifact format;
porting it is tracked in the [backlog](../docs/flat-v-carve/backlog.md). Saved artifacts are not carried
across engines: regenerate them from the schema-5 document.

An approximately 0.1 mm wood finish is the same job with a geometry tolerance of 0.005 mm and a motion tolerance of 0.05 mm — planned in ~8.5 s on the measured machine — plus a 0.05 mm floor ridge when extra finish margin matters (~10.4 s). Raising the import geometry tolerance is the largest computation saving, but the planner requires the verification tolerance to cover at least eight geometry tolerances, so keep 0.05 mm verification. The preset *documents* were deleted with the schema diet: a schema-3 job is no longer readable, and the settings are a few fields rather than a second document to keep in step.

Engine 0.7.2 imports `../real_data/flower_box.svg` at 0.005 mm tolerance without editing the source. Spatial indexes replace repeated all-edge topology, containment, distance, and stock-query scans; selected regions use a batch union and recorded cutter sweeps use bounded batches with balanced merges. Consecutive vertices may coalesce on the precision grid only after local topology checks; erased rings, new nonlocal contacts, and changed crossings still fail. Import scales roughly linearly with input size (measured 0.063 s / 6.5 MB at 9,943 vertices through 5.994 s / 307 MB at 994,300 vertices on Windows x64); the 100× case is import-only and does not establish full-CAM scaling. Reproduce the 1×/10×/100× import cases with:

```powershell
cargo build --release --locked --workspace --examples --bins
./scripts/benchmark-import.ps1 -OutputDirectory artifacts/import-scalability-new
```

The benchmark repeats the real path at unchanged physical size and tolerance. It
records component/vertex counts, area, time, peak process working set, and
source hash. It does not establish 100× full CAM or deeply connected artwork
performance. Current bounds are 32 MB SVG, 200,000 XML nodes, two million
flattened vertices and 64 MB per job document, and 8 MB per exported program.
Dense intersection arrangements and excessive spatial candidate pairs have
separate guards. V-bit budgets remain explicit per job, up to 65,536 paths and
one million motions/curve segments/quality samples; hitting a budget never
means complete. Retained plans and prepared bundles live in memory in the
planning process and are pruned oldest-terminal-first, so an output can always
be re-read byte-identically while its bundle is retained.

## M6 LinuxCNC export

```powershell
cargo run --release --locked -p cam-app -- collection apply-machine fixtures/v5/full-job.json --profile fixtures/gui2/machine.json --name Workbench --output artifacts/m6-example/configured.json
cargo run --release --locked -p cam-app -- collection export artifacts/m6-example/configured.json --output artifacts/m6-example/sequential
cargo run --release --locked -p cam-app -- collection export artifacts/m6-example/configured.json --layout one --output artifacts/m6-example/one
```

Export requires a new output directory and publishes the ordered files with
`manifest.json` and `report.json`. The default `--layout sequential` writes one
numbered file per contiguous tool stage (`01-…ngc`), and `--layout one` writes
the single `sequence.ngc`. Every written file is the bundle's exact checked
byte content: a save that fails can be retried and reproduces the same bytes.
V-bit rest machining requires the matching endmill program to have run on the
same stock, even though each file establishes its own modal state.

The supplied [macro profile](fixtures/m6/macro-stock-bottom.json) follows the user-described Z-only M6 TLO with stock-bottom/worktable zero, T1/T2, and `G0 Z150` then X0 Y0 after M6. G54, six decimals, clockwise spindle, coolant off, and zero added spin-up dwell are editable initial choices. With 8 mm stock, the 2 mm depth cap outputs Z6 and a 5 mm planning clearance outputs Z13. Z150 is in the selected work frame. See the [profile contract](fixtures/m6/README.md) for setup and clearance assumptions.

Macro-managed output preserves TLO; tool-table output applies the configured
G43 H mapping. The current collection path checks original plan structure and decoded numeric motions; detailed M5 quality analysis is separate. Every
modal/tool/feed/coordinate block is checked by a strict numeric subset reader,
and a failed or inconclusive check publishes nothing. The applied machine
configuration's profile supplies `decimal_places` as the **minimum** output
precision; export raises it up to nine decimals when needed to preserve every
motion's direction and required travel and reports the precision used.

The eight release expectations and saved-byte readbacks in
[fixtures/m6/cases.json](fixtures/m6/cases.json) are reproduced by the core
post-processor tests; `scripts/check-m6.ps1` still drives the removed
`cam plan`/`cam export`/`cam verify-gcode` CLI and has not been ported. The
[LinuxCNC guide](../docs/flat-v-carve/linuxcnc.md) records
contracts and limits. LinuxCNC preview/simulation with the actual
macro/configuration remains pending; the bundled cutting settings are synthetic
fixtures.

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

The strict M1 input format is defined by [`ModelInput`](crates/cam-core/src/preview.rs) and illustrated by the fixtures. Set `ticks_per_mm` to `null` for automatic precision selection; `geometry_tolerance_mm` and `preview_depth_tolerance_mm` control different errors. Exact-fit lines/points have zero clearance margin and do not establish a usable entry. This remains a separate geometry experiment format; portable jobs use [`CamJobV5`](crates/cam-core/src/project/v5/mod.rs).

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
.\target\release\cam.exe inspect artifacts/windows/job.json --output artifacts/windows/inspection.json
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

### Static browser build

`cam-gui` also runs without `cam.exe`: the engine is compiled to WebAssembly
and loaded by the browser application. Build and preview it with:

```powershell
./scripts/build-gui.ps1 -Target web
node crates/cam-gui/web/serve.mjs
```

The artifact is `artifacts/gui/browser/`. Serve that directory over localhost or
HTTPS and open `/web/index.html`, or open
`http://127.0.0.1:5182/web/index.html` from the development server. Building it
requires wasm-pack and the wasm32-unknown-unknown Rust target. See the
[cam-gui README](crates/cam-gui/README.md) for the browser workflow and limits.

`cam-core` contains in-memory geometry contracts, narrow dependency adapters, SVG normalization, portable jobs, cutter/target models, independent distance queries, both planners, linear motions, stock analysis, and preview calculations. It has no filesystem or process access. `cam-app` handles command arguments, fixtures, file output and build metadata. Collection export produces checked G-code; the separate geometry experiments produce diagnostic JSON/SVG.

The capability sections above explain behavior and limits. The [documentation index](../docs/flat-v-carve/README.md) links current architecture and contracts; the [backlog](../docs/flat-v-carve/backlog.md) tracks unresolved implementation, controller validation and release work. Completed milestone evidence remains in Git history.
