#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod paths;

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{watch, Mutex, OnceCell};

const DOWNLOADS_FILE: &str = "downloads.json";

/// Загрузка в реестре (downloads.json): переживает перезапуск приложения (аудит P1-8).
#[derive(Serialize, Deserialize, Clone, Debug)]
struct DlRecord {
    id: String,
    link: String,
    dest: String,
    name: Option<String>,
    total: Option<u64>,
    bytes: u64,
    /// running | paused | done | error
    state: String,
    message: Option<String>,
}

struct Job {
    cancel: watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct AppState {
    node: OnceCell<Arc<core::Node>>,
    jobs: Mutex<HashMap<String, Job>>,
    downloads: Mutex<Vec<DlRecord>>,
    /// Ссылки, пришедшие извне, пока страница их не забрала (вместо sleep — аудит P1-14).
    pending_links: std::sync::Mutex<Vec<String>>,
}

impl AppState {
    async fn node(&self) -> Result<Arc<core::Node>, String> {
        self.node
            .get_or_try_init(|| async {
                core::Node::start(&core::data_dir())
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"))
            })
            .await
            .cloned()
    }

    async fn update(&self, id: &str, f: impl FnOnce(&mut DlRecord)) {
        let mut list = self.downloads.lock().await;
        if let Some(r) = list.iter_mut().find(|r| r.id == id) {
            f(r);
        }
        save_downloads(&list);
    }
}

fn load_downloads() -> Vec<DlRecord> {
    std::fs::read(core::data_dir().join(DOWNLOADS_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_downloads(list: &[DlRecord]) {
    let dir = core::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        if let Err(e) = paths::write_private(&dir.join(DOWNLOADS_FILE), &json) {
            eprintln!("не удалось сохранить список загрузок: {e}");
        }
    }
}

#[derive(Serialize, Clone)]
struct LinkInfo {
    id: String,
    name: Option<String>,
    size: Option<u64>,
    /// ID узла отправителя — показывается в окне подтверждения.
    from: String,
}

#[derive(Serialize, Clone)]
struct Progress {
    id: String,
    bytes: u64,
    total: Option<u64>,
    speed: f64,
    state: String, // running | done | error | paused
    message: Option<String>,
    /// Имя, если узнали его у раздающего (ссылка без имени); иначе None — интерфейс не трогает имя.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// Текущая скорость по окну последних секунд, а не средняя от старта (аудит P1-13).
struct Speed {
    window: VecDeque<(Instant, u64)>,
}

impl Speed {
    fn new() -> Self {
        Self { window: VecDeque::new() }
    }
    fn push(&mut self, bytes: u64) -> f64 {
        let now = Instant::now();
        self.window.push_back((now, bytes));
        while self.window.len() > 2
            && now.duration_since(self.window[0].0) > Duration::from_secs(5)
        {
            self.window.pop_front();
        }
        let (t0, b0) = self.window[0];
        let dt = now.duration_since(t0).as_secs_f64();
        if dt < 0.2 {
            return 0.0;
        }
        bytes.saturating_sub(b0) as f64 / dt
    }
}

#[tauri::command]
fn inspect_link(link: String) -> Result<LinkInfo, String> {
    let l = core::parse_link(&link).map_err(|e| e.to_string())?;
    Ok(LinkInfo {
        id: l.ticket.hash().to_string(),
        name: l.name,
        size: l.size,
        from: l.ticket.addr().id.to_string(),
    })
}

/// Что внутри раздачи — по явной просьбе пользователя («Что внутри?»): только тогда узел подключается
/// к отправителю до решения скачивать.
#[derive(Serialize)]
struct PeekInfo {
    total: u64,
    name: Option<String>,
    files: Vec<(String, u64)>,
    free: Option<u64>,
    /// Почему не хватит места; None — хватает.
    space_error: Option<String>,
}

#[tauri::command]
async fn peek_link(state: State<'_, AppState>, link: String, dest: String) -> Result<PeekInfo, String> {
    let l = core::parse_link(&link).map_err(|e| e.to_string())?;
    let node = state.node().await?;
    let p = node.peek(&l.ticket).await.map_err(|e| format!("{e:#}"))?;
    let dest = PathBuf::from(dest);
    let space_error = node.check_space(l.ticket.hash(), p.total, &dest).await.err().map(|e| format!("{e:#}"));
    Ok(PeekInfo { total: p.total, name: p.name, files: p.files, free: core::free_space(&dest), space_error })
}

/// Системное уведомление о готовой загрузке (видно, даже когда окно закрыто и TikDL в строке меню).
fn notify_done(app: &AppHandle, path: Option<&str>) {
    use tauri_plugin_notification::NotificationExt;
    let name = path
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = app.notification().builder().title("Готово").body(format!("Скачано: {name}")).show();
}

/// Показать окно (оно могло быть скрыто: закрытое окно не завершает приложение, раздачи идут).
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Значок в строке меню: окно можно закрыть, раздачи продолжаются; «Выйти» — настоящий выход.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    let open = MenuItem::with_id(app, "open", "Открыть TikDL", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выйти из TikDL", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("TikDL — раздачи работают, пока приложение запущено")
        .menu(&menu)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
fn default_download_dir() -> String {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
fn take_pending_links(state: State<'_, AppState>) -> Vec<String> {
    state
        .pending_links
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

#[tauri::command]
async fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DlRecord>, String> {
    Ok(state.downloads.lock().await.clone())
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    link: String,
    dest: String,
) -> Result<String, String> {
    let l = core::parse_link(&link).map_err(|e| e.to_string())?;
    let id = l.ticket.hash().to_string();
    {
        let mut list = state.downloads.lock().await;
        match list.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                r.state = "running".into();
                r.message = None;
                r.link = link.clone();
                r.dest = dest.clone();
            }
            None => list.insert(
                0,
                DlRecord {
                    id: id.clone(),
                    link: link.clone(),
                    dest: dest.clone(),
                    name: l.name.clone(),
                    total: l.size,
                    bytes: 0,
                    state: "running".into(),
                    message: None,
                },
            ),
        }
        save_downloads(&list);
    }
    spawn_job(app, &state, l, id.clone(), PathBuf::from(dest)).await?;
    Ok(id)
}

async fn spawn_job(
    app: AppHandle,
    state: &AppState,
    l: core::Link,
    id: String,
    dest: PathBuf,
) -> Result<(), String> {
    let node = state.node().await?;
    let mut jobs = state.jobs.lock().await;
    if let Some(j) = jobs.get(&id) {
        if !j.handle.is_finished() {
            return Ok(());
        }
    }
    jobs.retain(|_, j| !j.handle.is_finished());
    let (cancel, cancel_rx) = watch::channel(false);
    let mut total = l.size;
    let id2 = id.clone();
    let app2 = app.clone();
    let handle = tokio::spawn(async move {
        let st = app2.state::<AppState>();
        let emit = |p: Progress| {
            let _ = app2.emit("progress", p);
        };
        emit(Progress {
            id: id2.clone(),
            bytes: 0,
            total,
            speed: 0.0,
            state: "running".into(),
            message: Some("Подключение…".into()),
            name: None,
        });
        // Ссылка без размера (голый тикет): узнаём размер и имя у раздающего, иначе полоса стоит на месте.
        if total.is_none() {
            if let Ok(peek) = node.peek(&l.ticket).await {
                total = Some(peek.total);
                let name = peek.name.clone();
                st.update(&id2, |r| {
                    r.total = Some(peek.total);
                    if r.name.is_none() {
                        r.name = name.clone();
                    }
                })
                .await;
                emit(Progress {
                    id: id2.clone(),
                    bytes: 0,
                    total,
                    speed: 0.0,
                    state: "running".into(),
                    message: Some("Подключение…".into()),
                    name,
                });
            }
        }
        // Места не хватит — отказ до первого байта, а не забитый диск посреди загрузки.
        let space = match total {
            Some(t) => node.check_space(l.ticket.hash(), t, &dest).await,
            None => Ok(()),
        };
        let mut speed = Speed::new();
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut last_bytes = 0u64;
        let res = match space {
            Err(e) => Err(e),
            Ok(()) => node
            .download(&l.ticket, &dest, cancel_rx, |bytes| {
                last_bytes = bytes;
                let s = speed.push(bytes);
                if last_emit.elapsed() < Duration::from_millis(250) {
                    return;
                }
                last_emit = Instant::now();
                emit(Progress {
                    id: id2.clone(),
                    bytes,
                    total,
                    speed: s,
                    state: "running".into(),
                    message: None,
                    name: None,
                });
            })
            .await,
        };
        // Состояние решает сама задача — «пауза» не может прийти после «готово».
        let (state_name, message, bytes) = match &res {
            Ok(path) => ("done", Some(path.to_string_lossy().to_string()), total.unwrap_or(last_bytes).max(last_bytes)),
            Err(e) if e.is::<core::Cancelled>() => ("paused", None, last_bytes),
            Err(e) => ("error", Some(format!("{e:#}")), last_bytes),
        };
        st.update(&id2, |r| {
            r.state = state_name.into();
            r.message = message.clone();
            r.bytes = bytes;
        })
        .await;
        if state_name == "done" {
            notify_done(&app2, message.as_deref());
        }
        emit(Progress {
            id: id2.clone(),
            bytes,
            total,
            speed: 0.0,
            state: state_name.into(),
            message,
            name: None,
        });
    });
    jobs.insert(id, Job { cancel, handle });
    Ok(())
}

#[tauri::command]
async fn pause_download(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let jobs = state.jobs.lock().await;
    match jobs.get(&id) {
        // Задача сама сообщит «paused» (или «done», если уже экспортирует на диск).
        Some(j) if !j.handle.is_finished() => {
            let _ = j.cancel.send(true);
        }
        _ => {
            drop(jobs);
            state.update(&id, |r| {
                if r.state == "running" {
                    r.state = "paused".into();
                }
            })
            .await;
            if let Some(r) = state.downloads.lock().await.iter().find(|r| r.id == id).cloned() {
                let _ = app.emit(
                    "progress",
                    Progress { id, bytes: r.bytes, total: r.total, speed: 0.0, state: r.state, message: r.message, name: None },
                );
            }
        }
    }
    Ok(())
}

/// Убрать загрузку из списка; недокачанное освобождается сборщиком мусора.
#[tauri::command]
async fn remove_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if let Some(j) = state.jobs.lock().await.remove(&id) {
        let _ = j.cancel.send(true);
        let _ = j.handle.await;
    }
    {
        let mut list = state.downloads.lock().await;
        list.retain(|r| r.id != id);
        save_downloads(&list);
    }
    if let (Ok(node), Ok(hash)) = (state.node().await, id.parse()) {
        node.forget_download(hash).await.map_err(|e| format!("{e:#}"))?;
    }
    Ok(())
}

#[derive(Serialize, Clone)]
struct ShareProgress {
    done: usize,
    total: usize,
}

#[tauri::command]
async fn share_path(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<core::Shared, String> {
    let node = state.node().await?;
    node.share(&PathBuf::from(path), |done, total| {
        let _ = app.emit("share-progress", ShareProgress { done, total });
    })
    .await
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn list_shares(state: State<'_, AppState>) -> Result<Vec<core::Shared>, String> {
    Ok(state.node().await?.list_shares().await)
}

#[tauri::command]
async fn stop_share(state: State<'_, AppState>, root: String) -> Result<(), String> {
    state.node().await?.stop_share(&root).await.map_err(|e| format!("{e:#}"))
}

fn print_share(s: &core::Shared) {
    let state = if s.state == "active" { "раздаётся" } else { "НЕ раздаётся: исходные файлы изменились" };
    println!(
        "{}\n  {} · файлов: {}, пропущено скрытых: {} · {}\n  {}\n",
        s.source, s.root, s.files, s.skipped, state, s.link
    );
    if let Some(w) = &s.web_link {
        println!("  для мессенджеров: {w}\n");
    }
}

/// Режим без окна для сервера.
async fn cli(args: &[String]) -> anyhow::Result<()> {
    let node = core::Node::start(&core::data_dir()).await?;
    let res = cli_inner(&node, args).await;
    node.shutdown().await;
    res
}

async fn cli_inner(node: &core::Node, args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("get") => {
            let link = args.get(1).ok_or_else(|| anyhow::anyhow!("нужна ссылка"))?;
            let dest = args.get(2).ok_or_else(|| anyhow::anyhow!("нужна папка"))?;
            let l = core::parse_link(link)?;
            let (_keep, never) = watch::channel(false);
            let p = node
                .download(&l.ticket, Path::new(dest), never, |b| eprint!("\r{b} байт"))
                .await?;
            eprintln!("\nГотово: {}", p.display());
        }
        Some("serve") => {
            for p in &args[1..] {
                print_share(&node.share(Path::new(p), |_, _| {}).await?);
            }
            if args.len() == 1 {
                for s in node.list_shares().await {
                    print_share(&s);
                }
            }
            println!("Раздача идёт. Узел: {}. Ctrl+C — остановить.", node.endpoint().id());
            tokio::signal::ctrl_c().await?;
        }
        Some("shares") => {
            for s in node.list_shares().await {
                print_share(&s);
            }
        }
        Some("stop") => {
            let root = args.get(1).ok_or_else(|| anyhow::anyhow!("нужен идентификатор раздачи"))?;
            node.stop_share(root).await?;
            println!("Раздача остановлена: {root}");
        }
        _ => unreachable!(),
    }
    Ok(())
}

const USAGE: &str = "tikdl get <ссылка> <папка> | serve [путь ...] | shares | stop <id>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str);
    if matches!(cmd, Some("get" | "serve" | "shares" | "stop")) {
        if cmd == Some("get") && args.len() < 3 || cmd == Some("stop") && args.len() < 2 {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        if let Err(e) = rt.block_on(cli(&args)) {
            eprintln!("Ошибка: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    tauri::Builder::default()
        // Должен быть первым: второй запуск (клик по ссылке) передаёт ссылку в уже открытое окно (аудит P1-7).
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            downloads: Mutex::new(load_downloads()),
            ..Default::default()
        })
        .setup(|app| {
            use tauri_plugin_deep_link::DeepLinkExt;
            build_tray(app)?;
            #[cfg(any(target_os = "linux", windows))]
            let _ = app.deep_link().register_all();

            let queue = |handle: &AppHandle, urls: Vec<String>| {
                if let Ok(mut p) = handle.state::<AppState>().pending_links.lock() {
                    p.extend(urls);
                }
                let _ = handle.emit("links-pending", ());
            };
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |ev| {
                queue(&handle, ev.urls().iter().map(|u| u.to_string()).collect());
                show_main(&handle);
            });
            // Ссылка, с которой приложение было запущено: ждёт, пока страница её заберёт.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                queue(app.handle(), urls.iter().map(|u| u.to_string()).collect());
            }

            // Узел — сразу при старте: прежние раздачи доступны без единого клика (аудит P1-9),
            // прерванные выходом загрузки продолжаются сами (аудит P1-8).
            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let st = h.state::<AppState>();
                if let Err(e) = st.node().await {
                    let _ = h.emit("node-error", e);
                    return;
                }
                let resume: Vec<DlRecord> = st
                    .downloads
                    .lock()
                    .await
                    .iter()
                    .filter(|r| r.state == "running")
                    .cloned()
                    .collect();
                for r in resume {
                    if let Ok(l) = core::parse_link(&r.link) {
                        let _ = spawn_job(h.clone(), &st, l, r.id.clone(), PathBuf::from(&r.dest)).await;
                    }
                }
                let _ = h.emit("node-ready", ());
            });
            Ok(())
        })
        // Закрыть окно = спрятать: узел продолжает раздавать, приложение живёт в строке меню.
        .on_window_event(|w, ev| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            inspect_link,
            peek_link,
            default_download_dir,
            take_pending_links,
            list_downloads,
            start_download,
            pause_download,
            remove_download,
            share_path,
            list_shares,
            stop_share
        ])
        .build(tauri::generate_context!())
        .expect("ошибка запуска приложения")
        .run(|app, ev| {
            // Записать базы хранилищ перед выходом, иначе теряется последняя пачка изменений.
            match ev {
                tauri::RunEvent::Exit => {
                    if let Some(node) = app.state::<AppState>().node.get().cloned() {
                        tauri::async_runtime::block_on(node.shutdown());
                    }
                }
                // macOS: щелчок по значку в Dock возвращает спрятанное окно.
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => show_main(app),
                _ => {}
            }
        });
}
