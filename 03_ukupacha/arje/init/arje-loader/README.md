# arje-loader

> Kernel loader for [arje](../../README.md).

Decides which kernel to boot (linux, arje-kernel, wawa-kernel), loads image + initrd, transfers control. Supports optional verified-boot (ed25519 signature of the image).

## Deps

- `nix`, `ed25519-dalek`
