# agora-net-brahman

Puente entre [`agora-gossip`](../agora-gossip/) (protocolo puro Announce/Request/Bundle) y [`brahman-net`](../../../shared/card/card-net/) (capa P2P libp2p compartida).

## Qué resuelve

Antes de esta crate, `agora-gossip` era transport-agnóstico pero sin transporte real: definía el protocolo y dejaba el wire al caller. `agora-net-brahman` aporta ese wire **reutilizando el mismo nodo libp2p** que `MingaPeer` ya abre — un solo `PeerId`, una sola tabla Kademlia, dos sub-protocolos de stream coexistiendo:

- `/minga/sync/1.0.0` — sincronización del grafo CAS de minga.
- `/agora/gossip/1.0.0` — convergencia del TrustGraph de ágora.

Esto cumple la promesa del README de ágora: *"agora corre sobre la red de pares de minga cuando ambos están activos"*.

## Uso

```rust
use std::sync::Arc;
use card_net::BrahmanNet;
use agora_net_brahman::AgoraNet;
use minga_p2p::MingaPeer;

// 1. Un solo nodo libp2p compartido.
let net = Arc::new(BrahmanNet::new()?);

// 2. Minga lo adopta.
let minga = MingaPeer::open_with_node(minga_keypair, repo_path, Arc::clone(&net))?;
let _ = minga.run_passive_accept();

// 3. Ágora lo adopta también — mismo PeerId.
let agora = AgoraNet::sharing(Arc::clone(&net), trust_graph);
let _ = agora.run_passive_accept();

// 4. Listen una vez: cubre los dos protocolos.
let addr = minga.listen("/ip4/0.0.0.0/tcp/4001".parse()?).await;
```

Para procesos que sólo corren ágora:

```rust
let agora = AgoraNet::standalone(trust_graph)?;
```

## Protocolo en el cable

Una ronda de gossip activa es **PUSH desde el iniciador**:

1. A → B: `Announce(haves_A)`.
2. B → A: `Request(haves_A − haves_B)` (si B está atrasado).
3. A → B: `Bundle(esas atestaciones)`.
4. B mergea — cada `Attestation::verify` corre antes de aceptar.

Para sync bidireccional cada lado ejecuta su propio `gossip_with`. No hay handshake compartido — más simple, más auditable, y permite que sólo uno de los dos tenga conectividad saliente.

Framing: `u32 LE len || postcard(Message)`, idéntico al de minga. Cota dura por frame: 16 MB.

## Demo

```sh
cargo run -p agora-net-brahman --example convergencia_minga
```

Arma un solo `BrahmanNet`, lo adopta `MingaPeer` y `AgoraNet`, y muestra que ambos protocolos viven sobre el mismo `PeerId` con un solo `listen`.
