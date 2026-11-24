//! Ядро: iroh-узел с постоянным ключом и постоянным хранилищем.
//! Постоянный ключ = ссылки не ломаются при смене сети/IP.
//! Постоянное хранилище = недокачанное продолжается после паузы/перезапуска.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
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

pub const SCHEME: &str = "tikdl://get/";

pub struct Node {
    pub store: FsStore,
    mem: iroh::address_lookup::MemoryLookup,
    pub router: Router,
}

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tikdl")
}

fn load_or_create_key(dir: &Path) -> Result<SecretKey> {
    let path = dir.join("node.key");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Ok(SecretKey::from_bytes(&arr));
        }
    }
    let key = SecretKey::generate();
    std::fs::write(&path, key.to_bytes()).context("не удалось сохранить ключ узла")?;
    Ok(key)
}

impl Node {
    pub async fn start(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let key = load_or_create_key(dir)?;
        let store = FsStore::load(dir.join("store")).await?;
        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(key)
            .bind()
            .await?;
        let mem = iroh::address_lookup::MemoryLookup::new();
        endpoint.address_lookup()?.add(mem.clone());
        let blobs = BlobsProtocol::new(&store, None);
        let router = Router::builder(endpoint)
            .accept(iroh_blobs::ALPN, blobs)
            .spawn();
        Ok(Self { store, mem, router })
    }

    pub fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    /// Раздать файл или папку. Возвращает ссылку tikdl://get/<тикет>?size=..&name=..
    pub async fn share(&self, path: &Path) -> Result<String> {
        let path = std::path::absolute(path)?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "files".into());

        let mut files: Vec<(String, PathBuf)> = Vec::new();
        if path.is_file() {
            files.push((name.clone(), path.clone()));
        } else if path.is_dir() {
            for e in walkdir::WalkDir::new(&path).follow_links(false) {
                let e = e?;
                if !e.file_type().is_file() {
                    continue;
                }
                let rel = e.path().strip_prefix(&path)?;
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
            bail!("в папке нет файлов");
        }

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
        Ok(format!(
            "{SCHEME}{ticket}?size={total}&name={}",
            urlencoding::encode(&name)
        ))
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
            .store
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
        // Удерживаем скачанное в хранилище (для докачки/повторной раздачи).
        self.store
            .tags()
            .set(format!("dl-{root}"), req)
            .await?;

        let collection = Collection::load(root, self.store.as_ref()).await?;
        let mut first: Option<PathBuf> = None;
        for (name, hash) in collection.iter() {
            let target = safe_join(dest, name)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            self.store.blobs().export(*hash, &target).await?;
            if first.is_none() {
                let top = name.split('/').next().unwrap_or(name);
                first = Some(dest.join(top));
            }
        }
        Ok(first.unwrap_or_else(|| dest.to_path_buf()))
    }
}

/// Защита от имён вида "../../etc/passwd" в чужой коллекции.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let mut out = base.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => out.push(p),
            _ => bail!("недопустимое имя файла в раздаче: {rel}"),
        }
    }
    Ok(out)
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
