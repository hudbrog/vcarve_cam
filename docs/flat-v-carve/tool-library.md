# Local tool library backend

The Rust backend stores named endmill/V-bit definitions and their optional cutting
presets. It supports create, read, replace, duplicate, delete, import/export,
capture from a job, and explicit application to a job. The CLI and reusable Rust
API are implemented, with [browser controls and local HTTP transport](web-ui/tool-library-ui.md)
for the same records and revision checks.

## Data ownership

- A `LibraryTool` contains an ID, display name, complete cutter geometry, optional
  plunge/ramp capabilities, and `cutting_presets`.
- A `CuttingPreset` has an ID and name, optional material/machine context labels,
  and optional spindle RPM, cutting/plunge feeds (mm/min), stepdown and stepover
  (mm). Context labels do not select machine profiles or generate recommendations.
- Tool IDs are unique throughout the library. Preset IDs are unique within their
  owning tool. IDs use 1–100 ASCII letters, digits, underscores or hyphens; they
  are never file paths. Names/context labels allow 1–1000 bytes of non-control
  text and cannot be whitespace-only. `null` represents absent context or values.
- Library schema **1** and its monotonically increasing `revision` are separate
  from job schema **3**, engine version, and plan identity. No job schema migration
  or engine version change is needed for this feature.
- Machine tool numbers, work offsets, M6 contracts, stock, target settings,
  planning controls, and tolerances remain job/machine-profile data.

The library starts empty. No fixture tools, cutting values, or inferred capability
choices are installed. Cutter geometry uses the existing Rust model checks, and
cutting values share `ToolSettings::validate` with jobs. Unset cutting values are
allowed; supplied numbers must be finite and positive. Endmill plunge capability
cannot contradict its geometry. Context-dependent planning checks still run
when planning; saving a preset does not certify a toolpath or cutting conditions.

## Applying a selection

`apply_to_job` returns a new candidate job. The input job and library are unchanged.
It resolves an explicit tool ID and optional preset ID, checks the requested
endmill/V-bit role, and replaces that job slot with copied values. Slot lookup
uses the operation's ID, independent of the tools array order. The job keeps its
own tool IDs and its other settings, including machine tool-number mappings.

Without a preset, **all five cutting values become unset**. With a preset, its
values, including nulls, replace those fields. Neither mode carries over the
previous cutter's cutting settings. Geometry and capability values always come
from the selected tool. Resulting job validation also checks configured stock
depth against usable cutter length/height; failure returns no candidate.

The frontend can compare the original and candidate job to present changed
values, then apply the candidate as one undoable edit. Every actual serialized
change follows the existing plan-fingerprint rules and requires replanning.
Library edits/deletions have no effect on previously saved jobs or plans. A saved
job remains usable after the library is removed. Library names, IDs, revisions,
and material labels are not added as hidden fields to the strict job schema.

## Rust API

`cam_core::tool_library` owns serializable records, `LibraryChange`, validation,
and immutable operations. `ToolLibrary::changed(expected_revision, change)`
returns a validated next revision or an error; it never partly changes the input.
`LibraryTool::from_settings` captures dimensions/capabilities from a job tool.
`CuttingPreset::from_settings` separately captures its cutting values. Both
validate supplied data. Core code still has no filesystem or process access.

`cam_storage::tool_library::ToolLibraryStore` owns filesystem persistence shared
by the CLI and server. `cam_app::tool_library` re-exports the same API for
compatibility. Its public
operations are:

| Method | Behavior |
| --- | --- |
| `new(directory)` | Select an explicit local data directory. |
| `initialize()` | Create an empty revision 0; reject existing data, including corrupt data. |
| `load()` | Read the full validated library; callers can use `tool(id)` / `preset(id)`. |
| `change(expected_revision, change)` | Commit one validated transaction with conflict detection. |
| `import_json(expected_revision, json)` | Merge new tool IDs in one transaction; reject any collision. |
| `export_json()` | Return validated schema-versioned portable JSON. |
| `apply_to_job(expected_revision, job, slot, tool_id, preset_id)` | Resolve a selection against the reviewed revision and return a candidate job. |

The local `cam-web` service calls this API directly, configures its application
data directory, and returns `StoreError` codes/messages through its existing
session-protected loopback boundary. It accepts no browser-provided filesystem
paths and does not invoke the CLI. The core library and store add no network access.

## Persistence and conflicts

The chosen directory contains `library.json` and a stable `library.lock` file.
Writers take an OS-managed exclusive lock, reload the current library, compare
the expected revision, validate the entire change, and write/flush a temporary
file in that directory before replacing `library.json` by rename. Readers see
one complete published revision. Rejected edits/imports preserve existing bytes.

`LIBRARY_BUSY` means another writer holds the lock. `LIBRARY_CONFLICT` means the
expected revision is stale. Reload before retrying or presenting changes again;
do not silently overwrite with a new revision. The same expected-revision check
applies to tool selection so a caller cannot unknowingly apply an edited preset.
Imported revision numbers are ignored for commit numbering: successful imports
increment the destination revision exactly once.

The lock file stays in place; the OS releases the lock when its handle closes.
Use the API for edits and keep the directory on a local filesystem. These checks
coordinate participating clients; manually editing/replacing library files
outside the API is not a supported concurrent-edit protocol. This is not a
network-shared tool library. Flushed replacement protects against partial writes;
full power-loss recovery and filesystem-specific durability are not certified.
An interrupted pre-publication write can leave an unused `library-*.tmp` file;
it is never loaded as the library or automatically promoted.

JSON inputs/outputs are bounded at 8,000,000 bytes, with at most 1,000 tools and
100 presets per tool. Revisions stop at 9,007,199,254,740,991 so browser JSON
numbers remain exact. Unknown/duplicate JSON fields, unsupported schemas,
duplicate record IDs, invalid dimensions and nonfinite/nonpositive supplied
cutting values are rejected. Missing/corrupt data is never replaced by defaults.

## CLI

Run from `flat-v-carve/` after `cargo build --workspace --locked`:

```powershell
.\target\debug\cam.exe tool-library init artifacts/my-tools
.\target\debug\cam.exe tool-library list artifacts/my-tools
```

Capture a tool already configured in your own job. Including `--preset` explicitly
saves cutting values as well as dimensions/capabilities. Material/machine labels
are optional and require a preset:

```powershell
.\target\debug\cam.exe tool-library capture artifacts/my-tools --expected-revision 0 --job my-job.json --slot endmill --tool mill-1 --name "My endmill" --preset setup-1 --preset-name "My recorded setup"
.\target\debug\cam.exe tool-library apply artifacts/my-tools --expected-revision 1 --job another-job.json --slot endmill --tool mill-1 --preset setup-1 --output configured-job.json
.\target\debug\cam.exe tool-library export artifacts/my-tools --output tools-backup.json
.\target\debug\cam.exe tool-library init artifacts/second-library
.\target\debug\cam.exe tool-library import artifacts/second-library --expected-revision 0 --input tools-backup.json
```

The `capture` command also accepts `--slot vbit`. Omit `--preset`/`--preset-name`
to save dimensions/capabilities only. There is no automatic selection of the
first preset when applying a tool. Export and apply require a new output file;
they never overwrite existing files, including inputs or the live library.

Use `change` with a strict JSON operation file for direct record management:

```json
{
  "kind": "duplicate_tool",
  "tool_id": "mill-1",
  "new_id": "mill-2",
  "name": "Second endmill"
}
```

```powershell
.\target\debug\cam.exe tool-library change artifacts/my-tools --expected-revision 1 --input change.json
```

| `kind` | Additional fields |
| --- | --- |
| `add_tool`, `replace_tool` | `tool`: complete `LibraryTool` record; replace requires an existing ID. |
| `remove_tool` | `tool_id`; removes the tool and its nested presets. |
| `duplicate_tool` | `tool_id`, `new_id`, `name`; copies nested presets under the new tool. |
| `add_preset`, `replace_preset` | `tool_id`, `preset`: complete `CuttingPreset` record. |
| `remove_preset` | `tool_id`, `preset_id`. |
| `duplicate_preset` | `tool_id`, `preset_id`, `new_id`, `name`. |
| `import` | `library`: complete `ToolLibrary` snapshot; same merge semantics as the import command. |

Replacements are complete records, not JSON patches. Duplicates require unused
IDs; imports reject collisions even when the colliding records are identical.
An import either commits all tools or commits none.

`init`, `list`, `change`, `capture`, and `import` write the complete library JSON
to stdout. `export` and `apply` write their requested file. Success exits 0;
argument, validation, conflict, or I/O errors exit 2 with a diagnostic on stderr.
See `cam tool-library --help` for all flags.

## Verification

The core tests cover strict schemas, numeric/cutter validation, bounded records,
CRUD/import rollback, snapshot independence, role/ID preservation, null handling,
depth rejection, and stale-plan fingerprint rejection. Application tests cover
reopening files, rejected-write preservation, corrupt input, lock release,
simultaneous writers, and CLI capture/import/export/apply plus Rust job validation.

```powershell
cargo test --workspace --locked --offline -j 1
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
```
