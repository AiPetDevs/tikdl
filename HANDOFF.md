# HANDOFF — TikDL

## State (2026-11-24)
Version 0.1.0, last code commit `51e6b5f`. Built app: `src-tauri/target/release/bundle/macos/TikDL.app` (matches code).
Done in this session: audit (`docs/AUDIT_2026-11-23.md`), P0 and P1 fixes, UI rebuilt by ui-architect (bar like the
owner's TransferBar sample), owner remark R3 (bar moves for links without size), weak points batch (owner asks A5–A10).

## Verified (live, on the .app)
- Free-space check refuses before writing (150 MB disk image vs 400 MB file).
- No double disk use on macOS: export is a copy that APFS clones; internal copy GC'd in 15 s; repeat download works.
- Link has no IP (node id + relay only); download through it works.
- «Что внутри?» on the confirm card: files, sizes, real total, free space; nothing downloaded.
- Bare link (no `?size=&name=`): size learned from sender, progress, file identical (rechecked 2026-11-23 23:43).
- All gates green: `cargo test` 6/6, `ui_check` 0/0/0, ESLint clean, `ui_smoke` 4/4.

## NOT verified
- Menu bar icon, window close → app stays, sharing while the window is closed, «Открыть»/«Выйти» (W7).
- Drag files onto the window in the real app (W8; only the mock UI was checked).
- Notification «Готово» seen by eye (W9; no error in the log).
- The bar's look after the last core changes (screen was locked; data part checked).
- https link via GitHub Pages (needs repo on GitHub + Settings → Pages → Source = GitHub Actions).
- Windows/Linux at all; CI never run; clippy never run locally.
Reason for W7–W9: the Mac screen was locked; macOS blocks window capture and input then.

## Next step (exact)
With the screen unlocked: run W7, W8, W9 from `docs/plans/ACCEPT.md` on the `.app` (driver: computer-use `app_*` on
`app.tikdl.desktop`; close via menu File → Close Window; drag needs full-screen control from Finder). Record with
`accept.py run`, then close owner asks A5–A8 (`python3 ~/.claude/hooks/owner_asks.py --done ID "…"`, run from the
project root one level up). Then update `docs/answers/weak-points.html`.

## Open owner asks
A5 «Исправь», A6 menu bar, A7 drag, A8 notification — status wip (made, not verified). A9, A10 done.

## Traps
- Do NOT export downloads with `ExportMode::TryReference`: the store keeps an `External` entry pointing at the moved
  file; it cannot be deleted (delete/GC are crate-private) and the next download of the same data fails with
  `Error::Io`. Use the default Copy (reflink on APFS). Details: `docs/IDEAS_LEDGER.md`.
- `ui/site/tokens.css` is a local copy only for previewing the landing page; delete it after, or `ui_check` reports
  duplicate blocks (it is in `.gitignore`).
- Launch the app with `open` (not the binary directly), otherwise its window may land on another Space and ignore clicks.
- The CLI and the app share one data folder: do not run both on the same profile; use `HOME=<dir>` for the CLI.
- Landing page navigates to `tikdl://`; in Playwright use `waitUntil: 'commit'`.
