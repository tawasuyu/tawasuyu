# format — the native format of tawasuyu

Canonical types of the **content-addressed DAG** (BLAKE3 + postcard),
shared between host and the `wawa` kernel. `#![no_std]` — it crosses the border
to the bare-metal kernel by `path`. It is the format in which ALL the suite works
natively (foreign formats come in through `shared/foreign-*` and are converted to
this).

## Modules

- `tipos` — objects, hashes, content identities.
- `cable` — references between objects (DAG edges).
- `firma` — Ed25519 signatures and verification.
- `pruebas` — capability revocation proofs (WAWA.md §14.1.3).
- `grafo` — DAG construction/traversal.
- `constantes` — format parameters (sizes, versions).

## Status (2026-05-31)

### Done
- Canonical DAG types (objects, cables, hashes) in `no_std`, validated on
  `wasm32-unknown-unknown` by `scripts/check-shared-cores.sh`.
- Ed25519 signature/verification (`firma`) and revocation proofs (`pruebas`),
  canonical shared kernel↔host for the §14.1.3 enforcement.
- `lib.rs` (2327 LOC) **split into thematic modules** (cable/firma/grafo/…).
- Broad suite (~52 tests).

### Pending
- On-disk format versioning/migration (a version field exists; upgrade
  policies still to be defined).
- More coverage of the end-to-end revocation paths.

## Place in the repo

`shared/format` — shared `no_std` core. Consumed by apps, `agora` and the
`wawa` kernel.
