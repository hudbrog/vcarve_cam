# Complete recorded-motion preview

The unchanged `real_data/flower_box-svg.job (2).json` produces 384,245 motions.
The old service sent the first 20,000 (5.2%) and discarded the rest from the
preview. Those 20,000 contained all 17,072 endmill motions but only 2,928 V-bit
motions, including 2,836 cuts. Missing finishing paths were a display cutoff.

## What changed

The service writes an additional temporary preview file containing every exact
recorded motion in pages of at most 20,000. Worker IPC still contains only the
first page, page offsets, and bounded reports. The first result response includes
`nextMotionOffset`; authenticated `tasks/{id}/motions/{offset}` requests retrieve
subsequent pages. `planning.previewMotions` denotes the page size. Summary
`previewMotionCount` now equals `motionCount`, with zero omitted motions.

The browser validates each page's task identity, immutable summary, offset,
continuation, and motion fields. It reports loading progress and publishes only
the complete assembled preview. Failed, aborted, missing, or inconsistent pages
cannot be mistaken for a complete result. Existing task freshness rules still
hide motions after job edits, stage changes, or service restarts.

Connected segments share SVG subpaths, retaining every XYZ endpoint. Each SVG
path contains at most 4,096 motions. Tool and layer filters still use the recorded
metadata; travel is hidden by default and can be displayed explicitly. “Fit plan”
uses an iterative bounds calculation, avoiding argument-stack overflow on large
motion arrays. No machining coordinates, feeds, tolerances, or fingerprints change.

The sidecar uses the same exclusive creation, ownership, cancellation, eviction,
and delete-on-close lifecycle as the saved plan. Active page reads retain the
file until completion. Reports and worker messages retain their existing limits.
The browser's assembled motion array and drawing memory still scale with the
plan; paging bounds individual transfers, not total browser memory.

## Segment lengths and entry count

| Recorded kind | Count | Median XYZ length |
| --- | ---: | ---: |
| Endmill cuts | 16,872 | 0.05161 mm |
| V-bit cuts | 125,098 | 0.06091 mm |
| V-bit plunges | 60,485 | 1.22038 mm |
| V-bit approaches / retracts | 60,549 each | 30 / 30.09299 mm |

Short segments reflect the finely flattened source contours and the V-bit's
adaptive XYZ/clearance checks. Many separate V-bit candidates and their final
finish executions also add entry/travel motions. Thus motion count is not the
number of visually distinct lines in top view. There is room to investigate
machining-path joining separately, but replacing curves or connecting separate
cutting paths needs renewed clearance and stock checks. This preview fix joins
drawing commands without changing the generated machining plan.

## Validation on Windows, 2026-09-06

The unchanged flower loaded **384,245 / 384,245** motions across **20 pages** in
**2,861 ms** in the HTTP integration check. Assembled preview JSON was 87,730,894
bytes; individual result pages remained below 16 MB. The canonical plan was
110,745,717 bytes. Every motion field matched the canonical artifact, and the
artifact matched the same-engine CLI output apart from its final newline.
The retained file also reopened for independent verification with the expected
one-cell-budget `inconclusive` result (a transport check, not a quality verdict).

Checks passed: 26 Rust service tests, 98 frontend tests, 27 live integration
checks including the real flower, formatting, strict service Clippy, TypeScript,
and contract drift checks. Regression coverage includes page boundaries, late
motions, disconnected segments, hidden travel, stale/missing pages, cancellation,
temporary-file leases, empty plans, and fitting a 384,245-motion plan.

The browser rendered 202,505 cutting motions in 51 SVG paths with travel hidden,
and all 384,245 motions in 97 paths with travel enabled. V-bit filtering retained
185,583 cutting motions; Fit plan and zoom worked on the complete data. The
rebuilt portable executable also passed all 26 default live integration checks.

Reproduce after building the production UI and release executables:

```powershell
$env:CAM_TEST_REAL_DATA = '1'
pnpm --dir flat-v-carve/web check:live
```

Rebuild/restart the portable application and regenerate the plan to use the
complete preview. Existing generated machining artifacts do not need altered
cutting settings.
