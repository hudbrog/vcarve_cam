# Schema diet: one document model, no conversion

Date: 2026-09-14\
Context: §0 of [field-testing-fixes-plan.md](field-testing-fixes-plan.md) —
saved jobs do not have to survive — and the plan in
[schema-diet-plan.md](schema-diet-plan.md). Engine 0.7.7.

## What the repository had

Three job models, and the same file shape doing two jobs at once: `job::Job`
(schema 3) was both the engine's planning input and a document the CLI, the
HTTP service and half the test suite parsed; `project::CamJob` (schema 4) was
both the collection substrate and a document; and `project::v5::CamJobV5`
(schema 5) was the document the GUI actually edits. Two conversion chains
(1/2/3 → 4 and 4 → 5) plus a file-backed tool library kept every one of them
alive.

## What it has now

**Exactly one parseable job document: schema 5.** `job::Job` and
`project::CamJob` keep their structs — the engine's `&Job` interface is
load-bearing and rewriting it would be a different project — but lose every
parse surface: no `schema_version`, no `from_json`/`from_svg`/`to_json`. The
engine input is built from a document or from the substrate, never from a
file. `project::v5::authoring` holds the one constructor an SVG import uses, so
the workspace and `cam import` produce the same document.

Nothing converts an old document any more. It is refused by name, with the
reason, and the caller's project is left alone:

| Entry point | Diagnostic | Behaviour |
| --- | --- | --- |
| `cam collection open/inspect/...`, `cam inspect` | `COLLECTION_SCHEMA_UNSUPPORTED` | "this is a schema N job; schema 5 is the only document model, and older documents are not converted" |
| the workspace (`Open job`, drop, recovery) | `CAM_JOB_SCHEMA_VERSION` | the failure is reported in the status line and the open document is untouched |

## Deleted

| Removed | Was |
| --- | --- |
| `cam-core/src/project/migrate.rs`, `project/v5/migrate.rs` | schema 1/2/3 → 4 and 4 → 5 conversion, including `migrate_v4`/`migrate_json` |
| `cam-storage` (crate) and its `tool-library` binary surface | the file-backed library directory, lock file and revisions |
| `cam tool-library ...` | library init/list/change/capture/apply/import/export |
| `cam sequence open/apply-profile/plan/export` | the schema-4 sequence CLI |
| `cam select`, `cam validate-job`, `cam plan`, `cam verify`, `cam export`, `cam verify-gcode` | the schema-3 job commands; their capability lives in `cam collection` |
| `cam-server`'s `library.rs`, `planning.rs`, `planning_worker.rs`, `verification.rs`, `artifact.rs`, `exporting.rs`, `motion_preview.rs` | the HTTP planning/verification/export/library API |
| `cam-service/src/{document,sequence,inspection}.rs` | the pre-collection document and sequence service surfaces |
| workspace menu `File → Import older job`, `Command::Migrate`, `IoKind::Migrate` | the GUI's forward-import entry |
| `knife::interpretation`, `command_svg`-era renderers (`job_svg`, `combined_svg`, `plan_svg`, `verification_svg`) | the interpretation mode that decided an import from the selected operation, and the previews of the deleted artifact formats |
| `job::MachineProfile`, `Job::machine_profile`, `CamJob::legacy_machine_profile`, `CamJobV5::legacy_machine_profile` | the schema-1/2/3 machine settings block embedded in a document: the engine input copied it across and the export gate compared the exporting profile against it. The schema-5 document keeps only the *applied* configuration, which is a copy of the reusable profile with no dependency on the file, so the field, its validation and both conversions are gone. |

The engine's own M6 profile type (`post::LinuxCncProfile`, schema 1, applied to a
substrate job by `apply_legacy_profile`) stays: it is the engine's export input
and its fixture corpus, not a document, and rewriting the engine is outside
this diet by design.

`cam serve` is now static hosting only: GET/HEAD of one UI directory, with the
boundary check on host/origin. The browser build plans in its own WebAssembly
worker, and the GUI owns library persistence (`library.json` import/export),
so there is no server-side planning, verification, export or library state to
keep in step.

## The CLI today

| Command | Meaning |
| --- | --- |
| `cam import <svg> --output <job.json> [--tolerance mm]` | one schema-5 document: the embedded artwork, a page-sized stock, the tolerances, **no operations** |
| `cam inspect <job.json> --output <inspection.json>` | read-only document view; the same implementation as `cam collection inspect` |
| `cam collection open/inspect/plan/apply-machine/apply-profile/reset/reapply/apply-tool/set-mapping/resolve-profile/knife-evidence/export` | the whole collection surface, documents only |
| `cam serve --ui-dir <directory>` | static hosting for the browser build |
| `cam geometry-spike`, `cam target-demo`, `cam target-preview`, `cam validate-model` | M0/M1 geometry evidence, unchanged |

Selection is the one open decision the plan recorded and this slice did not
take: selection belongs to an operation in schema 5, so an operation-scoped
`cam collection select` is the wanted shape, and it is not added yet.

Two command-line defects were found while rewriting the CLI tests and fixed:
`cam inspect` did not pass its own verb through to the collection surface (so
it reported "unknown collection command <path>"), and `cam collection
apply-machine` printed "0 mapping rows" from a projection that no longer
exists instead of counting the mappings it wrote. `cam collection
--library` now also accepts the **catalog file the workspace exports**
(`{schema, id, library}`), so a user can feed back `library.json` as it is
instead of unpacking it by hand.

## Fixtures

| Directory | Now holds |
| --- | --- |
| `fixtures/m3`, `fixtures/m4` | **engine inputs** in the substrate shape (no `schema_version`, no `machine_profile` block), loaded through the fixture-only loader; the planner tests compare against them |
| `fixtures/v4` | the same substrate copies the engine plan comparisons use (`rectangle`, `contact-line`, `resource-limit`) |
| `fixtures/v5` | the documents: `full-job.json` (face + carve + profile + knife over one artwork item) and `full-job-machine.json` (the same with an applied machine configuration) |
| `fixtures/gui2` | the stored collection document (`flower.job.json`) and its reusable machine configuration (`machine.json`) the CLI test drives |
| `real_data` | the tester's schema-5 jobs; the dead schema-3 preset documents (`flower_box-wood-balanced.job.json`, `flower_box-wood-finish.job.json`) are gone, and the READMEs now describe the settings instead of pointing at a converter |

## Evidence

| Command | Result |
| --- | --- |
| `cargo test --workspace --no-fail-fast` | 63 binaries, 622 passed, 0 failed, 4 ignored (exit 0) |
| `cargo clippy --workspace --all-targets` | clean, 0 warnings |
| `cargo fmt --all -- --check` | clean |

The same suite runs unchanged after the engine-input slice: no test was
deleted, and because no stored expectation pinned a fingerprint *value* — the
tests compare fingerprints to each other — the changed input hash needed no
fixture edits at all. The plan round-trip and stale-plan tests, which were the
point of the embedded job, now exercise the input's own form. The rebuilt
native GUI passes the smoke with an unchanged exported program hash, which is
the evidence that the region round-trip is exact: the target the second run
plans against is the one the first run planned against.

Tests that pin the new behaviour:

| Test | What it pins |
| --- | --- |
| `cam-service/tests/collection.rs::open_reads_schema_five_and_refuses_every_other_schema` | the one model, and refusal by name for older *and* newer |
| `cam-app/tests/jobs_cli.rs::import_writes_the_one_document_model_holding_the_artwork` | import writes a schema-5 document with no operations and a page-sized stock, and the document survives losing its source file |
| `cam-app/tests/jobs_cli.rs::an_older_or_future_document_is_refused_by_name_without_being_converted` | refusal code and message, no inspection written |
| `cam-app/tests/jobs_cli.rs::unsupported_artwork_is_refused_without_writing_a_document` | text is still refused, a refused import writes nothing, and the source is never the output |
| `cam-app/tests/collection_cli.rs::a_stored_document_exports_retained_files_matching_their_manifest` | a stored document plus the copied machine configuration exports checked bytes with their manifest |

## Follow-up slice: the engine input stops being a job

The one place the diet stopped short was the V-carve engine's front door:
`job::Job` was still built by an adapter that had to fabricate an inert source
snapshot (`filename: "collection"`, an empty SVG) because the engine type
expected one. That is now gone.

`job::VcarveInput` is the engine's only input, and it is not a job:

| Carried | Where it comes from |
| --- | --- |
| `region` (the resolved selected union, on its own grid) | the caller's geometry authority: the schema-5 resolver or the substrate import |
| `source_error_mm` (flattening + source-snapping bound) | the same import that produced the region |
| `stock`, `operation`, `tools`, `tolerances`, `endmill_planning`, `vbit_planning` | `PlanContext` fused with the operation's settings, in one constructor (`operations::flat_vcarve::vcarve_input`) |

What that removed:

* `to_legacy_job` (substrate → engine) and `to_legacy_job_v5` (document →
  engine), and with them the last reason to build a `Job`;
* the self-importing entry points `plan_endmill(&Job)` and
  `plan_combined(&Job)`, plus the `_with_region` variants: the region travels
  inside the input, so there is one entry point per stage;
* `Job::inspect()`, `JobInspection` and the `missing_machining_fields` list,
  which existed for the deleted `cam validate-job` command;
* `input_hash`'s document shape: the plan's input fingerprint now hashes the
  plan's own input, and both fingerprints change once (regenerated, not
  migrated).

The engine artifacts carry that input, so a stored plan can still rebuild the
target it was planned against. The region serialises as its snapping grid plus
grid coordinates and reloads through the validated
`Region::from_grid_rings`, never as trusted geometry. `FixtureJob` keeps the
fixture form readable — `fixtures/m3` and `fixtures/m4` still name an SVG and
the ids selected from it, and resolve through the one importer — and the
benchmark examples read the plan's `input` back with the hidden
`job::input_from_json`.

`cam-service`'s `summary.rs`, a projection of the legacy plan types left over
from the deleted HTTP planning API with no callers, is deleted with them.

Browser and review material moved with the code: `gui4-scenario.mjs` no longer
walks "Import older job" (the entry is gone) — it opens an engine-shaped file
and asserts the refusal leaves the project untouched, then saves and reopens
the portable document.

## Still open

* **`cam collection select`.** Recorded above: not implemented, deliberately.
* **Recovery envelope.** The workspace recovery record is still numbered
  `schema 3`, which is now only its own format version (it embeds a schema-5
  job). Renaming it is cosmetic and was left alone.
* **Milestone reproduction scripts.** `scripts/benchmark-m5.ps1`,
  `scripts/check-m6.ps1`, `scripts/benchmark-flower.ps1`,
  `scripts/benchmark-settings.mjs` and `scripts/analyze-motions.mjs` drive the
  deleted `cam plan`/`cam verify`/`cam export` commands (and parse the plan
  artifact those commands wrote). They are not run by CI and were left in
  place with the staleness recorded in the README and the `fixtures/m5` and
  `fixtures/m6` READMEs; porting them to `cam collection` (or deleting them) is
  its own slice.
