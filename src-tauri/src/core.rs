//! Ядро: iroh-узел с постоянным ключом и постоянным хранилищем.
//! Постоянный ключ = ссылки не ломаются при смене сети/IP.
//! Постоянное хранилище = недокачанное продолжается после паузы/перезапуска.
//!
//! Что отдаётся в сеть (аудит P0-2, P1-9/11):
//! - `store/`     — раздачи пользователя. Router отдаёт из него ТОЛЬКО хеши активных раздач
//!                  (белый список `allow`: корень + всё внутри). Остановленная раздача или раздача,
//!                  чьи исходные файлы изменились, не отдаётся.
//! - `downloads/` — скачанное; к Router не подключено вовсе. Тег `dl-<root>` держит недокачанное
//!                  (докачка после паузы/перезапуска), после экспорта снимается — GC освобождает место.
//! Реестр раздач — `shares.json` рядом с ключом.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use futures_lite::StreamExt;
use iroh::{
    endpoint::{presets, BindOpts},
    protocol::Router,
    Endpoint, SecretKey,
};
use iroh_blobs::{
    api::{
        blobs::{AddPathOptions, ImportMode},
        downloader::{DownloadProgressItem, Shuffled},
    },
    format::collection::Collection,
    protocol::{ChunkRanges, ChunkRangesSeq, GetRequest},
    hashseq::HashSeq,
    provider::events::{AbortReason, EventMask, EventSender, ProviderMessage, RequestMode},
    store::{
        fs::{options::Options, FsStore},
        GcConfig,
    },
    ticket::BlobTicket,
    BlobFormat, BlobsProtocol, Hash, HashAndFormat,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};

use crate::once::{self, OnceRec, OnceTicket};
use crate::paths;

pub const SCHEME: &str = "tikdl://get/";
const SHARES_FILE: &str = "shares.json";

/// Ошибка-метка: загрузку остановил пользователь (пауза), это не сбой.
#[derive(Debug)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("остановлено пользователем")
    }
}
impl std::error::Error for Cancelled {}

/// Исходный файл раздачи: по размеру и времени изменения узнаём, что его поменяли.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SourceFile {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

impl SourceFile {
    fn read(path: &Path) -> Result<Self> {
        let m = std::fs::metadata(path).with_context(|| format!("нет доступа к {}", path.display()))?;
        Ok(Self { path: path.to_path_buf(), size: m.len(), mtime: mtime_secs(&m) })
    }
    fn unchanged(&self) -> bool {
        match std::fs::metadata(&self.path) {
            Ok(m) => m.len() == self.size && mtime_secs(&m) == self.mtime,
            Err(_) => false,
        }
    }
}

fn mtime_secs(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Запись реестра раздач (shares.json).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShareRecord {
    /// Record id: the root for a public share (as in older versions), `once-<16 hex>` for a one-time share —
    /// the same content can be shared both ways, so nothing is keyed by the root.
    #[serde(default)]
    pub id: String,
    pub root: String,
    pub name: String,
    pub source: PathBuf,
    pub files: Vec<SourceFile>,
    pub skipped: usize,
    pub total: u64,
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<OnceRec>,
}

impl ShareRecord {
    pub(crate) fn unchanged(&self) -> bool {
        self.files.iter().all(SourceFile::unchanged)
    }
}

/// Раздача для интерфейса.
#[derive(Serialize, Clone, Debug)]
pub struct Shared {
    pub id: String,
    /// "forbidden": why TikDL does not share this folder.
    pub why: Option<String>,
    pub root: String,
    pub name: String,
    pub source: String,
    pub link: String,
    /// https-ссылка для мессенджеров и почты (они не делают кликабельным tikdl://). Есть, если при
    /// сборке задан адрес страницы-прокладки TIKDL_LINK_BASE (GitHub Pages проекта).
    pub web_link: Option<String>,
    pub files: usize,
    /// Скрытые и служебные файлы, которые не вошли в раздачу папки (.git, .env, .DS_Store …).
    pub skipped: usize,
    pub total: u64,
    /// "active" — раздаётся; "changed" — исходные файлы изменились или пропали, не раздаётся;
    /// "forbidden" — папка из тех, что TikDL не раздаёт (раздача от старой версии), не раздаётся.
    pub state: String,
    /// One-time share: its stage, the bound device (8 hex) and when it stops by itself.
    pub once: Option<OnceView>,
}

#[derive(Serialize, Clone, Debug)]
pub struct OnceView {
    /// waiting | claimed | used | expired
    pub state: String,
    pub device: Option<String>,
    pub expires: u64,
}

pub struct Node {
    /// Раздачи пользователя. Только это хранилище подключено к Router (и только по белому списку).
    pub store: FsStore,
    /// Скачанное. Не подключено к Router — наружу не отдаётся.
    dl: FsStore,
    mem: iroh::address_lookup::MemoryLookup,
    pub router: Router,
    dir: PathBuf,
    /// Хеши, которые сеть может отдать: корни активных раздач и всё внутри них.
    allow: Arc<RwLock<HashSet<Hash>>>,
    shares: Arc<Mutex<Vec<ShareRecord>>>,
    once: Arc<once::Host>,
    sweeper: tokio::task::JoinHandle<()>,
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

pub(crate) async fn bind_endpoint(key: SecretKey, port: u16) -> Result<Endpoint> {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
    Ok(Endpoint::builder(presets::N0)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr_with_opts(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)), BindOpts::default())?
        .bind_addr_with_opts(
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
            BindOpts::default().set_is_required(false),
        )?
        .bind()
        .await?)
}

fn gc_interval() -> Duration {
    // TIKDL_GC_SECS — для проверок. По умолчанию раз в 15 с: внутренняя копия скачанного освобождается
    // почти сразу после выкладки в папку (там, где файловая система не умеет клонировать, она занимает место).
    let secs = std::env::var("TIKDL_GC_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15u64);
    Duration::from_secs(secs.max(1))
}

const STORE_BUSY: &str =
    "хранилище TikDL уже открыто другим запуском (окно приложения или консольная команда) — закройте его";

impl Node {
    pub async fn start(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let key = load_or_create_key(dir)?;
        let store = FsStore::load(dir.join("store")).await.context(STORE_BUSY)?;
        let dl_dir = dir.join("downloads");
        let mut opts = Options::new(&dl_dir);
        opts.gc = Some(GcConfig { interval: gc_interval(), add_protected: None });
        let dl = FsStore::load_with_opts(dl_dir.join("blobs.db"), opts)
            .await
            .context(STORE_BUSY)?;

        // Постоянный порт: прямые адреса из уже разосланных ссылок работают сразу после перезапуска,
        // не дожидаясь поиска узла через relay/DNS (без этого старые ссылки оживали только через ~20 с).
        let port_file = dir.join("port");
        let saved: u16 = std::fs::read_to_string(&port_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let endpoint = match bind_endpoint(key.clone(), saved).await {
            Ok(e) => e,
            Err(e) if saved != 0 => {
                eprintln!("порт {saved} занят ({e:#}), выбран другой — старые ссылки найдут узел чуть позже");
                bind_endpoint(key, 0).await?
            }
            Err(e) => return Err(e),
        };
        if let Some(p) = endpoint.bound_sockets().iter().find(|a| a.is_ipv4()).map(|a| a.port()) {
            let _ = std::fs::write(&port_file, p.to_string());
        }
        let mem = iroh::address_lookup::MemoryLookup::new();
        endpoint.address_lookup()?.add(mem.clone());

        // Каждый входящий запрос проверяется по белому списку; get_many и push запрещены.
        let allow: Arc<RwLock<HashSet<Hash>>> = Arc::default();
        let mask = EventMask {
            get: RequestMode::Intercept,
            get_many: RequestMode::Disabled,
            ..EventMask::DEFAULT
        };
        let (events, mut rx) = EventSender::channel(64, mask);
        let gate = allow.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let ProviderMessage::GetRequestReceived(msg) = msg {
                    let ok = gate
                        .read()
                        .map(|a| a.contains(&msg.inner.request.hash))
                        .unwrap_or(false);
                    let verdict = if ok { Ok(()) } else { Err(AbortReason::Permission) };
                    let _ = msg.tx.send(verdict).await;
                }
            }
        });
        let blobs = BlobsProtocol::new(&store, Some(events));
        let router = Router::builder(endpoint)
            .accept(iroh_blobs::ALPN, blobs)
            .spawn();

        let shares = Arc::new(Mutex::new(load_shares(dir)));
        let host = once::Host::new(store.clone(), dir.to_path_buf(), shares.clone());
        // One-time links: endpoints come up in the background; expiry is checked every minute.
        tokio::spawn(host.clone().start_all());
        let h = host.clone();
        let sweeper = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                h.sweep().await;
            }
        });
        let node = Self {
            store,
            dl,
            mem,
            router,
            dir: dir.to_path_buf(),
            allow,
            shares,
            once: host,
            sweeper,
        };
        node.refresh_allow().await;
        if let Err(e) = node.remove_orphans(&dl_dir.join("data")).await {
            eprintln!("не удалось убрать лишние файлы из хранилища загрузок: {e:#}");
        }
        Ok(node)
    }

    /// Корректное завершение: закрыть сеть и записать базы хранилищ на диск.
    /// Без этого последняя пачка изменений базы (до 1 с) теряется, а файлы остаются сиротами.
    pub async fn shutdown(&self) {
        self.sweeper.abort();
        self.once.sweep().await; // saves the last-activity marks
        self.once.stop_all().await; // before the store: their handlers serve from it
        let _ = self.router.shutdown().await;
        let _ = self.store.shutdown().await;
        let _ = self.dl.shutdown().await;
    }

    /// Файлы в `downloads/data`, о которых не знает база (след аварийного выхода), занимают место
    /// навсегда — GC их не видит. Вызывается только при старте, до любых загрузок.
    async fn remove_orphans(&self, data: &Path) -> Result<()> {
        let Ok(dir) = std::fs::read_dir(data) else { return Ok(()) };
        let mut known = HashSet::new();
        let mut list = self.dl.blobs().list().stream().await?;
        while let Some(h) = list.next().await {
            known.insert(h?.to_string());
        }
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some((stem, _)) = name.split_once('.') else { continue };
            if stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()) && !known.contains(stem) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        Ok(())
    }

    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }


    /// Раздача из папки, которую TikDL не раздаёт (могла остаться от версии до запрета): не отдаётся.
    /// Вторая проверка — файлы из папки TikDL, попавшие в раздачу старой версией.
    fn forbidden(&self, r: &ShareRecord) -> bool {
        self.forbidden_why(r).is_some()
    }

    /// Why an old record is not served (the precise reason for its row).
    fn forbidden_why(&self, r: &ShareRecord) -> Option<String> {
        if let Some(why) = paths::forbidden_share(&r.source, dirs::home_dir().as_deref(), &self.dir) {
            return Some(why.to_string());
        }
        r.files
            .iter()
            .any(|f| f.path.starts_with(&self.dir))
            .then(|| "в раздаче есть файлы TikDL (ключ узла или хранилище)".to_string())
    }

    /// Пересобрать белый список из реестра: только раздачи с неизменёнными исходными файлами
    /// и не из запрещённых папок.
    async fn refresh_allow(&self) {
        let records = self.shares.lock().await.clone();
        let mut set = HashSet::new();
        // One-time shares are never served by the node's own id: only their throwaway endpoints serve them.
        for r in records.iter().filter(|r| r.once.is_none() && r.unchanged() && !self.forbidden(r)) {
            let Ok(root) = r.root.parse::<Hash>() else { continue };
            match members(&self.store, root).await {
                Ok(m) => set.extend(m),
                Err(e) => eprintln!("раздача {} недоступна: {e:#}", r.name),
            }
        }
        if let Ok(mut a) = self.allow.write() {
            *a = set;
        }
    }

    fn view(&self, r: &ShareRecord) -> Shared {
        let why = self.forbidden_why(r);
        let state = if why.is_some() {
            "forbidden"
        } else if r.unchanged() {
            "active"
        } else {
            "changed"
        };
        let link = match &r.once {
            None => r
                .root
                .parse::<Hash>()
                .map(|root| make_link(&self.endpoint().addr(), root, r.total, &r.name))
                .unwrap_or_default(),
            // The link is handed out only while nobody has claimed it; after that it is useless to anyone else.
            Some(o) => match (&o.token, self.once.addr(&r.id)) {
                (Some(t), Some(addr)) if o.state == "waiting" => data_encoding::HEXLOWER
                    .decode(t.as_bytes())
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                    .map(|token| format!("{SCHEME}{}", OnceTicket { addr, token }.encode()))
                    .unwrap_or_default(),
                _ => String::new(),
            },
        };
        let once = r.once.as_ref().map(|o| OnceView {
            state: o.state.clone(),
            device: o.bound.as_deref().map(once::short),
            expires: o.expires(r.created),
        });
        Shared {
            id: r.id.clone(),
            why,
            once,
            root: r.root.clone(),
            name: r.name.clone(),
            source: r.source.to_string_lossy().to_string(),
            web_link: web_link(&link),
            link,
            files: r.files.len(),
            skipped: r.skipped,
            total: r.total,
            state: state.into(),
        }
    }

    /// Список раздач (заодно перепроверяет исходные файлы и белый список).
    /// Changes whenever a one-time share changes stage or its link becomes available.
    pub fn watch_once(&self) -> tokio::sync::watch::Receiver<u64> {
        self.once.changed.subscribe()
    }

    pub async fn list_shares(&self) -> Vec<Shared> {
        self.refresh_allow().await;
        let records = self.shares.lock().await.clone();
        records.iter().rev().map(|r| self.view(r)).collect()
    }

    /// Остановить раздачу: убрать из белого списка, реестра и снять тег.
    pub async fn stop_share(&self, id: &str) -> Result<()> {
        {
            let mut shares = self.shares.lock().await;
            let before = shares.len();
            shares.retain(|r| r.id != id);
            if shares.len() == before {
                bail!("такой раздачи нет");
            }
            save_shares(&self.dir, &shares)?;
        }
        self.once.stop(id).await;
        self.refresh_allow().await;
        self.store.tags().delete(format!("share-{id}")).await?;
        Ok(())
    }

    /// One-time share: a new link for the same files (the old one, claimed or not, stops working at once).
    /// For "the wrong device opened it". No re-hashing.
    pub async fn renew_once(&self, id: &str) -> Result<Shared> {
        self.once.stop(id).await; // frees its place in the limit before the new endpoint comes up
        let rec = {
            let mut shares = self.shares.lock().await;
            let r = shares.iter_mut().find(|r| r.id == id).ok_or_else(|| anyhow!("такой раздачи нет"))?;
            if r.once.is_none() {
                bail!("это не одноразовая раздача");
            }
            once_room(&shares, id)?;
            let r = shares.iter_mut().find(|r| r.id == id).ok_or_else(|| anyhow!("такой раздачи нет"))?;
            r.once = Some(OnceRec::new().0);
            r.created = now_secs();
            let rec = r.clone();
            save_shares(&self.dir, &shares)?;
            rec
        };
        self.once.spawn(&rec).await?;
        self.wait_online(&rec).await;
        Ok(self.view(&rec))
    }

    async fn wait_online(&self, r: &ShareRecord) {
        // До 5 с ждём связи с relay, чтобы в ссылку попал адрес посредника (у одноразовой без него ссылки нет).
        let ep = match &r.once {
            Some(_) => None,
            None => Some(self.endpoint().clone()),
        };
        let fut = async {
            match ep {
                Some(ep) => {
                    ep.online().await;
                }
                None => {
                    while self.once.running(&r.id) && self.once.addr(&r.id).is_none() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), fut).await;
    }

    /// Раздать файл или папку. В папке скрытые и служебные файлы пропускаются (аудит P0-3);
    /// явно выбранный файл раздаётся всегда. on_progress(готово_файлов, всего_файлов).
    /// `one_time` — a one-time link: one device, burned once received (once.rs).
    pub async fn share(&self, path: &Path, one_time: bool, mut on_progress: impl FnMut(usize, usize)) -> Result<Shared> {
        // Системный диск и папки, домашняя папка целиком, ключи и настройки программ, папка TikDL —
        // не раздаются никогда (одна точка для окна и консоли).
        if let Some(why) = paths::forbidden_share(path, dirs::home_dir().as_deref(), &self.dir) {
            bail!("{why}");
        }
        if one_time {
            once_room(&self.shares.lock().await, "")?; // fail fast, before hashing; re-checked under the lock below
        }
        let path = std::path::absolute(path)?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "files".into());

        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let mut skipped = 0usize;
        if path.is_file() {
            // an explicitly chosen link to a file: share the file itself (checked above by its real place)
            files.push((name.clone(), std::fs::canonicalize(&path)?));
        } else if path.is_dir() {
            // TikDL's own folder and the targets of hidden entries of home folders (~/.ssh → ~/Dropbox/ssh) never
            // leave the machine, even inside a shared parent. Compared by identity, folders only.
            let prune = paths::forbidden_ids(dirs::home_dir().as_deref(), &self.dir);
            let walk = walkdir::WalkDir::new(&path).follow_links(false).into_iter();
            for e in walk.filter_entry(|e| {
                !e.file_type().is_dir() || paths::file_id(e.path()).is_none_or(|id| !prune.contains(&id))
            }) {
                // a folder the user cannot read (system folders on a disk root) is skipped, not fatal
                let e = match e {
                    Ok(e) => e,
                    Err(err) if err.io_error().is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied) => {
                        skipped += 1;
                        continue;
                    }
                    Err(err) => return Err(err.into()),
                };
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
        let mut sources = Vec::new();
        let mut total: u64 = 0;
        let mut keep = Vec::new();
        on_progress(0, count);
        for (i, (rel, p)) in files.into_iter().enumerate() {
            // The file must still be a regular file, not a link swapped in after the walk (it could point at keys).
            if !std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
                bail!("файл {} подменили во время раздачи — раздайте заново", p.display());
            }
            // Снимок до хеширования: если файл поменяют во время/после — раздача станет "changed".
            let src = SourceFile::read(&p)?;
            total += src.size;
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
            sources.push(src);
            on_progress(i + 1, count);
        }
        let collection: Collection = items.into_iter().collect();
        let tt = collection.store(&self.store).await?;
        let root = tt.hash();
        let (id, once) = if one_time {
            let id = format!("{}-{}", once::PREFIX, &Hash::new(SecretKey::generate().to_bytes()).to_hex()[..16]);
            (id, Some(OnceRec::new().0))
        } else {
            (root.to_string(), None)
        };
        // Постоянный тег, чтобы раздача пережила перезапуск.
        self.store
            .tags()
            .set(format!("share-{id}"), tt.hash_and_format())
            .await?;
        self.store.sync_db().await?;

        let record = ShareRecord {
            id,
            once,
            root: root.to_string(),
            name,
            source: path,
            files: sources,
            skipped,
            total,
            created: now_secs(),
        };
        {
            let mut shares = self.shares.lock().await;
            if record.once.is_some() {
                if let Err(e) = once_room(&shares, "") {
                    drop(shares);
                    let _ = self.store.tags().delete(format!("share-{}", record.id)).await;
                    return Err(e);
                }
            }
            shares.retain(|r| r.id != record.id);
            shares.push(record.clone());
            save_shares(&self.dir, &shares)?;
        }
        if record.once.is_some() {
            self.once.spawn(&record).await?;
        } else {
            self.refresh_allow().await;
        }
        self.wait_online(&record).await;
        Ok(self.view(&record))
    }

    /// Состав раздачи у самого раздающего: имена и размеры файлов, общий размер. Нужен, когда ссылка
    /// не несёт размер (иначе полоса не может показать долю) и по кнопке «Что внутри?» до скачивания.
    /// Тянет только корень и список имён (их потом использует и сама загрузка) и последний кусок
    /// каждого файла — для проверенных размеров. Список имён больше 16 МБ не берём (чужой отправитель).
    pub async fn peek(&self, ticket: &BlobTicket) -> Result<Peek> {
        const MAX_META: u64 = 16 * 1024 * 1024;
        let root = ticket.hash();
        let addr = ticket.addr().clone();
        self.mem.add_endpoint_info(addr.clone());
        let fut = async {
            let conn = self.endpoint().connect(addr.id, iroh_blobs::ALPN).await?;
            let (hs, sizes) = iroh_blobs::get::request::get_hash_seq_and_sizes(&conn, &root, MAX_META, None)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            let meta_size = sizes.first().copied().unwrap_or(0);
            if meta_size > MAX_META {
                bail!("список файлов раздачи слишком большой ({meta_size} байт)");
            }
            let total = hs.len() as u64 * 32 + sizes.iter().sum::<u64>();
            let req = GetRequest::new(root, ChunkRangesSeq::from_ranges([ChunkRanges::all(), ChunkRanges::all()]));
            self.dl.remote().execute_get(conn.clone(), req).await.map_err(|e| anyhow!("{e}"))?;
            let collection = Collection::load(root, self.dl.as_ref()).await?;
            let files: Vec<(String, u64)> = collection
                .iter()
                .zip(sizes.iter().skip(1))
                .map(|((n, _), sz)| (n.clone(), *sz))
                .collect();
            let name = files.first().map(|(n, _)| n.split('/').next().unwrap_or(n).to_string());
            Ok::<_, anyhow::Error>(Peek { total, name, files })
        };
        tokio::time::timeout(Duration::from_secs(15), fut)
            .await
            .map_err(|_| anyhow!("раздающий не ответил за 15 с"))?
    }

    /// Хватит ли места: в хранилище загрузок — на недокачанное, в папке назначения — на всё.
    pub async fn check_space(&self, root: Hash, total: u64, dest: &Path) -> Result<()> {
        let local = self
            .dl
            .remote()
            .local(HashAndFormat::hash_seq(root))
            .await
            .map(|l| l.local_bytes())
            .unwrap_or(0);
        let remaining = total.saturating_sub(local);
        // запас: дерево проверки (~0.4 %) и служебные файлы
        let margin = |n: u64| n / 100 + 64 * 1024 * 1024;
        let store_buf = self.dir.join("downloads");
        let store_dir = store_buf.as_path();
        let dest_dir = existing_ancestor(dest);
        let check = |dir: &Path, need: u64| -> Result<()> {
            let Ok(free) = fs4::available_space(dir) else { return Ok(()) };
            if free < need {
                bail!(
                    "не хватает места на диске ({}): нужно {}, свободно {}",
                    dir.display(),
                    human(need),
                    human(free)
                );
            }
            Ok(())
        };
        if same_volume(store_dir, &dest_dir) {
            // Выкладка в папку — клон блоков там, где ФС это умеет (APFS на macOS); иначе на время
            // выкладки нужна вторая копия (до чистки через ~15 с).
            let export_extra = if cfg!(target_os = "macos") { 0 } else { total };
            check(&dest_dir, remaining + export_extra + margin(total))
        } else {
            check(store_dir, remaining + margin(remaining))?;
            check(&dest_dir, total + margin(0))
        }
    }

    /// Скачать по тикету в папку dest. on_progress(скачано_байт, включая скачанное раньше).
    /// `cancel` = true останавливает сетевую часть; экспорт на диск не прерывается.
    /// `id` — the download record (tag `dl-<id>`); the tag stays until `forget_download` (after the caller has
    /// recorded what it needs, e.g. the pending "done" of a one-time link).
    pub async fn download(
        &self,
        ticket: &BlobTicket,
        id: &str,
        dest: &Path,
        mut cancel: watch::Receiver<bool>,
        mut on_progress: impl FnMut(u64),
    ) -> Result<PathBuf> {
        let root = ticket.hash();
        let req = HashAndFormat::hash_seq(root);
        let tag = format!("dl-{id}");
        // Тег ДО скачивания: GC не тронет недокачанное, докачка после паузы и перезапуска.
        self.dl.tags().set(tag.clone(), req).await?;
        self.dl.sync_db().await?;

        // Сообщаем узлу известные адреса раздающего из тикета.
        let addr = ticket.addr().clone();
        self.mem.add_endpoint_info(addr.clone());
        let providers = Shuffled::new(vec![addr.id]);
        // The pool's default connect timeout (1 s) is too short for a first connection through a relay or
        // a lookup by id; a slow first connect then failed the whole download with "Unable to download".
        let mut pool = iroh_blobs::util::connection_pool::Options::default();
        pool.connect_timeout = Duration::from_secs(15);
        let mut stream = iroh_blobs::api::downloader::Downloader::builder(&self.dl, self.endpoint())
            .pool_options(pool)
            .build()
            .download(req, providers)
            .stream()
            .await?;
        loop {
            let item = tokio::select! {
                item = stream.next() => item,
                _ = cancel.wait_for(|c| *c) => return Err(Cancelled.into()),
            };
            match item {
                None => break,
                Some(DownloadProgressItem::Progress(n)) => on_progress(n),
                Some(DownloadProgressItem::Error(e)) => bail!(
                    "не удалось скачать: раздача остановлена, её файлы изменились или раздающий не в сети ({e})"
                ),
                Some(DownloadProgressItem::DownloadError) => {
                    bail!("не удалось скачать: раздающий недоступен, прервал связь или раздача остановлена")
                }
                Some(_) => {}
            }
        }
        drop(stream);

        let collection = Collection::load(root, self.dl.as_ref()).await?;

        // 1) Проверяем все имена до записи на диск: чужая коллекция не должна выйти за dest.
        //    Группируем по верхнему уровню (файл или папка), порядок сохраняем.
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<(Option<String>, Hash)>> = HashMap::new();
        for (name, hash) in collection.iter() {
            paths::safe_join(dest, name).map_err(|e| anyhow!(e))?;
            let (top, rest) = match name.split_once('/') {
                Some((t, r)) => (t.to_string(), Some(r.to_string())),
                None => (name.to_string(), None),
            };
            if !groups.contains_key(&top) {
                order.push(top.clone());
            }
            groups.entry(top).or_default().push((rest, *hash));
        }

        // 2) Каждый верхний элемент пишется во временное скрытое имя и переименовывается целиком:
        //    в dest не бывает полузаписанных файлов, существующее не перезаписывается (аудит P0-6, P1-12).
        let mut first: Option<PathBuf> = None;
        for top in order {
            let entries = groups.remove(&top).unwrap_or_default();
            let is_dir = entries.iter().any(|(rest, _)| rest.is_some());
            let final_name = paths::unique_name(dest, &top, is_dir);
            let part_name = paths::part_name(&final_name);
            let part = paths::safe_join(dest, &part_name).map_err(|e| anyhow!(e))?;
            remove_any(&part);
            let res: Result<()> = async {
                for (rest, hash) in entries {
                    let target = match rest {
                        Some(r) => paths::safe_join(&part, &r).map_err(|e| anyhow!(e))?,
                        None => part.clone(),
                    };
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    // Копия клонированием (reflink): на APFS, Btrfs, XFS, ReFS блоки общие — место не
                    // удваивается. Перенос (TryReference) не годится: хранилище помнит файл по пути, а
                    // пользователь его переименует или удалит — и повторная загрузка ломается.
                    // Внутренняя копия освобождается чисткой через ~15 с после снятия тега.
                    self.dl.blobs().export(hash, &target).await?;
                }
                Ok(())
            }
            .await;
            if let Err(e) = res {
                remove_any(&part);
                return Err(e);
            }
            let final_path = dest.join(&final_name);
            std::fs::rename(&part, &final_path)
                .with_context(|| format!("не удалось переименовать в {}", final_path.display()))?;
            first.get_or_insert(final_path);
        }

        Ok(first.unwrap_or_else(|| dest.to_path_buf()))
    }

    /// Снять тег загрузки: скачанное уже на диске или загрузку отменили — GC освободит внутреннюю копию
    /// (аудит P1-10).
    pub async fn forget_download(&self, id: &str) -> Result<()> {
        self.dl.tags().delete(format!("dl-{id}")).await?;
        self.dl.sync_db().await?;
        Ok(())
    }

    /// Ticket to download by: as is for a regular link; for a one-time link the claim binds this device first
    /// and tells the root.
    pub async fn resolve(&self, link: &Link) -> Result<BlobTicket> {
        match link {
            Link::Blob { ticket, .. } => Ok(ticket.clone()),
            Link::Once(t) => {
                if self.once.is_mine(t.addr.id) {
                    bail!("это ваша собственная одноразовая ссылка — откройте её на том устройстве, которому она предназначена");
                }
                self.mem.add_endpoint_info(t.addr.clone());
                let root = once::claim(self.endpoint(), t).await?;
                Ok(BlobTicket::new(t.addr.clone(), root, BlobFormat::HashSeq))
            }
        }
    }

    /// One-time link: everything is on disk — the sender burns the link.
    pub async fn once_done(&self, t: &OnceTicket) -> Result<()> {
        self.mem.add_endpoint_info(t.addr.clone());
        once::done(self.endpoint(), t).await
    }

    /// This device's id as shown next to one-time links (8 hex), to compare with the sender's row.
    pub fn short_id(&self) -> String {
        once::short(&self.endpoint().id().to_string())
    }
}

fn remove_any(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
    let _ = std::fs::remove_file(p);
}

/// Адрес для ссылки: ID узла и relay, без IP-адресов — ссылку пересылают кому угодно, а IP раздающего
/// не должен уходить вместе с ней. Прямое соединение iroh всё равно наладит через relay.
/// Без relay (нет интернета) остаётся только полный адрес, иначе по ссылке не найти.
/// Room for one more live one-time link (not counting `except` — the record being renewed).
fn once_room(shares: &[ShareRecord], except: &str) -> Result<()> {
    let live = shares.iter().filter(|r| r.id != except && r.once.as_ref().is_some_and(|o| o.live())).count();
    if live >= once::MAX_LIVE {
        bail!("одновременно может действовать не больше {} одноразовых ссылок — остановите ненужные", once::MAX_LIVE);
    }
    Ok(())
}

pub(crate) fn public_addr(addr: &iroh::EndpointAddr) -> iroh::EndpointAddr {
    let relays: Vec<_> = addr.addrs.iter().filter(|a| a.is_relay()).cloned().collect();
    if relays.is_empty() {
        addr.clone()
    } else {
        iroh::EndpointAddr::from_parts(addr.id, relays)
    }
}

/// https-ссылка на страницу-прокладку: тикет после «#» — браузер не отправляет его на сервер.
pub fn web_link(link: &str) -> Option<String> {
    let base = option_env!("TIKDL_LINK_BASE").filter(|b| !b.is_empty())?;
    Some(format!("{base}#{}", link.strip_prefix(SCHEME).unwrap_or(link)))
}

fn make_link(addr: &iroh::EndpointAddr, root: Hash, total: u64, name: &str) -> String {
    let ticket = BlobTicket::new(public_addr(addr), root, BlobFormat::HashSeq);
    format!("{SCHEME}{ticket}?size={total}&name={}", urlencoding::encode(name))
}

fn load_shares(dir: &Path) -> Vec<ShareRecord> {
    let path = dir.join(SHARES_FILE);
    let mut list: Vec<ShareRecord> = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            eprintln!("{} повреждён ({e}), список раздач начат заново", path.display());
            Vec::new()
        }),
        Err(_) => Vec::new(),
    };
    // records from older versions: id = root
    for r in list.iter_mut().filter(|r| r.id.is_empty()) {
        r.id = r.root.clone();
    }
    list
}

/// Корень раздачи и все хеши внутри (метаданные коллекции + файлы).
pub(crate) async fn members(store: &FsStore, root: Hash) -> Result<Vec<Hash>> {
    let bytes = store.blobs().get_bytes(root).await?;
    let seq = HashSeq::try_from(bytes)?;
    let mut v = vec![root];
    v.extend(seq.iter());
    Ok(v)
}

pub(crate) fn save_shares(dir: &Path, shares: &[ShareRecord]) -> Result<()> {
    let json = serde_json::to_vec_pretty(shares)?;
    paths::write_private(&dir.join(SHARES_FILE), &json).context("не удалось сохранить список раздач")
}

/// Состав раздачи, узнанный у раздающего.
#[derive(Serialize, Clone)]
pub struct Peek {
    pub total: u64,
    pub name: Option<String>,
    pub files: Vec<(String, u64)>,
}

/// Свободное место на диске, где лежит (или будет лежать) путь.
pub fn free_space(p: &Path) -> Option<u64> {
    fs4::available_space(existing_ancestor(p)).ok()
}

fn human(n: u64) -> String {
    let units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} Б") } else { format!("{v:.1} {}", units[i]) }
}

/// Папка назначения может ещё не существовать — место считаем по ближайшей существующей.
fn existing_ancestor(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    while !cur.exists() {
        match cur.parent() {
            Some(parent) => cur = parent.to_path_buf(),
            None => break,
        }
    }
    cur
}

fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_ascii_lowercase());
        root(a) == root(b)
    }
}

/// Разбор ссылки tikdl://get/<тикет>?size=..&name=.., «голого» тикета (в т.ч. от sendme) или одноразовой
/// tikdl://get/once<…> (в ней нет ни имени, ни размера, ни хеша).
pub enum Link {
    Blob { ticket: BlobTicket, size: Option<u64>, name: Option<String> },
    Once(OnceTicket),
}

impl Link {
    /// Id of the download record: the root for a regular link (as in older versions), `once-…` for a one-time one.
    pub fn record_id(&self) -> String {
        match self {
            Link::Blob { ticket, .. } => ticket.hash().to_string(),
            Link::Once(t) => t.record_id(),
        }
    }
    pub fn size(&self) -> Option<u64> {
        match self {
            Link::Blob { size, .. } => *size,
            Link::Once(_) => None,
        }
    }
    pub fn name(&self) -> Option<String> {
        match self {
            Link::Blob { name, .. } => name.clone(),
            Link::Once(_) => None,
        }
    }
    /// Sender's endpoint (for a one-time link — its throwaway one).
    pub fn from(&self) -> String {
        match self {
            Link::Blob { ticket, .. } => ticket.addr().id.to_string(),
            Link::Once(t) => t.addr.id.to_string(),
        }
    }
    pub fn once(&self) -> Option<&OnceTicket> {
        match self {
            Link::Once(t) => Some(t),
            Link::Blob { .. } => None,
        }
    }
}

pub fn parse_link(s: &str) -> Result<Link> {
    let s = s.trim();
    let s = s.strip_prefix(SCHEME).unwrap_or(s);
    if s.starts_with(once::PREFIX) {
        return Ok(Link::Once(OnceTicket::decode(s.trim_end_matches('/'))?));
    }
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
    Ok(Link::Blob { ticket, size, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GC on the downloads store frees a blob once its tag is removed (audit P1-10).
    #[tokio::test]
    async fn gc_frees_untagged_download() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("tikdl-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut opts = Options::new(&dir);
        opts.gc = Some(GcConfig { interval: Duration::from_millis(300), add_protected: None });
        let store = FsStore::load_with_opts(dir.join("blobs.db"), opts).await?;
        let data = vec![7u8; 3_000_000];
        let tt = store.add_bytes(data).temp_tag().await?;
        let hash = tt.hash();
        store.tags().set("dl-x", tt.hash_and_format()).await?;
        drop(tt);
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(store.blobs().has(hash).await?, "tagged blob must survive GC");
        store.tags().delete("dl-x").await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let present = store.blobs().has(hash).await?;
        let file = dir.join("data").join(format!("{}.data", hash.to_hex()));
        eprintln!("after untag: has={present} file_exists={}", file.exists());
        assert!(!present, "untagged blob must be collected");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Address over localhost: tests must not depend on the internet (relay, DNS).
    pub(super) fn local_addr(ep: &Endpoint) -> iroh::EndpointAddr {
        let port = ep.bound_sockets().iter().find(|a| a.is_ipv4()).map(|a| a.port()).expect("ipv4 socket");
        iroh::EndpointAddr::new(ep.id()).with_ip_addr(std::net::SocketAddr::from(([127, 0, 0, 1], port)))
    }

    /// Fresh temp folder per test (several tests run in parallel in one process).
    pub(super) fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tikdl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    pub(super) fn sample(path: &Path, len: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(path, &data).expect("sample file");
        data
    }

    /// Two nodes in one process: a share is downloaded byte for byte; the main node refuses a hash outside
    /// its shares.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn share_and_download_between_two_nodes() -> Result<()> {
        let base = temp("two-nodes");
        let file = base.join("hello.bin");
        let data = sample(&file, 1_500_000);
        let a = Node::start(&base.join("a")).await?;
        let b = Node::start(&base.join("b")).await?;
        let shared = a.share(&file, false, |_, _| {}).await?;
        let root: Hash = shared.root.parse()?;
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest)?;
        let ticket = BlobTicket::new(local_addr(a.endpoint()), root, BlobFormat::HashSeq);
        let (_keep, never) = watch::channel(false);
        let out = tokio::time::timeout(Duration::from_secs(60), b.download(&ticket, &root.to_string(), &dest, never.clone(), |_| {})).await??;
        assert_eq!(std::fs::read(&out)?, data, "downloaded file must match");

        let stranger = Hash::new(b"not shared");
        let bad = BlobTicket::new(local_addr(a.endpoint()), stranger, BlobFormat::HashSeq);
        assert!(b.download(&bad, "bad", &dest, never, |_| {}).await.is_err(), "hash outside shares must be refused");
        a.shutdown().await;
        b.shutdown().await;
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    /// One-time share of `file` on `a`; returns its record id and a ticket with a localhost address.
    async fn once_share(a: &Node, file: &Path) -> Result<(String, OnceTicket)> {
        let sh = a.share(file, true, |_, _| {}).await?;
        let rec = a.shares.lock().await.iter().find(|r| r.id == sh.id).cloned().expect("record");
        let token = data_encoding::HEXLOWER.decode(rec.once.as_ref().unwrap().token.as_ref().unwrap().as_bytes())?;
        let ep = a.once.endpoint(&sh.id).expect("throwaway endpoint");
        Ok((sh.id, OnceTicket { addr: local_addr(&ep), token: token.try_into().unwrap() }))
    }

    async fn once_state(a: &Node, id: &str) -> String {
        a.shares.lock().await.iter().find(|r| r.id == id).and_then(|r| r.once.clone()).map(|o| o.state).unwrap_or_default()
    }

    /// The whole life of a one-time link: B gets the file byte for byte, C is refused while B holds it,
    /// "done" burns it (endpoint gone, key wiped), nobody gets it afterwards; the node's own id never serves it;
    /// the shared store survives the throwaway router's shutdown.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn once_link_one_device_then_burned() -> Result<()> {
        let base = temp("once-life");
        let file = base.join("secret.bin");
        let data = sample(&file, 1_200_000);
        let a = Node::start(&base.join("a")).await?;
        let b = Node::start(&base.join("b")).await?;
        let c = Node::start(&base.join("c")).await?;
        let (id, t) = once_share(&a, &file).await?;
        let link = parse_link(&format!("{SCHEME}{}", t.encode()))?;
        assert!(link.once().is_some(), "once link parses");
        assert_eq!(once_state(&a, &id).await, "waiting");

        // the node's own endpoint does not serve the one-time root, even to someone who knows it
        let root: Hash = a.shares.lock().await.iter().find(|r| r.id == id).unwrap().root.parse()?;
        let direct = BlobTicket::new(local_addr(a.endpoint()), root, BlobFormat::HashSeq);
        assert!(b.peek(&direct).await.is_err(), "main id must not serve a one-time share");

        // a wrong token does not bind the link
        let wrong = OnceTicket { addr: t.addr.clone(), token: [9u8; 32] };
        assert!(c.resolve(&Link::Once(wrong)).await.is_err(), "wrong token refused");
        assert_eq!(once_state(&a, &id).await, "waiting", "wrong token must not bind");
        let mut changes = a.watch_once();
        changes.mark_unchanged();

        let ticket = b.resolve(&link).await?;
        assert_eq!(ticket.hash(), root);
        assert_eq!(once_state(&a, &id).await, "claimed");
        assert!(changes.has_changed()?, "the window must be told about the claim");
        changes.mark_unchanged();
        let err = c.resolve(&link).await.err().map(|e| format!("{e:#}")).unwrap_or_default();
        assert!(err.contains("другом устройстве"), "second device refused while claimed: {err}");
        let c_ticket = BlobTicket::new(t.addr.clone(), root, BlobFormat::HashSeq);
        assert!(c.peek(&c_ticket).await.is_err(), "gate refuses an unbound device");

        let dest = base.join("dest");
        std::fs::create_dir_all(&dest)?;
        let (_k, never) = watch::channel(false);
        let out = b.download(&ticket, &link.record_id(), &dest, never, |_| {}).await?;
        assert_eq!(std::fs::read(&out)?, data, "byte-identical");
        b.forget_download(&link.record_id()).await?;
        b.once_done(link.once().unwrap()).await?;
        assert_eq!(once_state(&a, &id).await, "used");
        assert!(changes.has_changed()?, "the window must be told about the burn");
        let rec = a.shares.lock().await.iter().find(|r| r.id == id).cloned().unwrap();
        assert!(rec.once.as_ref().unwrap().key.is_none() && rec.once.as_ref().unwrap().token.is_none(), "secrets wiped");
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(a.once.endpoint(&id).is_none(), "throwaway endpoint shut down");
        assert!(a.store.blobs().has(root).await?, "shared store alive after the throwaway router shut down");

        let t0 = std::time::Instant::now();
        let rc = c.resolve(&link).await;
        eprintln!("after burn, C: {:?} in {:?}", rc.as_ref().err().map(|e| format!("{e:#}")), t0.elapsed());
        assert!(rc.is_err(), "nobody after the burn");
        let rb = b.resolve(&link).await;
        eprintln!("after burn, B: {:?} in {:?}", rb.as_ref().err().map(|e| format!("{e:#}")), t0.elapsed());
        assert!(rb.is_err(), "not even the same device");
        assert!(t0.elapsed() < Duration::from_secs(50), "refusal within the timeouts");
        for n in [a, b, c] {
            n.shutdown().await;
        }
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    /// Two devices claim at the same moment: exactly one wins.
    /// Critic review 2026-11-25: own link refused; «Новая ссылка» gets a new endpoint that a late spawn of the old
    /// record cannot replace; after stop a late spawn brings nothing back; an expired link cannot be claimed even
    /// before the sweeper runs; the 11th live link is refused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn once_link_renew_stale_spawn_expiry_limit() -> Result<()> {
        let base = temp("once-edges");
        let file = base.join("f.bin");
        sample(&file, 20_000);
        let a = Node::start(&base.join("a")).await?;
        let b = Node::start(&base.join("b")).await?;

        let (id, t) = once_share(&a, &file).await?;
        let err = format!("{:#}", a.resolve(&Link::Once(t.clone())).await.unwrap_err());
        assert!(err.contains("собственная"), "own link: {err}");

        let old = a.shares.lock().await.iter().find(|r| r.id == id).cloned().expect("record");
        a.renew_once(&id).await?;
        let new_ep = a.once.endpoint(&id).expect("renewed endpoint").id();
        assert_ne!(new_ep, t.addr.id, "new link, new endpoint");
        a.once.spawn(&old).await?; // late start_all with the old record
        assert_eq!(a.once.endpoint(&id).expect("still up").id(), new_ep, "stale spawn must not replace the new link");

        let rec = a.shares.lock().await.iter().find(|r| r.id == id).cloned().expect("record");
        let token = data_encoding::HEXLOWER.decode(rec.once.as_ref().unwrap().token.as_ref().unwrap().as_bytes())?;
        let nt = OnceTicket { addr: local_addr(&a.once.endpoint(&id).unwrap()), token: token.try_into().unwrap() };
        b.resolve(&Link::Once(nt)).await?; // the new link works

        a.stop_share(&id).await?;
        a.once.spawn(&rec).await?; // late spawn after stop
        assert!(a.once.endpoint(&id).is_none(), "no zombie endpoint after stop");

        // expired but not yet swept
        let (id2, t2) = once_share(&a, &file).await?;
        if let Some(r) = a.shares.lock().await.iter_mut().find(|r| r.id == id2) {
            r.created = 0;
        }
        let err = format!("{:#}", b.resolve(&Link::Once(t2)).await.unwrap_err());
        assert!(err.contains("больше не действует"), "expired: {err}");
        a.stop_share(&id2).await?;

        // limit
        for i in 0..once::MAX_LIVE {
            let f = base.join(format!("l{i}.bin"));
            sample(&f, 1000 + i);
            a.share(&f, true, |_, _| {}).await?;
        }
        let f = base.join("over.bin");
        sample(&f, 999);
        let err = format!("{:#}", a.share(&f, true, |_, _| {}).await.unwrap_err());
        assert!(err.contains("не больше"), "11th link: {err}");

        for n in [a, b] {
            n.shutdown().await;
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn once_link_concurrent_claims_one_winner() -> Result<()> {
        let base = temp("once-race");
        let file = base.join("f.bin");
        sample(&file, 10_000);
        let a = Node::start(&base.join("a")).await?;
        let b = Node::start(&base.join("b")).await?;
        let c = Node::start(&base.join("c")).await?;
        let (_id, t) = once_share(&a, &file).await?;
        let (lb, lc) = (Link::Once(t.clone()), Link::Once(t));
        let (rb, rc) = tokio::join!(b.resolve(&lb), c.resolve(&lc));
        assert_eq!(rb.is_ok() as u8 + rc.is_ok() as u8, 1, "exactly one claim wins");
        for n in [a, b, c] {
            n.shutdown().await;
        }
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    /// Restart of the sender while claimed: the binding is still there (only B), B resumes; a public share of the
    /// same file is independent of the one-time one (stopping it does not touch the one-time record).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn once_link_survives_sender_restart_and_public_share() -> Result<()> {
        let base = temp("once-restart");
        let file = base.join("f.bin");
        let data = sample(&file, 800_000);
        let a = Node::start(&base.join("a")).await?;
        let b = Node::start(&base.join("b")).await?;
        let c = Node::start(&base.join("c")).await?;
        let (id, t) = once_share(&a, &file).await?;
        eprintln!("restart: endpoint before public share: {:?}", a.once.endpoint(&id).map(|e| e.id()));
        let public = a.share(&file, false, |_, _| {}).await?;
        assert_ne!(public.id, id);
        eprintln!("restart: endpoint after public share: {:?} state {}", a.once.endpoint(&id).map(|e| e.id()), once_state(&a, &id).await);
        let r = b.resolve(&Link::Once(t.clone())).await;
        eprintln!("restart: first claim {:?}", r.as_ref().err().map(|e| format!("{e:#}")));
        r?;
        a.stop_share(&public.id).await?;
        assert_eq!(once_state(&a, &id).await, "claimed", "stopping the public share keeps the one-time one");
        // a new public share of another file must not overwrite the binding
        let other = base.join("other.bin");
        sample(&other, 1000);
        a.share(&other, false, |_, _| {}).await?;

        a.shutdown().await;
        drop(a);
        let a = Node::start(&base.join("a")).await?;
        let mut ep = None;
        for _ in 0..50 {
            ep = a.once.endpoint(&id);
            if ep.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let t2 = OnceTicket { addr: local_addr(&ep.expect("endpoint back after restart")), token: t.token };
        assert_eq!(t2.addr.id, t.addr.id, "same throwaway id after restart");
        let l2 = Link::Once(t2);
        let t0 = std::time::Instant::now();
        let rb = b.resolve(&l2).await;
        eprintln!("restart: B {:?} in {:?}", rb.as_ref().err().map(|e| format!("{e:#}")), t0.elapsed());
        let ticket = rb?;
        let rc = c.resolve(&l2).await;
        eprintln!("restart: C {:?}", rc.as_ref().err().map(|e| format!("{e:#}")));
        assert!(rc.is_err(), "binding survived the restart");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest)?;
        let (_k, never) = watch::channel(false);
        let out = b.download(&ticket, &l2.record_id(), &dest, never, |_| {}).await?;
        assert_eq!(std::fs::read(&out)?, data);
        for n in [a, b, c] {
            n.shutdown().await;
        }
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    #[test]
    fn links_parse_both_kinds() {
        let t = OnceTicket { addr: iroh::EndpointAddr::new(SecretKey::generate().public()), token: [7u8; 32] };
        let s = format!("{SCHEME}{}", t.encode());
        let l = parse_link(&s).unwrap();
        assert_eq!(l.once().unwrap().token, [7u8; 32]);
        assert!(l.name().is_none() && l.size().is_none());
        assert!(l.record_id().starts_with("once-") && l.record_id().len() == 21);
        let bt = BlobTicket::new(t.addr.clone(), Hash::new(b"x"), BlobFormat::HashSeq);
        let l = parse_link(&format!("{SCHEME}{bt}?size=5&name=a%20b")).unwrap();
        assert_eq!((l.size(), l.name().as_deref()), (Some(5), Some("a b")));
        assert_eq!(l.record_id(), Hash::new(b"x").to_string());
    }

}
