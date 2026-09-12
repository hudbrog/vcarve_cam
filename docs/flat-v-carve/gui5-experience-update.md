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
