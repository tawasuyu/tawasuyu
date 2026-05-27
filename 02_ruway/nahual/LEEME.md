# nahual

> `nahual` (náhuatl: *espíritu acompañante*). Visores cotidianos sobre Llimphi.

Conjunto mínimo de viewers que el usuario espera de un escritorio: shell de archivos, viewer de texto, viewer de imagen. Implementados con la misma framework de UI; comparten preferencias con el resto del monorepo via `wawa-config`. Más un meta-runtime para definir nuevos viewers via schema sin escribir Rust desde cero.

## Instalación

```sh
cargo run --release -p nahual-shell-llimphi
cargo run --release -p nahual-file-explorer-llimphi
cargo run --release -p nahual-text-viewer-llimphi
cargo run --release -p nahual-image-viewer-llimphi
```

## Compatibilidad

- **Linux / macOS / Windows** — UI Llimphi nativa.
- **Wawa** — los viewers compilan adentro del kernel; el file explorer habla con `wawa-fs`.

## Crates

| Crate | Rol |
|---|---|
| [`meta-schema`](libs/meta-schema/README.md) | Schema declarativo de viewers. |
| [`meta-runtime`](libs/meta-runtime/README.md) | Runtime que monta un viewer desde schema. |
| [`nahual-shell-llimphi`](nahual-shell-llimphi/README.md) | Shell de archivos: navegación + acciones básicas. |
| [`nahual-file-explorer-llimphi`](nahual-file-explorer-llimphi/README.md) | File explorer con tree + previews. |
| [`nahual-text-viewer-llimphi`](nahual-text-viewer-llimphi/README.md) | Viewer de texto plano. |
| [`nahual-image-viewer-llimphi`](nahual-image-viewer-llimphi/README.md) | Viewer de imagen (PNG/JPEG/WebP). |

## Consideraciones

- **Visualizadores, no editores.** Si querés editar el archivo, `nada`. Si querés editar la imagen, `pineal` o un editor externo.
- El meta-runtime permite **definir un viewer en JSON** y obtener una app Llimphi sin código.
