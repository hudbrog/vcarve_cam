# Complete plans on disk, bounded browser responses

The live service previously serialized the complete plan into a string capped at
16 MB, then embedded that string in a worker reply capped at 32 MB. The unchanged
saved flower job produces about 111 MB of compact plan JSON. The failure was in
private worker transport, before the browser received a result. Raising the caps
would still duplicate complete plans during serialization, retention, download,
and submission to verification/export.

## Storage and transport

The parent service reserves a unique temporary plan file with exclusive creation.
The planning worker streams the canonical plan JSON into that file through a
1 MiB buffer. It flushes the file before returning its bounded summary, motion
preview, and stock inspection. The parent installs the file only after a successful
worker exit and task acceptance. Partial files from failures or cancellation never
become downloadable results. Write failures report `PLAN_ARTIFACT_IO`.

Complete plan bytes are absent from both worker replies and subsequent
verification/export requests. Those requests carry a service-generated path and
the existing immutable identity. HTTP clients still supply only task IDs and
validated job/profile metadata; they cannot choose or retrieve arbitrary paths.

Verification and export stream-deserialize the retained file directly into typed
plan records. Loading retains engine-version, job/motion fingerprint, execution,
clearance, stock, and finish checks. Nested endmill data no longer passes through
a generic JSON tree and another string. Canonical plan hashes are also calculated
without a complete serialization buffer. Artifact schema, engine version, and
fingerprint bytes are unchanged; cached analysis remains untrusted.

Fresh reconstruction produces an immutable `AuthenticatedPlan`. Verification
and export consume that object without serializing the plan or reconstructing M4
again. Existing entry points accepting a mutable `CombinedPlan` first revalidate
its typed records and discard supplied analysis. This replaces hidden JSON
round-trips inside M5/M6 as well as those in service transport; later changes to
the caller's source cannot alter the authenticated snapshot.

The browser now receives the summary and first page of up to 20,000 motions,
then requests every remaining page before publishing the complete preview. The
[complete preview report](complete-motion-preview.md) describes this follow-up
and its measurements. Stock slices retain their independent geometry budgets. An
explicit artifact download reads at most 64 KiB per chunk and advertises the full
content length. The service does not clone the complete file into an HTTP body.

## Ownership and limits

Reference-counted leases keep files alive for retained results, queued/running
verification or export, and active downloads. Eviction releases the result's
lease without interrupting these users. Cancellation reaps the worker before
releasing its source/output file. Normal shutdown clears retained results after
workers stop. The last lease removes the exact temporary file, without recursive
directory cleanup. Windows additionally uses delete-on-close handles so process
termination releases temporary storage. Abrupt termination on other platforms
can leave files for the operating system's temporary-directory cleanup.

Wire version **ui-7** advertises `planning.artifactBytes: null`: the live service
has no separate limit on serialized plan size. This is not unlimited computation.
Disk capacity, planner motion/path/sample budgets, worker memory, the five-minute
task timeout, one active calculation, four unfinished tasks, four retained results,
and 128 task records still apply. A download may outlive result eviction. Reports
remain capped at 16 MB, program sets at 8 MB, and worker replies at 32 MB.

The legacy CLI/string-based portable-plan APIs still enforce their existing
128 MB file/string limit. The live service uses the new stream-loading path for
its own generated files. Larger live artifacts therefore require that path for
reopening; this change does not claim arbitrary-size CLI file support or bounded
memory for the planner's actual motion vectors.

## Regression checks

Unit and integration coverage includes streamed serialization above 128 MB with
a sub-1-KiB worker reply, streamed plan loading above the string API limit,
unchanged fingerprints and cached-analysis rejection, stale motions/job rejection,
truncated data, partial-file cleanup, downloads surviving eviction, and queued
verification/export leases surviving source-result eviction.

From `flat-v-carve`, after building the production UI and release executables:

```powershell
cargo test --release --locked -p cam-core -p cam-server
pnpm --dir web test
$env:CAM_TEST_REAL_DATA = '1'
pnpm --dir web check:live
# A separate larger geometry case, with unchanged cutting/accuracy settings:
$env:CAM_TEST_LARGE_PLAN = '1'
pnpm --dir web check:live -t 'two flower copies'
```

The real-data case imports the original SVG, opens the unchanged saved job,
compares every saved artifact byte with the same-engine CLI (apart from its final
newline), checks every motion in the complete paged preview, and reopens the retained plan for
independent verification. The larger case repeats the flower at unchanged physical
size and accuracy on a wider page, then downloads and verifies a plan above
128 MB. Its saved job is a generated regression fixture, not a change to
`real_data`. Both checks deliberately restrict verification to one cell; the
expected `inconclusive` result tests transport/authentication and does not claim
machining approval. Measurements are written to `web/test-results/live/`.

## Original measurements before complete preview paging, 2026-09-06

| Input | Complete plan bytes | Browser preview bytes | Recorded motions |
| --- | ---: | ---: | ---: |
| Unchanged saved flower job | 110,745,717 | 3,961,182 | 384,245 |
| Two flower copies with the same cutting/accuracy settings | 221,806,245 | 3,979,507 | 768,501 |

Both plans completed, downloaded in full, and reopened for independent
verification with the expected one-cell `inconclusive` result. Doubling the
artwork roughly doubled the complete artifact while leaving the browser preview
at about 4 MB. The unchanged flower artifact plus the CLI's final newline retains
the prior SHA-256 `b1979b992f547b2938762d4f1868934b80df2b8171d74d5eeeff9c2a5db790c5`.
The two-copy artifact SHA-256 is
`5982344725070f64981e872ec28f31bb0031e2f765d2c8a3098e7d487949a76e`.

All 163 core tests, 24 service tests, and 92 frontend tests pass. The final portable
build passes 27 live integration checks including the larger case; the separate
unchanged-flower check also passed earlier. Strict Clippy, formatting, TypeScript,
contract drift checks, and the portable build pass. The existing Vite bundle-size
advisory remains. The usual `artifacts/portable/cam.exe` has been updated to the
tested executable (SHA-256
`446023c0f1ab3588b05adc43c2ae6881584695a714529142f3972f51dd6c7c1b`).
