# khipu-app

> Llimphi UI over the core of [khipu](../README.md). The user binary.

Desktop app: list of notes sorted by current mass, inline editor (lightweight Markdown), quick capture (`Ctrl+N`), fuzzy search. Each redraw recomputes mass via [`khipu-gravity`](../khipu-gravity/README.md) and shows notes with `mass > threshold`; fallen ones are accessed via the "archive" menu.

## Usage

```sh
cargo run --release -p khipu-app
```

## Deps

- [`khipu-core`](../khipu-core/README.md), [`khipu-gravity`](../khipu-gravity/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `text-editor`, `text-input`, `list`
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/) for shared prefs
