# minga-core

> Model of [minga](../README.md): peer, chunk, address.

`Peer { id: PublicKey, addr }`, `Chunk { hash: Blake3, data }`, `Address = Blake3`. Types without network — imported by core and transport.

## Deps

- `serde`, `blake3`, `ed25519-dalek`
