# agora-core

> Modelo de identidad de [agora](../LEEME.md): identidades, claims, atestaciones firmadas ed25519.

`IdentityKind` — `Person | Community | Alliance | Institution` — comparten la misma estructura: una pubkey, un tipo, un nombre legible. La forma es fractal: que una comunidad atestigüe sobre una persona es la misma operación que una institución atestiguando sobre una comunidad.

Un `Claim` es una tripleta determinista — `sujeto · predicado = valor` (más `issued_at`). Una `Attestation` es un claim firmado por un `Keypair`. La verificación cubre dos cosas: (1) la firma cubre el claim bajo `attester_key`; (2) `attester` coincide con el id derivado de `attester_key` — nadie puede atribuir su firma a otra identidad.

`Attestation::stable_hash()` es `BLAKE3(claim.canonical_bytes() || attester_key || signature)`. Ed25519 aquí es determinista, así que el mismo par `(clave, claim)` produce siempre los mismos bytes y el mismo hash — la base de la convergencia por gossip.

## Deps

- `ed25519-dalek`, `serde`, `blake3`, `thiserror`
