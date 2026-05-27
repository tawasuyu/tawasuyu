# puriy-app

> Binario de [puriy](../README.md). Autodetect target Llimphi vs headless.

CLI mínimo. Si encuentra `WAYLAND_DISPLAY` o `DISPLAY`, abre la ventana Llimphi con [`puriy-llimphi`](../puriy-llimphi/README.md). Si no, corre en modo headless dumpeando el árbol de boxes — útil para CI y para verificar el motor sin pantalla.

## Uso

```sh
# auto-detect
cargo run --release -p puriy-app -- https://example.com

# forzar headless
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Deps

- [`puriy-engine`](../puriy-engine/README.md), [`puriy-llimphi`](../puriy-llimphi/README.md)
