# wawa (userspace)

> Pareja userspace de `03_ukupacha/wawa`: panel de control + CLI.

Acá vive lo que el operador de Wawa usa **desde un host Linux** (no desde adentro del kernel): el panel Llimphi para ver/mutar config, y `wawactl` para operaciones desde terminal. La parte de kernel/bootloader/filesystem está en `03_ukupacha/wawa/`. Detalle en [SDD.md](SDD.md).

## Instalación

```sh
# panel desktop (Llimphi)
cargo run --release -p wawa-panel-llimphi

# CLI
cargo run --release -p wawactl
```

## Compatibilidad

- **Linux** — primary host. Habla con `wawa-kernel` via virtio-console o socket Unix.
- **macOS / Windows** — sólo si Wawa corre en VM accesible (TCP).

## Crates

| Crate | Rol |
|---|---|
| [`wawa-panel-llimphi`](wawa-panel-llimphi/README.md) | Panel de control Llimphi: estado de apps, config, recursos. |
| [`wawactl`](wawactl/README.md) | CLI: `wawactl status`, `wawactl deploy`, etc. |

## Consideraciones

- **Userspace, no kernel.** Si necesitás tocar boot/fs/proc del Wawa, andá a `03_ukupacha/wawa`.
- El panel y `wawactl` comparten el modelo de config con el shell del escritorio (via `shared/wawa-config`).
