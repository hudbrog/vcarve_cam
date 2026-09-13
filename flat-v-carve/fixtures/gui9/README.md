# GUI9 fixtures — larger jobs and deeper inspection

## `flower-box-batch.job.json`

The GUI9a larger-job fixture. It is the accepted GUI2 carving — the same
`flower_box.svg` source bytes, the same endmill/V-bit, the same feeds, the same
1 mm depth — placed as a batch of six identical copies on one stock:

| Property | Value |
| --- | --- |
| Artwork copies | 6 (3 columns × 2 rows, 10 mm apart) |
| Operations | 6 × Flat V-carve, one per copy |
| Stock | 620 × 210 × 20 mm, explicit `setup.stock.xy` |
| Planned motions | 137,717 (accepted single-copy reference: 22,883) |
| Plan time | ~19.5 s on the development machine (release) |

Reproduce it from the committed source job:

```powershell
cd flat-v-carve
node fixtures/gui9/create-batch-fixture.mjs        # 3 x 2 columns of copies
node fixtures/gui9/create-batch-fixture.mjs 2 1    # a smaller batch
```

The fixture exists because the display had only ever been measured on a single
copy. It raises the display's motion count above the previous 100,000-motion
admission limit without changing machining intent, so the same automated
workflow (open → generate → scrub → export) exercises many motion pages and the
full checkpoint ladder instead of three pages and eighteen checkpoints.

Its stock rectangle is stated explicitly so the display grid — and therefore
the transferred checkpoint size — is a property of the fixture rather than of
the automatic artwork margin.

Measurement harness:

```powershell
cd flat-v-carve
$env:CAM_GUI9_MEASURE_OUT = 'artifacts/gui/gui9a-large-job.json'
cargo test --release -p cam-gui --test large_job --locked -- --ignored --nocapture
```

`cargo test -p cam-gui --test large_job` (debug) runs the admission test only;
the end-to-end batch measurement is the explicit release run above, because
debug timing is not a measurement.
