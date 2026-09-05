# M6 LinuxCNC export fixtures

`macro-stock-bottom.json` records the machine behavior described by the user:

- M6 establishes Z-only tool-length compensation with work Z0 at the stock bottom/worktable. XY offsets are unchanged.
- T1 is the endmill and T2 the V-bit.
- After M6, `G0 Z150` followed by XY transit to X0 Y0 is the user-specified safe sequence; the macro's exact sensor return coordinates are not required.

G54, six decimal places, clockwise spindle, coolant off, and zero programmed spin-up dwell are editable initial choices. Zero dwell adds no timed wait; spindle-at-speed interlocking or a suitable configured delay is a machine responsibility. Feeds/RPM come from the authenticated job, not the profile. The fixture jobs contain synthetic cutting settings.

`clearance_z_mm` is above stock **top** and must match the plan. The safe-retract Z and optional startup/fixed return positions use the selected **machine work frame**. With 8 mm stock, a planned depth of 2 mm outputs Z6; a 5 mm planning clearance outputs Z13. Z150 is a G54 work coordinate, not G53 machine Z.

The safe-retract contract means the new compensated tool tip is at or below the retract plane and its upward corridor is unobstructed; the XY transit at that plane must also be clear. Initial startup must already be suitable for calling the machine's M6 macro. No motion is emitted with an unknown initial tool before the first M6. Subsequent lowering/transit uses the configured planning clearance.

The profile requires no G52/G92 compensation, no work-frame rotation, and no XY tool offsets. Modal setup clears local offsets using G92.1 but never cancels or overwrites macro-managed G43/G43.1 TLO. `reviewed` records the declared contract; it is not proof of machine testing. The actual macro/configuration and LinuxCNC simulation remain to be checked.

`tool-table-synthetic.json` exercises a different, entirely synthetic contract: stock-top G55, post-managed G43 H11/H12, a known compensated return position, spindle directions, timed spin-up, and coolant. Its fixed XYZ and tool-table contents are not machine recommendations.

`cases.json` defines eight release expectations. Build the release CLI, then run from PowerShell 7:

```powershell
./scripts/check-m6.ps1
```

The script regenerates plans with the current engine, exports new bundles, rereads every successful saved program, compares byte hashes, and checks that rejected cases publish no `.ngc`. Use `-OutputDirectory artifacts/m6-another-run` to preserve earlier output. The report and G-code are generated artifacts and stay ignored by Git.

The strict zero-ridge contact case is inconclusive at the original-plan gate (its floor bound cannot be resolved within the resource/arithmetic limits). M5's separate rounded-coordinate check can prove a failure for that fixture, but M6 already withholds output at the original gate. Both outcomes prevent export. Use `-CaseId strict-contact,coarse-rounding,cell-limit` to rerun only those expectations in a new output directory.
