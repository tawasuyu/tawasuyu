# agora-store

> Atomic JSON persistence for the [agora](../README.md) `TrustGraph`, with re-verification on load.

`save(path, &graph)` writes a versioned snapshot atomically (tmp + fsync + rename). `load(path)` reads to a private mirror struct and **reconstructs** the graph by calling `add_attestation` once per entry — so signatures are re-verified at load. A tampered file is a load error, not silent corruption: the contract that *"the graph only stores verifiable evidence"* extends to disk.

What this crate does **not** persist: private keys. The seed/Keypair never crosses the serde surface here — that belongs to [`agora-keystore`](../agora-keystore/README.md), where it is encrypted with a user passphrase.

## Deps

- [`agora-core`](../agora-core/README.md), [`agora-graph`](../agora-graph/README.md), `serde`, `serde_json`, `thiserror`
