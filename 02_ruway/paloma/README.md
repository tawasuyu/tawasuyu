# paloma

*Léelo en español: [LEEME.md](LEEME.md).*

> `paloma` (the carrier pigeon). Kind: **native mail client on Llimphi**.

The suite's email, native and browser-free. `paloma` replaces Gmail/Outlook
without leaning on a JIT-driven web app — it's the first utility of Tanda 1 in
`/APPS-NATIVAS.md` (the everyday "Google Workspace"). It speaks IMAP (inbox) and
SMTP (outbox) against any server and renders natively on Llimphi; when a message
needs rich HTML, `puriy` can paint it.

Like the rest of the suite, paloma is one domain = an agnostic `*-core` plus
interchangeable Llimphi frontends. The domain logic never knows who draws it.

## Why

Mail should not require a 100 MB web runtime to read three paragraphs. paloma is
a real native client: offline-first cache, synchronous Elm UI loop, and the
heavy stuff (semantic search, LLM assistance, signing, P2P delivery) wired in as
optional, local-first capabilities rather than cloud features. It also opens a
door SMTP can't: a **sovereign rail** where the address *is* a public key, so
"From" spoofing is structurally impossible.

## Anatomy of the crates

```
paloma-core      — agnostic model: addresses, messages, mailboxes, threads,
                   accounts, the transport trait, text search, signing bytes,
                   multilienzo bodies. No network, no UI.
paloma-config    — multi-account config (cuentas.json): accounts + active one,
                   provider presets (Gmail/Outlook), auth method (password|oauth2),
                   legacy cuenta.json migration. Agnostic; serde + file IO only.
paloma-net       — MIME bridge + IMAP (fetch) + SMTP (send); the NetBackend that
                   implements MailBackend against real servers. Secret is a
                   password or an OAuth2 access token (XOAUTH2).
paloma-oauth     — OAuth2 authorizer (bin): Authorization Code + PKCE loopback;
                   opens the browser, stores access/refresh tokens (0600).
paloma-store     — native on-disk cache (postcard + BLAKE3), offline-first.
paloma-llimphi   — three-pane frontend (mailboxes · threads · reading) + compose;
                   defines the capability traits, backend-agnostic.
paloma-app       — the launchable binary (`paloma`): wires NetBackend, store,
                   identity, and the optional engines; demo fallback.
paloma-semantic  — embedding index (rimay-verbo) for search by meaning: async
                   compute, synchronous pure ranking.
paloma-sign      — Ed25519 sign/verify over agora-core (the wire signature format).
paloma-rail      — sovereign P2P mail: the signed RailEnvelope, address = pubkey.
paloma-rail-net  — TcpRail, real network transport for the rail routed by identity.
paloma-contacts  — contact book: alias → address, hand-editable JSON.
```

## Distinctive capabilities

- **Search by meaning (rimay).** `paloma-semantic` builds an embedding index
  over cached messages; ranking is synchronous and pure for the UI loop, compute
  is async off-thread. Needs a `rimay-verbo` daemon to be useful; without it the
  semantic mode (🧠) gracefully falls back to exact search.
- **LLM-native mail (pluma-llm), local-first.** Per-thread **Summarize** and
  **AI Draft** actions via the `LlmAssistant` trait, dispatched off the UI thread.
  With `PLUMA_LLM_BACKEND=ollama` the mail never leaves the machine. No real
  backend → the ✨ buttons simply don't appear.
- **Ed25519 signing (agora).** Outgoing mail carries a signature over a canonical
  byte form (sender/recipients/subject/body, CRLF-normalized to survive the
  SMTP/MIME round-trip). The reader recomputes and verifies, painting a status
  badge. Today `Verified` means *integrity*; binding pubkey ↔ contact (agora's
  web of trust) is the next step.
- **Sovereign P2P rail.** Suite-to-suite mail with no SMTP: the unit is a signed
  `RailEnvelope` and **the address is the public key** (`<hex>@rail.suyu`), so
  there is no From spoofing — the signature binds sender, recipient, and body
  (forwarding to a third party doesn't validate). `paloma-rail-net::TcpRail`
  routes by identity over TCP; compose routes mixed recipients (rail addresses
  go sealed over the rail, the rest over SMTP).
- **Language multilienzo, like pluma.** A message can carry alternate bodies
  (`MailCuerpo { lang, tone, body_text }`); compose derives them with the LLM
  (✨ Lienzo +ES/+EN/+QU), they travel with the message, and the reading pane
  shows the lienzo in the reader's own language with a selector to switch.
- **Contact book.** Write to "Ana", not a 64-char hex. `paloma-contacts` resolves
  aliases → addresses (hand-editable JSON) on compose, and a ＋ Contact button
  saves a sender; for rail mail the saved address *is* the authenticated identity.

## Running it

```bash
# the app
cargo run -p paloma-app --release         # or: -p paloma-app --bin paloma

# UI demo (seeded MockBackend, no account needed)
cargo run -p paloma-llimphi --example buzon_demo --release

# headless connection probe (real IMAP+SMTP, no GUI; Gmail-aware)
cargo run -p paloma-app --bin paloma-test --release
```

Account config lives in `~/.config/paloma/cuentas.json` (or `PALOMA_CONFIG`): a
hand-editable JSON holding **several accounts** and which one is `active`. The
old single-account `cuenta.json` is migrated automatically. The easiest way to
edit it is the **Correo** diente of the wawa control panel (`wawa-panel`):
account list (add/duplicate/delete), per-account IMAP/SMTP, a provider preset
(Gmail/Outlook) that autofills servers, and the auth method. Secrets never live
in the file. With no config/credentials, or if IMAP fails, the app falls back to
the demo backend and says so in the status bar.

### Authentication: password or OAuth2

Each account picks an auth method:

- **Password** — the secret comes from the environment (`PALOMA_PASSWORD`); on
  Gmail use an **app** password.
- **OAuth2 (`XOAUTH2`)** — for Gmail/Outlook, which closed IMAP/SMTP to
  passwords. Register an OAuth *desktop* app at the provider, paste its
  `client_id` into the account (PKCE public client; `client_secret` stays empty
  unless required), then run the authorizer:

  ```bash
  paloma-oauth <account-id>          # Authorization Code + PKCE loopback flow
  paloma-oauth <account-id> --force  # force the browser flow (re-consent)
  ```

  It opens the browser, receives the code on `127.0.0.1`, and writes the token
  to `~/.config/paloma/oauth-<id>.json` (0600). The panel's **Autorizar** button
  runs the same helper. After that first authorization, **`paloma-app` refreshes
  the access token automatically** (via the stored `refresh_token`, in
  `paloma-oauth`'s lib): once on startup and **mid-session** too — the backend asks
  for a fresh token on every SMTP send and, if an IMAP op fails **with an auth
  error** (the server rejects the expired token), reconnects with a new token and
  retries once (other failures — network, missing mailbox — surface as-is). So you
  don't re-run the helper every hour — only if the refresh token itself is revoked.
  It authenticates over `XOAUTH2`.

## Key environment variables

- `PALOMA_PASSWORD` (or `PALOMA_IMAP_PASSWORD` / `PALOMA_SMTP_PASSWORD`) — account
  secret; on Gmail use an **app** password. `PALOMA_EMAIL` + `PALOMA_SEND_TEST=1`
  drive `paloma-test`.
- `PALOMA_FETCH_LIMIT` — how many recent messages to fetch (default 200).
- `PALOMA_RAIL_BIND` / `PALOMA_RAIL_PEERS` — listen address and dial list
  (`host:port,…`) for the network rail; without them the rail runs in-process
  loopback.
- `PALOMA_SEMANTIC` — `mock` for the deterministic embedder; otherwise it connects
  to the `rimay-verbo` daemon socket.
- `PLUMA_LLM_BACKEND` / `PLUMA_LLM_MODEL` — pick the LLM backend (e.g. `ollama`
  for local-first); without a real backend the assistant stays off.

## Status / pending

Cores, store, frontend, signing, the rail (protocol + TCP transport),
multilienzo (language), semantic search, and the contact book are implemented
and tested. Honest gaps:

- **Verify against a real server.** The IMAP (TLS/STARTTLS/plain) and SMTP paths
  are type-checked and exercised by tests but want a real-credentials run (use
  `paloma-test`).
- **Trust, not just integrity.** Bind `pubkey ↔ contact` via agora's web of trust
  so `Verified` means identity, and move the identity seed to an encrypted
  keystore (today it's `~/.config/paloma/identity.seed`, 0600, in the clear).
- **Rail discovery/NAT.** The rail assumes you know the peer's `host:port`;
  discovery and NAT traversal (via `card-net`/DHT) are pending.
- **Multilienzo by tone**, and binding the SMTP signature to the lienzos (over
  the rail they are already signed inside the envelope).
- **Calendar/Contacts (CalDAV/CardDAV)** sharing the account layer.
- **Rich HTML via puriy** on demand (today: stripped text).
- **Canvas inbox by meaning** — ordering by topic/attention (khipu-style) instead
  of a chronological list.
