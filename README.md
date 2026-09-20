# V-carve CAM

A Rust CAM application for ordered Face, Flat V-carve, Profile, Drag knife and
Drill operations. Flat V-carve combines endmill clearing and V-bit finishing
toward sloped walls, flat floors in broad regions and shallower narrow details.

The shared [cam-gui](flat-v-carve/crates/cam-gui/README.md) application runs as a
native window or in a browser with a WebAssembly engine. From `flat-v-carve`:

```powershell
cargo run -p cam-gui --release --locked
# Build distributable artifacts:
./scripts/build-gui.ps1
./scripts/build-gui.ps1 -Target web
./scripts/build-portable.ps1
```

The portable `cam.exe` supplies the CLI and static browser hosting; the native
GUI ships as `cam-gui.exe`. CI builds and tests the workspace. See the
[workspace README](flat-v-carve/README.md) for installation, CLI examples,
build/test commands and [Windows setup](flat-v-carve/README.md#windows-setup).

Jobs embed SVG artwork and their tool/machine settings in one schema-5 document.
Planning records ordered motions and stock history. Export checks the retained
plan and emitted numeric program under an explicit LinuxCNC machine contract;
detailed stock-quality analysis and display simulation have separate guarantees.
Actual controller qualification and a measured machining trial remain open.

Start with the [documentation index](docs/flat-v-carve/README.md):

- [Architecture](docs/flat-v-carve/architecture.md) and [technical design](docs/flat-v-carve/technical-design.md)
- [Job and operation contracts](docs/flat-v-carve/job-model.md) and [GUI architecture](docs/flat-v-carve/gui-architecture.md)
- [Tool library](docs/flat-v-carve/tool-library.md) and [LinuxCNC output](docs/flat-v-carve/linuxcnc.md)
- [Remaining work](docs/flat-v-carve/backlog.md)

Completed plans and review logs are preserved in Git history. Fixture
documentation stays beside its inputs in `flat-v-carve/fixtures/`.
