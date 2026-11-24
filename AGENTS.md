# TikDL — agent entry

Read order: this file → `HANDOFF.md` → `docs/plans/ACCEPT.md` (owner scenarios and results).

## What it is
Desktop app (Tauri 2 + Rust, iroh 1.5 / iroh-blobs 0.105) to share and download files/folders by link, peer-to-peer.
Also a CLI (`tikdl serve|get|shares|stop`). User-facing text is Russian; code and docs in English.

## Layout
- `src-tauri/src/core.rs` — transfer core: node, stores (`store/` = shares, served; `downloads/` = received, never served),
  share, download, peek (contents before download), space check, link format.
- `src-tauri/src/main.rs` — window, commands, tray (menu bar), notifications, deep links, CLI.
- `src-tauri/src/paths.rs` — safe names, export targets, hidden-file filter (tests here).
- `ui/` — interface by the ui-architect skill: `tokens.css` → `kit/` (registry `docs/UI_COMPONENTS.md`, showcase
  `ui/index.html#/kit`) → `screens/`; data `api.js` → `state.js`. `ui/site/` — https landing page for messenger links.
- `.github/workflows/` — `build.yml` (dmg/exe, sets `TIKDL_LINK_BASE`), `pages.yml` (publishes `ui/site`).

## Rules
- Interface: ui-architect. `ui-check.json` and `ui-check.baseline.json` change only by the owner.
- "Done" = owner scenario in `docs/plans/ACCEPT.md` run on the rebuilt `.app` after the last edit (`accept.py`).
- Owner remarks on UI → `docs/UI_REMARKS.md`, fixed as a class.
- Git author is not configured on this Mac: commit with `git -c user.name=TikDL -c user.email=tikdl@localhost commit`.

## Checks
```
cd src-tauri && cargo test                     # 6 tests
node scripts/ui_check.mjs && npx eslint ui     # UI gates (npm i once)
(cd ui && python3 -m http.server 8765 --bind 127.0.0.1 &) && node scripts/ui_smoke.mjs
cd src-tauri && cargo tauri build              # → target/release/bundle/macos/TikDL.app
```
Live tests: sender = CLI with its own profile (`HOME=<dir> target/release/tikdl serve <file>`), receiver = the app.
App data: `~/Library/Application Support/tikdl/` (`downloads.json`, `shares.json`, `store/`, `downloads/`).
