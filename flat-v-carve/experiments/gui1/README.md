# GUI1 candidate experiment

**Current direction: continue egui/eframe evaluation.** The user removed browser
screen-reader access as a requirement on 2026-09-11. The observed browser
accessibility-tree limitation is accepted; keyboard/focus/input behavior and
native accessibility remain required. Final GUI1 qualification is still pending.
See the [framework decision](../../../docs/flat-v-carve/ui-framework-decision.md).
This is a runnable, deliberately incomplete risk experiment, not GUI2 or a new preferred application.

From PowerShell in this directory:

```powershell
./launch.ps1 -Target native
./launch.ps1 -Target web
```

The web command serves [egui](http://127.0.0.1:5181/web/index.html) and the
[two-field DOM comparison](http://127.0.0.1:5181/web/dom-probe.html) on localhost.
Run one server. Close the native test window before rebuilding its executable.
Prerequisites: Rust 1.95/MSVC, wasm32-unknown-unknown target, wasm-pack 0.15.0
(verify your local version), Node 24 or later. Dependency versions/features are
explicit in Cargo.toml and the independent Cargo.lock. Browser build inherits
the existing `flat-v-carve/.cargo/config.toml` getrandom WASM flag.

Already built artifacts live in `target/release/cam-gui1-desktop.exe` and `pkg/`.
To serve a previously built browser package without rebuilding: `node web/serve.mjs`.
WebGPU is required; WebGL2, Firefox, Linux, and Safari are not qualified.

## Five-minute review

1. Choose **Flower reference**. The exact embedded job is planned by Rust in a
   disposable process/Worker. Expect 7,048 roughing and 15,835 finishing motions.
   Cyan is endmill, amber is V-bit; both use actual recorded motion endpoints.
2. Switch Top/Isometric, drag to rotate, zoom, choose Endmill/V-bit paths, and
   move **Stock checkpoint** backward. Stock removal and the physical stock box
   are displayed at bounded, exact motion prefixes. The UI labels the preview
   cell size and finer reference cell size. Uncheck **Stock preview** for the
   original motion-line timeline. Path filtering preserves cumulative stock.
3. Type `-`, `1.`, or invalid text into Maximum depth. Reverse artwork order;
   select operation 2 and return to 1. Text belongs to stable source/operation/field
   IDs. Inspector fields are explicitly input probes, not machining edits.
4. Start **Busy worker**, continue typing, then **Cancel**. Prior display remains.
   Native reports kill/reap time after supervisor observation. Browser reports
   Worker.terminate call time only; actual stopped CPU latency is unknown.
5. **Risk probes** offers S/M/L line loads, a compute-child crash, a renderer
   visibility failure/resume, and denied-write injection. Renderer injection is
   not actual device-loss recovery. It retains the document and reports failure.
6. Save/recover raw draft JSON. Enable denied-write injection to see an explicit
   error and keep editing; disable it before retry. These are manual portable
   draft files, not automatic crash recovery or real quota exhaustion.
7. **Prepare checked small reference** exercises retained core plan/output checks.
   The unchanged contact-line fixture fails `M5_FLOOR_RIDGE`; expect **failed**,
   zero checked programs, and disabled Save checked bytes. No settings are relaxed
   to produce G-code. Save reference job preserves exact input bytes independently
   of the experimental raw fields. Native save success reports its byte hash;
   browser success says only Download requested. **Prepare checked flower** uses
   the unchanged real job and `real_data/machine-profile.json`; native M6 checks
   passed and retained `combined.ngc` with 22,883 motions and two tool changes.
8. Compare the DOM page: two labelled controls, reorder, type partial text while
   the same Rust worker runs, and cancel. Browser accessibility exposes these
   controls; the egui canvas does not. This earlier comparison is optional and no
   longer a framework gate. Native screen-reader and both-target IME tours remain pending.

## Checks and evidence capture

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo test --locked --test interaction dense_layout -- --ignored
node web/capture.mjs
node web/compare-simulation.mjs
node web/compare-wasm-simulation.mjs
```

The ignored image tests are opt-in GPU tests, separately executed locally at
1280×800 and 1440×900. Goldens are checked in. Regenerate intentionally with
`$env:UPDATE_SNAPSHOTS='true'`, then remove that variable and rerun for a real
comparison. The behavior harness includes a negative assertion control. The
goldens cover dense shell layout, not custom viewport geometry correctness.
`capture.mjs` records five small planning runs, a separate checked-output run,
one flower run, input hashes and five native process cancellation samples.
Use `--capture <directory> flower export` on the executable to investigate the
flower output contract separately; its native report is captured in the evidence.
This establishes software checks, not a new physical machining trial.

## Boundaries and remaining risks

- No domain crate or incumbent UI was changed. Schema-4 migration is captured
  through the existing service; the spike's open adapter supports legacy combined
  jobs only and rejects unsupported input without replacing its prior result.
- The narrow Rust heightfield port matches the original TS on full-cell native
  comparisons; the actual WASM module matches every preview checkpoint. See
  [simulation evidence](../../../docs/flat-v-carve/gui1-simulation-evidence.md).
  The 512-cell preview preset is explicitly coarser (flower 0.3609375 mm versus
  0.025 mm reference), has at most 18 checkpoints and a 20 MiB retained-cell cap.
  Arbitrary stock seeks, picking, blade glyph, checkpoint paging, incremental
  tile transport and complete S/M/L memory qualification remain unfinished.
- JSON transport and a whole retained GPU batch are intentionally unoptimized.
  No O(N) mesh construction runs in layout, but draft serialization on save and
  browser JSON decode/transfer still require replacement/budget measurements.
- Source/operation lists are synthetic identity probes, not an artwork collection.
  No machine settings, schema fields, or output eligibility are invented by them.
- Real file-drop import, automatic recovery, actual quota/write failures, library
  conflicts, keyboard-only dialogs, IME and native screen-reader behavior are unverified.
- Browser screen-reader access is outside the required scope. Other unfinished
  probes still gate qualification. Next work continues the egui prototype and
  simulator comparison described in the decision.

Fonts: epaint's bundled Hack, Ubuntu Light, Noto Emoji, and emoji-icon font.
Their license texts accompany the crate; packaged redistribution must carry the
applicable font notices as well as Rust dependency notices. No external icon pack
or generated visual asset was added. See `THIRD-PARTY.md`.
