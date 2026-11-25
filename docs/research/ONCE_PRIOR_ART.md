# Prior art: one-time links and private device-to-device transfer (for TikDL)

Date: 2026-11-25. Labels: [V] = verified in the opened source; [P] = probable (inferred from a source, not stated); [A] = my assumption.

## 1. Per-system notes

### Firefox Send (Mozilla, discontinued Sept 2021; fork timvisee/send)
- [V] The key sits in the URL `#fragment`, and the server gets ciphertext plus a signing key. Download needs a signed server nonce, so it is an authenticated request. https://github.com/mozilla/send/blob/master/docs/encryption.md
- [V] The download counter is incremented, and the file deleted when `dl >= dlimit`, only in the pipe `finish` handler, so only after the whole response was written. If `req` is `aborted`, a `cancelled` flag is set and nothing is counted. https://raw.githubusercontent.com/timvisee/send/master/server/routes/download.js
- [V] The service was discontinued; the repo is archived. https://github.com/mozilla/send
- [P] Consequence: an interrupted download can be retried any number of times, and so can one by a different person, until one completes. "Used" = first completed download.
- [P] Preview bots cannot burn it: they don't have the fragment key and don't run the signed-nonce JS flow.

### OnionShare
- [V] The default is "Stop sharing after files have been sent". Unchecking it allows multiple downloads and individual files. https://docs.onionshare.org/2.6/en/features.html
- [V] It uses Tor client auth (a private key) by default, so knowing the .onion address alone gives nothing. An eavesdropper on the channel used to send the address can tell OnionShare is in use. https://docs.onionshare.org/2.6/en/security.html
- [V] Public mode disables the key. Persistent tabs keep the same address and key across restarts, and auto-stop timers exist. https://docs.onionshare.org/2.6/en/advanced.html
- [V] In code: with autostop on, `index()` and `download()` serve `denied.html` when `download_in_progress or transfer_complete`. `transfer_complete=True` only if not cancelled, and a cancel resets the in-progress flag. Range requests (206, ETag/412) are supported; multipart ranges are not. https://raw.githubusercontent.com/onionshare/onionshare/main/cli/onionshare_cli/web/share_mode.py
- [V] PR #1236: completion is inferred from Tor rendezvous circuits closing, because Stem `get_streams()` was unreliable, and OnionShare waits for circuits to close before killing tor so the last bytes aren't cut off. https://github.com/onionshare/onionshare/pull/1236
- [P] Pitfalls: a second device is denied while the first is downloading, which acts as a lock. A cancelled transfer re-opens the share to anyone. If one Range request finishes its generator, it may set `transfer_complete` even though the file isn't fully fetched. That last point is not verified; it is a hypothesis from reading the code summary.

### Magic Wormhole
- [V] Codes are one-time use; SPAKE2 PAKE; the default code is 16 bits, so an attacker (including a malicious mailbox server) gets one guess at 1/65536. A failed MitM shows up as a "crowded"/"scary" error. There is an optional `--verify` string. There are no built-in timeouts, and both sides must stay online. https://magic-wormhole.readthedocs.io/en/latest/welcome.html
- [V] Attacks considered: the secret words must not be used as the channel id, because an offline dictionary attack would work. Nameplate reuse rules halve the attack. Mailbox DoS has no good mitigation. https://magic-wormhole.readthedocs.io/en/latest/attacks.html
- [V] The transit relay pairs connections by a token that is an HKDF of the transit key and forwards ciphertext. Passive observers see record sizes, timing, and IPs. https://magic-wormhole.readthedocs.io/en/latest/transit.html
- [P] Resume: not described in the welcome page.

### croc
- [V] The code phrase drives PAKE for E2E. Direct connection is tried first, with relay fallback. Resume of interrupted transfers is listed as a feature. https://github.com/schollz/croc
- [V] Stored (async) transfers: link `https://host/s/id#v1.decryption-key`, the key in the fragment, and the server sees only the id. The download limit defaults to 1 and the lifetime to 1 day, from upload completion. A download is counted after the receiver has "download, authenticate, and verify every file", counted per transfer, not per file. The sender can revoke with `croc --revoke` using a local receipt. Docs: "anyone who has it can decrypt the files and claim one of the allowed downloads". https://github.com/schollz/croc

### Wormhole.app
- [V] The key is in the URI fragment, never sent to the server. Transfer uses WebRTC P2P or a server upload, and the server sees room id, salts, ciphertext, and auth tokens. https://wormhole.app/security
- Download limit and expiry semantics were not found in that page (not verified).

### Bitwarden Send
- [V] The link is `https://vault.bitwarden.com/#/send_id/encryption_key`, the fragment is not sent, and the server sees only the id and ciphertext. https://bitwarden.com/help/send-encryption/
- [V] Max access count semantics: a text Send counts link opens, a file Send counts downloads of contents. https://bitwarden.com/help/send-lifespan/
- [V] There are expiration and deletion dates (max 31 days), an optional password, and an option to hide the sender email. https://bitwarden.com/help/about-send/

### 1Password item sharing
- [V] A share secret is generated on the device, and the server never gets it. Sharing can go to "anyone with the link" or only to specific emails, with email verification before access. Expiry is chosen by the sender, and sharing can be stopped at any time. https://support.1password.com/share-items-security/ , https://support.1password.com/share-items/
- I did not find the exact expiry list (for example, "1 view") on the opened pages.

### PrivateBin
- [V] The key is in the fragment, the server has zero knowledge, but access logs may still reveal who accessed a paste (first). https://privatebin.info/
- [V] CHANGELOG: in 0.14 the paste was destroyed on open. In 0.20 burn-after-reading pastes are "deleted only after the paste was successfully decrypted via callback", which "prevents accidental deletion by chatbots following URLs". In 1.3.5 they avoid exposing burn messages from cache (#630). In 1.7.0 they ask for confirmation before loading burn-after-reading pastes (#1237). https://www.github.com/PrivateBin/PrivateBin/blob/master/CHANGELOG.md

### LocalSend
- [V] A REST protocol with no external servers, multicast discovery, a fingerprint (cert SHA-256 under HTTPS), and a receiver accept/reject prompt. An optional PIN is passed as a query parameter (401 if wrong). Each session has a per-file token. The download API (reverse transfer) uses plain HTTP because browsers reject self-signed certs. https://github.com/localsend/protocol

### sendme (n0)
- [V] The ticket stays valid if the IP changes, because it uses 256-bit node ids. The provider runs until Ctrl-C. iroh-blobs handles verified streaming and resume. Relay fallback; TLS. There is no one-time or access-control feature in the README. https://github.com/n0-computer/sendme

### iroh / iroh-blobs (n0)
- [V] "Unlike what the name might imply, tickets are not single-use tokens." App tickets may contain secrets. Normal tickets contain endpoint id, relay URL, and direct addresses (TikDL strips the IPs). https://docs.iroh.computer/concepts/tickets
- [V] The relay sees "which pairs are exchanging traffic, when, and how much", not content. The DNS discovery record (dns.iroh.link) publishes the public key and relay. https://docs.iroh.computer/concepts/security-privacy
- [V] 0.90 blog: access control exists via provider events, per node id for connections and per request for Push. "it would be useful to also have access control for Get requests ... already been requested by users". https://iroh.computer/blog/iroh-blobs-0-90-new-features
- [V] iroh-blobs 0.105.0 `provider::events`: types ClientConnected{connection_id, endpoint_id: Option<EndpointId>}, ConnectionClosed, RequestReceived, TransferStarted, TransferProgress, TransferCompleted{stats}, TransferAborted, AbortReason, ConnectMode, RequestMode {None, Notify, Intercept, NotifyLog, InterceptLog, Disabled}, ThrottleMode. https://docs.rs/iroh-blobs/latest/iroh_blobs/provider/events/index.html , https://docs.rs/iroh-blobs/latest/iroh_blobs/provider/events/enum.RequestMode.html , https://docs.rs/iroh-blobs/latest/iroh_blobs/provider/events/struct.ClientConnected.html , https://docs.rs/iroh-blobs/latest/iroh_blobs/provider/events/struct.TransferCompleted.html
- [V] Official example `examples/limit.rs` covers limit_by_node_id (ConnectMode::Intercept, AbortReason::Permission), limit_by_hash (get: Intercept, get_many Disabled "could be used to bypass"), throttle, and max connections. https://github.com/n0-computer/iroh-blobs/blob/main/examples/limit.rs
- [V] Third party: ringdrop's RingGate checks a membership registry on every request, and failures are silently dropped. It has no one-time or expiry feature. https://rikettsie.github.io/posts/ringdrop-p2p-file-drop/

### W3C TAG "Good Practices for Capability URLs"
- [V] Put the secret in the fragment to avoid Referer leakage; third-party scripts can still read it. Use https. Capability URLs should expire, and single use is one option. Return 410/404 after expiry. The owner should be able to revoke, with multiple URLs per resource so one can be revoked alone. GET should not have side effects ("encode access permissions ... not actions"). List the URL space in robots.txt and rate-limit it. URLs leak via logs and history. https://www.w3.org/2001/tag/doc/capability-urls/

### Messenger link previews
- [V] Mysk 2020: iMessage, WhatsApp, Signal (if enabled), and Viber build previews on the sender side. Discord (up to 15 MB), Slack (up to 50 MB), LinkedIn (50 MB, runs JS), and FB Messenger/Instagram (no limit) fetch server-side and download content. Slack caches about 30 min. Telegram was not covered. https://www.mysk.blog/2020/10/25/link-previews/
- [V] Slackbot-LinkExpanding fetches minimal bytes with Range and does not honor robots.txt. https://api.slack.com/robots
- [V] TelegramBot "(like TwitterBot)" fetches to render previews. https://radar.cloudflare.com/bots/directory/telegrambot
- [P] None of these send the `#fragment` (HTTP never does), and none speak iroh. So a TikDL link, whose ticket is in the fragment and which needs an iroh client to fetch, cannot be consumed by a preview bot. The only exception would be if the landing page JS itself contacts the sender, and a JS-executing fetcher (LinkedIn-like) loads it.
- [V] General pattern (blog-level sources): GET = status only, consume on an explicit action. https://hashnode.com/posts/making-one-time-links-actually-one-time-fragments-preview-bots-and-an-atomic-delete/6adfa3253b9bd4b0552f0ce8

## 2. When does a link count as "used"? (comparison)
| System | Counted at | Resume / retry |
|---|---|---|
| Firefox Send | response fully written (finish) | aborted = not counted; retry unlimited |
| OnionShare | generator finished without cancel; locks others while in progress | cancel re-opens to anyone; Range partially supported |
| croc stored | receiver downloaded+authenticated+verified all files | not described |
| Bitwarden file Send | each content download | n/a |
| PrivateBin | after successful client-side decrypt callback (since 0.20) + confirm click (1.7.0) | n/a |
| Magic Wormhole | the code is consumed at the PAKE/nameplate claim (first peer) | no resume described |

## 3. Implications for TikDL [P/A]
- The cleanest "one recipient device" binding available in iroh is the first `ClientConnected.endpoint_id` that issues a valid Get. After that, ConnectMode::Intercept or per-request Intercept rejects other endpoint ids and allows the bound one to resume. This matches the OnionShare lock but survives interruption, unlike Firefox Send, where anyone could retry.
- "Burn" should happen on verified completion of the whole HashSeq. That means sender-side TransferCompleted for every child, or a receiver ack. It should not happen on first byte (resume breaks) or on first connect (preview/probe or a network failure burns it).
- The binding must persist on disk because TikDL resumes across restarts, and the receiver must keep a stable endpoint key for the same reason. Ephemeral receiver keys would break resume.
- A bearer ticket today contains the persistent sender endpoint id, so all links are linkable to one sender, and the relay/DNS sees the same id. For private shares, consider a fresh endpoint key per one-time share (as OnionShare uses a new onion per non-persistent share). Revocation then = drop that endpoint.
- Root-hash knowledge is not a secret per se: anyone who learns the hash plus the endpoint can fetch while the share is active. A one-time share needs an extra capability (a secret token in the ticket fragment, checked at first request), or the "first endpoint wins" race is open to anyone who sees the link first. The croc docs say this explicitly: "anyone who has it can ... claim one of the allowed downloads".
- iroh-blobs Get requests have no built-in auth token field. A secret must be carried via a custom ALPN/handshake, or by deriving the hash/endpoint itself as the secret (a per-share ephemeral endpoint id functions as an unguessable capability).
- Out-of-band verification: an optional short code (Magic Wormhole --verify / LocalSend PIN style) or a receiver-side accept prompt on the sender ("device X wants to download – allow?") mitigates the race in which the link is intercepted.
