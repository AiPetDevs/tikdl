# TikDL — agent entry

Read order: this file → `HANDOFF.md` → `docs/plans/ACCEPT.md` (owner scenarios and results). Before reading code — the
passport (below).

## What it is
Desktop app (Tauri 2 + Rust, iroh 1.5 / iroh-blobs 0.105) to share and download files/folders by link, peer-to-peer.
Also a CLI (`tikdl serve|get|shares|stop`). User-facing text is Russian; code and docs in English.

## Layout
- `src-tauri/src/core.rs` — transfer core: node, stores (`store/` = shares, served; `downloads/` = received, never served),
  share, download, peek (contents before download), space check, link format.
- `src-tauri/src/lib.rs` — window, commands, jobs, tray (menu bar), notifications, deep links, CLI; `main.rs` only
  dispatches CLI vs window. `android.rs` + `gen/android/…/ShareService.kt` — Android file import and foreground service.
- `src-tauri/src/once.rs` — one-time links: throwaway endpoint per link, claim protocol, gate over iroh-blobs,
  expiry sweep, change signal (`docs/ONCE_LINKS.md`).
- `src-tauri/src/paths.rs` — safe names, export targets, hidden-file filter, forbidden-share rules
  (`forbidden_share`: system disk/folders, whole home, keys and settings in home, TikDL's own folder; tests here).
- `ui/` — interface by the ui-architect skill: `tokens.css` → `kit/` (registry `docs/UI_COMPONENTS.md`, showcase
  `ui/index.html#/kit`) → `screens/`; data `api.js` → `state.js`. `ui/site/` — https landing page for messenger links.
- `.github/workflows/` — `build.yml` (dmg/exe, sets `TIKDL_LINK_BASE`), `pages.yml` (publishes `ui/site`).

## Passport (map of the app — read before the code)
`passport/` holds the component map, one card per feature (path through the code by symbol), data files, settings and
playbooks; `passport/view.html` is the same for people. Ask the map instead of reading files in a row:
    node scripts/passport.mjs where <symbol|file|component|feature>   # owner, lines, callers, callees, features
    node scripts/passport.mjs impact <symbol>                          # what depends on it; covering tests
Task → playbook: `passport/ops/` (debug, symptoms, tune, isolate, add-feature, release).
Changed a feature path, a component contract, a command/event/route, a data file or a setting → update its card in the
same change; then `node scripts/passport.mjs build && node scripts/passport.mjs check` must be green.
Never edit `passport/generated/` or `passport/view.html`.

## Rules
- The repo is public: no owner data in files, logs, screenshots or commit authors (see HANDOFF «Traps»).
- Interface: ui-architect. `ui-check.json` and `ui-check.baseline.json` change only by the owner.
- "Done" = owner scenario in `docs/plans/ACCEPT.md` run on the rebuilt `.app` after the last edit (`accept.py`).
- Owner remarks on UI → `docs/UI_REMARKS.md`, fixed as a class.
- Git author for this repo is set locally (`git config user.name TikDL`, `user.email tikdl@localhost`) — never commit with a personal identity.

## Checks
```
cd src-tauri && cargo test                     # 14 tests
node scripts/ui_check.mjs && npx eslint ui     # UI gates (npm i once)
npm run passport:check                         # passport matches the code
(cd ui && python3 -m http.server 8765 --bind 127.0.0.1 &) && node scripts/ui_smoke.mjs
cd src-tauri && cargo tauri build              # → target/release/bundle/macos/TikDL.app
```
Live tests: sender = CLI with its own profile (`HOME=<dir> target/release/tikdl serve <file>`), receiver = the app.
App data: `~/Library/Application Support/tikdl/` (`downloads.json`, `shares.json`, `store/`, `downloads/`).
