# agora-core

> Modelo de [agora](../README.md): hilo, mensaje, autor, firma.

`Thread { id, root_msg }`, `Message { author, body, signature, reply_to }`, `Author = ed25519::PublicKey`. Verifica firma al recibir. Sin red, sin storage — sólo tipos.

## Deps

- `ed25519-dalek`, `serde`, `blake3`
