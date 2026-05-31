# wawa-explorer

> Host-side viewer of Wawa's DAG.

Runs on a Linux host and reads Wawa's filesystem **without mounting anything**: opens the `.img`, walks the content-addressed DAG, shows the tree with detail in Llimphi. Akasha client (raw sockets) to inspect a running Wawa. Useful for debugging, forensics, and education.

## Install

```sh
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img
cargo run --release -p wawa-explorer-llimphi -- akasha://<host>:<port>
```

## Compatibility

- **Linux** — raw sockets require `CAP_NET_RAW` or `setcap`.
- **macOS** — `.img` mode only.
- **Windows** — `.img` mode only.

## Crates

| Crate | Role |
|---|---|
| [`wawa-explorer-core`](wawa-explorer-core/README.md) | `.img` reader, DAG decode. |
| [`wawa-explorer-aoe`](wawa-explorer-aoe/README.md) | Akasha client (raw sockets). |
| [`wawa-explorer-llimphi`](wawa-explorer-llimphi/README.md) | UI: tree + detail panel. |

## Considerations

- **Read-only.** Doesn't mutate the DAG or the live system.
- Akasha is a custom protocol; raw sockets require elevated permissions or `cap_net_raw=p`.
- Useful for validating what `wawa-fs` materializes when something doesn't add up.

## Estado (2026-05-31)

### Hecho
- `wawa-explorer-core`: lector de `.img` y decodificación del DAG direccionado por contenido (modo offline/forense en Linux/macOS/Windows), con ejemplo `dump`.
- `wawa-explorer-aoe`: cliente Akasha sobre raw sockets para inspeccionar un Wawa vivo — `anunciar_canal` + `servir` (el cable del lazo en vivo), con fragmentación de objetos grandes (Fase 65) y ejemplos `solicitar`/`servir_release`.
- `wawa-explorer-llimphi`: UI tree + panel de detalle; abre tanto `.img` como `akasha://host:port`. Menús principal + contextuales (lote 4).

### Pendiente
- "Process monitor" de Wawa (censo de tareas del executor + balizas del compositor) — pieza futura del lado wawa, fuera de este crate (ver sandokan SDD §6.4).
- Capacidad de escritura/edición sigue deliberadamente ausente: el visor es read-only por diseño.
