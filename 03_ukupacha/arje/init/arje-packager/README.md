# arje-packager

> Packager of [arje](../../README.md): builds ISO / .img.

Takes a manifest + sources and produces a bootable artifact. ISO targets x86_64 BIOS/UEFI; `.img` for VMs and embedded.

## Usage

```sh
cargo run --release -p arje-packager -- build --target iso
```

## Deps

- [`arje-cas`](../../runtime/arje-cas/README.md), [`arje-snapshot`](../arje-snapshot/README.md)
