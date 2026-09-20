# Project documentation

These documents describe the current implementation and useful remaining work.
Completed milestone plans, progress logs, review captures and concept artwork
are available in Git history rather than maintained alongside the code.

| Guide | Contents |
| --- | --- |
| [Architecture](architecture.md) | Product boundary, crates, ownership and data flow |
| [Technical design](technical-design.md) | Coordinates, tolerances, cutter/target geometry and verification |
| [Job and operation contracts](job-model.md) | Schema 5, geometry references, ordered operations and invalidation |
| [GUI architecture](gui-architecture.md) | Editing, workers, persistence, viewport and simulation |
| [Tool library](tool-library.md) | Reusable tools/profiles, copied assignments and revision checks |
| [LinuxCNC output](linuxcnc.md) | Machine contracts, checked output, readback and validation limits |
| [UI assets](ui-asset-manifest.md) | Procedural artwork and visual conventions |
| [Backlog](backlog.md) | Unfinished work and explicit optional extensions |

For running, building and testing, use the
[workspace README](../../flat-v-carve/README.md) and
[GUI README](../../flat-v-carve/crates/cam-gui/README.md). Fixture READMEs remain
beside their inputs. Third-party notices and license files remain beside the
applications that use them.

When changing behavior, update its guide and relevant fixture instructions.
Keep the backlog limited to unresolved work, with code pointers and a concrete
completion condition; remove an item when it lands.
