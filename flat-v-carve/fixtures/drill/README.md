# Drill marker fixtures

`markers.svg` exercises every marker style the drill point catalogue derives:

- `<circle>` elements are exact markers: the importer captures the analytic
  center and radius before flattening discards them, so the hole position and
  the marker diameter are exact. `hole-a`…`hole-c` are 5 mm circles;
  `large` is a 12 mm circle drawn unfilled with a hairline stroke — same
  exact treatment.
- A filled dot drawn as a `<path>` (here a 4×4 mm square, `dot`) carries no
  analytic center: the catalogue derives its area centroid and an
  area-equivalent diameter, which is how stroke-to-path circles and imported
  DXF polylines read too.
- A filled circle yields exactly one point: its analytic center suppresses
  the derived centroid, so nothing is listed twice.

Run the catalogue over the fixture with the pinned toolchain:

```sh
cargo run --release --locked -p cam-app -- import fixtures/drill/markers.svg --output artifacts/drill/job.json
```

The engine-side expectations are pinned in `crates/cam-core/tests/drill_points.rs`
and `crates/cam-core/tests/drill_core.rs`.
