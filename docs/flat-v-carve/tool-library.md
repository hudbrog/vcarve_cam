# Tool library and applied resources

The GUI owns reusable tool and machine resources and imports/exports portable
JSON. There is no server-side library backend or `cam tool-library` command.
Core operations work on values in memory; platform adapters own persistence.

## Ownership

| Data | Owner |
| --- | --- |
| Cutter geometry and optional capabilities | A library tool; copied into a job tool when applied |
| Cutting values | Milling/drill presets and separate knife presets; copied into a specific operation assignment |
| Modified/reset baseline | The assignment's copied profile values and provenance |
| Machine configuration | Reusable configuration copied into the job's one applied snapshot |
| T/H mapping | Job-tool identity, independent of operation order |

Library tools include endmills, V-bits, passive knives and drills. Optional
shaft diameter/stickout support assembly display. Milling presets carry spindle,
feeds, stepdown and stepover; drills reuse the applicable fields of this preset
type. Knife presets carry swivel feed without spindle settings. Material/machine
labels are descriptive context, not automatic cutting recommendations.

Library schema 1 and its revision are independent of schema-5 jobs. IDs are
stable identifiers, not names or file paths. `cam-core/src/tool_library.rs`
validates IDs, finite dimensions/values, supported geometry and content bounds
(8 MB, 1,000 tools, 100 presets per tool). Missing cutting values are allowed in
a library; planning still requires the operation's necessary settings.

## Apply, reset and reapply

`project/v5/resources.rs` addresses an assignment by operation ID and role
(endmill, V-bit, milling, knife or drill). Applying a preset copies every
applicable field, including unset values, and stores a baseline. Modified is
computed against that copy. Reset restores the copied baseline; Reapply reads
an explicitly supplied current library revision. Neither silently follows later
library edits.

Choosing a different tool without a preset clears its applicable cutting fields
and old baseline; applying the same tool can retain values for revalidation.
Geometry/capabilities live on the job tool, so changes to a shared cutter affect
its users, while cutting assignments remain independent. Changes validate as a
candidate before becoming one document edit. Removing the external library does
not invalidate the job.

Resource drafts have their own revision/conflict checks and persistence, apart
from document Undo and recovery. A failed write must retain the draft and report
the actual save outcome. Core library updates use
`ToolLibrary::changed(expected_revision, change)` and never partially mutate
the input. See `cam-gui/src/resources.rs` for the GUI resource envelope and
`cam-core/tests/collection_resources.rs` for assignment semantics.

## CLI

The collection CLI consumes explicit resource files. From `flat-v-carve`:

```powershell
cargo run --locked -p cam-app -- collection apply-profile fixtures/gui4/lettering.job.json --library fixtures/gui5/library.json --library-id gui5-lettering-library --operation carving --role endmill --tool endmill --preset rough --output detailed.json
cargo run --locked -p cam-app -- collection apply-machine fixtures/gui2/flower.job.json --profile fixtures/gui2/machine.json --name Workbench --output configured.json
```

`--library` accepts the library object or the GUI-exported catalog envelope
(`{schema, id, library}`). Use `cam collection --help` for apply-tool,
reset, reapply, mapping and profile-resolution commands. See the
[job model](job-model.md) and [LinuxCNC guide](linuxcnc.md) for how those snapshots
bind planning and export.
