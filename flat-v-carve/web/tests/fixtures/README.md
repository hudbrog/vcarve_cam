# Verification capture

`m5-limited.json` contains the M5 report and source plan fingerprints from Rust 0.7.2:

1. Generate a combined plan from `fixtures/m4/narrow-channel.json`.
2. Run `cam verify <plan.json> --output <report.json> --max-cells 1`.
3. Keep the report's `verification`, `input_fingerprint`, and `motion_fingerprint` fields.

The exhausted resource budget deliberately produces inconclusive evidence. This is a synthetic contract fixture, not an application machining preset. Live integration tests separately regenerate the full CLI/HTTP reports and compare every field.

## M6 export captures

`m6-passed.json` and `m6-coarse.json` are captured from `pnpm check:live` with Rust 0.7.2. Both use `m4/narrow-channel.json` and `m6/macro-stock-bottom.json`; the latter changes precision to zero and is rejected with `POST_ROUNDING`. The test writes the report and exact program bytes plus source task to `test-results/live/`. Copy those captures here with task IDs replaced by `source-plan`/`export-task` and instance IDs by 32 `a` characters; other evidence and byte hashes remain unchanged. These are contract-test inputs, never application presets.
