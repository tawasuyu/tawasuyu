# agora

> Public square. Federated identity and a trust graph that no one runs.

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
| [`agora-app`](agora-app/README.md) | Llimphi UI: identities, attestations, composer, policy. |

## Considerations

- **Pubkey identity**, never email or phone.
- The graph stores only **verifiable** evidence: any attestation with a broken signature is rejected at ingest.
- The verdict on a claim is not a property of the graph — `TrustPolicy` is **negotiated** per reader, and two readers may disagree legitimately on the same evidence.
- Self-attestation is preserved but flagged separately from third-party endorsement.
- Plays well with `minga`: when both are active, agora rides the same `BrahmanNet` node (one PeerId, one Kademlia, two stream protocols).
