# arje-installer

> Interactive installer of [arje](../../README.md).

Disk partitioning, image installation, [`arje-loader`](../arje-loader/README.md) registration, post-install config. Interactive TUI or `--unattended` with manifest.

## Usage

```sh
cargo run --release -p arje-installer
```

## Deps

- [`arje-packager`](../arje-packager/README.md), [`arje-loader`](../arje-loader/README.md), `nix`
