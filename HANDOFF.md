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

- 2026-11-24: menu bar icon, close → keeps sharing, «Открыть»/«Выйти» (W7); drag file+folder from Finder (W8);
  bar on a bare link (R3). Notification is sent (in macOS notification DB) — see below.

## NOT verified
- Notification banner seen by eye: System Settings → Notifications → TikDL «Allow notifications» is OFF on this Mac.
  Owner's choice to turn it on. (A real user gets the macOS prompt on first notification — not checked.)
- Dock-icon click reopening the window.
- https link via GitHub Pages (needs repo on GitHub + Settings → Pages → Source = GitHub Actions).
- Windows/Linux at all; CI never run; clippy never run locally.

## Next step (exact)
dmg built 2026-11-24: `../_RESULTS/TikDL_0.1.0_universal.dmg` (universal, ad-hoc signed). Next: owner installs it; for wider sharing — Apple Developer
certificate (signing + notarization). Build: `cd src-tauri && cargo tauri build --target universal-apple-darwin`.

## Open owner asks
None from this session (A5–A10, A13, A14 closed).

## Traps
- Do NOT export downloads with `ExportMode::TryReference`: the store keeps an `External` entry pointing at the moved
  file; it cannot be deleted (delete/GC are crate-private) and the next download of the same data fails with
  `Error::Io`. Use the default Copy (reflink on APFS). Details: `docs/IDEAS_LEDGER.md`.
- `ui/site/tokens.css` is a local copy only for previewing the landing page; delete it after, or `ui_check` reports
  duplicate blocks (it is in `.gitignore`).
- Launch the app with `open` (not the binary directly), otherwise its window may land on another Space and ignore clicks.
- The CLI and the app share one data folder: do not run both on the same profile; use `HOME=<dir>` for the CLI.
- Landing page navigates to `tikdl://`; in Playwright use `waitUntil: 'commit'`.
