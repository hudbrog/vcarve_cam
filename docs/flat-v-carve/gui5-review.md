# GUI5 review recipe

GUI5 implementation and automated verification are complete; consult
[the progress report](gui5-progress.md) for evidence and build IDs. This
recipe is for final user and machine-specific review.
GUI5 lives in the production `flat-v-carve/crates/cam-gui` crate.

## Launch and portable starting point

Run `flat-v-carve/artifacts/gui/native/cam-gui.exe`, or from `flat-v-carve` run
`node crates/cam-gui/web/serve.mjs` and open
`http://127.0.0.1:5182/web/index.html` in Chrome.
GUI4 review binaries are preserved in `flat-v-carve/artifacts/gui4`.

Open `flat-v-carve/fixtures/gui4/lettering.job.json`. Its geometry, cutting
values and applied machine settings are already portable. In **Tool library**,
load the local library, then **Import library** using
`flat-v-carve/fixtures/gui5/library.json`. Import only changes the library edit
buffer. Inspect it, then choose **Save library** explicitly.

## Profile application and job Undo

Select **Roughing assignment**, the library Endmill and Lettering rough profile,
then **Apply cutting profile**. Close the library and inspect Cutting: the
roughing profile is Applied. Change Roughing feed to 999; it becomes Modified.
**Reset roughing overrides** restores the copied baseline, including unset
values if the source profile was partial. Reset requires no library access.

Select **Finishing assignment** and apply the V-bit's Lettering finish profile.
The roughing copy stays unchanged. Choose Combined and Generate, inspect both
endmill and V-bit stages, and prepare/save checked output.

In the library, edit one profile and Save library. The existing job does not
change. **Reapply reviewed profile** copies the loaded saved revision to the
selected operation/role. Job Undo restores the previous copied values while the
global library stays at its committed revision. Applied/Modified compares the
applicable cutting values against the stored baseline; Custom has no baseline.

## Library editing, persistence and conflicts

Create an endmill or V-bit and enter its geometry explicitly; new cutting
profiles start with unset cutting fields. Edit or duplicate a tool/profile;
duplicate IDs are distinct. **Capture job geometry** copies geometry and
capabilities only. **Capture assignment as profile** separately captures that
role's cutting values. No assignment application occurs on library Save.

Save performs a conditional revision check. In two browser tabs (same origin),
load the same revision, save a change in the first and attempt Save in the
second. The second save must report a conflict and retain its edit buffer.
**Compare stored revision** preserves those edits while loading the other
revision. Choose **Reload stored library** to discard the buffer, or
**Overwrite reviewed revision** to save the explicitly reviewed replacement.
Another writer between Compare and Overwrite must still cause a conflict.

Import/export transfers one catalog file containing library tools/profiles and
machine configurations. Unavailable resources do not invalidate valid copies
already embedded in a job. Closing the library window keeps its unsaved edit
buffer in the running application; Save or Export before restarting.

Native resource storage uses the user's application data directory under
`flat-v-carve/resources`. Browser resources use a separate IndexedDB record.
Library storage and its revision are separate from job recovery and job Undo.
Read, quota, denied-write and revision errors preserve the edit buffer.

## Physical job tools

**Add geometry to job** copies a library tool without assigning it. An unchanged
snapshot with the same provenance may be reused; matching IDs from another
library, matching diameters and changed geometry cannot overwrite another
physical tool. References to missing tools also reserve their IDs.

Open **Job tools**. The usage rows project actual operation/role assignments;
unused tools remain visible. Select a copied tool and **Use tool in assignment**.
Switching physical tools clears that role's cutting fields and applied baseline,
leaving its sibling assignment unchanged. Apply a profile or enter new values.

**Edit copied geometry** displays the selected job tool's dimensions and
capabilities. Review the affected assignment rows and **Apply copied geometry**.
Shared assignments keep their cutting values for revalidation. A changed job
revision prevents applying an older geometry edit buffer. Used-geometry changes
require Generate; unused-tool changes can reuse an unchanged retained carving.

## Machine configuration and output

Import a schema-2 configuration into the library or capture the current applied
machine. Edit clearance, work offset, precision, compensation, coolant, path
control, startup position and M6 contract as appropriate. Save the library,
then explicitly **Apply reviewed machine** to copy it into the job's single
applied machine object. A conflicting setup clearance is reported without
silently changing the setup.

Machine also exposes job-local work offset, compensation, coolant, path control,
spinup, precision and the active endmill/V-bit T/H mappings. Setup continues to
own work zero and stock. Unused tools need no mappings for the executable scope;
conflicting used T numbers or missing required H values block checked output.
Mapping changes do not change stable job tool IDs, geometry or cutting values.

Generate, simulate both stages, prepare and save the exact checked bytes. Save
the portable job, reopen it without any library or profile file, edit/reset an
assignment and repeat generation/preparation. The reopened job has copied
resources but no retained execution/output authority.

## Verification commands and boundaries

From `flat-v-carve`:

```text
cargo test -p cam-gui --release --locked
cargo test -p cam-core --release --locked --test collection_resources --test tool_library
cargo clippy -p cam-gui --all-targets --locked -- -D warnings
./scripts/build-gui.ps1
./scripts/build-gui.ps1 -Target web
node crates/cam-gui/web/smoke.mjs --gui5 --port=9336
node crates/cam-gui/web/smoke.mjs --gui4 --port=9337
node crates/cam-gui/web/smoke.mjs --gui3 --port=9338
node crates/cam-gui/web/smoke.mjs --port=9339
```

One Flat V-carve operation remains the supported production GUI scope; multiple
operations and Face are GUI7; standalone Drag knife is GUI6 in the revised plan.
Catalogs are capped at 8 MB, with core limits of
1,000 tools and 100 profiles per tool and a GUI limit of 100 configurations.
Job/motion limits remain unchanged. Browser automation and native worker tests
do not constitute physical machining or native-window user acceptance.
