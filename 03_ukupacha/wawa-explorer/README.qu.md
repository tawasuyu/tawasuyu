<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# wawa-explorer

> Wawa-pa DAG host-pi ñawi.

Linux host-pi puriq, Wawa-pa filesystem-ta **mana monturayuq** ñawinchaq: `.img` kichaspaq, content-addressed DAGta puriq, sach'ata Llimphi-pi rikun. Akasha cliente (raw sockets) kawsaq Wawata qhawanapaq. Debugging, forensics, yachay.

## Churay

```sh
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img
cargo run --release -p wawa-explorer-llimphi -- akasha://<host>:<port>
```

## Tinkuy

- **Linux** — raw sockets `CAP_NET_RAW` utaq `setcap` munanankama.
- **macOS** — `.img` modo-lla.
- **Windows** — `.img` modo-lla.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`wawa-explorer-core`](wawa-explorer-core/README.md) | `.img` ñawinchaq, DAG ñawiq. |
| [`wawa-explorer-aoe`](wawa-explorer-aoe/README.md) | Akasha cliente. |
| [`wawa-explorer-llimphi`](wawa-explorer-llimphi/README.md) | UI: sach'a + detalle panel. |

## Yuyaykunaq

- **Read-only.** Mana DAGta nillanta kawsaq sistematachu tikran.
- Akasha sapan protocolo; raw sockets kallpawan permisos munanan.
- `wawa-fs` paqarisqa rikuy mana tinkukuptin yanapan.
