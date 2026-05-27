# minga-core

> Modelo de [minga](../README.md): peer, chunk, address.

`Peer { id: PublicKey, addr }`, `Chunk { hash: Blake3, data }`, `Address = Blake3`. Tipos sin red — los importan core y transport.

## Deps

- `serde`, `blake3`, `ed25519-dalek`
