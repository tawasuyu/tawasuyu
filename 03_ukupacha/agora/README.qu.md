<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# agora

> Rikukuq llaqta. Federasqa identidad, mana piqpa kamachisqan iniy-sach'a.

`agora` monorepu identidad ladu kan. Sapanka identidad — runa, ayllu, alianza, kamachiq — huk ed25519 puyay clave kan. Sapanka rimay payqa `Claim` kan. Sapanka yanapay `Attestation` kan: huk claim atestiguador firmasqa. Ágorapa cheqaq mana servidorpa: piqkuna ima atestiguanku, sapanka rikuq `TrustPolicy`-nta wak munasqa-rayku.

Hatun registro mana kan, moderar algoritmu mana kan, feed mana kan. Misma simi — pubkey + firmasqa atestaciones — runamantapacha kamachi institucionkamapacha llapanchu chaska.

## Churay

```sh
cargo run --release -p agora-app
```

## Tinkuy

- **Linux / macOS / Windows / Wawa** — Rust ch'uya tukuy crates.
- Lokal waqaychay; opcional convergencia `minga`-pa peers-wan.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`agora-core`](agora-core/LEEME.md) | Identidades, claims, ed25519 firmasqa atestaciones. |
| [`agora-graph`](agora-graph/LEEME.md) | TrustGraph: verificasqa atestaciones + corroboración + negociasqa política. |
| [`agora-store`](agora-store/LEEME.md) | JSON waqaychay, kargaspa firmata kutichi-verificay. |
| [`agora-keystore`](agora-keystore/LEEME.md) | Cifrasqa seed waqaychay (Argon2 + ChaCha20-Poly1305). |
| [`agora-gossip`](agora-gossip/LEEME.md) | Transport-mana-akllaq anti-entropy protocolu. |
| [`agora-net-brahman`](agora-net-brahman/LEEME.md) | libp2p chaka: `/agora/gossip/1.0.0` `BrahmanNet` patanpi (minga-wan compartisqa). |
| [`agora-app`](agora-app/LEEME.md) | Llimphi UI: identidades, atestaciones, compositor, política. |

## Yuyaykunaq

- **Puyay clave identidad**, manan email manan teléfono.
- Grafu apaq **verificay atinata** llapan: firmaynin p'akisqa atestación, manapuni chaskina.
- Veredicto mana grafupa, `TrustPolicy` sapanka rikuqpaq **negociasqa** — iskay rikuqkuna kasqallaq evidenciata mana hukniyninta munankuman.
- Auto-atestación waqaychasqa kashan, ichaqa wakichaq runakunamanta t'aqasqa.
- `minga`-wan tinkun: iskayninku kawsaqtin, ágora kasqa `BrahmanNet` nodo patanpi puriy (huk PeerId, huk Kademlia, iskay sub-protocolo).
