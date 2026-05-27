# khipu-app

> UI Llimphi sobre el core de [khipu](../README.md). Binario del usuario.

App de escritorio: lista de notas ordenadas por masa actual, editor inline (Markdown ligero), captura rápida (`Ctrl+N`), búsqueda fuzzy. Cada redibujo recalcula masa con [`khipu-gravity`](../khipu-gravity/README.md) y muestra notas con `mass > umbral`; las que cayeron se acceden por menú "archivo".

## Uso

```sh
cargo run --release -p khipu-app
```

## Deps

- [`khipu-core`](../khipu-core/README.md), [`khipu-gravity`](../khipu-gravity/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `text-editor`, `text-input`, `list`
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/) para preferencias compartidas
