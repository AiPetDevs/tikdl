# Плейбук: сборка и выпуск

1. Все проверки проекта и `node scripts/passport.mjs check` — зелёные.
2. Команды сборки — `passport/passport.json → run`.
3. Версия — во всех местах, где она записана, включая `passport/passport.json → version`.

## В этом проекте
4. `npm run check` (ui_check, ESLint, ui_smoke, cargo test) и `npm run passport:check`.
5. Сборка: macOS — `cd src-tauri && cargo tauri build --target universal-apple-darwin`; Android — `cargo tauri android build` (подпись — README).
6. Остальные системы собирает `.github/workflows/build.yml` на push в main; страницу-прокладку публикует `pages.yml`.
7. Версия: `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `passport/passport.json`.
