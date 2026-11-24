# Acceptance — P0 fixes (2026-11-23)

Owner scenario. "Done" only after each line was run on the live artifact after the last edit.

| # | Where / on what | Action | Expected |
|---|-----------------|--------|----------|
| 1 | UI (browser pane, mocked Tauri API) | `open-link` event arrives | Link goes to the field, a confirm card shows name, size, sender id; nothing is downloaded until "Скачать" |
| 2 | UI | "Отмена" on the confirm card | Card hides, no `start_download` call |
| 3 | UI | Paste link, press "Скачать" | `start_download` called once (explicit user action, no extra confirm) |
| 4 | `paths.rs` tests (`rustc --test`) | Names `../x`, `a/../b`, `/etc/x`, `C:x`, `a\b`, `f:ads`, `CON`, `nul.txt`, `x.`, empty | All rejected |
| 5 | `paths.rs` tests | Export target exists (file, folder, broken symlink) | New name `name (1).ext` / `folder (1)`; existing untouched |
| 6 | `paths.rs` tests | Folder walk: `.git/…`, `.env`, `.DS_Store`, `Thumbs.db`, `desktop.ini` | Skipped and counted; normal files kept |
| 7 | `paths.rs` tests (unix) | Write key; existing key with 0644 | File mode 0600 after write and after load |
| 8 | Code review of `core.rs` | Who serves what | Router serves only the `store/` (shares); downloads go to `downloads/` store, never served |
| 9 | `tauri.conf.json` + UI | CSP set, no inline script | UI still works under the CSP in the browser pane |
| 10 | `cargo check`, `cargo test` | — | PASSED 2026-11-23: compiles with no warnings, 5/5 tests |
| 11 | Live CLI, two profiles (HOME=A serves, HOME=B gets) | Folder with a.txt, 5 MB bin, .env, .git, .DS_Store; get twice into a dest with keep.txt | PASSED: 2 files shared, 3 hidden skipped; bytes identical; second get → `Проект (1)`; keep.txt untouched; receiver key 0600; 5 MB in `downloads/`, `store/` empty (72 KB) |
| — | GUI app (Tauri window, deep link) | — | NOT run: only the UI page with a mocked API was checked |

# Acceptance — P1 fixes (2026-11-23)

| # | Where / on what | Action | Expected |
|---|-----------------|--------|----------|
| P1-1 | Live CLI: A serves, B gets | Normal folder share | Download works (allowlist lets active share through) |
| P1-2 | Live CLI: B tries a hash A has but did not share (e.g. a file A only downloaded, or a stopped share) | `get` with a ticket for that hash | Refused (permission), nothing written |
| P1-3 | Live CLI: A serves file, then source file is modified, A restarts | `get` | Share marked changed and not served; clear message in `serve` output |
| P1-4 | Live CLI: B gets with `TIKDL_GC_SECS=5` | After success wait > GC interval | `downloads/` shrinks back (internal copy freed); exported file intact |
| P1-5 | Code + live CLI: download interrupted (kill B mid-transfer), then `get` again | Resume | Second run starts from already-downloaded bytes (progress > 0 immediately) |
| P1-6 | Export | Any download | No `.tikdl-part` leftovers in dest after success |
| P1-7 | UI (mocked API) | Restart page with saved downloads/shares | Lists restored; "Остановить" on share calls `stop_share`; pause/resume buttons consistent |
| P1-8 | `cargo check` + `cargo clippy -D warnings` + `cargo test` | — | Pass |
| P1-9 | CI YAML | Parse | Valid YAML; no `macos-13`; `permissions: contents: write`; check job with fmt/clippy/test |
| — | GUI single-instance + deep link in the real app | — | NOT verifiable here headlessly (needs bundled .app and a click on a tikdl:// link) |

## P1 results (2026-11-23, last build after the last edit)

| # | Result |
|---|--------|
| P1-1 | PASSED: folder share (2 files, 2 hidden skipped) downloaded byte-identical |
| P1-2 | PASSED: after `tikdl stop <id>` a fresh profile gets "не удалось скачать: раздача остановлена…", nothing written |
| P1-3 | PASSED: source edited → `serve` shows "НЕ раздаётся: исходные файлы изменились", download refused |
| P1-4 | PASSED: receiver `downloads/` 1 181 008 KB → 56 KB after GC; exported 1.2 GB file intact. Found and fixed on the way: CLI exited without closing the stores → last DB batch lost → orphan files GC never sees. Now: `Node::shutdown`, `sync_db` after download/share, orphan sweep at start (3016 KB → 48 KB on an affected profile) |
| P1-5 | PASSED: killed at 45 088 854 bytes, second run starts at 45 088 854, result identical |
| P1-6 | PASSED: no `.tikdl-part` leftovers after kill or success; existing `Док/` untouched, new copy `Док (1)` |
| P1-7 | PASSED (mocked API, under CSP): restored paused/done downloads, startup link waits for confirm, resume/pause/remove, shares list with stop and "changed" state; no console errors |
| P1-8 | PARTIAL: `cargo check`, `cargo test` (6/6) pass with no warnings; clippy NOT run locally (static.rust-lang.org unreachable) — runs in CI |
| P1-9 | PASSED: YAML parses; jobs check + build; `permissions: contents: write`; macOS universal build |
| — | NOT verified: real GUI window, single-instance with a real tikdl:// click, CI run on GitHub |

## GUI (.app) results (2026-11-23, release bundle after the last edit)

Real app `src-tauri/target/release/bundle/macos/TikDL.app`, driven in the background; sender = CLI in a separate profile.

| Check | Result |
|-------|--------|
| Window opens, both tabs render | PASSED |
| `open tikdl://…` while app runs (= click in a browser) | PASSED: still 1 process (single-instance), confirm card in the same window, nothing written before "Скачать" |
| Confirm → download | PASSED: 30 MB folder, byte-identical, sender's hidden `.secret` not included |
| Pause → quit app → relaunch | PASSED: card restored "На паузе — скачано 76.2 МБ"; "Продолжить" finished 500 MB from there, byte-identical, no `.tikdl-part` |
| Relaunch → share served with zero clicks | FAILED first: old link dead for ~20 s after restart (new random port; found only via relay/DNS). FIXED: port saved in `port` and reused → old link works 0 s after restart (CLI and app) |
| Share tab after relaunch | PASSED: share restored "Раздаётся" with link; "Остановить" → fresh receiver refused, `shares.json` empty |
| Empty state after stopping last share | FAILED first (text missing) → FIXED (`refreshShares` after stop), re-checked in rebuilt app |
| "Убрать" on downloads | PASSED: list empty, `downloads.json` = [] |
| Share via "Файл…/Папку…" dialog | NOT checked: system Open panel can't be driven in background (window on another Space); same `Node::share` code is covered by CLI tests |

# Acceptance — UI rebuild by ui-architect (2026-11-23)

Owner remarks R1, R2 (`docs/UI_REMARKS.md`). "Done" only after each line was run on the rebuilt .app after the last edit.

| # | Where | Action | Expected |
|---|-------|--------|----------|
| U1 | .app window, screenshots `shots/` at two widths | Look | No "TikDL" inside the window (title bar only); nothing said twice |
| U2 | .app, Скачать | Start a download (link from CLI sender) | One row: name + state tag; on the next line a capsule with "N из M · скорость" inside and ⏸ ✕ on the same line; no text buttons |
| U3 | .app | ⏸ then ▶ | Tag "на паузе", icon turns ▶; ▶ resumes from the same bytes |
| U4 | .app | ✕ once, then again | First press arms (red, hint), nothing removed; second press removes the row |
| U5 | .app | Download finishes | Green full-width capsule, tag "готово", no ⏸/✕; reveal-in-Finder icon on the row |
| U6 | .app, Раздать | Existing share | Row: source + tag "раздаётся"; capsule-like link field with copy and stop icons on the same line; stop arms first |
| U7 | `node scripts/ui_check.mjs`, `node scripts/ui_smoke.mjs` | Run | Both green; showcase `#/kit` shows every state |

## UI rebuild results (2026-11-23, rebuilt .app after the last edit; sender = CLI in a separate profile)

| # | Result |
|---|--------|
| U1 | PASSED: no "TikDL" inside the window, tabs on top. Found on the way (same class R1) and FIXED: incoming link was shown twice (confirm card + input field) and stayed in the field; share row showed a truncated full path instead of its name |
| U2 | PASSED: row "ui-test.bin" + chip "подключение"; capsule "31.8 МБ из 400.0 МБ · 63.5 МБ/с" with ⏸ ✕ on the same line |
| U3 | PASSED: ⏸ → chip "на паузе", ▶ icon, "253.2 МБ из 400.0 МБ"; ▶ resumed from 253.2 МБ; file byte-identical to the source |
| U4 | PASSED: first ✕ turned red, download kept going (200 МБ); second ✕ removed the row, no leftovers in Downloads |
| U5 | PASSED: green full capsule "400.0 МБ", chip "готово", reveal + remove-from-list icons; restored after app restart |
| U6 | PASSED: share row "Фото проба" + "раздаётся"; link capsule "… 1 файл · 2.9 МБ"; copy → check icon, clipboard holds the link; stop: first press arms, second stops, empty state shown, `shares.json` = [] |
| U7 | PASSED: `ui_check` 0 violations / 0 debt / 0 exceptions; ESLint structure rules clean; `ui_smoke` 4/4 (screen + `#/kit`, 1440 and 900 px), screenshots looked at |
| — | NOT checked: share via the system Open panel (cannot be driven in the background); light theme only in the headless browser, not in the .app |

## Bare ticket without size (owner remark R3, 2026-11-23, rebuilt .app)

| # | Action | Result |
|---|--------|--------|
| U8 | Open a bare `tikdl://get/<ticket>` (no `?size=&name=`) from a CLI sender, 600 МБ | PASSED: row shows the real name «ui-test2.bin» and «145.7 МБ из 600.0 МБ · 48.5 МБ/с», the stripe grows; finished green, file byte-identical |
| U9 | Incoming link while the confirm card is open; Return pressed in the window | PASSED: nothing starts; focus is on «Отмена». Before the fix a link started downloading ~0.5 s after arriving (focus was on «Скачать») |
| — | NOT checked: a sender that is offline at start (probe times out after 15 s → size stays unknown, «передано N»), and a bare ticket from sendme |

## 2026-11-23 23:16 · Ответ «какие слабые места остались» — страница
- written: 2026-11-23T23:16:16-05:00
- where: файл tikdl/docs/answers/weak-points.html в браузере
- data: текущий код и аудит TikDL на 23.11.2026
- expect: страница открывается без интернета, слабые места разложены по степени вреда, есть порядок работ
- run: 2026-11-23T23:16:16-05:00 · result: ok · evidence: <project>/docs/plans/evidence/weak_points_page.png (mtime 2026-11-23T23:16:16-05:00) · note: страница отрисована в Chromium, снимок во всю высоту
