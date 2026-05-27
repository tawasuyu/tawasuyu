# arje-loader

> Loader del kernel para [arje](../../README.md).

Decide qué kernel arrancar (linux, arje-kernel, wawa-kernel), carga la imagen + initrd, transfiere control. Soporta verified-boot opcional (firma ed25519 de la imagen).

## Deps

- `nix`, `ed25519-dalek`
