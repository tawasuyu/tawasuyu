# supay-app-llimphi

> Binario de [supay](../README.md).

Arranca el driver con `wawa-config-llimphi` (theme, controles). Maneja input → `DoomEngine::push_key`. Loop a la cadencia del motor.

## Uso

```sh
cargo run --release -p supay-app-llimphi
```

## Deps

- [`supay-doom-llimphi`](../supay-doom-llimphi/README.md), [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
