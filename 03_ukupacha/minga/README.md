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
