# LinuxCNC output and machine contracts

## Supported workflow

The public workflow starts from a schema-5 document with an applied machine
configuration. From `flat-v-carve`:

```powershell
cargo run --release --locked -p cam-app -- collection apply-machine fixtures/v5/full-job.json --profile fixtures/gui2/machine.json --name Workbench --output configured.json
cargo run --release --locked -p cam-app -- collection export configured.json --output new-export-directory
```

The default CLI layout is ordered sequential files; `--layout one` writes one
program. `--through <operation-id>` limits execution to the enabled prefix.
Export writes a new directory containing checked program bytes, `manifest.json`
and `report.json`. Retrying a retained bundle saves the same bytes. Files must
run in manifest order on the same stock; a repeated tool does not permit moving
its later cuts ahead of intervening stages.

`SequenceProfile` in `cam-core/src/post/sequence.rs` is the reusable schema-2
configuration resolved from the document's applied machine snapshot. It owns
work offset, clearance, output precision, T/H mappings, compensation, path
control, spindle dwell, coolant and the M6 contract. Work-zero datum belongs to
the job setup; spindle direction belongs to operation assignments. Optional
rapid rate and holder geometry support display simulation, not controller
timing or authoritative holder-clearance proof.

## Checks and output identity

The retained collection path runs basic plan checks and requires complete
generation and resolved process state. It emits G0/G1, supported G2/G3 arcs and
G4 dwells, then independently reads the actual bytes to compare numeric motion,
tool changes and process state. Knife output also carries replay constraints.
The report binds output hashes, scope and execution identity. Output precision
starts at the profile's minimum and can increase up to nine decimals to retain
required motion; the report records the result and motion-profile observations.

Detailed M5 stock-quality verification is a separate engine capability. It is
not automatically run by ordered collection export. The earlier authenticated
combined-plan postprocessor and its tests exercise that stronger stock-analysis
path; the removed `cam plan`, `cam verify`, `cam export` and `cam verify-gcode`
commands are not supported user entry points. A successful collection export
must not be described as an M5 quality certification.

The subset reader validates generated syntax and semantics; it is not a general
LinuxCNC interpreter. It does not execute expressions, probing macros or the
machine configuration. See `cam-core/tests/sequence_export.rs`,
`sequence_bundle.rs`, `knife_output.rs` and `drill_core.rs`, plus
`cam-service/tests/retained.rs` and `cam-app/tests/collection_cli.rs`.

## Modal state and tool changes

Programs establish millimeters, absolute XYZ, XY plane and units-per-minute
feed. Cutter compensation is planned geometrically. `exact_path` emits G61;
bounded blending emits G64 with declared parameters. G61 exact path and G61.1
exact stop are distinct modes. Blending changes the controller's possible path
and must stay within the applicable declared tolerance; knife blending is
bounded by replay headroom.

Select one length-compensation contract:

- **Macro managed:** the M6 macro establishes the required compensation; the
  post preserves it rather than blindly clearing it.
- **Tool table:** the post applies the configured G43 H mapping after M6.

The post retracts/stops and selects the mapped tool, restores required modal
state according to the M6 return contract, and establishes spindle/coolant/feed
before cutting. Passive knife stages keep the spindle off. Machine-owned
tool-change movements are assumptions explicitly separated from checked plan
motions. The post does not invent hidden probing or parking behavior.

## Fixture machine assumptions

The [M6 fixtures](../../flat-v-carve/fixtures/m6/README.md) preserve the
user-described Z-only macro TLO contract: work Z0 at stock bottom/worktable,
then work-coordinate Z150 followed by X0 Y0 after M6. With 8 mm stock, internal
cut Z=-2 maps to output Z6, and internal clearance Z5 maps to Z13. Z150 is in
the selected work frame, not an implicit machine-coordinate move.

Those fixture profiles are synthetic test inputs, including editable choices
for work offset, tool numbers, spindle, coolant and dwell. The schema-1
`LinuxCncProfile` used by the older engine fixtures is distinct from the
schema-2 reusable collection configuration. Use the collection fixture in the
command above for the current public workflow.

## Remaining validation

An initial physical trial reported micro-segment motion, spindle sequencing and
feature-ordering problems; corresponding software changes are implemented.
That history does not qualify all current generated output for the real
controller. Actual macro/tool-table/INI/HAL inspection, matching controller
preview and a measured coupon remain in the [backlog](backlog.md).

Neither numeric readback nor raster simulation establishes hidden M6/probing
paths, fixture/holder clearance, spindle interlocks, loads, controller dynamics
or actual table/stock measurements.
