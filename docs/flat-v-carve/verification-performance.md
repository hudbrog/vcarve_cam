# Conclusive flower verification under 20 seconds

Measured on native Windows on 2026-09-06, engine 0.7.5. The web service's verification of the unchanged `real_data/flower_box-svg.job (2).json` now returns **passed in 7.26–8.00 seconds**, with zero unresolved cells and all default verification budgets and machining tolerances preserved.

## Measurements

| Build/path | Run 1 | Run 2 | Run 3 | Outcome |
| --- | ---: | ---: | ---: | --- |
| Original private worker | 210.354 s | — | — | Inconclusive |
| Earlier optimized development service | 8.721 s | 8.662 s | 8.709 s | Inconclusive |
| Earlier optimized portable app | 13.070 s | 9.381 s | 10.159 s | Inconclusive |
| Final portable app, browser adapter | **7.256 s** | **7.298 s** | **8.000 s** | **Passed** |

Browser-adapter timings cover verification submission, the same 700 ms polling cadence used by the UI, and downloading/validating the final report. Each run submits a new task ID and performs a fresh calculation. These are not cached-report timings. The combined plan is generated once before the verification measurements, as in the UI workflow.

Every run uses 1,000,000 maximum cells, depth 24, 4,096 reachability cells, 512 maximum depth bands, 64 findings, and original coordinates. Motion tolerance remains 0.01 mm, verification tolerance 0.05 mm, and the requested ridge/detail limits 0.1 mm. The plan has 17,063 endmill motions and 28,501 V-bit motions. Its input and motion fingerprints are identical to the original measured plan.

The final report evaluates 519,395 cells, has no findings or unresolved cells, and covers the complete depth range with 68 continuous bands. Maximum-error uncertainty is `0.046756319 mm`, below the requested `0.05 mm`. All three reports have identical numerical bounds:

| Maximum error | Lower bound (mm) | Upper bound (mm) | Limit (mm) |
| --- | ---: | ---: | ---: |
| Overcut | 0 | 3.3463e-10 | 0.05 |
| Floor ridge | 0.083624204 | 0.099993924 | 0.1 |
| Cutter-limited detail | 0 | 0 | 0.1 |
| Other reachable residual | 0.003243627 | 0.049999945 | 0.05 |

These are original-coordinate bounds for the normalized geometric model. Area and volume bounds retain spatial uncertainty; no rounded flower output or machine program is claimed by this measurement.

Final evidence is in `flat-v-carve/artifacts/verification-investigation/conclusive-portable/`, including measurements, the accepted plan summary, source-job SHA-256, and full reports. Earlier results remain in the `service/` and `portable/` sibling directories. The final executable is `flat-v-carve/artifacts/verification-conclusive/cam.exe`, SHA-256 `0A02E0F91526301AE64455D20D8D1D04EFB54BBEF6424D5E09E7541D0E51C276`.

## Changes

1. Depth-band occupancy intervals already apply throughout each closed band. Seed them with physical depth limits and refine to the same tolerance, instead of requiring a boundary for every distinct motion endpoint Z. Continuous rising/falling V-bit sweeps retain their existing analytical treatment.
2. Extend target/sweep correlation from an affine face roof to the convex distance to a whole boundary segment. This includes cells beyond segment endpoints and eliminates the need to refine every cell until it fits beside one short flower boundary segment. The existing boundary BVH finds candidate roofs. Any improving edge must be within `slope * (maximum corner cut depth + current residual upper bound)` of the cell; an outward arithmetic reserve protects that search.
3. Reuse the cell-center boundary sample. The former box query repeated that sample and performed two unused corner containment/distance queries solely to validate their coordinates. Cheap range checks retain that validation.
4. Retain a small plan-completion receipt in the service ledger when generating the plan. Verification checks the file against that receipt and recomputes its job/motion/execution fingerprints, then independently checks the actual cuts. It avoids regenerating candidates, preview stock polygons, and sampled M4 quality reports. The receipt is internal parent/worker data; public requests cannot supply it. Imported/portable artifacts without a service receipt continue through full reconstruction. Cancellation, source-file leases, task identities, and result retention remain intact.

This work does not introduce verification-result caching or change Export to reuse an earlier M5 report.

## Why the stronger bound covers the whole cell

For any boundary segment `e`, nominal target depth `T(p)` is at most `distance(p,e)/slope`, including outside the region and where the depth cap applies. Distance to a segment is convex. The unclamped removal `U(p)` from one continuous linear XYZ V-bit sweep is concave. If that same sweep cuts all four corners with the required arithmetic reserve, concavity establishes positive removal throughout the rectangle, and total removal `A(p)` is at least `U(p)` there.

Consequently `T(p)-A(p) <= distance(p,e)/slope-U(p)`. The right-hand side is convex, so its maximum over the rectangle is bounded by its four corner values. This does not assume that `e` is the nearest boundary feature, that corner projections lie on the interior of `e`, or that XY/Z motion is sampled. The existing floating-point reserves remain in the bound.

This generalization alone makes the unchanged flower plan conclusive within the time target. Multi-sweep unions, priority-queue changes, and resumable verification were not needed or introduced. Different sweeps reaching the target at all four corners still do not prove the interior is cleared; a new regression explicitly exercises that gap.

## Earlier cell-budget diagnostic

Before the convex segment-distance bound, a diagnostic on the same saved combined plan doubled the cell budget to the supported maximum of 2,000,000. It still returned `inconclusive`. This was a CLI diagnostic with saved-plan reconstruction, not another measurement of the retained web-service path.

| Cell budget | Evaluated cells | Unresolved rectangles | Unresolved fraction of the domain bounding box |
| --- | ---: | ---: | ---: |
| 1,000,000 | 999,999 | 11 | 76.1911% |
| 2,000,000 | 1,999,999 | 20 | 59.3577% |

The rectangles are disjoint terminal cells, so their areas can be added. These percentages describe the enclosing XY domain, including space outside the selected artwork; they are not percentages of defective or uncut material. The count can increase as large unresolved rectangles split. In both runs, one remaining rectangle covers the entire right half of the domain. Depth-first traversal exhausts the cell budget before refining that region. The deepest cell is only at level 13, below the configured depth limit of 24.

Both runs prove an overcut upper bound of about `3.35e-10 mm` and zero unreachable detail for the ideal pointed V-bit. Neither finds a failed criterion. However, the floor-ridge maximum remains enclosed by `[0.0751125, 2] mm` against a `0.1 mm` limit. Other reachable residual remains enclosed by approximately `[0.00325, 2] mm` against `0.05 mm`. The `2 mm` upper endpoints are loose bounds reaching the job's full depth cap, not measured defects. A pass requires every applicable upper bound to satisfy its criterion, maximum-error enclosure widths at most `0.05 mm`, and completion of the remaining motion/report checks.

The two-million-cell evidence is in `flat-v-carve/artifacts/verification-investigation/conclusive-2m.summary.json`; the full CLI report is beside it. These historical results motivated tighter regional bounds rather than a larger default cell budget.

## Validation and reproduction

- Native release workspace tests pass, including continuous clearance, stock bounds, motion/rounding challenges, saved-artifact replay, and cancellation/retention tests.
- New tests cover thousands of distinct continuous Z endpoints without a depth-band-limit failure, interior points within analytical cone bands, receipt/full-replay report parity, changed job/motion/execution rejection, and rejection of receipts in public HTTP requests.
- Two additional unit regressions exercise a rising XYZ sweep beyond both faces at a reentrant corner (including finite tips and large translations), and an interior gap despite all four corners reaching the target. The corner bound is compared to an independent motion sampling reference with an explicit sampling-error enclosure.
- TypeScript checking passes. Five targeted final-portable integration tests cover asset integrity, CLI/report parity for passed/failed/inconclusive verification, and live cancellation. All 100 frontend tests also passed during the earlier optimization; application UI code is unchanged by this follow-up.
- Strict workspace/all-target Clippy and formatting/diff checks pass.
- The opt-in performance test now requires that each of three fresh requests returns `passed`, has zero unresolved cells, meets maximum-error uncertainty, and completes in less than 20 seconds, including report transport. An ignored core benchmark also verifies the unchanged saved flower motions directly; it measured 9.409 seconds.

From `flat-v-carve/web` after building the portable executable:

```powershell
$env:CAM_TEST_EXE = 'D:\proj1\flat-v-carve\artifacts\verification-conclusive\cam.exe'
$env:CAM_FLOWER_VERIFY = '1'
$env:CAM_FLOWER_OUTPUT = 'D:\proj1\flat-v-carve\artifacts\verification-investigation\conclusive-portable'
node .\node_modules\vitest\vitest.mjs run integration/flower-verification.test.ts
```

Use the updated executable with `serve --open --port 0` to open a separate local instance. Generate the combined plan once in that instance so its completion receipt is available for verification.
