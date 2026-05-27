# agora-core

> Model of [agora](../README.md): thread, message, author, signature.

`Thread { id, root_msg }`, `Message { author, body, signature, reply_to }`, `Author = ed25519::PublicKey`. Verifies signatures on receive. No network, no storage — just types.

## Deps

- `ed25519-dalek`, `serde`, `blake3`
