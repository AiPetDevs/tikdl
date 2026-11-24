//! Ядро: iroh-узел с постоянным ключом и постоянным хранилищем.
//! Постоянный ключ = ссылки не ломаются при смене сети/IP.
//! Постоянное хранилище = недокачанное продолжается после паузы/перезапуска.
//!
//! Два хранилища (аудит P0-2):
//! - `store/`     — только то, что пользователь сам раздал; его отдаёт сеть (Router + BlobsProtocol);
//! - `downloads/` — скачанное; в сеть не отдаётся никогда, получатель не становится раздающим.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use futures_lite::StreamExt;
use iroh::{endpoint::presets, protocol::Router, Endpoint, SecretKey};
use iroh_blobs::{
    api::{
        blobs::{AddPathOptions, ImportMode},
        downloader::{DownloadProgressItem, Shuffled},
    },
    format::collection::Collection,
    store::fs::FsStore,
    ticket::BlobTicket,
    BlobFormat, BlobsProtocol, HashAndFormat,
};

use crate::paths;

pub const SCHEME: &str = "tikdl://get/";

pub struct Node {
    /// Раздачи пользователя. Только это хранилище подключено к Router.
    pub store: FsStore,
    /// Скачанное. Не подключено к Router — наружу не отдаётся.
    dl: FsStore,
    mem: iroh::address_lookup::MemoryLookup,
    pub router: Router,
}

/// Итог подготовки раздачи.
#[derive(serde::Serialize, Clone)]
pub struct Shared {
    pub link: String,
    pub files: usize,
    /// Скрытые и служебные файлы, которые не вошли в раздачу папки (.git, .env, .DS_Store …).
    pub skipped: usize,
    pub total: u64,
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tikdl")
}

/// Ключ узла: только владелец может читать (0600), запись атомарная.
/// Ошибка чтения — ошибка, а не тихая замена ключа (иначе все ссылки молча ломаются).
fn load_or_create_key(dir: &Path) -> Result<SecretKey> {
    let path = dir.join("node.key");
    match std::fs::read(&path) {
        Ok(bytes) => {
            if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
                paths::restrict_permissions(&path).context("не удалось закрыть доступ к ключу узла")?;
                return Ok(SecretKey::from_bytes(&arr));
            }
            let bad = dir.join("node.key.bad");
            std::fs::rename(&path, &bad).context("не удалось отложить повреждённый ключ узла")?;
            eprintln!(
                "Ключ узла повреждён (сохранён как {}). Создан новый ключ — старые ссылки перестанут работать.",
                bad.display()
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("не удалось прочитать ключ узла"),
    }
    let key = SecretKey::generate();
    paths::write_private(&path, &key.to_bytes()).context("не удалось сохранить ключ узла")?;
    Ok(key)
}

impl Node {
    pub async fn start(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let key = load_or_create_key(dir)?;
        let store = FsStore::load(dir.join("store")).await?;
        let dl = FsStore::load(dir.join("downloads")).await?;
        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(key)
            .bind()
            .await?;
        let mem = iroh::address_lookup::MemoryLookup::new();
        endpoint.address_lookup()?.add(mem.clone());
        // Наружу отдаётся только хранилище раздач; `dl` сюда не попадает.
        let blobs = BlobsProtocol::new(&store, None);
        let router = Router::builder(endpoint)
            .accept(iroh_blobs::ALPN, blobs)
            .spawn();
        Ok(Self { store, dl, mem, router })
    }

    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    /// Раздать файл или папку. Ссылка: tikdl://get/<тикет>?size=..&name=..
    /// В папке скрытые и служебные файлы пропускаются (аудит P0-3); явно выбранный файл раздаётся всегда.
    pub async fn share(&self, path: &Path) -> Result<Shared> {
        let path = std::path::absolute(path)?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "files".into());

        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let mut skipped = 0usize;
        if path.is_file() {
            files.push((name.clone(), path.clone()));
        } else if path.is_dir() {
            for e in walkdir::WalkDir::new(&path).follow_links(false) {
                let e = e?;
                if !e.file_type().is_file() {
                    continue;
                }
                let rel = e.path().strip_prefix(&path)?;
                if paths::is_hidden_path(rel) {
                    skipped += 1;
                    continue;
                }
                let rel = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                files.push((format!("{name}/{rel}"), e.path().to_path_buf()));
            }
        } else {
            bail!("путь не найден: {}", path.display());
        }
        if files.is_empty() {
            if skipped > 0 {
                bail!("в папке только скрытые и служебные файлы ({skipped}) — они не раздаются");
            }
            bail!("в папке нет файлов");
        }
        let count = files.len();

        let mut items = Vec::new();
        let mut total: u64 = 0;
        let mut keep = Vec::new();
        for (rel, p) in files {
            total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            // TryReference: не копировать файл в хранилище, а ссылаться на оригинал.
            let tag = self
                .store
                .add_path_with_opts(AddPathOptions {
                    path: p,
                    format: BlobFormat::Raw,
                    mode: ImportMode::TryReference,
                })
                .await?;
            items.push((rel, tag.hash));
            keep.push(tag);
        }
        let collection: Collection = items.into_iter().collect();
        let tt = collection.store(&self.store).await?;
        let root = tt.hash();
        // Постоянный тег, чтобы раздача пережила перезапуск.
        self.store
            .tags()
            .set(format!("share-{root}"), tt.hash_and_format())
            .await?;

        // Ждём связи с relay (до 5 с), чтобы в ссылку попал и relay, и прямые адреса.
        // Сам ключ узла остаётся главным: при смене IP узел находится по нему.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.endpoint().online(),
        )
        .await;
        let ticket = BlobTicket::new(self.endpoint().addr(), root, BlobFormat::HashSeq);
        let link = format!(
            "{SCHEME}{ticket}?size={total}&name={}",
            urlencoding::encode(&name)
        );
        Ok(Shared { link, files: count, skipped, total })
    }

    /// Скачать по ссылке/тикету в папку dest. on_progress(скачано_байт).
    pub async fn download(
        &self,
        ticket: &BlobTicket,
        dest: &Path,
        mut on_progress: impl FnMut(u64),
    ) -> Result<PathBuf> {
        let root = ticket.hash();
        let req = HashAndFormat::hash_seq(root);
        // Сообщаем узлу известные адреса раздающего из тикета.
        let addr = ticket.addr().clone();
        self.mem.add_endpoint_info(addr.clone());
        let providers = Shuffled::new(vec![addr.id]);
        let mut stream = self
            .dl
            .downloader(self.endpoint())
            .download(req, providers)
            .stream()
            .await?;
        while let Some(item) = stream.next().await {
            match item {
                DownloadProgressItem::Progress(n) => on_progress(n),
                DownloadProgressItem::Error(e) => bail!("ошибка загрузки: {e}"),
                DownloadProgressItem::DownloadError => {
                    bail!("не удалось скачать: раздающий недоступен или прервал связь")
                }
                _ => {}
            }
        }
        // Удерживаем скачанное в своём (не раздаваемом) хранилище — для докачки.
        self.dl
            .tags()
            .set(format!("dl-{root}"), req)
            .await?;

        let collection = Collection::load(root, self.dl.as_ref()).await?;

        // 1) Проверяем все имена до записи на диск: чужая коллекция не должна выйти за dest.
        let mut entries = Vec::new();
        for (name, hash) in collection.iter() {
            paths::safe_join(dest, name).map_err(|e| anyhow!(e))?;
            let (top, rest) = match name.split_once('/') {
                Some((t, r)) => (t.to_string(), Some(r.to_string())),
                None => (name.to_string(), None),
            };
            entries.push((top, rest, *hash));
        }

        // 2) Ничего не перезаписываем (аудит P0-6): занятое имя верхнего уровня → «имя (1)».
        //    Папка переименовывается целиком, поэтому всё её содержимое пишется в новое место.
        let mut renamed: HashMap<String, String> = HashMap::new();
        let mut first: Option<PathBuf> = None;
        for (top, rest, hash) in entries {
            let new_top = match renamed.get(&top) {
                Some(n) => n.clone(),
                None => {
                    let n = paths::unique_name(dest, &top, rest.is_some());
                    renamed.insert(top.clone(), n.clone());
                    n
                }
            };
            let rel = match &rest {
                Some(r) => format!("{new_top}/{r}"),
                None => new_top.clone(),
            };
            let target = paths::safe_join(dest, &rel).map_err(|e| anyhow!(e))?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            self.dl.blobs().export(hash, &target).await?;
            if first.is_none() {
                first = Some(dest.join(&new_top));
            }
        }
        Ok(first.unwrap_or_else(|| dest.to_path_buf()))
    }
}

/// Разбор ссылки tikdl://get/<тикет>?size=..&name=.. или «голого» тикета (в т.ч. от sendme).
pub struct Link {
    pub ticket: BlobTicket,
    pub size: Option<u64>,
    pub name: Option<String>,
}

pub fn parse_link(s: &str) -> Result<Link> {
    let s = s.trim();
    let s = s.strip_prefix(SCHEME).unwrap_or(s);
    let (t, query) = match s.split_once('?') {
        Some((t, q)) => (t, Some(q)),
        None => (s, None),
    };
    let ticket: BlobTicket = t.trim_end_matches('/').parse().context("неверный тикет")?;
    let mut size = None;
    let mut name = None;
    if let Some(q) = query {
        for kv in q.split('&') {
            match kv.split_once('=') {
                Some(("size", v)) => size = v.parse().ok(),
                Some(("name", v)) => name = urlencoding::decode(v).ok().map(|c| c.into_owned()),
                _ => {}
            }
        }
    }
    Ok(Link { ticket, size, name })
}
