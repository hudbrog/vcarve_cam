# Grbl 1.1 milling export

Select **Create Grbl 1.1 machine** in the GUI machine library or apply the example profile from the CLI. Use a three-axis mill configured for spindle mode. The profile selects the work coordinate system (G54–G59), clearance above stock, minimum decimal places, spindle spin-up dwell and coolant. Select flood only on a machine equipped for it. Enable counterclockwise spindle support only if the spindle and wiring provide it. [Grbl configuration](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Configuration)

From `flat-v-carve`, these commands export the supplied two-pocket job into a new directory.

```powershell
cargo run --locked -p cam-app -- collection apply-machine fixtures/pocket/two-pockets.job.json --profile fixtures/gui2/grbl-machine.json --name Router --output configured-grbl.json
cargo run --locked -p cam-app -- collection export configured-grbl.json --output grbl-output
```

**Run the files in manifest order on the same stock.** Each numbered `.nc` file covers consecutive stages using one physical tool. The manifest lists the job tool ID, work frame, job datum, clearance in work coordinates and the files that precede it. Before each file fit the specified tool, preserve the work XY origin, establish work Z for that tool against the job datum, and provide a clear vertical path to clearance. The procedure assumes inactive G92 shifts and tool-length offsets. Each file establishes its modes, positions Z before XY and ends at clearance with spindle and coolant off.

The CLI also accepts `--through <operation-id>` for an enabled prefix and `--layout one` for a single-tool scope. The GUI saves all programs, `manifest.json` and `report.json` in one ZIP. A returning tool gets a new file at its position in the sequence.

The writer consumes rapid, linear, XY circular/helical and dwell actions with explicit feed, spindle and coolant requirements. Preparation checks each requested machine capability. An explicit exact-path or bounded-blending request yields a diagnostic for its source stage. Grbl accepts G61 without selecting a distinct planner behavior. [Grbl 1.1h command parser](https://github.com/gnea/grbl/blob/v1.1h.20190825/grbl/gcode.c)

Readback checks the actual blocks, including work coordinates, arc geometry, process state, Grbl numeric precision and the 79-character executable block limit. Qualification of a particular machine also requires checking its settings, tool-change procedure and measured cuts. [Grbl check mode](https://github.com/gnea/grbl/wiki/Grbl-v1.1-Commands#c---check-gcode-mode)
