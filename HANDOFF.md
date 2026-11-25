# HANDOFF — TikDL

## State (2026-11-25)
Version 0.1.0, last code commit `51e6b5f`. Built app: `src-tauri/target/release/bundle/macos/TikDL.app` (matches code).
Done in this session: audit (`docs/AUDIT_2026-11-23.md`), P0 and P1 fixes, UI rebuilt by ui-architect (bar like the
owner's TransferBar sample), owner remark R3 (bar moves for links without size), weak points batch (owner asks A5–A10).

## Verified (live, on the .app)
- Free-space check refuses before writing (150 MB disk image vs 400 MB file).
- No double disk use on macOS: export is a copy that APFS clones; internal copy GC'd in 15 s; repeat download works.
- Link has no IP (node id + relay only); download through it works.
- «Что внутри?» on the confirm card: files, sizes, real total, free space; nothing downloaded.
- Bare link (no `?size=&name=`): size learned from sender, progress, file identical (rechecked 2026-11-23 23:43).
- All gates green (2026-11-25): `cargo test` 8/8, `ui_check` 0/0/0, ESLint clean, `ui_smoke` 4/4.
- 2026-11-25: owner's decision — system disk/folders are never shared (no confirmation dialog). `paths::forbidden_share`
  in `Node::share`: system disk, system folders, whole home and its parents, hidden entries in home, ~/Library (except
  iCloud Drive / CloudStorage), AppData, TikDL's folder; compared by inode, so symlinks, letter case and
  /System/Volumes/Data firmlinks do not bypass it. Old share records from such folders → state "forbidden", not served.
  Verified live: CLI refuses 14 paths incl. aliases; app shows «Applications: это системная папка — TikDL её не раздаёт»;
  a forbidden old record shows «запрещено» and «Убрать из списка» removes it. The dmg in `../_RESULTS` predates this.

- 2026-11-25: one-time links (owner asks A20/A21). Design + verification: `docs/ONCE_LINKS.md`; explanation for the
  owner: `docs/answers/once-links.html`; prior art: `docs/research/ONCE_PRIOR_ART.md`. Throwaway endpoint per link
  (`src-tauri/src/once.rs`), claim binds the first device, done ack burns it. Live: 1.5 GB, A SIGKILLed at 133 MB, B
  refused, sender crashed + restarted, A resumed from 133 MB, byte-identical, burned; release window followed stages
  (`docs/plans/evidence/once_*`). `cargo test` 14/14.
- 2026-11-24: menu bar icon, close → keeps sharing, «Открыть»/«Выйти» (W7); drag file+folder from Finder (W8);
  bar on a bare link (R3). Notification is sent (in macOS notification DB) — see below.

## NOT verified
- Notification banner seen by eye: System Settings → Notifications → TikDL «Allow notifications» is OFF on this Mac.
  Owner's choice to turn it on. (A real user gets the macOS prompt on first notification — not checked.)
- Dock-icon click reopening the window.
- https link via GitHub Pages (needs repo on GitHub + Settings → Pages → Source = GitHub Actions).
- Windows/Linux at all; CI never run; clippy never run locally.
- One-time links across two real networks (checked on one Mac through the relay only); picking the file for a one-time
  share through the window's file dialog (the dialog runs out of process, automation could not type into it — the
  switch, rows and live stages were checked with a share created by the CLI in a test profile).

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
- Live tests with background CLI jobs: `kill -INT` does nothing (background jobs ignore SIGINT) — use `kill -9` for a
  "pause", and wait until the old process is gone before restarting on the same profile (store lock).
- Launch the app with `open` (not the binary directly), otherwise its window may land on another Space and ignore clicks.
- The CLI and the app share one data folder: do not run both on the same profile; use `HOME=<dir>` for the CLI.
- Landing page navigates to `tikdl://`; in Playwright use `waitUntil: 'commit'`.

## Android (2026-11-25)
- Code split: `src-tauri/src/lib.rs` (app, `run()` = mobile entry) + `main.rs` (desktop: CLI, then `run()`).
  Tray, single-instance, CLI, hide-on-close are `#[cfg(desktop)]`. Android project: `src-tauri/gen/android`.
- `src-tauri/src/android.rs` (JNI): picked `content://` file → copied to app-private `imports/` and shared;
  `ShareService.kt` foreground service runs exactly while shares exist (notification «TikDL раздаёт файлы»).
  `MainActivity.kt`: Back = move to background (finishing the activity made Tauri exit the process).
- Data dir = Tauri `app_data_dir()/tikdl`; downloads go to `/storage/emulated/0/Download`.
- UI: `data-platform="android"` on <html>; `DesktopOnly()` hides folder pickers and Finder; status-bar padding.
- Build: `JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home" ANDROID_HOME=~/Library/Android/sdk
  NDK_HOME=~/Library/Android/sdk/ndk/28.2.13676358 cargo tauri android build --apk --target aarch64 --debug`.
- Verified on emulator `Medium_Phone_API_37.0`: tikdl:// link opens confirm (`evidence/android_link_confirm.png`),
  Mac→phone 30 MB sha1 match; phone→Mac via system picker 12 MB sha1 match (`evidence/android_sharing.png`),
  also after Back (app in background); history restored after restart.
- Not done: release APK (signing key, size; debug is 454 MB), folder sharing (Android picker gives files only),
  imported copies use extra space and are not deleted on stop, https links open the landing page (scheme then
  opens app), notification permission prompt on Android 13+ not asked explicitly, not tested on a real phone.
