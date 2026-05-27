<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# wawa (userspace)

> `03_ukupacha/wawa`-pa userspace pareja: control panel + CLI.

Kayqa Wawa operador **Linux host-manta** usaqkuna (mana kernel ukhumanta): Llimphi panel estado/config, `wawactl` terminal opera-paq. Kernel/bootloader/filesystem `03_ukupacha/wawa/`-pi. Detalle [SDD.md](SDD.md)-pi.

## Churay

```sh
cargo run --release -p wawa-panel-llimphi
cargo run --release -p wawactl
```

## Tinkuy

- **Linux** — ñawpaq host. `wawa-kernel`-wan virtio-console utaq Unix socket-rayku rimaq.
- **macOS / Windows** — Wawa atisqa VM-pi (TCP).

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`wawa-panel-llimphi`](wawa-panel-llimphi/README.md) | Llimphi control panel. |
| [`wawactl`](wawactl/README.md) | CLI: `wawactl status`, `wawactl deploy`, etc. |

## Yuyaykunaq

- **Userspace, mana kernel.** Boot/fs/proc → `03_ukupacha/wawa`.
- Panel + `wawactl` config monorepupa escritorio-shell-wan huñunku (`shared/wawa-config`-rayku).
