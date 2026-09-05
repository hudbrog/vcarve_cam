# M6 LinuxCNC postprocessor

Date: 2026-09-05\
Engine: 0.7.1\
Target: native Windows x64, Rust 1.95.0 MSVC\
Status: postprocessor and numeric readback implemented; actual machine macro/configuration and LinuxCNC preview/simulation validation remain pending.

## Implemented behavior

`cam export` consumes an authenticated combined plan and an explicit, separately versioned `LinuxCncProfile`. It verifies original motions, generates linear G0/G1 output in G61 exact-path mode, reads the actual output bytes, compares every decoded motion with its source, and runs M5 stock verification on the decoded coordinates. It returns no executable program when a required check fails or remains inconclusive. `cam verify-gcode` repeats the saved-byte checks without trusting the export report.

The exporter preserves motion count, ordering, feeds, and cutter roles. Endmill work precedes V-bit work. Empty stages generate neither a program nor a tool change. Combined output is `combined.ngc`; per-tool output is `endmill.ngc` and/or `vbit.ngc`. Each file establishes its own modes and finishes with spindle/coolant off and M2. V-bit rest machining still requires the corresponding endmill stock history, recorded in both the report and file comment.

## Machine contract supplied for this implementation

The user described M6 as introducing TLO so work Z0 is the stock bottom/worktable, preserving XY offsets, and leaving the tool raised at the sensor. They selected provisional T1/T2 and specified a safe Z150 move followed by X0 Y0, without relying on the sensor return coordinates. The [editable profile](../../flat-v-carve/fixtures/m6/macro-stock-bottom.json) records that contract. G54, six decimal places, clockwise spindle, coolant off, and zero added dwell are initial profile choices, not measured machine settings. RPM and cutting/plunge/ramp feeds remain the saved job's settings.

The internal stock-top convention is unchanged. Output uses

```text
Z_work = Z_plan + stock_thickness_mm    (stock-bottom datum)
Z_work = Z_plan                         (stock-top datum)
```

For 8 mm stock, the top is work Z8 and a 2 mm cut is work Z6. Five millimeters above stock is work Z13. Output formatting happens **after** this translation; the reader subtracts the datum offset before stock analysis. It does not substitute ordinary rounding in the original coordinate frame. Stock thickness and declared safety coordinates must fit the selected output precision. Decimal addition is normalized to that precision so, for example, 8.2 + 1.1 does not fail because of binary floating-point representation.

`safe_retract` treats the first post-M6 Z move from an unknown sensor position as a machine-contract action. The contract establishes an unobstructed upward corridor to work Z150 and clear XY travel there. The next known position is X0 Y0 Z150. The exporter then lowers vertically to planning clearance and links to the stage start. The report counts those first two blocks separately from modeled motion and known clearance links. It does not claim M5 proved an unknown sensor corridor or fixture clearance.

Startup with unknown XYZ is supported only under this contract: the machine must already be ready for M6; the post emits no axis motion before the first tool change. Other supported contracts require a known caller or fixed return position, expressed for the **new tool tip after compensation**. G43 itself does not physically move the axes, but changes the coordinates applicable to subsequent movement; a physical no-motion tool change is not evidence that the old displayed tool-tip position remains valid. [LinuxCNC G43](https://linuxcnc.org/docs/stable/html/gcode/g-code.html#gcode:g43)

## Modal and compensation sequence

Every file stops the spindle and coolant, then establishes millimeters, XY plane, absolute XYZ, units-per-minute feed, cutter compensation off, canned cycles off, G61, the selected work offset, and G92.1. Each nonempty stage stops spindle/coolant, emits mapped Tn M6, stops spindle/coolant again, and restores the full setup. The required contract excludes work-frame rotation, XY tool offsets, and G52/G92 use for compensation.

For `macro_managed`, no G43, G43.1, or G49 is emitted. For `tool_table`, G43 Hn is applied using the explicit H mapping. No tool length is invented. Standard LinuxCNC M6 does not change TLO itself; the actual custom macro remains machine-owned. [LinuxCNC M6](https://linuxcnc.org/docs/stable/html/gcode/m-code.html#mcode:m6)

After safe positioning, the post emits G97 Sn, M3 or M4, G4 with the configured spin-up dwell, and the chosen coolant state. Each feed move has an explicit F word; feeds are represented independently of XYZ decimal precision. A zero dwell adds no timed wait. The machine's spindle-at-speed interlock or a configured delay must establish readiness. G61 is exact-path mode, distinct from G61.1 exact stop and G64 blending. [LinuxCNC path modes](https://linuxcnc.org/docs/stable/html/gcode/g-code.html#gcode:g61)

There are no arbitrary header/footer/macro templates, G28/G30/G53 moves, probing commands, controller-side XY cutter compensation, arcs, or blending. These would require additional interpretation and verification contracts.

## Readback and artifact identity

The reader accepts a deliberately small ASCII numeric subset, not arbitrary LinuxCNC programs. It independently requires the emitted block grammar and modal/tool/spindle sequence, parses XYZ/F/S/T/H/P words, tracks known positions and clearance links, and enforces source motion ordering. Comments can be removed without affecting validation; only inert full-line CAM comments are accepted. Expressions, active comments, unrecognized blocks, missing modes, wrong tool/offset/feed words, changed coordinates, extra motion, and trailing blocks after M2 fail. Output has no lines longer than 240 characters.

Formatting-induced collapse, reversal, and loss of required Z displacement fail rather than silently dropping moves. The independently reconstructed list must match the translated/formatted plan and pass M5 overcut, residual, ridge, detail, and entry/clearance checks. Reconstruction uses stock-top coordinates and the decoded clearance/start position, retaining the original target model.

Reports bind the authenticated plan verification, full profile snapshot and fingerprint, layout, datum translation, decoded motion fingerprint, and SHA-256 of each exact program byte stream. They record motion, known-link, machine-contract positioning, and tool-change counts, stock prerequisites, diagnostics, and bounds. The result is deterministic for the same engine, inputs, options, and output bytes. A profile edit changes export identity even when the geometric path is unchanged.

Exports publish a new directory by writing a temporary sibling and renaming it after all files are ready. Existing bundles are not overwritten. Failed/inconclusive geometry or readback publishes a report-only bundle and exits 1; invalid profiles, stale plans, arguments, and I/O errors exit 2. This prevents old successful G-code being mistaken for a new failed run. The core itself does no filesystem or machine I/O.

Job schema 3 and plan schema 1 are unchanged. The new profile has schema 1 and is supplied with `--profile`; existing editable job machine settings must agree with it when set. Legacy free-text M6 descriptions are not export authority. Engine 0.7.1 invalidates older saved plans; regenerate from their portable jobs.

## Validation

### Rest-planning correction found during export review

The previous floor planner used remaining stock only as a Boolean gate and then generated parallel lanes across the entire floor. Its subsequent air proof required the complete V-bit sweep to fit in a single recorded endmill sweep. In the island fixture, equal 2 mm cutter radii plus the numerical guard prevented that proof, leaving almost all of the broad raster in the output.

Engine 0.7.1 generates inward floor contours and clips them to the region needed to reach actual residual endmill stock. The support region includes the cutter footprint at the permitted ridge height and a guard; it does not merely clip tip centers to residual material. Narrow leftover cores retain local raster coverage. Final boundary and rising medial paths remain mandatory, and the independent continuous stock verifier still decides acceptance. The endmill's 0.5 mm wall allowance requires several nearby floor passes in addition to the wall contour.

For the same 40 × 30 mm island fixture, 2 mm depth, tools, and cutting settings:

| Measurement | Previous 0.7.0 | Corrected 0.7.1 |
|---|---:|---:|
| Endmill motions | 866 | 866, identical coordinates and feeds |
| V-bit motions | 5,189 | 1,994 |
| V-bit entries | 1,000 | 58 |
| V-bit XY cutting distance | 6,471.00 mm | 1,955.19 mm |
| Total recorded motions | 6,055 | 2,860 |
| Combined G-code bytes | 231,049 | 112,589 |

Motion count includes links and depth entries. Curved contour approximations still use individual straight G1 segments, so cutting-segment count alone is not a measure of cutting distance. The corrected island passes original and six-decimal M5 checks, translated output verification, and saved-byte readback. Regression tests also cover clipping across holes without joining removed intervals and an acute triangular floor with no endmill access.

### Software and fixture checks

The regression suite adds numeric-path tampering, missing post-M6 modal setup, wrong tool and H mappings, spindle/feed changes, unsafe positioning edits, rounded motion collapse, resource exhaustion, fractional stock datum/clearance, deterministic identity, immutable output bundles, stale plans, and standalone per-tool setup. Both compensation policies and an empty endmill stage are covered.

Release fixture expectations and saved-byte readbacks are defined in [M6 cases](../../flat-v-carve/fixtures/m6/cases.json) and reproduced by [check-m6.ps1](../../flat-v-carve/scripts/check-m6.ps1). The synthetic table profile exercises a known post-compensation return point; the macro profile exercises the user-described stock-bottom convention and unknown sensor coordinates.

Native Windows debug and release builds pass all 146 tests in each profile; Clippy with warnings denied and formatting checks also pass. Regenerated engine 0.7.1 exports in `artifacts/m6-rest` match all eight fixture expectations; each successful export also passes readback of its saved program bytes:

| Fixture/profile/layout | Result | Recorded motions | G-code bytes |
|---|---|---:|---:|
| Narrow channel / macro / combined | Passed | 84 | 3,617 |
| Wide floor / macro / combined | Passed | 392 | 15,607 |
| Wide floor / macro / per-tool | Passed | 392 | 16,017 |
| Island / macro / combined | Passed | 2,860 | 112,589 |
| Finite tip / tool table / combined | Passed | 260 | 10,598 |
| Strict zero-ridge contact | Inconclusive, no G-code | — | 0 |
| Zero-decimal formatting | Failed, no G-code | — | 0 |
| One-cell verification limit | Inconclusive, no G-code | — | 0 |

Strict-contact export stops at the original-coordinate M5 gate, which is inconclusive; it does not reach the separate rounded-coordinate failure previously recorded by the M5 fixture runner. These are expected rejection cases, not successful machining programs.

### Configurable zero allowance by default

Following review of the remaining perimeters, new SVG imports and new UI example jobs default `operation.wall_allowance_mm` to 0 mm. The field remains editable, and loading a saved job preserves its explicit value or unset state. Existing regression fixtures retain their deliberate allowances; the table above uses the island's original 0.5 mm setting.

A copy of that island with only the allowance changed to zero is saved in `artifacts/m6-zero-allowance/island.job.json`. It generates 1,016 endmill motions and 611 V-bit motions (1,627 total), 53 V-bit entries, and 615.45 mm of V-bit XY cutting travel. Its 63,932-byte combined program passes original stock verification, translated numeric output verification, and saved-byte readback. Localized corner cleanup, configured depth passes, and final finishing remain. The verified floor-ridge upper bound is 0.1131251 mm, below the 0.15 mm fixture limit.

This default change was checked with native Windows debug/release builds, 28 import/job tests, 37 frontend tests, frontend contract/CLI round-trip checks, the frontend production build, and Rust formatting. An actual CLI import confirms a saved zero allowance. No schema migration or reinterpretation of existing plans is involved.

No LinuxCNC preview, rs274 interpreter execution, or physical machine validation is claimed. Both installed WSL Ubuntu environments were checked; neither supplied a LinuxCNC executable/interpreter. Obtaining the actual macro, tool table, INI/HAL configuration, checking work Z150 and travel limits, and running a matching LinuxCNC preview remain M6 integration acceptance work. The geometry/report checks do not establish fixture/holder clearance, hidden probing paths, spindle interlocks, cutting loads, controller dynamics, or actual table/stock measurements.
