# Schema diet — one document model

Date: 2026-09-14\
Context: the compatibility policy in
[field-testing-fixes-plan.md](field-testing-fixes-plan.md) §0 ("saved jobs do
not have to survive") and open question 9 there.\
Status: landed — stages 1–4, plus the follow-up slice that turned the engine's
input into `VcarveInput` instead of a job (see the progress note).
`cam collection select` remains the one open decision, and the milestone
reproduction scripts still drive the deleted CLI. See
[schema-diet-progress.md](schema-diet-progress.md).

## What the repository holds today

Three job models, and only one of them is a document:

| Model | Where | Role |
| --- | --- | --- |
| `job::Job` (schema 3) | `cam-core/src/job.rs` | **Two roles at once.** The engine's planning input (`pocket::plan_endmill`, `vcarve::plan_combined`, `post`, `verification`, `stock`, `target` all take `&Job`), *and* a parseable document the CLI, the web server and half the test suite read and write. |
| `project::CamJob` (schema 4) | `cam-core/src/project.rs` | The collection-shaped substrate: the contour catalogue is built from it, and the v5 planner converts to it (`to_legacy_job_v5`, `to_legacy_job`). Also a parseable document. |
| `project::v5::CamJobV5` (schema 5) | `cam-core/src/project/v5/` | The document the GUI edits, `cam collection` reads and writes, and every new feature targets. |

Two migration chains keep the old parse paths alive: `project/migrate.rs`
(1/2/3 → 4) and `project/v5/migrate.rs` (4 → 5), plus the entry points that call
them (`cam collection open`, the GUI's *Import older job*, `cam sequence`,
`cam-server`'s job endpoints, `cam-storage`'s library capture).

## Target

**Exactly one parseable job document: schema 5.** Everything else is either an
in-memory substrate or deleted.

* The engine input keeps its algorithms — rewriting `pocket`, `vcarve`, `post`
  and `verification` is a different project — but loses its job shape: no
  `source`, no `import`, no `selected_region_ids`, no `schema_version`. It
  becomes `job::VcarveInput`, built **only** by fusing the planner context,
  the operation's settings and the resolved region
  (`operations::flat_vcarve::vcarve_input`), so no source is imported or
  re-imported anywhere below the planner. Its serde form exists because the
  plan embeds the input it was generated from, not to read a job.
* `project::CamJob` likewise stays as the in-memory collection substrate and
  loses its document surface.
* Every migration module, migration command, migration UI entry, and the
  fixtures that exist only to be migrated, are deleted.
* Legacy machine-profile and tool-library conversion (the `legacy_machine_profile`
  field, `apply_legacy_profile`, the server's file-backed library) is deleted
  with them.

## The CLI

`cam` stays, on schema 5:

| Command | Fate |
| --- | --- |
| `cam import <artwork.svg> --output <job.json>` | Re-pointed at schema 5: writes the same document an SVG import produces in the GUI (artwork, page-sized stock, tolerances, no operations). |
| `cam inspect <job.json>` | Re-pointed at schema 5: the artwork tree, geometry bounds, per-operation issues. |
| `cam collection open/inspect/plan/export/...` | Kept; `open` becomes schema-5 only (it is the schema-5 CLI surface already). |
| `cam collection select` | **New**: selection belongs to an operation in schema 5, so the command names the operation. |
| `cam select`, `cam validate-job`, `cam plan`, `cam verify` (schema-3 job commands) | Deleted; their capability lives in `cam collection` (`plan`, `export`) and `cam inspect`. |
| `cam sequence ...` | Deleted: superseded by `cam collection`. |
| `cam serve --ui-dir ...` | Kept as static hosting for the browser build. The HTTP planning/verification task API and the server-side tool library go: the browser GUI plans in a wasm worker and never calls them. |
| `cam tool-library ...` | Deleted with the file-backed library; the GUI owns library persistence (import/export of `library.json` stays). |
| `cam export`, `cam verify-gcode`, `cam geometry-spike`, `cam target-demo`, `cam target-preview`, `cam validate-model` | Kept if their plan/machine input is schema-5; otherwise re-pointed. |

## Execution order

1. **Remove the conversion of old documents.** Delete `project/migrate.rs`,
   `project/v5/migrate.rs`, the migration entry points (CLI, service, GUI menu,
   recovery gate, examples) and the tests whose only subject is migration.
   Store nothing in a superseded schema.
2. **One construction path for the engine input.** Give the substrate a
   document-free constructor (`from document`), re-point every test and caller
   at it, then delete `Job`/`CamJob`'s parse surfaces and their fixtures.
3. **Port the CLI.** `cam import`/`inspect` speak schema 5; delete the
   schema-3 job commands and `cam sequence`; add the operation-scoped
   `collection select`.
4. **Delete the old settings and library surfaces.** `legacy_machine_profile`
   and its conversion, `cam-storage`, `cam tool-library`, the server's
   planning/verification/library endpoints.

Each stage ends with the workspace suite, clippy and fmt clean, a progress note
under `docs/flat-v-carve/`, and no fixture left in a schema the code cannot
read.

## Risks and open decisions

* **Engine input churn.** The engine's `&Job` interface is load-bearing; the
  diet must not become an engine rewrite. What goes is the *parse*, not the
  struct.
* **Deleting product surfaces.** `cam serve`'s HTTP API, `cam sequence` and
  `cam tool-library` are removed on the evidence that the shipped UI does not
  use them (the browser GUI plans in-process and persists its own library).
  If a consumer exists outside this repository, that evidence changes and the
  endpoint should be ported instead.
* **`cam verify`.** M5 adaptive verification is bound to the M4 `CombinedPlan`.
  If the port does not reach it in this pass, the capability must be reachable
  from `cam collection` before the schema-3 command is deleted, not after.
