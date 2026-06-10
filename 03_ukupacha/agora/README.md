# agora

> Public square. Federated identity and a trust graph that no one runs.

![agora-app: seven tiles over the same TrustGraph — identities with real Ed25519 keys, signed and verified attestations, a 2-of-2 multisig and wawa's control-plane envelopes](https://tawasuyu.net/03_ukupacha/agora/pantallazo.png)

`agora` is the identity layer of the monorepo. Each identity — person, community, alliance, institution — is an ed25519 public key. Each assertion about it is a `Claim`. Each endorsement is an `Attestation`: a claim signed by an attester. The truth of the agora is not dictated by a server: it emerges from who attests to what, weighted by a `TrustPolicy` that every reader negotiates for themselves.

There is no central registry, no moderation algorithm, no feed. The same shape — pubkey + signed attestations — covers a single person, a neighborhood community, a federation, or an institution.

## Install

```sh
cargo run --release -p agora-app
```

## Compatibility

- **Linux / macOS / Windows / Wawa** — all crates are pure Rust.
- Local persistence; optional convergence with peers from the `minga` mesh.

## Crates

| Crate | Role |
|---|---|
| [`agora-core`](agora-core/README.md) | Identities, claims, ed25519-signed attestations. |
| [`agora-graph`](agora-graph/README.md) | TrustGraph: verified attestations + corroboration + negotiated policy. |
| [`agora-store`](agora-store/README.md) | Atomic JSON persistence with re-verification on load. |
| [`agora-keystore`](agora-keystore/README.md) | Encrypted private-seed storage (Argon2 + ChaCha20-Poly1305). |
| [`agora-gossip`](agora-gossip/README.md) | Transport-agnostic anti-entropy protocol over signed attestations. |
| [`agora-net-brahman`](agora-net-brahman/README.md) | libp2p bridge: registers `/agora/gossip/1.0.0` over `BrahmanNet` (shared with minga). |
| [`agora-channel`](agora-channel/README.md) | Signing adapter to wawa's `format::Canal` contract: signed roots/releases, channel-history verification, capability grants by bytecode hash (§14.1.3). |
| [`agora-cli`](agora-cli/README.md) | Shell CLI: identities (rotate/revoke), attestations, export/import, channels, and the `wawa` ceremonies (publicar / concesion / anunciar / revocar). |
| [`agora-app`](agora-app/README.md) | Llimphi UI: identities, attestations, composer, policy, wawa control tiles (capability/release). |

## Considerations

- **Pubkey identity**, never email or phone.
- The graph stores only **verifiable** evidence: any attestation with a broken signature is rejected at ingest.
- The verdict on a claim is not a property of the graph — `TrustPolicy` is **negotiated** per reader, and two readers may disagree legitimately on the same evidence.
- Self-attestation is preserved but flagged separately from third-party endorsement.
- Plays well with `minga`: when both are active, agora rides the same `BrahmanNet` node (one PeerId, one Kademlia, two stream protocols).

## Status (2026-06-10)

### Done
- Complete identity core: `agora-core` (identities, claims, Ed25519 attestations, multisig, lifecycle) + `agora-graph` (TrustGraph with corroboration and negotiated policy) + `agora-store` (atomic JSON persistence with re-verification on load) + `agora-keystore` (encrypted seeds Argon2 + ChaCha20-Poly1305).
- End-to-end key rotation/revocation (SDD #4, phases 1–5): primitives in `agora-core`, tombstones in the TrustGraph, persistence in the snapshot, CLI `identidad rotar/revocar`, and mirror in the Wawa kernel (`verificar_revocacion`, canonicalized to `format`).
- Control plane over Wawa: revocation overlay wired to the kernel + boot seam that anchors the offline grants (§14.1.3), `wawa concesion` ceremony, and host↔guest TAP transport that closes the Akasha-over-Ether bridge.
- P2P transport: `agora-gossip` (transport-agnostic anti-entropy) + `agora-net-brahman` (libp2p bridge `/agora/gossip/1.0.0` convergent with minga over a single `BrahmanNet`). Person discovery by `DhtKey::Persona` (Phase 2b).
- Llimphi UI `agora-app` with tiles (identities, attestations, multisig, policy, capability, release) + main and context menus; `agora-channel` (proposal/release forge) with demo and e2e migration test.

### Pending
- Capability table by bytecode hash (WAWA.md §14.1.3): **code-complete** — primitives (`firmar/verificar_capacidad`), kernel mirror (`verificar_concesion_capacidad`), intersection wired at load (`permisos_efectivos_de`), operator tool (`agora-cli wawa concesion`), boot anchoring (`sembrar_concesion`) and automated ceremony (`scripts/wawa-conceder-genesis.sh`) already exist. Only the **operator step** remains: run the ceremony with the slot-0 seed (seeds `assets/concesiones/`) and then flip `MODO_CAPACIDAD_ESTRICTO_GLOBAL = true`.
- Network convergence beyond the agora/minga pair: still no mass discovery nor rate-limit hardening in real adversarial scenarios.
- The 17 applications prioritized in `APLICACIONES.md` are design/roadmap; only the substrate (identity + trust graph) is implemented.
