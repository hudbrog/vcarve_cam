# GUI3 complete vertical slice — technical completion

GUI3a, GUI3b and GUI3c are implemented in the production `cam-gui` crate.
Starting commit: `5938e725043a8b250c4d51efa2ee81bcd103b8fd` (GUI2b).
Delivery commit: the GUI3 implementation commit containing this report,
verified on 2026-09-12 (Europe/Moscow).
Runnable review builds and the [manual recipe](gui3-review.md) are available.
User review remains **pending**. Software verification does not establish user
acceptance or a physical machining trial.

## Requirement audit

| Requirement | Implementation and verified evidence |
| --- | --- |
| GUI3a: filled-region picking, holes, stable qualified IDs | `artwork_view.rs` uses catalogue rings and full geometry references. Geometry tests and the browser lettering/overlap tour verify holes, distinct coincident owners and candidate cycling. |
| Temporary multi-selection and explicit Use/Add/Remove | The browser verifies Shift multi-selection and all three assignment actions. Ordinary picks and overlap cycling leave document revision and machining assignment unchanged. |
| Move/rotate/uniform scale and numeric equivalents | Gestures use the shared core placement conversion. Camera tests cover top/isometric/yaw; the browser verifies move, rotate, scale, numeric origin, one-gesture Undo, Escape and rejection of a drag interrupted by Undo. |
| Move lettering, select a subset, Generate/Simulate/Export | The browser imports the L/O fixture, moves it, assigns only L, generates both stages, samples actual stock and downloads output whose SHA-256 equals the retained checked bytes. |
| GUI3b: all supported local cutting controls | Shared binders expose top reference/offset, roughing strategy/entry, optional assignment values/capabilities and all rough/finish policy limits. Typed setters reject invalid, fractional-count and nonfinite input without changing the job. |
| Independent assignments, unset values, mode switching and reopen | Tests verify sibling assignments/tool geometry are unchanged, wrong tool kinds are rejected and explicit tool changes clear only their assignment. Inactive finishing values/policy survive portable save/reopen and are excluded from Endmill-only execution/identity. Browser reopen switches back to Combined and generates successfully. |
| GUI3c: depth/detail, stage views and corresponding comparison | Actual packed heightfield cells provide depth and precision readouts; one frame can be pinned. Tests cover tile boundaries and changed stage motion counts. Rendering/picking clip to the same stage/playhead span. Browser detail 0.1 → 0.05 marks the result stale, regenerates and compares the same XY at corresponding final-stage progress. |
| Currentness and issue navigation | Core structured missing-field diagnostics route to exact controls; the browser verifies reveal, scroll and keyboard focus. Tests verify routing independently of message text. Partial results show issues and cannot enable export when core checks fail. Output-only reference repairs do not incorrectly block generation. |
| Recovery and previous workflow | Recovery preserves raw text, qualified temporary selection, gesture mode and inspection XY without artifact trust. Native workflow tests and GUI2 flower/new-SVG browser tours cover recovery, saves, stage playback, backward scrubbing, cancellation and fresh-worker generation. |
| Review delivery | Native/browser builds succeed; GUI2b native/browser builds are preserved. Placement, stale-state and comparison screenshots were visually reviewed, along with flower and new-SVG regression screenshots. |

## Executed verification

From `flat-v-carve`:

| Command | Result |
| --- | --- |
| `cargo test -p cam-gui --release --locked` | 46 passed: 34 unit, 7 authoring, 1 native worker, 4 workflow. |
| `cargo test -p cam-core --release --locked --test project_v5 --test collection_machining --test collection_resources --test sequence_plan` | 35 passed, including migrated/legacy plan equivalence and scope/resource behavior. |
| `cargo clippy -p cam-gui --all-targets --locked -- -D warnings` | Passed. |
| `./scripts/build-gui.ps1` | Windows release build passed. |
| `./scripts/build-gui.ps1 -Target web` | WASM release and offline bundle passed. |
| `node crates/cam-gui/web/smoke.mjs --gui3 --port=9336` | Final-build lettering/placement/settings/inspection/export/reopen tour passed; no console errors. |
| `node crates/cam-gui/web/smoke.mjs --authoring --port=9337` | New-SVG GUI2 regression passed; no console errors. |
| `node crates/cam-gui/web/smoke.mjs --port=9338` | Flower GUI2 regression passed; no console errors. |
| `git diff --check` | Passed. |

Logs: `artifacts/gui/gui3-tests.txt` and `gui3-core-tests.txt`.
Evidence directories relative to `flat-v-carve/artifacts/gui/browser-smoke`:

- `2026-09-11T22-06-37.990Z`: final GUI3 tour, `evidence.json`, four lettering screenshots, portable job and exact checked `sequence.ngc`.
- `2026-09-11T21-57-46.576Z`: flower regression, `evidence.json`, `workspace.png`, job and checked output.
- `2026-09-11T21-52-04.049Z`: new-SVG regression, `evidence.json` and `new-artwork.png`.

GUI2 regression tours ran before the final readiness correction. Final core/GUI
tests and the GUI3 browser tour include that correction: an unused dangling
output mapping does not block generation, and spindle direction stays optional
at generation because the machine configuration may resolve it in preparation.

## Builds and exercised targets

- Windows native: `flat-v-carve/artifacts/gui/native/cam-gui.exe`.
  SHA-256: `36db158c8bbc94a21b40d17b85de73c0cdf05d80ba78a9ff70ef7bbf39f3fe95`.
  Release build and actual child-process Open/Edit/Generate/Seek/Prepare/Save/Reopen/Stop test passed. Native window interaction was not re-run in this review.
- Browser: `flat-v-carve/artifacts/gui/browser`; offline build
  `bba188968c7cd8850a140caf2b3ff5dd975bde79dad8e02d8dec5c4b0b19d095`.
  Chrome `152.0.7977.83`, Windows, headless WebGPU with
  `--enable-unsafe-webgpu`, actual WASM worker and pointer/keyboard/file controls.
  Physical GPU identity was not recorded; this is not platform/performance qualification.
- Previous GUI2b: `flat-v-carve/artifacts/gui2b/native` and `browser`, rebuilt
  from the recorded starting commit. Native SHA-256:
  `9769679d60cc24bc76c09e596c0d97dbb59513b470dc10cc1161d08bf4f4e038`;
  browser build `2a7c5714a0af`. `SOURCE-COMMIT.txt` records provenance.

## Contract and scope notes

New modules are `artwork_view`, `viewport_artwork`, `viewport_inspection` and
`issues`. Existing app/authoring/inspector/recovery/render/picking/session modules
connect them to the retained workflow. Core additions expose canonical placement
conversion and located readiness inspection. Schema-5 inactive finishing policy
is preserved/validated and omitted from Endmill-only execution and machining
identity. Retained worker protocols advance to revision 3 so older workers cannot
silently omit component rings. Recovery gains an inspection pane and view state;
old recovery compatibility was not required.

`fixtures/gui3/lettering.svg` has separate L/O components and a hole;
`overlap.svg` exercises coincident picking. Review uses an explicit 2.5 mm
endmill. A 3 mm exact-fit cutter correctly produces `ZERO_MARGIN_ACCESS`; core
checks were not relaxed. Both 0.1 and 0.05 mm detail fixtures generate complete
plans. The sampled interior depth is 0.9999 mm in both results, so its zero delta
is a valid unchanged sample.

Inspection reports display-grid samples, not CAD measurements or machining
tolerance certification. One pinned frame is retained, is not serialized and
confers no export authority. Inactive ramp values are recovery-only editor text
because the canonical entry enum stores one active strategy. The existing
100,000-motion GUI admission limit and conservative revision-based invalidation
remain. Large-job, new-platform and physical machining qualification were not
performed.

GUI4 collection editing, GUI5 global resource management, GUI6 Face operations
and GUI11 cutover remain outside GUI3. The next dependent slice is GUI4; it needs
its own collection/replacement/reference-repair evidence. No user feedback or
acceptance has yet been recorded for this GUI3 build.
