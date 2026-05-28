# agora-core

> Identity model of [agora](../README.md): identities, claims, ed25519-signed attestations.

`IdentityKind` — `Person | Community | Alliance | Institution` — share the same shape: a pubkey, a kind, a display name. The structure is fractal: a community attesting about a person is the same operation as an institution attesting about a community.

A `Claim` is a deterministic triple — `subject · predicate = value` (plus `issued_at`). An `Attestation` is a claim signed by a `Keypair`. Verification covers two things: (1) the signature covers the claim under `attester_key`; (2) `attester` matches the id derived from `attester_key` — nobody can attribute their signature to another identity.

`Attestation::stable_hash()` is `BLAKE3(claim.canonical_bytes() || attester_key || signature)`. Ed25519 here is deterministic, so the same `(key, claim)` pair always yields the same bytes and the same hash — the basis of gossip convergence.

## Deps

- `ed25519-dalek`, `serde`, `blake3`, `thiserror`
