# agora-gossip

> Anti-entropy transport-agnóstico para el `TrustGraph` de [agora](../LEEME.md).

Dos pares convergen intercambiando hashes de las atestaciones que tienen. Tres variantes de mensaje bastan: `Announce(Digest)` — *"tengo estas"* — broadcastea un `BTreeSet<AttestationHash>`; `Request(Vec<Hash>)` — *"mandame estas"* — pide la diferencia; `Bundle(Vec<Attestation>)` — *"aquí van"* — cierra la ronda. La identidad es `Attestation::stable_hash` (BLAKE3 sobre claim+key+firma), determinista entre máquinas.

Sin IO, sin firmar nada, sin descubrir peers, sin storage. El caller elige el transporte (libp2p en [`agora-net-brahman`](../agora-net-brahman/LEEME.md), Akasha-Over-Ether en Wawa, JSON por SCP en sneakernet) y el wire es sólo `serde`. Cada `Bundle` vuelve a entrar por `TrustGraph::add_attestation`, así las firmas se re-verifican — un par malicioso no puede inyectar evidencia falsa por gossip.

## Deps

- [`agora-core`](../agora-core/LEEME.md), [`agora-graph`](../agora-graph/LEEME.md), `serde`
