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

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Запись реестра раздач (shares.json).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShareRecord {
    pub root: String,
    pub name: String,
    pub source: PathBuf,
    pub files: Vec<SourceFile>,
    pub skipped: usize,
    pub total: u64,
    pub created: u64,
}

impl ShareRecord {
    fn unchanged(&self) -> bool {
        self.files.iter().all(SourceFile::unchanged)
    }
}

/// Раздача для интерфейса.
#[derive(Serialize, Clone, Debug)]
pub struct Shared {
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
    /// "active" — раздаётся; "changed" — исходные файлы изменились или пропали, не раздаётся.
    pub state: String,
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
    shares: Mutex<Vec<ShareRecord>>,
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

async fn bind_endpoint(key: SecretKey, port: u16) -> Result<Endpoint> {
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

        let shares = load_shares(dir);
        let node = Self {
            store,
            dl,
            mem,
            router,
            dir: dir.to_path_buf(),
            allow,
            shares: Mutex::new(shares),
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

    /// Корень раздачи и все хеши внутри (метаданные коллекции + файлы).
    async fn members(&self, root: Hash) -> Result<Vec<Hash>> {
        let bytes = self.store.blobs().get_bytes(root).await?;
        let seq = HashSeq::try_from(bytes)?;
        let mut v = vec![root];
        v.extend(seq.iter());
        Ok(v)
    }

    /// Пересобрать белый список из реестра: только раздачи с неизменёнными исходными файлами.
    async fn refresh_allow(&self) {
        let records = self.shares.lock().await.clone();
        let mut set = HashSet::new();
        for r in records.iter().filter(|r| r.unchanged()) {
            let Ok(root) = r.root.parse::<Hash>() else { continue };
            match self.members(root).await {
                Ok(m) => set.extend(m),
                Err(e) => eprintln!("раздача {} недоступна: {e:#}", r.name),
            }
        }
        if let Ok(mut a) = self.allow.write() {
            *a = set;
        }
    }

    fn view(&self, r: &ShareRecord) -> Shared {
        let state = if r.unchanged() { "active" } else { "changed" };
        let link = r
            .root
            .parse::<Hash>()
            .map(|root| make_link(&self.endpoint().addr(), root, r.total, &r.name))
            .unwrap_or_default();
        Shared {
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
    pub async fn list_shares(&self) -> Vec<Shared> {
        self.refresh_allow().await;
        let records = self.shares.lock().await.clone();
        records.iter().rev().map(|r| self.view(r)).collect()
    }

    /// Остановить раздачу: убрать из белого списка, реестра и снять тег.
    pub async fn stop_share(&self, root: &str) -> Result<()> {
        {
            let mut shares = self.shares.lock().await;
            let before = shares.len();
            shares.retain(|r| r.root != root);
            if shares.len() == before {
                bail!("такой раздачи нет");
            }
            save_shares(&self.dir, &shares)?;
        }
        self.refresh_allow().await;
        self.store.tags().delete(format!("share-{root}")).await?;
        Ok(())
    }

    /// Раздать файл или папку. В папке скрытые и служебные файлы пропускаются (аудит P0-3);
    /// явно выбранный файл раздаётся всегда. on_progress(готово_файлов, всего_файлов).
    pub async fn share(&self, path: &Path, mut on_progress: impl FnMut(usize, usize)) -> Result<Shared> {
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
        let mut sources = Vec::new();
        let mut total: u64 = 0;
        let mut keep = Vec::new();
        on_progress(0, count);
        for (i, (rel, p)) in files.into_iter().enumerate() {
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
        // Постоянный тег, чтобы раздача пережила перезапуск.
        self.store
            .tags()
            .set(format!("share-{root}"), tt.hash_and_format())
            .await?;
        self.store.sync_db().await?;

        let record = ShareRecord {
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
            shares.retain(|r| r.root != record.root);
            shares.push(record.clone());
            save_shares(&self.dir, &shares)?;
        }
        self.refresh_allow().await;

        // Ждём связи с relay (до 5 с), чтобы в ссылку попал и relay, и прямые адреса.
        // Сам ключ узла остаётся главным: при смене IP узел находится по нему.
        let _ = tokio::time::timeout(Duration::from_secs(5), self.endpoint().online()).await;
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
    pub async fn download(
        &self,
        ticket: &BlobTicket,
        dest: &Path,
        mut cancel: watch::Receiver<bool>,
        mut on_progress: impl FnMut(u64),
    ) -> Result<PathBuf> {
        let root = ticket.hash();
        let req = HashAndFormat::hash_seq(root);
        let tag = format!("dl-{root}");
        // Тег ДО скачивания: GC не тронет недокачанное, докачка после паузы и перезапуска.
        self.dl.tags().set(tag.clone(), req).await?;
        self.dl.sync_db().await?;

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

        // 3) Скачанное уже на диске — снимаем тег, GC освободит внутреннюю копию (аудит P1-10).
        self.dl.tags().delete(tag).await?;
        self.dl.sync_db().await?;
        Ok(first.unwrap_or_else(|| dest.to_path_buf()))
    }

    /// Забыть недокачанное: снять тег, GC освободит место.
    pub async fn forget_download(&self, root: Hash) -> Result<()> {
        self.dl.tags().delete(format!("dl-{root}")).await?;
        Ok(())
    }
}

fn remove_any(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
    let _ = std::fs::remove_file(p);
}

/// Адрес для ссылки: ID узла и relay, без IP-адресов — ссылку пересылают кому угодно, а IP раздающего
/// не должен уходить вместе с ней. Прямое соединение iroh всё равно наладит через relay.
/// Без relay (нет интернета) остаётся только полный адрес, иначе по ссылке не найти.
fn public_addr(addr: &iroh::EndpointAddr) -> iroh::EndpointAddr {
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
    match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            eprintln!("{} повреждён ({e}), список раздач начат заново", path.display());
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn save_shares(dir: &Path, shares: &[ShareRecord]) -> Result<()> {
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
}
