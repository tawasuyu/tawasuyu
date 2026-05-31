# minga

> `minga` (Quechua: *voluntary community labor*). Collaboration between nodes.

Peer-to-peer network for the monorepo. DHT + P2P + distributed VFS. Any node can contribute storage or compute; protocols guarantee that the bytes' identity (BLAKE3) is truth, not the path. Compatible with `wawa`'s ingestion and with `agora`'s gossip.

## Install

```sh
cargo run --release -p minga-cli
cargo run --release -p minga-explorer-llimphi
```

## Compatibility

- **Linux / macOS / Windows / Wawa** — native Rust + tokio for I/O.

Crates listed in [README.md](README.md).

## Considerations

- **Not BitTorrent**: model is content-addressed BLAKE3 (matches wawa), not torrent hash.
- **Privacy by default**: nothing is published without explicit user share.
- Designed for community/domestic latency, not global CDN.

## Estado (2026-05-31)

> Reporte técnico detallado en [REPORTE.md](REPORTE.md). Mapa arquitectónico en [ARQUITECTURA.md](ARQUITECTURA.md).

### Hecho
- VCS semántico P2P funcionalmente completo: `minga-core` (AST + CAS + MST + atestaciones + α-hashing por lenguaje), `minga-store` (sled: nodes/attestations/mst/roots/timestamps/path-history/alpha-paths/retractions), `minga-dht` (DhtKey tipado), `minga-p2p` (MingaPeer libp2p con sync, Kademlia, RootDeclaration y RetractPush en el wire), `minga-vfs` (FUSE + pretty-printer Python indent-aware).
- CLI rica (`minga-cli`): init, ingest, ingest-dir, watch (autoremove), log, show (+ diff/sexp), diff, blame, history, roots, signers, sign (vouching), retire, verify, prune (GC), sync (DHT lookup), listen (announce-all-roots), mount.
- Bundle offline ("USB-stick mode"): export/import single + export-all/import-all multi-bundle con zstd, re-verificación criptográfica end-to-end. Daemon HTTP read-only `serve` (axum) con auth Bearer opcional (`--token`/`MINGA_SERVE_TOKEN`).
- Convergencia con ágora: `MingaPeer` adopta `Arc<LibP2pNode>` compartido; un solo PeerId/listen sirve `/minga/sync/1.0.0` + `/agora/gossip/1.0.0`. Discovery de personas por `DhtKey::Persona` (Fase 2b).
- Frontends: `minga-explorer-llimphi` (dashboard con theme/lang reactivos vía wawa-config) + `shuma-module-minga` (tab del shell con raíces, verify, dot de retracciones); menús principal + contextuales (lote 2). `cargo check --workspace` verde.

### Pendiente
- `MingaPeer` genérico sobre `NodeStore` (backend sled directo en lugar de cargar todo a RAM en sync). Diferido con criterio: trigger cuando un repo real supere ~100k nodos (sin caso hoy).
- Roadmap "Grafo de la Verdad" (ver memoria del proyecto): GossipSub + VFS-lazy-P2P, reputación, +9 lenguajes — fuera del backlog ya cerrado.
