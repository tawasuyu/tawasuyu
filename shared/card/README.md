# card — identidad agnóstica de transporte

Contrato de identidad y membresía **independiente del transporte**: claves
Ed25519, `EspinaId` (hash de la clave pública), firmas y handshake de membresía
de una "espina" (red privada). El mismo contrato lo implementan dos transportes
distintos: `card-net` (libp2p) y `wawa-akasha` (protocolo propio de wawa).

## Subcrates

- **`card-core`** — el contrato agnóstico real: par Ed25519 → `EspinaId`,
  `Card` firmada, `handshake` de membresía (un miembro presenta su tarjeta
  firmada; el anfitrión la verifica contra la raíz de confianza de la espina).
  Sólo bytes firmados — no sabe de libp2p ni Akasha.
- **`card-net`** — espina dorsal P2P sobre **libp2p**: discovery (mDNS +
  Kademlia DHT), gossipsub, NAT traversal (Circuit Relay v2 + DCUtR + AutoNAT).
  Implementa el contrato de `card-core` sobre libp2p. Lo consume `khipu`.
- **`card-wit`** — **[DORMIDO]** binding WIT/wasm del contrato. Se reactiva
  cuando `card` cruce a apps WASM; hoy el contrato real es `card-core`.

## Estado (2026-05-31)

### Hecho
- `card-core`: identidad Ed25519 + `EspinaId` + handshake de membresía (contrato
  agnóstico declarado como la fuente de verdad).
- `card-net`: discovery (mDNS+DHT), gossipsub y NAT traversal completo
  (Relay v2 + DCUtR + AutoNAT); discovery de personas por `DhtKey::Persona`.

### Pendiente
- `card-wit`: dormido — binding WASM pendiente de reactivación.
- Espejo del contrato sobre `wawa-akasha` (transporte wawa) aún por cablear.
- Endurecer revocación / rotación de membresía de espina.

## Lugar en el repo

`shared/card` — contrato de identidad. `card-net` lo lleva a libp2p (khipu);
`agora` cubre firma/confianza de más alto nivel.
