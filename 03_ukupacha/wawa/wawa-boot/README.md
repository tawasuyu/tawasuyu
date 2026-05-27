# wawa-boot

> Bootloader of the [wawa](../README.md) kernel.

Minimal bootstrapping: reads the DAG image, verifies signature, loads [`wawa-kernel`](../wawa-kernel/README.md), hands over control. Compatible with direct UEFI and as a second stage of GRUB/systemd-boot.

## Build

```sh
cargo build --release -p wawa-boot
```

## Deps

- `uefi`, `ed25519-dalek`
