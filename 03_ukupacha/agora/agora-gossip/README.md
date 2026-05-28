# agora-gossip

> Transport-agnostic anti-entropy for the [agora](../README.md) `TrustGraph`.

Two peers converge by exchanging hashes of attestations they hold. Three message variants are enough: `Announce(Digest)` — *"I have these"* — broadcasts a `BTreeSet<AttestationHash>`; `Request(Vec<Hash>)` — *"send me these"* — picks up the diff; `Bundle(Vec<Attestation>)` — *"here they are"* — closes the round. Identity is `Attestation::stable_hash` (BLAKE3 over claim+key+signature), deterministic across machines.

No IO, no signing, no peer discovery, no storage. The caller picks the transport (libp2p in [`agora-net-brahman`](../agora-net-brahman/README.md), Akasha-Over-Ether in Wawa, SCP'd JSON over sneakernet) and the wire is just `serde`. Every `Bundle` re-enters `TrustGraph::add_attestation` so signatures are re-verified — a malicious peer cannot inject forged evidence by gossip.

## Deps

- [`agora-core`](../agora-core/README.md), [`agora-graph`](../agora-graph/README.md), `serde`
