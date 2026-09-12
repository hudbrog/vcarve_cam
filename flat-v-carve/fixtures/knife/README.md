# Knife blade-offset fixtures

`real-outline-0.25.job.json` is a trimmed derivative of a real report: the
Inkscape tracing job `real_data/knife`, whose selected outlines are cut with a
0.25 mm Roland drag knife. Only the `artwork-1-knife-outlines` item's
`outline-1` path is kept, with its exact coordinates, page size and
`centerline` interpretation; the tool, operation settings, stock and
tolerances are the job's. The chain's `source_revision.content_digest` is
recomputed for the trimmed source, so the reference still resolves.

That outline opens with a 0.0036 mm segment — the near-degenerate lead a
tracing tool leaves behind. The knife planner used to place the holder one
*fixed* millimetre ahead of the first tip vertex instead of one blade offset,
so the tip planted `1 - offset` mm along the chain; with a 0.25 mm blade the
independent replay rejected the plan (`KNIFE_TIP_ERROR`, 0.7465 mm against a
0.01 mm budget) and the whole operation published no motions. The fixture
pins that regression in `crates/cam-core/tests/knife_geometry.rs`.
