# ayni

> `ayni` (quechua: *reciprocity*). Sovereign person-to-person chat, local-first,
> serverless. The conversation treated as a reproducible cryptographic graph
> (BLAKE3 + DAG + postcard), `agora` Ed25519 identity, transport over
> `chasqui`/`minga`/`akasha`.

Full design document and thesis in [LEEME.md](LEEME.md). This page is
the live status summary.

## Crates

| Crate | Role |
|---|---|
| `ayni-core` | DAG of signed messages + membership/trust/receipts (`no_std`+alloc). |
| `ayni-crypto` | Ed25519 signing over agora + 1:1 E2EE (X25519/HKDF/ChaCha20-Poly1305). |
| `ayni-sync` | `Transporte` trait + `EnlaceTcp` + anti-entropy (Merkle diff). |
| `ayni-minga` | `EnlaceMinga`: P2P transport over libp2p. |
| `ayni-store` | DAG persistence + attachment blobs (dedup) over sled. |
| `ayni-app` | application core: transport + store + encryption + attachments + trust. |
| `ayni-cli` | terminal chat (bin `ayni`), thin frontend over `ayni-app`. |
| `ayni-llimphi` | Llimphi UI: chat + people + attachments + receipts. |
| `ayni-index` | local semantic search (rimay embeddings + cosine). |
| `ayni-ai` | multilienzo: translate/summarize/tone via `pluma-llm`. |

## Install

```sh
cargo run --release -p ayni-cli       # terminal chat
cargo run --release -p ayni-llimphi   # graphical UI
```

## Status (2026-05-31)

### Done

- **P0–P7 closed** (see LEEME.md phase by phase). `ayni-core` with signed DAG,
  deterministic topological order, and 17 green tests (membership/trust/receipts
  included).
- **1:1 E2EE** (P2): `CanalSeguro` X25519 + HKDF-SHA256 + ChaCha20-Poly1305, pair
  derived from the same agora seed; the wire only sees ciphertext.
- **Serverless** (P3): anti-entropy by Merkle diff, sled persistence, and
  `EnlaceMinga` (real P2P transport over libp2p) behind the same `Transporte` trait.
- **Local intelligence** (P4): `ayni-index` (cosine search) + `ayni-ai`
  (multilienzo translate/summarize/tone, deterministic Mock without credentials).
- **Cross-app** (P5): `Carga::Adjunto` as a live reference by hash, blobs
  deduplicated and content-verified.
- **Ayni on wawa** (P6/P6+): the same `ayni-core` runs as a WASM app on the
  bare-metal OS; it persists the conversation in the akasha object graph and broadcasts it
  over the OS's own network (EtherType `0x88B7`, no TCP/IP) with signature
  verification on receipt.
- **`ayni-app` + complete UI**: interchangeable transport (`--transporte tcp|minga`),
  local-first store, attaching with UX, symmetric receipts; two-column GUI
  (people/chat) with trust graph.
- **Menus** (batch 1): main menu + contextual menus in the Llimphi UI.

### Pending

- **Group MLS** (RFC 9420 / OpenMLS): group chat with forward + post-compromise
  secrecy. `CanalSeguro` is the seam where it will land; today's channel is 1:1 without PCS.
- **NAT traversal**: `minga`'s debt, not ayni's (today direct TCP + DHT on LAN).
- **In the wawa app**: complete anti-entropy over L2 (today the new arrives live,
  history reconciliation is missing) and session encryption.
