# supay-app-llimphi

> Binary of [supay](../README.md).

Starts the driver with `wawa-config-llimphi` (theme, controls). Handles input → `DoomEngine::push_key`. Loops at the engine's cadence.

## Usage

```sh
cargo run --release -p supay-app-llimphi
```

## Deps

- [`supay-doom-llimphi`](../supay-doom-llimphi/README.md), [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
