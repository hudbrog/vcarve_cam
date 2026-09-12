# GUI5 experience update

This update follows the user review of the native GUI5 build.

- In Flat V-carve, each tool assignment now has **From library** tool and
  cutting-profile selectors and **Apply tool & profile**. Tool-only selection
  is also available. These use the saved library revision; application is one
  job Undo transaction and does not save or modify the global library.
- A new SVG job starts with stock XY from the full SVG page, 18 mm thickness,
  5 mm clearance, setup-origin XY and stock-top Z. Tool geometry, cutting values
  and machining selection still require explicit input. Opening existing jobs
  preserves their values.
- **Stock & work zero → Stock XY from SVG page** captures the active artwork's
  page in physical millimeters, transformed by its placement and scale. For
  rotated pages it uses the enclosing axis-aligned rectangle. The source name
  is shown. Capture changes only stock XY and supports Undo; it does not link
  stock dimensions to future artwork changes.
- **Job settings** contains the planning tolerances. They remain portable
  job-level values because they affect generated paths, distinct from machine
  output precision and application display preferences.
- **Machine → Create or choose machine profile → New machine profile** creates
  a named configuration without a preexisting profile. The library also has
  separate **Tools & profiles** and **Machines** tabs. New profiles supply
  general editable settings; actual M6 behavior must be entered and reviewed
  before Save/Apply. T/H mappings remain explicit for the job's used tools.

The added resource tests cover physical page units and placement, preservation
of stock thickness/work zero/assignments, direct tool-only assignment semantics,
and completing, saving and applying a new machine configuration. Browser tours
exercise the operation selectors, profile creation, page capture/Undo, job
settings, real generation/simulation/checked export and portable recovery.

The previous native executable may remain open during this update. In that
case the new build is packaged separately at
`flat-v-carve/artifacts/gui/native-preview/cam-gui.exe` to preserve the active
editing session.

Verification for the initial experience update: 67 GUI tests passed, including native worker
process tests; Clippy passed with warnings denied. The final browser build is
`1432d147b4b4`. The GUI5 and new-job authoring tours passed with no console
errors, with evidence and reviewed screenshots in the local artifact folders
`browser-smoke/2026-09-12T09-21-14.626Z` and
`browser-smoke/2026-09-12T09-21-28.797Z` under `flat-v-carve/artifacts/gui`.
Native preview SHA-256:
`49df18b0ec60bf369eb0f557dbead07faf1d3d16b3ea85ce9c262b681f23f1c8`.

## Settings help and library application follow-up

- New SVG jobs use planner Start XY `(0, 0)`, motion tolerance `0.01 mm`,
  and verification tolerance `0.05 mm`. Existing jobs preserve their values;
  **Use default start XY** and **Use default planning tolerances** apply the
  defaults explicitly, with Undo. Planner Start XY is an approach/order hint
  in setup coordinates, separate from work zero and actual program-start XYZ.
- Circular **?** controls provide hover text and click-to-open explanations
  for numeric inspector fields, tool/profile fields, cutting choices, and
  machine settings. Machine help includes LinuxCNC G64 P/Q, compensation,
  work offsets, M6 return modes and each required M6 guarantee.
- **Tool-change notes / reference** identifies the reviewed manual section,
  macro revision or verified tool-change procedure. Missing M6 guarantees
  appear as individual instructions; core checked-output requirements remain
  enforced. The explanatory command source is the
  [LinuxCNC G-code reference](https://linuxcnc.org/docs/stable/html/gcode/g-code.html).
- Applied-machine fields label controller tool numbers **T** and tool-length
  table entries **H** explicitly. Machine profiles do not invent mappings for
  newly copied physical tools. **Use T numbers for H entries** is an explicit
  shortcut for the active tools when the controller uses matching entries.
  Switching to macro-managed compensation clears H entries and hides H fields.
- Fixed **Apply tool & profile**: it now selects/copies the physical library
  tool before applying its cutting profile, instead of changing only feeds
  on the old assignment. Geometry, plunge/ramp capability, and the library's
  specified spindle direction are copied in one Undo transaction. Stale raw
  geometry fields are cleared. Library cutters now expose rotation direction;
  new milling cutters start at CW. Applying only a cutting profile in the
  library manager retains its existing profile-only behavior.

Validation: 70 GUI tests and 20 core tool/resource tests passed; formatting,
Clippy with warnings denied, and diff whitespace checks passed. The regression
starts with blank cutter geometry/raw fields, verifies copied capabilities,
direction and cutting values, and checks exact Undo. The authoring browser tour
passed generation, simulation, checked output, save/reopen and recovery with
the new defaults and help popup (`browser-smoke/2026-09-12T09-56-10.854Z`).
The final GUI5 browser tour also passed with no console errors
(`browser-smoke/2026-09-12T09-58-40.402Z`), including blank-to-library cutter
application, T/H copy and macro-managed switching, M6/naive-CAM help popups,
library conflict handling, real stock simulation, exact checked G-code bytes,
and portable reopening without the library. Popup screenshots were reviewed.

Final browser build: `440b08f32efe`. The final native package is
`flat-v-carve/artifacts/gui/native-preview/cam-gui.exe`, SHA-256
`31c178b9d801f684e7b030b6128d69c48a987325f59d092c301d5c6e4d7a78c6`.
The standard native path was updated with the functional fixes, then opened
by the user; the last circular-button styling build is packaged separately
because Windows locks the running executable. No running sessions were closed.

## Library composition redesign

The user-approved list-and-editor concept is now implemented in the production
GUI. **Library** has prominent **Tools & profiles / Machines** tabs, a persistent
searchable item list, and an independently scrolling detail editor. The list
shows the selected item, cutter summary or machine work offset, and incomplete
machine/tool status. Creation is a full-width **New** action; the machine ID
entry appears only when creating a profile.

Machine settings are grouped into **General**, **Tool change & compensation**,
**Motion & coolant**, and **Program start & tool mappings**. Tool geometry,
capabilities and cutting profiles use grouped forms with paired fields and
explicit units. Help buttons remain available. Item duplication and geometry
capture/add actions are secondary menus; applied-job overrides are in a
collapsed section. Applying a tool and profile from the fixed footer uses the
same atomic copy command as the operation picker.

The footer keeps **Save changes** and **Use machine / Use tool & profile / Use
tool** visible. The tool target assignment is explicit. Save commits the library;
Use copies the selected saved item into the job. Pending edits disable Use.
Closing the window preserves the edit buffer. Loading occurs automatically;
**Reload saved library**, import/export and revision comparison live under
**Library actions**. Reload and whole-library replacement are disabled while
edits are pending. Import, save and conflict errors are shown within the library;
routine revision and unrelated job-status lines no longer occupy its header.

The browser regression now covers search, closing/reopening with unsaved edits,
the fixed footer, applying the selected tool/profile and undoing it as one
transaction, plus a 900×700 viewport. Existing coverage still exercises library
creation/import/export, profile edits, saved revision conflicts, machine creation,
job mappings, simulation, exact checked G-code and portable reopening without
the library. A focused native test checks that file-import errors remain visible
and leave the library buffer unchanged.

Final verification: 71 GUI tests passed; Clippy with warnings denied,
formatting and diff checks passed. The final browser build `ad9f60f7aa38`
passed the complete GUI5 workflow with no console errors; evidence and
reviewed normal/compact layout screenshots are in
`flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T10-31-00.603Z`.
The standard native package was refreshed at
`flat-v-carve/artifacts/gui/native/cam-gui.exe`, SHA-256
`52a24305538cca1a2b3727b228ed132c3000b546f31c2a647ee4e9460ba4ad85`.

## Export dialog and session recovery

Export now immediately opens a modal with a spinner while the worker prepares
and validates G-code. It keeps the current inspector tab. On success the dialog
shows the filename, size, toolpath checks, accepted machine/tool settings, and
G-code readback result. The fixed **Save as…** action opens the destination picker
only after preparation completes. Technical reports and hashes are expandable.
The old Machine inspector export section has been removed.

Preparation errors appear in the dialog. Cancelled or failed saves keep the
validated bytes available for another **Save as…** attempt; successful saves close
the dialog. Editing shortcuts, dropped files, background preview and playback
requests are suspended while the dialog is open. Export cancellation terminates
the worker and expires its retained plan, so generation is required again.
Revision checks still reject stale results and stale output.

The reported seven-versus-eight array error was confirmed in the native recovery
snapshot's tab scroll positions. Shorter scroll arrays now default added tabs to
zero without changing the job or undo history; invalid scroll values are still
rejected. Recovery-load failures now explain that automatic recovery is paused
while editing and explicit job saves remain available.

Verification: 75 GUI tests passed, including new export failure/stale-result/save
retry and seven-tab recovery regressions. Clippy, formatting and diff checks
passed. Browser build `25c49bb13164` was exercised with real progress, unchanged
navigation, normal/900×700 layouts, a denied destination, and SHA-256 comparison
of the downloaded program against the validated bytes. Screenshots/evidence:
`flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T11-01-04.402Z`.
The native package was rebuilt at `flat-v-carve/artifacts/gui/native/cam-gui.exe`.
