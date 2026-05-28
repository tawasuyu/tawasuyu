# agora-channel

> Bridge between [agora](../README.md) Ed25519 identities and the wawa release-channel contract in [`format`](../../../shared/format).

`format` declares the wire-level types — `Canal`, `RaizFirmada`, `mensaje_a_firmar` — but states explicitly that *"the verification lives in `agora` (or in `firma`)"*. This crate is that verification. It uses [`agora-core::Keypair`](../agora-core/README.md) to produce and check signatures over the canonical message that `format::mensaje_a_firmar(nombre_canal, timestamp, raiz)` defines.

## What it does

- `firmar_raiz(kp, canal_nombre, raiz, timestamp) -> RaizFirmada` — signs a manifest root for a channel and produces the wire entry.
- `verificar_raiz(autor, canal_nombre, raiz)` — re-verifies a single `RaizFirmada` against the channel author's public key. Catches forged, truncated or replayed signatures.
- `verificar_canal(canal)` — walks the whole `raices` history of a `Canal`, verifying each entry under the channel's `autor` and also enforcing that `timestamp`s are strictly monotonic (no past-after-future replays).
- `firmar_para_anuncio(kp, canal_nombre, raiz, timestamp) -> (AgoraId, Firma)` — produces just the `(autor, firma)` pair that goes into `MensajeAkasha::AnunciarCanal`, so a caller that *does* depend on the `akasha` crate can assemble the frame without this crate having to.

## What it deliberately does *not* do

- It does **not** depend on `akasha`. `MensajeAkasha::AnunciarCanal` lives in the wawa bare-metal stack and excludes itself from the global workspace. `agora-channel` produces the cryptographic pieces; the frame assembly belongs to the side that has the network types.
- It does **not** decide trust policy. Whether a channel author is trustworthy in the first place is a question for the local trust graph — `agora-channel` only asserts cryptographic facts ("this signature is valid").

## Closes

- `PLAN.md:177` (*"Identidad agora Ed25519 firmable — pendiente"*).
- `WAWA.md §14.1` (*"verificación de firma + re-anclaje quedan para userspace"*) — the userspace side of the signature verification now exists in pure Rust.

## Deps

- [`agora-core`](../agora-core/README.md) (for `Keypair` and `verify_signature`)
- [`format`](../../../shared/format/) (for `Canal`, `RaizFirmada`, `mensaje_a_firmar`, `AgoraId`, `Firma`, `Hash`)
- `thiserror`
