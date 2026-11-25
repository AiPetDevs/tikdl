
#[cfg(target_os = "android")]
mod android;
mod core;
mod once;
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
    /// One-time link: everything is on disk, the sender has not yet heard "done" (retried at start).
    #[serde(default)]
    ack_pending: bool,
    #[serde(default)]
    ack_tries: u32,
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
    /// Одноразовая ссылка: имени и размера в ней нет; скачать сможет одно устройство.
    once: bool,
    /// Этот компьютер (8 знаков) — сверить с отправителем, за кем закреплена ссылка.
    me: Option<String>,
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
async fn inspect_link(state: State<'_, AppState>, link: String) -> Result<LinkInfo, String> {
    let l = core::parse_link(&link).map_err(|e| e.to_string())?;
    let me = match l.once() {
        Some(_) => state.node.get().map(|n| n.short_id()),
        None => None,
    };
    Ok(LinkInfo { id: l.record_id(), name: l.name(), size: l.size(), from: l.from(), once: l.once().is_some(), me })
}

/// A one-time link this computer has already received: no claim over the network (it would only time out —
/// the sender's throwaway endpoint is gone), a plain answer instead.
async fn already_received(state: &AppState, l: &core::Link) -> Result<(), String> {
    if l.once().is_some() {
        let id = l.record_id();
        if state.downloads.lock().await.iter().any(|r| r.id == id && r.state == "done") {
            return Err("уже получено на этом компьютере — одноразовая ссылка сгорела".into());
        }
    }
    Ok(())
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
    already_received(&state, &l).await?;
    let node = state.node().await?;
    // one-time link: «Что внутри?» binds this device (the root is learned only from the claim)
    let ticket = node.resolve(&l).await.map_err(|e| format!("{e:#}"))?;
    let p = node.peek(&ticket).await.map_err(|e| format!("{e:#}"))?;
    let dest = PathBuf::from(dest);
    let space_error = node.check_space(ticket.hash(), p.total, &dest).await.err().map(|e| format!("{e:#}"));
    Ok(PeekInfo { total: p.total, name: p.name, files: p.files, free: core::free_space(&dest), space_error })
}

/// Системное уведомление о готовой загрузке (видно, даже когда окно закрыто и TikDL в строке меню).
fn notify_done(app: &AppHandle, path: Option<&str>) {
    use tauri_plugin_notification::NotificationExt;
    let name = path
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if let Err(e) = app.notification().builder().title("Готово").body(format!("Скачано: {name}")).show() {
        eprintln!("уведомление не показано: {e}");
    }
}

/// Показать окно (оно могло быть скрыто: закрытое окно не завершает приложение, раздачи идут).
fn show_main(app: &AppHandle) {
    #[cfg(desktop)]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    #[cfg(mobile)]
    let _ = app;
}

#[cfg(desktop)]
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
fn default_download_dir(app: AppHandle) -> String {
    #[cfg(target_os = "android")]
    {
        let _ = &app;
        // Shared "Download" folder: apps may create files there without a permission since Android 11.
        return "/storage/emulated/0/Download".into();
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
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
    already_received(&state, &l).await?;
    let id = l.record_id();
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
                    name: l.name(),
                    total: l.size(),
                    bytes: 0,
                    state: "running".into(),
                    message: None,
                    ack_pending: false,
                    ack_tries: 0,
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
    let mut total = l.size();
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
        // Одноразовая ссылка: сначала привязка этого устройства, корень раздачи узнаём из ответа.
        let ticket = match node.resolve(&l).await {
            Ok(t) => t,
            Err(e) => {
                let message = Some(format!("{e:#}"));
                st.update(&id2, |r| {
                    r.state = "error".into();
                    r.message = message.clone();
                })
                .await;
                emit(Progress { id: id2.clone(), bytes: 0, total, speed: 0.0, state: "error".into(), message, name: None });
                return;
            }
        };
        // Ссылка без размера (голый тикет, одноразовая): узнаём размер и имя у раздающего, иначе полоса стоит на месте.
        if total.is_none() {
            if let Ok(peek) = node.peek(&ticket).await {
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
            Some(t) => node.check_space(ticket.hash(), t, &dest).await,
            None => Ok(()),
        };
        let mut speed = Speed::new();
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut last_bytes = 0u64;
        let res = match space {
            Err(e) => Err(e),
            Ok(()) => node
            .download(&ticket, &id2, &dest, cancel_rx, |bytes| {
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
        if let Ok(path) = &res {
            // The file is in place: the record says "done" before anything else, so a crash or quit from here on
            // never restarts the download (no "name (1)", no false "сбой"). One-time link: "done" is owed to the
            // sender — recorded in the same write, before the tag goes, so it cannot be lost either.
            let (once, message, bytes) = (l.once().is_some(), path.to_string_lossy().to_string(), total.unwrap_or(last_bytes).max(last_bytes));
            st.update(&id2, |r| {
                r.state = "done".into();
                r.message = Some(message.clone());
                r.bytes = bytes;
                r.ack_pending |= once;
            })
            .await;
            if let Err(e) = node.forget_download(&id2).await {
                eprintln!("не удалось снять тег загрузки: {e:#}");
            }
            if let Some(t) = l.once() {
                send_done(&st, &node, &id2, t).await;
            }
        }
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

/// One-time link received: tell the sender (the link burns), then drop the token from the saved link.
/// Not delivered — retried at the next start, at most 5 times; the sender's link expires by itself anyway.
async fn send_done(st: &AppState, node: &core::Node, id: &str, t: &once::OnceTicket) {
    let ok = node.once_done(t).await;
    st.update(id, |r| {
        r.ack_tries += 1;
        if ok.is_ok() || r.ack_tries >= 5 {
            r.ack_pending = false;
            r.link.clear(); // the token is no use to anyone any more
        }
    })
    .await;
    if let Err(e) = ok {
        eprintln!("отправитель не получил «получено»: {e:#}");
    }
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
    if let Ok(node) = state.node().await {
        node.forget_download(&id).await.map_err(|e| format!("{e:#}"))?;
    }
    Ok(())
}

#[derive(Serialize, Clone)]
struct ShareProgress {
    done: usize,
    total: usize,
}

/// Запрещённые папки (системные, домашняя целиком, ключи программ, папка TikDL) отсекает Node::share.
#[tauri::command]
async fn share_path(app: AppHandle, state: State<'_, AppState>, path: String, once: bool) -> Result<core::Shared, String> {
    let node = state.node().await?;
    #[cfg(target_os = "android")]
    let path = if path.starts_with("content://") {
        let imports = IMPORTS_DIR.get().cloned().ok_or("нет папки для выбранных файлов")?;
        tokio::task::spawn_blocking(move || android::import_content(&path, &imports))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("{e:#}"))?
            .to_string_lossy()
            .to_string()
    } else {
        path
    };
    let r = node
        .share(&PathBuf::from(path), once, |done, total| {
            let _ = app.emit("share-progress", ShareProgress { done, total });
        })
        .await
        .map_err(|e| format!("{e:#}"));
    sync_service(&node).await;
    r
}

/// Android: copies of files picked in the system picker (content:// URIs) live here while shared.
#[cfg(target_os = "android")]
static IMPORTS_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Android: keep the foreground service (and its notification) exactly while something is shared.
async fn sync_service(node: &core::Node) {
    #[cfg(target_os = "android")]
    {
        let active = !node.list_shares().await.is_empty();
        if let Err(e) = android::set_sharing(active) {
            eprintln!("служба раздачи: {e:#}");
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = node;
}

#[tauri::command]
async fn list_shares(state: State<'_, AppState>) -> Result<Vec<core::Shared>, String> {
    Ok(state.node().await?.list_shares().await)
}

#[tauri::command]
async fn stop_share(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let node = state.node().await?;
    let r = node.stop_share(&id).await.map_err(|e| format!("{e:#}"));
    sync_service(&node).await;
    r
}

/// One-time share: a new link (the old one stops working, whoever holds it).
#[tauri::command]
async fn renew_share(state: State<'_, AppState>, id: String) -> Result<core::Shared, String> {
    state.node().await?.renew_once(&id).await.map_err(|e| format!("{e:#}"))
}

/// What the CLI says about a one-time link's stage.
fn once_stage(o: &core::OnceView) -> String {
    match o.state.as_str() {
        "waiting" => "одноразовая ссылка: ждёт получателя".into(),
        "claimed" => format!("одноразовая ссылка: закреплена за устройством {}", o.device.as_deref().unwrap_or("?")),
        "used" => format!("одноразовая ссылка: получено устройством {} — ссылка сгорела", o.device.as_deref().unwrap_or("?")),
        _ => "одноразовая ссылка: срок истёк".into(),
    }
}

#[cfg(desktop)]
fn print_share(s: &core::Shared) {
    let state = match (s.state.as_str(), &s.once) {
        ("active", Some(o)) => once_stage(o),
        // received or expired stays so even if the source files changed afterwards
        ("changed", Some(o)) if matches!(o.state.as_str(), "used" | "expired") => once_stage(o),
        ("active", None) => "раздаётся".into(),
        ("forbidden", _) => format!("НЕ раздаётся: {}", s.why.as_deref().unwrap_or("такие папки TikDL не раздаёт")),
        _ => "НЕ раздаётся: исходные файлы изменились".into(),
    };
    println!("{}\n  {} · файлов: {}, пропущено скрытых: {} · {}", s.source, s.id, s.files, s.skipped, state);
    if s.state == "active" && s.link.is_empty() && s.once.as_ref().is_some_and(|o| o.state == "waiting") {
        println!("  ссылки пока нет: нет связи с relay — она появится здесь, когда связь будет");
    }
    if s.state == "active" && !s.link.is_empty() {
        println!("  {}", s.link);
        if let Some(w) = &s.web_link {
            println!("  для мессенджеров: {w}");
        }
    } else if s.state != "active" || s.once.as_ref().is_some_and(|o| !matches!(o.state.as_str(), "waiting" | "claimed")) {
        println!("  убрать из списка: tikdl stop {}", s.id);
    }
    println!();
}

/// Режим без окна для сервера.
#[cfg(desktop)]
async fn cli(args: &[String]) -> anyhow::Result<()> {
    let node = core::Node::start(&core::data_dir()).await?;
    let res = cli_inner(&node, args).await;
    node.shutdown().await;
    res
}

#[cfg(desktop)]
async fn cli_inner(node: &core::Node, args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("get") => {
            let link = args.get(1).ok_or_else(|| anyhow::anyhow!("нужна ссылка"))?;
            let dest = args.get(2).ok_or_else(|| anyhow::anyhow!("нужна папка"))?;
            let l = core::parse_link(link)?;
            if l.once().is_some() {
                eprintln!("Одноразовая ссылка: закрепляю за этим устройством ({})…", node.short_id());
            }
            let ticket = node.resolve(&l).await?;
            let id = l.record_id();
            let (_keep, never) = watch::channel(false);
            let p = node
                .download(&ticket, &id, Path::new(dest), never, |b| eprint!("\r{b} байт"))
                .await?;
            node.forget_download(&id).await?;
            eprintln!("\nГотово: {}", p.display());
            if let Some(t) = l.once() {
                match node.once_done(t).await {
                    Ok(()) => eprintln!("Отправитель знает, что получено: ссылка сгорела."),
                    Err(e) => eprintln!("Отправитель не получил «получено» ({e:#}); его ссылка истечёт сама."),
                }
            }
        }
        Some("serve") => {
            // --once: the paths after it get one-time links (one device, burned once received)
            let once = args.iter().any(|a| a == "--once");
            let paths: Vec<&String> = args[1..].iter().filter(|a| *a != "--once").collect();
            let mut failed = 0;
            for p in &paths {
                // a refused path does not stop the others
                match node.share(Path::new(p.as_str()), once, |_, _| {}).await {
                    Ok(shared) => print_share(&shared),
                    Err(e) => {
                        failed += 1;
                        eprintln!("Ошибка: {p}: {e:#}\n");
                    }
                }
            }
            if !paths.is_empty() && failed == paths.len() {
                anyhow::bail!("ничего не раздаётся");
            }
            if paths.is_empty() {
                for s in node.list_shares().await {
                    print_share(&s);
                }
            }
            println!("Раздача идёт. Узел: {}. Ctrl+C — остановить.", node.endpoint().id());
            // one-time links: say when a stage changes (claimed by a device, received, expired)
            let mut rx = node.watch_once();
            let mut seen: HashMap<String, String> = node
                .list_shares()
                .await
                .into_iter()
                .filter_map(|s| s.once.as_ref().map(|o| (s.id.clone(), format!("{}|{}", o.state, s.link.is_empty()))))
                .collect();
            loop {
                tokio::select! {
                    r = tokio::signal::ctrl_c() => { r?; break; }
                    r = rx.changed() => { if r.is_err() { break; } }
                }
                for s in node.list_shares().await {
                    if let Some(o) = &s.once {
                        let now = format!("{}|{}", o.state, s.link.is_empty());
                        if seen.insert(s.id.clone(), now.clone()).is_some_and(|old| old != now) {
                            println!("{} — {}", s.name, once_stage(o));
                            if o.state == "waiting" && !s.link.is_empty() {
                                println!("  {}", s.link);
                            }
                        }
                    }
                }
            }
        }
        Some("shares") => {
            for s in node.list_shares().await {
                print_share(&s);
            }
        }
        Some("stop") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("нужен идентификатор раздачи"))?;
            node.stop_share(id).await?;
            println!("Раздача остановлена: {id}");
        }
        _ => unreachable!(),
    }
    Ok(())
}

#[cfg(desktop)]
const USAGE: &str = "tikdl get <ссылка> <папка> | serve [--once] [путь ...] | shares | stop <id>";

#[cfg(desktop)]
pub fn cli_main() -> bool {
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
        return true;
    }
    false
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main(app)));
    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            #[cfg(mobile)]
            if let Ok(d) = app.path().app_data_dir() {
                let _ = core::DATA_DIR_OVERRIDE.set(d.join("tikdl"));
                #[cfg(target_os = "android")]
                let _ = IMPORTS_DIR.set(d.join("imports"));
            }
            *app.state::<AppState>().downloads.blocking_lock() = load_downloads();
            use tauri_plugin_deep_link::DeepLinkExt;
            #[cfg(desktop)]
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
                let acks: Vec<DlRecord> = st.downloads.lock().await.iter().filter(|r| r.ack_pending).cloned().collect();
                if let Ok(node) = st.node().await {
                    for r in acks {
                        if let Ok(core::Link::Once(t)) = core::parse_link(&r.link) {
                            send_done(&st, &node, &r.id, &t).await;
                        }
                    }
                }
                if let Ok(node) = st.node().await {
                    // one-time links change stage on their own (claimed, received, expired): tell the window
                    let mut rx = node.watch_once();
                    let h2 = h.clone();
                    tauri::async_runtime::spawn(async move {
                        while rx.changed().await.is_ok() {
                            let _ = h2.emit("shares-changed", ());
                        }
                    });
                }
                if let Ok(node) = st.node().await {
                    sync_service(&node).await;
                }
                let _ = h.emit("node-ready", ());
            });
            Ok(())
        })
        // Закрыть окно = спрятать: узел продолжает раздавать, приложение живёт в строке меню.
        .on_window_event(|_w, _ev| {
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = _ev {
                api.prevent_close();
                let _ = _w.hide();
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
            stop_share,
            renew_share
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
