# GUI3 lettering review

Implementation and software verification are complete. User review is pending.
See `gui3-progress.md` for the requirement audit, build identities and evidence.

GUI3 is part of the production `cam-gui` crate. The previous GUI2 review build is
retained in `flat-v-carve/artifacts/gui2b`; new Windows and browser artifacts use
`flat-v-carve/artifacts/gui`. No legacy data migration is required for this UI.

Launch `flat-v-carve/artifacts/gui/native/cam-gui.exe` for the Windows review.
For the browser review, run `node crates/cam-gui/web/serve.mjs` from
`flat-v-carve` and open `http://127.0.0.1:5182/web/index.html` in Chrome.

## Source placement and assignment

Import `flat-v-carve/fixtures/gui3/lettering.svg`. The fixture contains a filled L
and a separate O with a hole. Click each letter; click inside the O's hole. An
ordinary click assigns that filled region to the operation's own geometry
selection; Shift-click adds or removes one member. Clicking a hole or a hidden or
locked source changes nothing. For overlapping sources, **Next overlap** cycles
the qualified filled component candidates and assigns the chosen owner.
**Cutting → Geometry to carve** lists the same selection for checkboxes, Select
all, Clear and explicit repair, and is the only place geometry is assigned: the
Artwork panel manages placement and sources only.

Choose **Move artwork**, drag the L, and Undo. Rotate and Scale use setup (0,0),
the image of the numeric page origin, as their pivot. One released gesture is one
Undo transaction; Escape cancels it. Numeric placement uses the same core
`scale × rotate(page − origin)` conversion. Use origin X = -5, Y = -3, rotation =
0, scale = 1 for the following tour. Click only the L, or check only it in
**Geometry to carve**.

## Configure the lettering job

These are explicit review-fixture values, not import defaults:

| Group | Values |
| --- | --- |
| Stock | Thickness 10; XY minimum (-10,-10), width 65, length 50 mm |
| Setup | Clearance 5 mm; start XY (0,0); motion tolerance 0.01, verification tolerance 0.05 mm |
| Endmill | Diameter 2.5 mm, cutting length 15 mm; plunge capable = yes |
| V-bit target | Included angle 90°; tip 0.1, cutting diameter 12, cutting height 5 mm |
| Operation | Stock-top reference, offset 0; maximum depth 1; wall allowance 0; floor ridge 0.1 mm |
| Roughing | Depth-dependent clearing, plunge entry; feed 1200, plunge 400 mm/min; stepdown 0.5, stepover 1 mm; spindle 12000 RPM, CW |
| Finishing | Combined mode; feed 1000, plunge 300 mm/min; stepdown 1, stepover 1 mm; spindle 12000 RPM, CW; plunge capable = yes; detail residual 0.1 mm |
| Review planner limits | Rough layers 30; finish paths 60000; quality sample spacing 0.5 mm; retain the explicitly selected policy's other displayed limits |
| Machine | Apply the example flower machine profile; explicitly map endmill T3 and V-bit T8 |

The 2.5 mm endmill avoids an exact-fit contact in this fixture. A 3 mm endmill
produces the core's `ZERO_MARGIN_ACCESS` issue. That partial result is inspectable
and cannot be exported; the UI must not call it ready.

Before entering all values, press Generate and select a missing-setting issue.
The owning pane opens, scrolls to the field and focuses numeric input. Locations
come from the core's structured diagnostics, not from parsing their messages.
Invalid numeric edits remain raw drafts and have an editor-owned field location.

All rough and finishing policy fields are exposed. To try ramp entry, enter both
angle (5°) and feed (250 mm/min), confirm ramp capability and choose **Ramp entry**.
Changing clearing strategy must preserve these values and the resource limits.
Switch back to Plunge for the review fixture. Inactive ramp text is an editor
draft retained by recovery; it is not a second active entry in the portable job.

## Inspect, compare, export and reopen

Generate, choose Simulate, and inspect the stock after the endmill and after the
V-bit. Stage path filters must clip exactly at stage boundaries, even when both
stages share one resident GPU page. Picking must use that same visible range.

Open **Inspect result**. Click near setup (9,23), inside the L, or enter Inspect
XY numerically. Readouts report the actual displayed cell centre, depth below
stock top, surface Z and sampling precision. They are display-grid measurements,
not machining tolerance certification. A picked motion also reports its stage,
tool and setup-coordinate endpoint.

At the final V-bit position, choose **Pin comparison**. Change Detail residual to
0.05 mm. The displayed result becomes stale. Generate again and return to Inspect
result. The pinned stage identity and fraction map to the new motion span; the
comparison reads both stock frames at the same setup XY. **Match pinned position**
restores that corresponding position after scrubbing. A missing stage is shown
as unavailable. Only one pinned stock frame is retained; it confers no export
authority. Inspection XY is recovered, while pinned derived data is not.

Prepare checked output and save the exact bytes. Switch to Endmill only, save
the job, reopen it, switch back to Combined and Generate. Finishing cutting values
and policy must remain unchanged. Inactive finishing policy is retained in the
schema-5 job and omitted from Endmill-only execution and machining identity.

## Verification commands

Run from `flat-v-carve`:

```
cargo test -p cam-gui --release --locked
cargo test -p cam-core --release --locked --test project_v5 --test collection_machining --test collection_resources --test sequence_plan
cargo clippy -p cam-gui --all-targets --locked -- -D warnings
./scripts/build-gui.ps1
./scripts/build-gui.ps1 -Target web
node crates/cam-gui/web/serve.mjs
node crates/cam-gui/web/smoke.mjs --gui3 --port=9336
node crates/cam-gui/web/smoke.mjs --authoring --port=9337
node crates/cam-gui/web/smoke.mjs --port=9338
```

The smoke tours use real file-drop, pointer, keyboard and download controls. The
application's browser probe is read-only. Screenshots and reports are written to
timestamped directories under `artifacts/gui/browser-smoke` and must be visually
reviewed in addition to checking the test result. Physical machining acceptance
remains separate from this software-fixture review.
