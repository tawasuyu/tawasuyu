<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# agora

> Rikukuq llaqta. Foro, rimaykuy, deliberar pisi identidadwan.

`agora` monorepu nodos hawapi rimaqkuna. Gossip protocolo `chasqui` patanpi; ed25519-pubkey identidad; sach'a-rimay lokal waqaychasqa. Hatun servidor mana munayuq, mana corporativo cuentas, mana algorítmika moderación.

## Churay

```sh
cargo run --release -p agora-app
```

## Tinkuy

- **Linux / macOS / Windows / Wawa** — Rust ch'uya tukuy crates.
- Lokal waqaychay; opcional sync `minga` ayllu-rednanpa peers.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`agora-core`](agora-core/README.md) | Modelo: sach'a, mensaje, autor, firma. |
| [`agora-graph`](agora-graph/README.md) | Sach'a grafu + relaciones. |
| [`agora-store`](agora-store/README.md) | Lokal waqaychay. |
| [`agora-gossip`](agora-gossip/README.md) | Gossip protocolo chasqui-rayku. |
| [`agora-app`](agora-app/README.md) | Llimphi UI. |

## Yuyaykunaq

- **Pubkey identidad**, manan email manan teléfono.
- Mana algorítmika feed: orden chronológico utaq sach'a-pi; runa qatipanankuna munaspa.
- `minga`-wan tinkun: agora `minga`-pa peer-red patanpi puriy iskayninku kawsaqtin.
