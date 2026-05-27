# arje-absorb

> Ingests an existing system → arje object of [arje](../../README.md).

Reads a Linux host **read-only** and produces an independent arje object that reproduces the relevant state (installed packages, configs, user files). Destroys nothing on the source.

## Usage

```sh
cargo run --release -p arje-absorb -- /path/to/system
```

## Deps

- [`arje-cas`](../../runtime/arje-cas/README.md)
