# Single executable build and investigation

Implementation added after merging `origin/codex/web-integration` into `main`
(merge `ddb370f`, containing UI commit `c0ab4ee`). The merged branch supplies the
live service, planning/cancellation, verification, export, and tool library.

From `flat-v-carve`, run `./scripts/build-portable.ps1` (`-Offline` when dependencies
are cached). Its output is `artifacts/portable/cam.exe`. Run `cam.exe serve --open`
for the browser workspace or use any existing CLI command. `--port 0` selects
an available loopback port; the default remains 4848. `--ui-dir` is an optional
development override, and `--library-dir` selects the persistent tool library.

`cam-app` now links `cam-server`; both use the new `cam-storage` crate for tool
library persistence. The old `cam_app::tool_library` API remains a re-export.
Compute workers run the same EXE with private IPC. Assets are embedded behind
the `bundled-ui` Cargo feature and served from static memory. A generated manifest
binds all frontend sources and output assets to their SHA-256 hashes and engine
version; native builds reject stale, missing, modified, or extra files. Ordinary
Cargo builds can still serve a development `web/dist` directory.

Validation commands:

```powershell
# From flat-v-carve, after building the UI:
cargo test --workspace --release --locked --features cam-app/bundled-ui
cargo clippy --workspace --all-targets --locked --features cam-app/bundled-ui -- -D warnings
./scripts/build-portable.ps1 -Offline
$env:CAM_TEST_EXE = (Resolve-Path artifacts/portable/cam.exe).Path
Push-Location web
pnpm test
pnpm check:contracts
pnpm check:live
Pop-Location
```

The rebuilt portable artifact is 7,040,512 bytes (SHA-256
`0dbe66d054c783ed626e3d79d42dc9f0d1f593eaf85820b38af18a120cc3c01f`). Its direct imports are Windows
system DLLs only (`bcryptprimitives`, `kernel32`, `ws2_32`, `ntdll`, and the
Windows synchronization API set), with no `VCRUNTIME140.dll`. All 209 Rust
release tests, 92 frontend tests, and 26 live integration checks passed. Live
checks used the copied EXE for server and CLI, with no UI directory and only OS
directories on the server's PATH; they include all asset hashes, planning and
cancellation, M5/M6 checks, and tool-library/CLI parity. A browser smoke check
confirmed the embedded UI renders and uses live Rust normalization/validation.
The final artifact may differ in size as concurrent engine development proceeds.
Ctrl+C shutdown was verified in a standalone Windows console with exit code 0.
The automation runner initially passed an inherited ignore-Ctrl+C flag to its
child processes; restoring normal handling in the test launcher before spawning
the EXE resolved the test. Windows documents that inheritance behavior in its
[console signal reference](https://learn.microsoft.com/en-us/windows/console/ctrl-c-and-ctrl-break-signals).
A clean-machine Windows compatibility test and default-browser auto-open check
remain outstanding. Linux/macOS portable artifacts are not verified.

## Original feasibility investigation

Investigated 2026-09-06 against engine 0.7.3 and the current U1 frontend.

One Windows executable can contain the Rust engine, its linked dependencies, a local HTTP service, and the compiled browser assets. Existing CLI commands and a new `serve` command can share that executable. Node.js, pnpm, Rust, and the source checkout would be build requirements only. Browser mode would use the user's installed browser.

The current code supports this direction, but the HTTP service and live frontend integration are still implementation work. The experiment below validates static runtime linking for the existing CLI; it does not claim to have built the combined server/UI executable.

## Evidence from this checkout

| Part | Finding |
| --- | --- |
| Native application | `crates/cam-app/Cargo.toml` already defines one `cam` binary linked to `cam-core`. |
| Command selection | `crates/cam-app/src/main.rs` dispatches CLI commands. There is no `serve` command yet; no arguments currently prints help. |
| Engine reuse | Import, inspection, planning, verification, and export have in-memory Rust entry points. The HTTP service can call these directly. |
| Application services | `crates/cam-app/src/lib.rs` already exposes tool-library persistence. Other orchestration and file handling currently live in CLI modules and need shared service functions. |
| Frontend | `web/package.json` builds React/TypeScript into static files with Vite. `vite.config.ts` uses relative asset paths. |
| Live integration | `web/src/App.tsx` defaults to `fixtureService`. `CamService` is an injectable but incomplete proposed interface; it has no live HTTP implementation or planning task methods. |
| Existing design | The architecture already proposes a native executable with bundled browser assets. The UI integration plan describes a loopback service and asynchronous operations. |

`pnpm build` passed, including TypeScript checking. Its three output files total **399,528 bytes** (about 0.40 MB): HTML, JavaScript, and CSS. React and the other browser dependencies are included in the built JavaScript; `node_modules` need not ship.

The existing `target/release/cam.exe` is **3,504,128 bytes**. Microsoft `dumpbin /DEPENDENTS` shows a dependency on `VCRUNTIME140.dll`, alongside Windows system/runtime DLLs.

A separate release build using the pinned Rust 1.95.0 toolchain, existing lockfile, and `-C target-feature=+crt-static` completed successfully offline. Its executable is **3,635,712 bytes**. Its direct DLL imports are only:

```text
bcryptprimitives.dll
api-ms-win-core-synch-l1-2-0.dll
KERNEL32.dll
ntdll.dll
```

This removes the separate Visual C++ runtime DLL dependency from the inspected binary. Windows system DLLs remain normal OS requirements. Rust documents this build flag and recommends inspecting the resulting executable, as done here. [Rust linkage reference](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes).

The copied executable passed `--help`, all **28 bundled geometry checks**, SVG import, and saved-job inspection from a separate working directory with `PATH` restricted to Windows directories. Import used an explicitly supplied artwork file; the geometry checks used fixtures already embedded in the executable. This was a smoke check on the current Windows machine, not a clean-machine compatibility test.

The test executable and outputs are under `flat-v-carve/artifacts/single-exe-investigation/`; build intermediates are under `flat-v-carve/target/single-exe-investigation/`. Both directories are ignored by Git. No application source or dependency changes were required for the experiment.

The static-runtime CLI plus the current uncompressed UI amounts to roughly **4.04 MB before adding HTTP service code and embedding overhead**. This is a measured baseline, not the final application's size.

## Recommended implementation

Keep one native Rust binary, add Axum/Tokio for HTTP, and embed the production `web/dist` directory with `rust-embed` or a generated `include_bytes!` asset table. Serve the bytes directly from memory with correct content types; no asset extraction is necessary. Rust's standard macro embeds file bytes at compile time, and `rust-embed` supports directory embedding in release builds. Its default debug behavior reads files from disk, so distribution checks must exercise the release binary. [Rust `include_bytes!`](https://doc.rust-lang.org/std/macro.include_bytes.html), [rust-embed behavior](https://docs.rs/rust-embed/latest/rust_embed/trait.RustEmbed.html).

Proposed command behavior, not yet implemented:

```text
cam.exe import artwork.svg --output job.json
cam.exe plan job.json --output plan.json
cam.exe serve
cam.exe serve --open
cam.exe serve --port 8080
```

Existing commands keep their current output and exit-code contracts. `serve` binds to loopback and prints its URL; `--open` also launches the default browser after the listener is ready. An automatically selected available port is a reasonable default, with explicit ports failing clearly if occupied. Keep the console subsystem for normal CLI input, output, and Ctrl+C handling.

The browser gets both assets and `/api/...` from that same executable and origin. The live frontend adapter invokes shared Rust application services, which call `cam-core`; there is no need for a Node server, a WebAssembly conversion, or shell-command wrappers around ordinary engine calls. Axum has a documented example of serving embedded assets through `rust-embed`. [Embedding example](https://docs.rs/crate/rust-embed/latest/source/examples/axum.rs).

Long planning and verification operations need a bounded worker queue so they do not block HTTP responses. The current core has no cooperative cancellation interface. Add cancellation checkpoints if responsive cancellation is required: aborting an HTTP request or a running Tokio blocking task does not stop the computation. If process isolation is later needed, the application can launch its own EXE in an internal worker mode, which still permits one-file distribution. [Tokio blocking-task and cancellation behavior](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).

Apply the existing integration plan's loopback binding, Host/Origin checks, and session authentication. Keep persistent jobs, tool libraries, recovery data, and exports in writable application/user directories, with an optional portable data directory. A single distribution file still creates user data during use.

## Delivery sequence

1. Add a release build script that builds the UI first and then Rust. Generate the asset manifest from that build; reject missing or inconsistent assets. Keep frontend and engine identity together in the resulting executable.
2. Add `serve`, embedded asset handling, capabilities, lifecycle/shutdown behavior, and the local session boundary. Preserve current CLI behavior.
3. Extract shared application operations and implement a validated live `CamService`, starting with import, inspection, draft validation, and tool-library operations. Bundle serving alone will leave the UI in fixture mode.
4. Add planning/verification/export task transport, progress, cancellation, artifact identity, and result/display integration. Preserve the engine's existing verification and export gates.
5. Test the release EXE alone in a fresh directory and on a supported Windows installation without developer tools or a separately installed VC runtime. Check every emitted asset, offline browser loading, live API/CLI parity, shutdown, and persistence. Audit DLL imports again after adding server dependencies.

Build one binary per supported OS and architecture. The Windows EXE is not the Linux or macOS artifact; static linking and clean-machine compatibility need separate verification on those targets. UI-only changes will also require rebuilding and redistributing the executable.

To reproduce the static-runtime CLI build from `flat-v-carve`, in a temporary PowerShell session:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --offline -p cam-app --bin cam --target x86_64-pc-windows-msvc --target-dir target/single-exe-investigation
```

This command uses already cached dependencies. It does not embed the UI or add server mode. For production, scope the flag to the Windows release target in the build configuration/script.
