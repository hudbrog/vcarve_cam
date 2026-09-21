# Pocket demonstration

`two-pockets.job.json` is a schema-5 job with two selected filled components,
one island, a 4 mm endmill, 2.3 mm depth in 1 mm maximum layers, 2 mm stepover,
0.2 mm roughing allowance removed by wall finishing, 1 mm center-path helix
radius, tangent arc lead-in and tangent line lead-out. No machine configuration
is applied. These are software demonstration values, not cutting recommendations.

Regenerate from the workspace root:

```powershell
cargo run -p cam-core --example pocket_fixture --locked --quiet |
  Set-Content fixtures/pocket/two-pockets.job.json -Encoding utf8NoBOM
```

The browser scenario edits a copy, verifies selection and numeric drafts,
applies the existing example machine for software readback, simulates both
pockets, downloads checked output, and reopens the saved job. It never sends
the program to a machine. See [the Pocket guide](../../../docs/flat-v-carve/pocket.md).

The example's `--measure` option measures generation and independent checks
without writing a job. `--measure-svg <path>` measures a selected real drawing
with a 2.5 mm endmill, 1 mm depth, plunge entry and 1.25 mm stepover.
