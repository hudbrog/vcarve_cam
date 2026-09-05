# Verification capture

`m5-limited.json` contains the M5 report and source plan fingerprints from Rust 0.7.2:

1. Generate a combined plan from `fixtures/m4/narrow-channel.json`.
2. Run `cam verify <plan.json> --output <report.json> --max-cells 1`.
3. Keep the report's `verification`, `input_fingerprint`, and `motion_fingerprint` fields.

The exhausted resource budget deliberately produces inconclusive evidence. This is a synthetic contract fixture, not an application machining preset. Live integration tests separately regenerate the full CLI/HTTP reports and compare every field.
