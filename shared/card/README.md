# card — transport-agnostic identity

Identity and membership contract **independent of the transport**: Ed25519
keys, `EspinaId` (hash of the public key), signatures and the membership handshake
of an "espina" (private network). The same contract is implemented by two different
transports: `card-net` (libp2p) and `wawa-akasha` (wawa's own protocol).

## Subcrates

- **`card-core`** — the actual agnostic contract: Ed25519 pair → `EspinaId`,
  signed `Card`, membership `handshake` (a member presents its signed
  card; the host verifies it against the espina's root of trust).
  Only signed bytes — it knows nothing of libp2p or Akasha.
- **`card-net`** — P2P backbone over **libp2p**: discovery (mDNS +
  Kademlia DHT), gossipsub, NAT traversal (Circuit Relay v2 + DCUtR + AutoNAT).
  Implements the `card-core` contract over libp2p. Consumed by `khipu`.
- **`card-wit`** — **[DORMANT]** WIT/wasm binding of the contract. It reactivates
  when `card` crosses to WASM apps; today the actual contract is `card-core`.

## Status (2026-05-31)

### Done
- `card-core`: Ed25519 identity + `EspinaId` + membership handshake (agnostic
  contract declared as the source of truth).
- `card-net`: discovery (mDNS+DHT), gossipsub and complete NAT traversal
  (Relay v2 + DCUtR + AutoNAT); person discovery by `DhtKey::Persona`.

### Pending
- `card-wit`: dormant — WASM binding pending reactivation.
- Mirror of the contract over `wawa-akasha` (wawa transport) still to be wired.
- Harden espina membership revocation / rotation.

## Place in the repo

`shared/card` — identity contract. `card-net` carries it to libp2p (khipu);
`agora` covers higher-level signing/trust.
