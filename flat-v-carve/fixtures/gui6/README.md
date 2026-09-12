# GUI6 knife fixtures

`chains.svg` contains an open corner and closed square. `knife.job.json` contains
explicit software-review stock, knife, feed, heading and machine settings and
generates 96 motions. These are not recommendations for a material or machine.

`flower-centerlines.svg` is a deliberate cutting-source derivative of
`../../../real_data/flower_box.svg`: the single path style changes from filled
black to an unfilled black stroke. Path data and page dimensions are unchanged.
`flower-knife.job.json` selects `path1-chain-7` and generates 813 motions. The
original filled artwork is not silently imported as a knife cut.

Reproduce from the Rust workspace after building the native worker:

```
node fixtures/gui6/create-review-fixture.mjs
node fixtures/gui6/create-review-fixture.mjs --flower
```

An optional executable path selects a particular review worker. Both fixture
jobs must pass actual generation and checked-output eligibility before the
script writes them. See `docs/flat-v-carve/gui6-review.md` at the repository root
for the full review recipe and explicit settings.
