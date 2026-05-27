# agora-net-brahman

Bridge between [`agora-gossip`](../agora-gossip/) (pure Announce/Request/Bundle protocol) and [`brahman-net`](../../../shared/card/card-net/) (shared libp2p P2P layer).

## What it solves

`agora-gossip` was transport-agnostic but transportless: it defined the protocol and left the wire to the caller. `agora-net-brahman` provides that wire by **reusing the same libp2p node** that `MingaPeer` already opens — one `PeerId`, one Kademlia table, two stream sub-protocols coexisting:

- `/minga/sync/1.0.0` — minga CAS graph sync.
- `/agora/gossip/1.0.0` — agora TrustGraph convergence.

Fulfils the README promise: *"agora runs over minga's peer network when both are active"*.

## Usage

```rust
use std::sync::Arc;
use card_net::BrahmanNet;
use agora_net_brahman::AgoraNet;
use minga_p2p::MingaPeer;

// 1. One shared libp2p node.
let net = Arc::new(BrahmanNet::new()?);

// 2. Minga adopts it.
let minga = MingaPeer::open_with_node(minga_keypair, repo_path, Arc::clone(&net))?;
let _ = minga.run_passive_accept();

// 3. Agora adopts it too — same PeerId.
let agora = AgoraNet::sharing(Arc::clone(&net), trust_graph);
let _ = agora.run_passive_accept();

// 4. listen() once covers both protocols.
let addr = minga.listen("/ip4/0.0.0.0/tcp/4001".parse()?).await;
```

For processes running only agora:

```rust
let agora = AgoraNet::standalone(trust_graph)?;
```

## Wire protocol

An active gossip round is **PUSH from the initiator**:

1. A → B: `Announce(haves_A)`.
2. B → A: `Request(haves_A − haves_B)` (if B is behind).
3. A → B: `Bundle(those attestations)`.
4. B merges — every `Attestation::verify` runs before acceptance.

For bidirectional sync each side runs its own `gossip_with`. There's no shared handshake — simpler, more auditable, and lets only one side need outbound connectivity.

Framing: `u32 LE len || postcard(Message)`, identical to minga's. Hard per-frame cap: 16 MB.

## Demo

```sh
cargo run -p agora-net-brahman --example convergencia_minga
```

Builds a single `BrahmanNet`, has both `MingaPeer` and `AgoraNet` adopt it, and shows both protocols living on the same `PeerId` under a single `listen`.
