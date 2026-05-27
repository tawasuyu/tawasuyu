# wawa-boot

> Bootloader del kernel de [wawa](../README.md).

Bootstrapping mínimo: lee la imagen del DAG, verifica firma, carga [`wawa-kernel`](../wawa-kernel/README.md), le pasa control. Compatible con UEFI directo y como segunda etapa de GRUB/systemd-boot.

## Build

```sh
cargo build --release -p wawa-boot
```

## Deps

- `uefi`, `ed25519-dalek`
