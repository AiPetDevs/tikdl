//! One-time links (docs/ONCE_LINKS.md): a share for exactly one device, burned once it is received.
//!
//! Each one-time share gets its own throwaway iroh endpoint (fresh key, never the node's own id), with two ALPNs:
//! - `tikdl/once/1` — claim protocol: the first device that presents the token from the link is bound to the share
//!   (TLS-authenticated endpoint id); "done" from that device burns the link (router shut down, key wiped).
//! - iroh-blobs, behind a Gate: only the bound device, only this share's hashes.
//! The records live in the node's one `shares` vector (one lock, one file): the binding can never be overwritten
//! by a stale copy.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use iroh::{
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler, Router},
    EndpointAddr, SecretKey,
};
use iroh_blobs::{
    provider::events::{AbortReason, EventMask, EventSender, ProviderMessage, RequestMode},
    store::fs::FsStore,
    Hash,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::core::{self, ShareRecord};

pub const ALPN: &[u8] = b"tikdl/once/1";
pub const PREFIX: &str = "once";
/// An unclaimed link lives this long; a claimed one — this long after the bound device's last activity.
pub const WAIT_SECS: u64 = 3 * 86_400;
pub const IDLE_SECS: u64 = 7 * 86_400;
/// Each live link is one more endpoint (sockets + relay connection).
pub const MAX_LIVE: usize = 10;

const OP_CLAIM: u8 = 1;
const OP_DONE: u8 = 2;
const OK: u8 = 0;
const TAKEN: u8 = 1;
const UNKNOWN: u8 = 2;
const GONE: u8 = 3;
const CHANGED: u8 = 4;
const FAILED: u8 = 5;

fn hex(b: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(b)
}

/// The way an endpoint id is shown on both sides (sender's row, recipient's card): first 8 hex characters.
pub fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// State of a one-time share, kept in shares.json (0600) next to the node key.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OnceRec {
    /// Secret key of the throwaway endpoint (hex). None once the link is used or expired.
    pub key: Option<String>,
    /// The token from the link (hex), kept only until the claim so the link can be copied again.
    pub token: Option<String>,
    pub token_hash: String,
    /// Endpoint id of the bound device (hex).
    pub bound: Option<String>,
    /// waiting | claimed | used | expired
    pub state: String,
    pub last_active: u64,
}

impl OnceRec {
    pub fn new() -> (Self, [u8; 32]) {
        let token = SecretKey::generate().to_bytes();
        let rec = Self {
            key: Some(hex(&SecretKey::generate().to_bytes())),
            token: Some(hex(&token)),
            token_hash: Hash::new(token).to_hex().to_string(),
            bound: None,
            state: "waiting".into(),
            last_active: core::now_secs(),
        };
        (rec, token)
    }
    pub fn live(&self) -> bool {
        matches!(self.state.as_str(), "waiting" | "claimed")
    }
    fn secret(&self) -> Option<SecretKey> {
        let b = data_encoding::HEXLOWER.decode(self.key.as_deref()?.as_bytes()).ok()?;
        Some(SecretKey::from_bytes(&<[u8; 32]>::try_from(b.as_slice()).ok()?))
    }
    /// When the link stops working by itself (unix seconds).
    pub fn expires(&self, created: u64) -> u64 {
        if self.bound.is_some() { self.last_active + IDLE_SECS } else { created + WAIT_SECS }
    }
}

/// What the link carries: the throwaway endpoint's address (id + relay, no IP) and the token. No name, size or hash.
#[derive(Clone, Debug)]
pub struct OnceTicket {
    pub addr: EndpointAddr,
    pub token: [u8; 32],
}

#[derive(Serialize, Deserialize)]
enum Wire {
    V1 { addr: EndpointAddr, token: [u8; 32] },
}

impl OnceTicket {
    pub fn encode(&self) -> String {
        let bytes = postcard::to_stdvec(&Wire::V1 { addr: self.addr.clone(), token: self.token }).expect("encode");
        format!("{PREFIX}{}", data_encoding::BASE32_NOPAD.encode(&bytes).to_ascii_lowercase())
    }
    pub fn decode(s: &str) -> Result<Self> {
        let body = s.strip_prefix(PREFIX).ok_or_else(|| anyhow!("не одноразовая ссылка"))?;
        let bytes = data_encoding::BASE32_NOPAD
            .decode(body.to_ascii_uppercase().as_bytes())
            .context("неверный тикет")?;
        let Wire::V1 { addr, token } = postcard::from_bytes(&bytes).context("неверный тикет")?;
        Ok(Self { addr, token })
    }
    /// Id of the download record on the recipient's side: stable across restarts, never the content hash
    /// (the same content can arrive by a public link too).
    pub fn record_id(&self) -> String {
        format!("{PREFIX}-{}", &Hash::new(self.token).to_hex()[..16])
    }
}

/// Throwaway endpoints of the live one-time shares.
#[derive(Debug)]
pub struct Host {
    pub store: FsStore,
    pub dir: std::path::PathBuf,
    /// The node's own share list (same Arc as Node.shares).
    pub shares: Arc<Mutex<Vec<ShareRecord>>>,
    routers: std::sync::Mutex<HashMap<String, Router>>,
    /// Bumped on every stage change (claimed, used, expired) and when a link becomes available
    /// (relay reached). The window and the CLI watch it instead of polling.
    pub changed: tokio::sync::watch::Sender<u64>,
    /// Set by stop_all (node shutdown): a late spawn from start_all must not bring an endpoint back up.
    closed: std::sync::atomic::AtomicBool,
}

impl Host {
    pub fn new(store: FsStore, dir: std::path::PathBuf, shares: Arc<Mutex<Vec<ShareRecord>>>) -> Arc<Self> {
        Arc::new(Self { store, dir, shares, routers: Default::default(), changed: tokio::sync::watch::channel(0).0, closed: Default::default() })
    }

    /// Address of the throwaway endpoint for the link (relay only, no IP), if it is up.
    pub fn addr(&self, id: &str) -> Option<EndpointAddr> {
        let r = self.routers.lock().ok()?;
        // Relay only, never IP addresses: a one-time link must not tell where the sender is. Until the
        // throwaway endpoint reaches its relay there is no link yet (UI: «подключение к сети…»).
        let addr = r.get(id)?.endpoint().addr();
        let relays: Vec<_> = addr.addrs.iter().filter(|a| a.is_relay()).cloned().collect();
        (!relays.is_empty()).then(|| EndpointAddr::from_parts(addr.id, relays))
    }

    /// Running with exactly this key (a renewed link has a new key).
    fn running_as(&self, id: &str, key: iroh::PublicKey) -> bool {
        self.routers.lock().map(|r| r.get(id).is_some_and(|r| r.endpoint().id() == key)).unwrap_or(false)
    }

    /// Is this one of our own throwaway endpoints (the sender opening their own one-time link)?
    pub fn is_mine(&self, endpoint: iroh::PublicKey) -> bool {
        self.routers.lock().map(|r| r.values().any(|r| r.endpoint().id() == endpoint)).unwrap_or(false)
    }

    pub fn running(&self, id: &str) -> bool {
        self.routers.lock().map(|r| r.contains_key(id)).unwrap_or(false)
    }

    /// Tests: the throwaway endpoint itself (to build a localhost address).
    #[cfg(test)]
    pub fn endpoint(&self, id: &str) -> Option<iroh::Endpoint> {
        self.routers.lock().ok()?.get(id).map(|r| r.endpoint().clone())
    }

    /// Bring up the endpoint of one live one-time share.
    pub async fn spawn(self: &Arc<Self>, rec: &ShareRecord) -> Result<()> {
        let once = rec.once.as_ref().ok_or_else(|| anyhow!("не одноразовая раздача"))?;
        let key = once.secret().ok_or_else(|| anyhow!("нет ключа одноразовой ссылки"))?;
        let me = key.public();
        if !once.live() || self.running_as(&rec.id, me) {
            return Ok(());
        }
        let root: Hash = rec.root.parse()?;
        let members: HashSet<Hash> = core::members(&self.store, root).await?.into_iter().collect();
        let endpoint = core::bind_endpoint(key, 0).await?;

        // Per-request check: only this share's hashes, only while its source files are unchanged.
        let mask = EventMask { get: RequestMode::Intercept, get_many: RequestMode::Disabled, ..EventMask::DEFAULT };
        let (events, mut rx) = EventSender::channel(32, mask);
        let (host, id) = (self.clone(), rec.id.clone());
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let ProviderMessage::GetRequestReceived(msg) = msg {
                    let ok = members.contains(&msg.inner.request.hash) && host.serving(&id).await;
                    let _ = msg.tx.send(if ok { Ok(()) } else { Err(AbortReason::Permission) }).await;
                }
            }
        });
        let router = Router::builder(endpoint)
            .accept(ALPN, Claim { id: rec.id.clone(), host: self.clone() })
            .accept(iroh_blobs::ALPN, Gate { id: rec.id.clone(), host: self.clone(), events })
            .spawn();
        // Stop, «Новая ссылка», expiry or shutdown may have happened while this endpoint was coming up (start_all
        // works on copies). Re-check under the share-list lock — the same lock stop/renew change the record under —
        // that this record is still live with this very key; otherwise the endpoint would hang on as a zombie or
        // replace the newer link's endpoint.
        let placed = {
            let shares = self.shares.lock().await;
            let current = shares.iter().any(|r| {
                r.id == rec.id
                    && r.once.as_ref().is_some_and(|o| o.live() && o.secret().map(|k| k.public()) == Some(me))
            });
            if current && !self.closed.load(std::sync::atomic::Ordering::SeqCst) {
                let mut map = self.routers.lock().map_err(|_| anyhow!("lock"))?;
                Ok(map.insert(rec.id.clone(), router))
            } else {
                Err(router)
            }
        };
        match placed {
            Ok(Some(old)) => {
                let _ = old.shutdown().await;
            }
            Ok(None) => {}
            Err(stale) => {
                let _ = stale.shutdown().await;
                return Ok(());
            }
        }
        // The link appears only once the relay is reached (addr()): tell the watchers when it does.
        let (host, id) = (self.clone(), rec.id.clone());
        tokio::spawn(async move {
            // no time limit: without a relay there is no link, so keep watching for as long as the endpoint is up
            while host.running(&id) && host.addr(&id).is_none() {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            host.bump();
        });
        Ok(())
    }

    pub fn bump(&self) {
        self.changed.send_modify(|n| *n += 1);
    }

    /// Shut the endpoint down (burn, expiry, stop, new link). Its id never appears again.
    pub async fn stop(&self, id: &str) {
        let r = self.routers.lock().ok().and_then(|mut m| m.remove(id));
        if let Some(r) = r {
            let _ = r.shutdown().await;
        }
    }

    pub async fn stop_all(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        let all: Vec<Router> = self.routers.lock().map(|mut m| m.drain().map(|(_, r)| r).collect()).unwrap_or_default();
        for r in all {
            let _ = r.shutdown().await;
        }
    }

    /// Bring up every live one-time share (at start, in the background: N endpoints must not delay the window).
    pub async fn start_all(self: Arc<Self>) {
        self.sweep().await;
        let recs: Vec<ShareRecord> = self.shares.lock().await.iter().filter(|r| r.once.as_ref().is_some_and(|o| o.live())).cloned().collect();
        for r in recs {
            if let Err(e) = self.spawn(&r).await {
                eprintln!("одноразовая ссылка {} не поднялась: {e:#}", r.name);
            }
        }
    }

    /// Expire links past their time; save the last-activity marks. Runs every minute while the node is up.
    pub async fn sweep(&self) {
        let now = core::now_secs();
        let mut dead = Vec::new();
        {
            let mut shares = self.shares.lock().await;
            let mut any = false;
            for r in shares.iter_mut() {
                let created = r.created;
                if let Some(o) = r.once.as_mut() {
                    any |= o.live();
                    if o.live() && now >= o.expires(created) {
                        o.state = "expired".into();
                        o.key = None;
                        o.token = None;
                        dead.push(r.id.clone());
                    }
                }
            }
            if any || !dead.is_empty() {
                if let Err(e) = core::save_shares(&self.dir, &shares) {
                    eprintln!("{e:#}");
                }
            }
        }
        if !dead.is_empty() {
            self.bump();
        }
        for id in dead {
            self.stop(&id).await;
        }
    }

    /// Bound device of a live share, if any.
    async fn bound(&self, id: &str) -> Option<String> {
        let shares = self.shares.lock().await;
        let o = shares.iter().find(|r| r.id == id)?.once.as_ref()?;
        if o.live() { o.bound.clone() } else { None }
    }

    /// The bound device may be served: link live and claimed, source files unchanged.
    async fn burned(&self, id: &str) -> bool {
        let shares = self.shares.lock().await;
        shares.iter().find(|r| r.id == id).and_then(|r| r.once.as_ref()).is_some_and(|o| o.state == "used")
    }

    async fn serving(&self, id: &str) -> bool {
        let shares = self.shares.lock().await;
        shares.iter().find(|r| r.id == id).is_some_and(|r| {
            r.once.as_ref().is_some_and(|o| o.state == "claimed" && core::now_secs() < o.expires(r.created)) && r.unchanged()
        })
    }

    async fn touch(&self, id: &str) {
        let mut shares = self.shares.lock().await;
        if let Some(o) = shares.iter_mut().find(|r| r.id == id).and_then(|r| r.once.as_mut()) {
            o.last_active = core::now_secs();
        }
    }

    /// One claim-protocol request. Every change goes through the node's share list and is saved before the reply.
    async fn handle(self: &Arc<Self>, id: &str, remote: &str, req: &[u8]) -> Vec<u8> {
        if req.len() != 33 {
            return vec![UNKNOWN];
        }
        let (op, token) = (req[0], &req[1..]);
        let mut shares = self.shares.lock().await;
        let Some(r) = shares.iter_mut().find(|r| r.id == id) else { return vec![UNKNOWN] };
        let root = r.root.clone();
        let unchanged = r.unchanged();
        let created = r.created;
        let Some(o) = r.once.as_mut() else { return vec![UNKNOWN] };
        // expiry is checked here too: the sweeper runs once a minute (and not at all while the laptop sleeps)
        if !o.live() || core::now_secs() >= o.expires(created) {
            return vec![GONE];
        }
        let token_ok = Hash::new(token).to_hex().to_string() == o.token_hash;
        let mine = o.bound.as_deref() == Some(remote);
        if o.bound.is_some() && !mine {
            return vec![TAKEN];
        }
        if !mine && !token_ok {
            return vec![UNKNOWN];
        }
        let before = o.clone();
        let reply = match op {
            OP_CLAIM => {
                if !unchanged {
                    return vec![CHANGED];
                }
                o.bound = Some(remote.to_string());
                o.state = "claimed".into();
                o.token = None;
                o.last_active = core::now_secs();
                let Ok(root) = root.parse::<Hash>() else { return vec![FAILED] };
                let mut v = vec![OK];
                v.extend_from_slice(root.as_bytes());
                v
            }
            OP_DONE if mine => {
                o.state = "used".into();
                o.key = None;
                o.token = None;
                o.last_active = core::now_secs();
                vec![OK]
            }
            _ => return vec![UNKNOWN],
        };
        let moved = o.state != before.state;
        if let Err(e) = core::save_shares(&self.dir, &shares) {
            eprintln!("{e:#}");
            // not saved — not granted: roll back so memory matches the file
            if let Some(o) = shares.iter_mut().find(|r| r.id == id).and_then(|r| r.once.as_mut()) {
                *o = before;
            }
            return vec![FAILED];
        }
        drop(shares);
        if moved {
            self.bump();
        }
        // a burned link's endpoint goes down in Claim::accept, once the reply has reached the recipient
        reply
    }
}

#[derive(Debug, Clone)]
struct Claim {
    id: String,
    host: Arc<Host>,
}

impl ProtocolHandler for Claim {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        let (mut send, mut recv) = conn.accept_bi().await.map_err(AcceptError::from_err)?;
        let req = recv.read_to_end(64).await.map_err(AcceptError::from_err)?;
        let reply = self.host.handle(&self.id, &remote, &req).await;
        send.write_all(&reply).await.map_err(AcceptError::from_err)?;
        send.finish().map_err(AcceptError::from_err)?;
        // returning drops the connection and can cut the reply: wait for the client to close
        let _ = tokio::time::timeout(Duration::from_secs(15), conn.closed()).await;
        // Burned: now that the recipient has the reply, take the endpoint down. Spawned — the router's shutdown
        // waits for its handlers, this one included.
        if self.host.burned(&self.id).await {
            let (host, id) = (self.host.clone(), self.id.clone());
            tokio::spawn(async move { host.stop(&id).await });
        }
        Ok(())
    }
}

/// iroh-blobs for the bound device only. Never delegates `shutdown`: BlobsProtocol::shutdown would shut down the
/// shared store of the whole node.
#[derive(Debug, Clone)]
struct Gate {
    id: String,
    host: Arc<Host>,
    events: EventSender,
}

impl ProtocolHandler for Gate {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        if self.host.bound(&self.id).await.as_deref() != Some(remote.as_str()) {
            conn.close(403u32.into(), b"not bound to this device");
            return Ok(());
        }
        self.host.touch(&self.id).await;
        let store: &iroh_blobs::api::Store = &self.host.store;
        iroh_blobs::provider::handle_connection(conn, store.clone(), self.events.clone()).await;
        Ok(())
    }
}

/// Recipient side: one request to the sender's throwaway endpoint.
pub async fn request(endpoint: &iroh::Endpoint, t: &OnceTicket, op: u8) -> Result<Vec<u8>> {
    let fut = async {
        let conn = endpoint.connect(t.addr.clone(), ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let mut req = vec![op];
        req.extend_from_slice(&t.token);
        send.write_all(&req).await?;
        send.finish()?;
        let resp = recv.read_to_end(64).await?;
        conn.close(0u32.into(), b"ok");
        Ok::<_, anyhow::Error>(resp)
    };
    tokio::time::timeout(Duration::from_secs(15), fut)
        .await
        .map_err(|_| anyhow!("отправитель не в сети, или ссылка уже не действует (получена, остановлена или истекла)"))?
        .map_err(|e| anyhow!("отправитель не в сети, или ссылка уже не действует ({e:#})"))
}

fn status(code: u8) -> Result<()> {
    match code {
        OK => Ok(()),
        TAKEN => bail!("ссылка уже использована на другом устройстве — попросите отправителя прислать новую"),
        UNKNOWN => bail!("ссылка недействительна — отправитель её остановил или выдал новую"),
        GONE => bail!("ссылка больше не действует: файлы уже получены или истёк срок"),
        CHANGED => bail!("отправитель изменил файлы — раздача остановлена"),
        _ => bail!("у отправителя не удалось сохранить привязку — попробуйте ещё раз"),
    }
}

/// Bind this device to the link (idempotent for the bound device) and learn the root of the share.
pub async fn claim(endpoint: &iroh::Endpoint, t: &OnceTicket) -> Result<Hash> {
    let r = request(endpoint, t, OP_CLAIM).await?;
    status(*r.first().unwrap_or(&FAILED))?;
    let bytes: [u8; 32] = r.get(1..33).and_then(|b| b.try_into().ok()).ok_or_else(|| anyhow!("неверный ответ отправителя"))?;
    Ok(Hash::from_bytes(bytes))
}

/// Tell the sender everything is on disk: the link burns.
pub async fn done(endpoint: &iroh::Endpoint, t: &OnceTicket) -> Result<()> {
    let r = request(endpoint, t, OP_DONE).await?;
    status(*r.first().unwrap_or(&FAILED))
}
