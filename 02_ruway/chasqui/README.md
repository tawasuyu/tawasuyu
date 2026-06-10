# chasqui

> `chasqui` (Quechua: *messenger of the Inca road*). Type broker + semantic Monads.

Dual domain (see `ARQUITECTURA.md`, the authoritative technical doc). **Brahman**: a deterministic type broker — modules (Cards) declare typed input/output flows and the broker matches consumer↔producer (exact + structural matching, priorities, per-context biases) without moving any data. **Nouser**: data intelligence — scans directories into Monads (semantic file clusters) enriched by swappable embedding providers behind the `nous` contract: `mock` (deterministic 32d) and `real` (ONNX 384d). Consumer discovery is local-first with DHT fallback (`card-sidecar::discovery::resolve_provider`), plus remote connect-and-consume over libp2p (`consume_remote`).

## Install

```sh
cargo run --release -p chasqui-core --bin chasqui        # Nouser CLI: scan|show|json|daemon|attract
cargo run --release -p chasqui-broker-explorer-llimphi   # broker probe UI
cargo run --release -p chasqui-explorer-llimphi          # Monad explorer UI
```

The Brahman broker is a library hosted by arje's init (`arje-zero`), not a standalone binary.

## Compatibility

- **Linux / macOS** — native Rust; handshake over Unix sockets.
- **Remote** — handshake over libp2p stream (`card-net`): relay + dcutr + autonat, NAT no longer blocks.
- The broker lives in the Init's memory — ephemeral by design, no snapshot/recover.

## Crates

| Crate | Role |
|---|---|
| [`chasqui-broker`](chasqui-broker/README.md) | Brahman: type-matching library (Exact/Structural, priorities, contexts). |
| [`card-handshake`](card-handshake/) | Init↔module handshake: Unix socket local, libp2p stream remote. |
| [`card-sidecar`](card-sidecar/) | Keeps the session alive + discovery (`resolve_provider`, `consume_remote`). |
| [`card-admin`](card-admin/) | Broker state snapshot (sessions + matches) — `brahman-status`. |
| [`chasqui-core`](chasqui-core/README.md) | Nouser: scanner, deterministic clustering, MonadDb, `chasqui` CLI. |
| [`chasqui-card`](chasqui-card/README.md) | Monad manifest + query client (`resolve_monad`). |
| [`chasqui-nous`](chasqui-nous/README.md) | Nous contract: JSON line-delimited over Unix socket. |
| [`chasqui-nous-mock`](chasqui-nous-mock/README.md) | Deterministic 32d pseudo-embeddings provider. |
| [`chasqui-nous-real`](chasqui-nous-real/README.md) | 384d ONNX embeddings provider (`embeddings` feature). |
| [`chasqui-broker-explorer-llimphi`](chasqui-broker-explorer-llimphi/README.md) | Broker probe UI: status + match timeline. |
| [`chasqui-explorer-llimphi`](chasqui-explorer-llimphi/README.md) | Monad explorer UI with semantic search. |

## Considerations

- **The broker matches types, it does not move data.** Each module opens its own data plane (`service_socket`).
- **Ephemeral by design.** The broker is the Init's in-memory registry of what's alive *now* — no persistence debt.
- **Not a pub/sub bus.** That original aspiration migrated to the Ayni domain; chasqui carries no app↔app messages in real time.
