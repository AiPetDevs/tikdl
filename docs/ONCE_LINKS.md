# One-time links: how they work (as built, 2026-11-25)

Owner's request (2026-11-25): «приватную передачу файла от устройства к устройству, чтоб ссылка была одноразовая.
Интегрируй эту возможность в раздачу».

Inputs: prior-art research `docs/research/ONCE_PRIOR_ART.md` (Firefox Send, OnionShare, Magic Wormhole, croc,
Bitwarden Send, PrivateBin, LocalSend…), iroh API facts (§7), two critic rounds on the plan (strategy B chosen),
one critic review of the code.

## 1. What we promise — honestly
| | Promise | Limit (said in README / UI) |
|---|---|---|
| G1 | Only the first device that presses «Что внутри?» or «Скачать» can download. Others get «ссылка уже использована на другом устройстве». | "Device" = the TikDL data folder (`node.key`). Another OS user, a wiped reinstall or a CLI with another HOME is another device. Whoever opens a leaked link first wins; the sender *sees* a device id they don't know (detection, not prevention). |
| G2 | After the bound device has everything on disk, the link dies for everyone. | Burn relies on the recipient's "done" ack. A modified client that never acks keeps access for itself only, until expiry. |
| G3 | Pause, network change, restart of either side do not burn the link and do not let anyone else in. | Only until expiry. |
| G4 | The link has no file name, size, content hash, sender's permanent node id or sender IP. | Hidden from whoever reads the link (messenger, preview bot). Not hidden from the relay operator (timing/IP correlation) or from the recipient after a direct connection (IP). The relay URL reveals the region. |
| G5 | Unclaimed link lives 3 days; claimed — 7 days after the last activity of the bound device. Sender can stop or issue a new link any time. At most 10 live one-time links. | |
| G6 | Sender sees: waiting → «закреплена: устройство xxxxxxxx» → «получено устройством xxxxxxxx — ссылка сгорела» / «срок истёк». The recipient's confirm card shows the same 8-hex device code. | |

## 2. Why this design (strategy B: a throwaway endpoint per link)
- **A — same endpoint, gate by remote id**: cheaper, but the link exposes the permanent node id → links are linkable
  to all public shares. Rejected (G4).
- **C — unguessable root, bind on first request**: no protocol, but still the permanent id in the link, and burn
  detection on the sender is unreliable (`TransferCompleted` ≠ received). Rejected.
- **D — one shared private endpoint for all one-time links**: links become linkable to each other and the id stays
  online after burns. Rejected.
- **B (chosen)**: every one-time share gets its own iroh Endpoint with a fresh SecretKey (like OnionShare's new onion
  address per share). After the burn that id never appears again. Cost: one UDP socket + relay connection per live
  link → cap of 10.

Prior art lessons applied: count "used" only after a complete, confirmed transfer (Firefox Send, croc, PrivateBin
≥0.20 — never on open, so preview bots can't burn it); bind to the first device instead of "anyone can retry"
(Firefox Send / OnionShare weakness); ticket after `#` on the https landing page, which never contacts the sender.

## 3. Protocol
**Link**: `tikdl://get/once<base32(postcard(Wire::V1 { addr, token[32] }))>`; `addr` = throwaway endpoint id + relay
URL only (no IPs; no link at all until the relay is reached). Old TikDL versions fail with «неверный тикет».
The landing page accepts `^(blob|once)…`.

**Router of the throwaway endpoint** (`once.rs`), two ALPNs:
- `tikdl/once/1` — claim protocol, one bi-stream: request `[op][token32]`, reply `[status](root32)`.
  op 1 = claim, op 2 = done. Status: 0 OK, 1 TAKEN, 2 UNKNOWN, 3 GONE, 4 CHANGED, 5 FAILED.
  The token is compared in constant time; the server waits for `conn.closed()` so the reply is not cut.
- `iroh_blobs::ALPN` behind a **Gate**: only the bound `conn.remote_id()` (TLS-authenticated), only this share's
  hashes (own EventSender interceptor). The Gate never delegates `shutdown` — `BlobsProtocol::shutdown` would shut
  the shared store down.

**Claim**: under the shares mutex (the same `Arc<Mutex<Vec<ShareRecord>>>` as `Node.shares`): if unbound — bind the
remote id, **persist first**, then reply OK with the root; on save failure roll back and reply FAILED. Repeat claims
by the bound device return OK (that is what makes resume and restart work). After binding, the token is wiped from
disk (only its hash stays).

**Peek = claim**: the card opens offline («Одноразовая ссылка: скачать сможет одно устройство», name/size hidden);
«Что внутри?» and «Скачать» claim.

**Download**: the normal downloader against the throwaway addr, tag `dl-{record id}` (`once-<16 hex of
BLAKE3(token)>`), so it never collides with a public download of the same content.

**Done**: after every file is renamed into place the recipient sets `ack_pending`, drops its tag, sends done (5
tries, then retried at next start; after that the record is closed and the token wiped). The sender checks token
and remote, sets `used`, persists, replies, wipes key and token, shuts the router down 500 ms later. No tombstone:
afterwards everyone gets the generic «отправитель не в сети, или ссылка уже не действует».

## 4. State
- Sender `ShareRecord`: `id` (`once-<16 hex>`; public shares keep id = root), `once: OnceRec { key, token,
  token_hash, bound, state: waiting|claimed|used|expired, last_active }`. Tag `share-{id}`. `shares.json` is 0600.
- Public allowlist (`refresh_allow`) skips one-time records — the main node never serves them.
- Sweeper every 60 s: expiry (3 days unclaimed from `created`, 7 days after `last_active`). `Node::shutdown` stops the
  sweeper and all throwaway routers before the stores shut down.
- Recipient `DlRecord`: `ack_pending`, `ack_tries`; a once record already done is refused locally: «уже получено на
  этом компьютере — одноразовая ссылка сгорела».

## 5. UI and CLI
- «Раздать»: kit Switch «Одноразовая ссылка» (hint: one device, burns after receipt). Rows: «одноразовая · ждёт»
  (link + «сгорит через N дн.», copy / new link / stop) → «закреплена» (device code, new link / stop) → «получено»
  (green) / «срок истёк» (new link). «Новая ссылка» = stop + fresh key and token for the same files, no re-hashing.
- CLI: `tikdl serve --once <path>` prints the link (or waits for the relay) and every stage change;
  `tikdl get <once-link> <dir>` claims, downloads, acks.

## 6. Verification
- `cargo test` (14, with §6a): one device then burned (wrong token doesn't bind, main id doesn't serve, C refused while B holds
  it, byte-identical, refusal after burn, store alive after router shutdown); concurrent claims → one winner; sender
  restart keeps binding + public share of the same file unaffected; both link kinds parse.
- Live (2026-11-25, `docs/plans/evidence/once_live_cli.txt` + window shots `once_*.jpg`): 1.5 GB file; A hard-killed
  (SIGKILL) at 133 MB; B refused «уже использована на другом устройстве»; sender hard-killed and restarted, still
  «закреплена»; A resumed from 133 MB (not from 0) to a byte-identical file; sender «получено … — ссылка сгорела»;
  A again and B again refused. The release window followed the stages by itself (waiting → закреплена → получено).
  Decoded link: no main id, no name, no IP — only the relay URL. Regular share still downloads on two devices.
  (Earlier "pauses" with `kill -INT` were not real: background jobs ignore SIGINT.)
- Found by the live run and fixed: the link carried the sender's IPv6 and Tailscale addresses when the relay was not
  reached yet (`public_addr` falls back to the full address). One-time links now never fall back.
- Found by the window check and fixed: the window never learned that a share was claimed or received (no event).
  `once::Host.changed` (watch) now bumps on every stage change and when the link appears; the app emits
  `shares-changed`, the CLI waits on the same watch. Test: `once_link_one_device_then_burned` asserts the bumps.

## 6a. Code review by the critic (2026-11-25) — fixed
| Finding | Fix |
|---|---|
| Recipient quits during the "done" ack → download restarts, «name (1)» or false «сбой» | record set to done together with `ack_pending` before the tag is dropped |
| Stop / «Новая ссылка» while start-up still brings endpoints up → zombie endpoint or the new link killed | `spawn` re-checks under the share-list lock that the record is live with this very key; `stop_all` closes the host |
| CLI missed the first stage change and the late link | stage map seeded before waiting |
| Burned endpoint shut down 500 ms after the reply (could cut it on a slow relay) | shut down after the recipient closes the connection |
| Limit of 10 not checked on renew and checked without the lock | `once_room` under the lock in `share` and `renew_once` |
| Expired link claimable until the sweeper (up to 60 s, longer after sleep) | expiry checked on every claim and every blob request |
| Received link shown as «файлы изменились» once its source changed | used/expired stages win in UI and CLI |
| Sender opening their own link bound it to themselves | refused: «это ваша собственная одноразовая ссылка…» |
Test `once_link_renew_stale_spawn_expiry_limit` covers own link, renew, stale spawn after renew and after stop,
expiry before the sweep and the 11th link. Not covered by tests: the recipient crash window (GUI code).

## 7. iroh API facts (iroh 1.5 / iroh-blobs 0.105, verified by a scratch crate)
- `ProtocolHandler::accept(conn)` has the TLS-authenticated `conn.remote_id()`; deny with `conn.close(403, …)`.
- Several Endpoint+Router pairs in one process work; one FsStore can be served by several endpoints.
- `BlobsProtocol::shutdown()` shuts down the store — never delegate it from a throwaway router.
- One Router per Endpoint; register both ALPNs before `spawn`.
- `TransferCompleted` is per request and means "written to the stream", not "received" → burn needs an ack.
- Endpoint id is an address, not a secret → a token is required.
- Throwaway endpoints use `presets::N0` (pkarr publishing), so a link survives a sender restart even if its relay
  changes; pkarr reveals only "a random id is on relay X".
- The blobs downloader pool's default connect timeout is 1 s; set to 15 s for all downloads.
