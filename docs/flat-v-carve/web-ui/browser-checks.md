# Setup editors and browser acceptance

Date: 2026-09-05\
Result: current setup editors and targeted U1 browser checks passed in the Codex embedded Chromium 152 browser on Windows. Rust service integration remains the next implementation step.

## Completed setup editors

- Travel: planning clearance Z and starting workpiece XY.
- Endmill: depth-dependent/deepest-region strategy, direct plunge/ramp entry, explicit ramp angle/feed, and layer/loop/motion limits.
- V-bit: all current planning path/motion/subdivision/depth-pass/cleanup limits, sample spacing/count, reachability cells, and stock preview slices.
- Machine profile: explicit ID, work offset, separate profile clearance, tool numbers, and multiline M6 description.

Optional blocks stay null when unset. Partly filled blocks retain text in recovery and identify the missing fields before portable download. Switching to plunge excludes ramp-only JSON fields and errors while retaining their draft text for switching back. Clearing a block is one undoable edit. Integer resource/tool fields reject fractional or inexact values; supported machining ranges remain Rust-owned. No presets, cutting values, work offset, or capability choice is inferred.

Different planning/profile clearances produce a linked review warning and preserve both values. M6 text remains descriptive and cannot enable machine output. Readable placement now stays visible while unrelated setup blocks are incomplete.

## Browser evidence

| Check | Result |
| --- | --- |
| All six steps at 1440×900, 1024×768, 736×900, 360×800, and 320×740, in light and dark themes | 60 combinations passed: no page/navigator horizontal overflow or controls extending beyond the viewport. |
| All six steps with 200% root text size at desktop, tablet, and phone widths (1440, 736, 360) | 18 combinations passed after making breakpoints relative to text size. |
| Visual inspection | Desktop light setup editor, phone dark machine profile, and enlarged-text layout inspected. |
| System appearance and reduced motion | System light/dark followed emulated OS preference; reduced-motion preference active with zero running animations. Temporary overrides were cleared. |
| Source controls | Inspecting, inclusion, and visibility remained independent; the B fixture retained two holes. Inclusion undo restored the previous choice. |
| Viewport | Fit job/inspected region, keyboard pan, zoom, and Home behaved correctly. |
| Panel resize | Pointer drag and arrow-key resize changed the inspector width. |
| Focus and shortcuts | Skip link focused the inspector; issue links opened collapsed sections and focused their fields. Workspace undo/redo and Save worked; text fields kept native undo. |
| Dialogs | Tab and Shift+Tab stayed inside both dialogs; Escape closed them and restored focus to their opener. |
| Planning inputs | Blank values, explicit zero, partial scientific notation, conditional ramp values, integer rejection, and clear/undo behaved correctly. |
| Recovery | Unfinished ramp text survived reload. An invalid recovery version stayed intact at failed startup; explicit replacement restored the example. The fault used an ignored test-only HTML harness, not production code. |
| File rejection | Unsupported schema produced a visible dialog error and left the current draft intact. |
| Actual download/reopen | Ramp parameters, resource settings, tool mapping, and multiline profile description survived browser download and file-picker reopen. |
| Rust parity | The actual downloaded incomplete job was accepted by the existing Rust 0.6.0 executable with 20 machining fields still unset. No Rust build or source modification was needed. |
| Output gating | Machine-program output remained disabled with planning/profile blocks populated. |
| Browser console | No application errors or warnings observed. |

Two defects were found and fixed during these checks: fixed-pixel breakpoints cramped the navigator/viewport with enlarged text, and the embedded browser could move focus outside the one-control shortcuts dialog. Container queries now account for root text size, the app respects browser font settings, and explicit modal focus containment covers both dialogs. Numeric units are associated with their fields for assistive technology, and dialogs have accessible names.

Local evidence is generated under `flat-v-carve/web/test-results/browser/` (ignored by Git): `checks.json`, `desktop-setup.png`, `phone-profile.png`, and the actual `roundtrip.job.json`. These are development artifacts, not machining presets or verification evidence. The workspace was returned to the bundled example and system theme, with viewport/media/text-size overrides reset.

## Reproducing the focused workflow

1. Run `pnpm dev --port 5175` in `flat-v-carve/web/` and open the bundled example. Keep the test settings synthetic.
2. Enter planning clearance 5 and start XY 0, 0. Check that the remaining block fields are linked as incomplete.
3. Choose depth-dependent clearing and ramp entry. Enter `1e-` as the ramp angle and 80 as ramp feed. Switch to plunge and back; reload and confirm the unfinished angle remains. Replace it with 2 and enter endmill limits 32, 128, and 10000.
4. Enter the V-bit limits/sample controls, check fractional rejection and explicit zero cleanup iterations, then clear and undo the entire block.
5. Create a test profile with G55, profile clearance 6, tools 8/12, and a multiline description. Verify the clearance mismatch and that machine output stays unavailable.
6. Download and reopen the job. Run the existing CLI's `validate-job` on the downloaded file. Open an unsupported schema and confirm rejection preserves the draft.
7. Exercise source inclusion/visibility, keyboard actions, both dialogs, themes, and the viewport matrix above. Verify inner panel overflow as well as overall page overflow.

`pnpm test` runs the 37 unit/contract regressions; `pnpm check:contracts` checks Rust field sets and captured geometry; `pnpm build` checks TypeScript and the static bundle. Browser checks here were performed through browser automation and are not represented as a headless test suite. Other browser engines and full screen-reader qualification remain release acceptance work.
