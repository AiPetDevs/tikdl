#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod paths;

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, OnceCell};

#[derive(Default)]
struct AppState {
    node: OnceCell<Arc<core::Node>>,
    jobs: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl AppState {
    async fn node(&self) -> Result<Arc<core::Node>, String> {
        self.node
            .get_or_try_init(|| async {
                core::Node::start(&core::data_dir())
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            })
            .await
            .cloned()
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

#[tauri::command]
fn default_download_dir() -> String {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    state: State<'_, AppState>,
    link: String,
    dest: String,
) -> Result<String, String> {
    let l = core::parse_link(&link).map_err(|e| e.to_string())?;
    let node = state.node().await?;
    let id = l.ticket.hash().to_string();
    let total = l.size;
    let id2 = id.clone();
    let mut jobs = state.jobs.lock().await;
    if let Some(h) = jobs.get(&id) {
        if !h.is_finished() {
            return Ok(id);
        }
    }
    let handle = tokio::spawn(async move {
        let started = Instant::now();
        let mut first: Option<u64> = None;
        let mut last_emit = Instant::now();
        let emit = |p: Progress| {
            let _ = app.emit("progress", p);
        };
        emit(Progress {
            id: id2.clone(),
            bytes: 0,
            total,
            speed: 0.0,
            state: "running".into(),
            message: Some("Подключение…".into()),
        });
        let res = node
            .download(&l.ticket, &PathBuf::from(&dest), |bytes| {
                let base = *first.get_or_insert(bytes);
                if last_emit.elapsed().as_millis() < 250 {
                    return;
                }
                last_emit = Instant::now();
                let secs = started.elapsed().as_secs_f64().max(0.001);
                emit(Progress {
                    id: id2.clone(),
                    bytes,
                    total,
                    speed: (bytes.saturating_sub(base)) as f64 / secs,
                    state: "running".into(),
                    message: None,
                });
            })
            .await;
        match res {
            Ok(path) => emit(Progress {
                id: id2.clone(),
                bytes: total.unwrap_or(0),
                total,
                speed: 0.0,
                state: "done".into(),
                message: Some(path.to_string_lossy().to_string()),
            }),
            Err(e) => emit(Progress {
                id: id2.clone(),
                bytes: 0,
                total,
                speed: 0.0,
                state: "error".into(),
                message: Some(e.to_string()),
            }),
        }
    });
    jobs.insert(id.clone(), handle);
    Ok(id)
}

#[tauri::command]
async fn pause_download(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    if let Some(h) = state.jobs.lock().await.remove(&id) {
        h.abort();
    }
    let _ = app.emit(
        "progress",
        Progress {
            id,
            bytes: 0,
            total: None,
            speed: 0.0,
            state: "paused".into(),
            message: None,
        },
    );
    Ok(())
}

#[tauri::command]
async fn share_path(state: State<'_, AppState>, path: String) -> Result<core::Shared, String> {
    let node = state.node().await?;
    node.share(&PathBuf::from(path)).await.map_err(|e| e.to_string())
}

/// Режим без окна для сервера:  tikdl serve /путь/к/папке
async fn serve_cli(paths: Vec<String>) -> anyhow::Result<()> {
    let node = core::Node::start(&core::data_dir()).await?;
    for p in paths {
        let s = node.share(&PathBuf::from(&p)).await?;
        println!("{p}\n  файлов: {}, пропущено скрытых: {}\n  {}\n", s.files, s.skipped, s.link);
    }
    println!("Раздача идёт. Узел: {}. Ctrl+C — остановить.", node.endpoint().id());
    tokio::signal::ctrl_c().await?;
    node.router.shutdown().await?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("get") && args.len() >= 3 {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let res = rt.block_on(async {
            let l = core::parse_link(&args[1])?;
            let node = core::Node::start(&core::data_dir()).await?;
            let p = node
                .download(&l.ticket, &PathBuf::from(&args[2]), |b| eprint!("\r{b} байт"))
                .await?;
            eprintln!("\nГотово: {}", p.display());
            node.router.shutdown().await?;
            anyhow::Ok(())
        });
        if let Err(e) = res {
            eprintln!("Ошибка: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().map(String::as_str) == Some("serve") {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        if let Err(e) = rt.block_on(serve_cli(args[1..].to_vec())) {
            eprintln!("Ошибка: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            use tauri_plugin_deep_link::DeepLinkExt;
            #[cfg(any(target_os = "linux", windows))]
            let _ = app.deep_link().register_all();
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |ev| {
                for url in ev.urls() {
                    let _ = handle.emit("open-link", url.to_string());
                }
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.set_focus();
                }
            });
            // Ссылка, с которой приложение было запущено.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                let h = app.handle().clone();
                let urls: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                    for u in urls {
                        let _ = h.emit("open-link", u);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            inspect_link,
            default_download_dir,
            start_download,
            pause_download,
            share_path
        ])
        .run(tauri::generate_context!())
        .expect("ошибка запуска приложения");
}
