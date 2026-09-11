# GUI1 third-party inventory

This is an experimental build, not an installation/distribution qualification.
`licenses/dependency-inventory.json` records exact Cargo package versions and
declared license expressions from the locked metadata. It intentionally includes
development and non-host target packages; it is broader than the shipped set.
Local CAM crates have their existing project license status, not a newly assigned
license. A release must select the actual target dependency closure and bundle its
required notices/license texts, honoring the recorded expressions and exceptions.

Principal runtime libraries: egui/eframe/egui-wgpu, wgpu, rfd, wasm-bindgen,
serde/serde_json and bytemuck declare permissive MIT/Apache-2.0 alternatives or
equivalents as recorded in the inventory. Do not treat this summary as replacing
the individual notices, font licenses or target-specific exceptions.

The default-font crate embeds four font families. Their original notices were
copied verbatim from epaint_default_fonts 0.33.3 into `licenses/`:

- Hack: `Hack-Regular.txt` (MIT and inherited Bitstream Vera terms).
- Noto Emoji: `OFL.txt` (SIL Open Font License).
- Ubuntu Light: `UFL.txt` (Ubuntu Font Licence).
- emoji-icon-font: `emoji-icon-font-mit-license.txt`.

No font was modified; preserve notices and applicable reserved-name restrictions
if future work changes them. No separate stock imagery, generated raster, or
third-party icon pack is used. This experiment does not grant a production
framework/distribution decision.
