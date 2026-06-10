# nahual

> `nahual` (náhuatl: *espíritu acompañante*). Visores cotidianos sobre Llimphi.

El "abridor universal" de la suite: un shell de archivos que discierne cualquier archivo **por contenido** (`shuma-discern` → `viewer_registry::pick`) y lo despacha a uno de 12 visores in-process — texto, imagen (pan/zoom, EXIF), video (AV1/WebM/GIF), audio (con espectro en vivo), card, tree (JSON/TOML), hex, tabla (CSV/TSV), markdown, mapa (GeoJSON/GPX/KML, ruteo A*, basemap PMTiles/MVT), archive (zip/tar), fuente — más un despacho web (el HTML lanza `puriy`). Completan el front universal una galería de miniaturas y el trait `Source` (POSIX · imagen wawa `.img` · nouser · minga). Implementados con la misma framework de UI; comparten preferencias con el resto del monorepo via `wawa-config`.

## Instalación

```sh
cargo run --release -p nahual-shell-llimphi     # shell + los 12 visores
cargo run --release -p nahual-gallery-llimphi   # galería de miniaturas
```

Los crates de visores son bibliotecas que el shell monta; sólo el shell y la galería son binarios.

## Compatibilidad

- **Linux / macOS / Windows** — UI Llimphi nativa.
- **Wawa** — el shell navega imágenes wawa `.img` (objetos content-addressed) a través del adapter `Source` de `nahual-source-core`, host-side sobre `wawa-explorer-core`.

Diseño de detección/despacho y registro de visores en [ARQUITECTURA.md](ARQUITECTURA.md).

## Crates

| Crate | Rol |
|---|---|
| [`nahual-shell-llimphi`](nahual-shell-llimphi/README.md) | Shell de archivos (bin): navegación + despacho por contenido a los visores. |
| `nahual-gallery-llimphi` | Galería de miniaturas (bin): zoom de grilla, EXIF, slideshow, ordenamiento. |
| [`nahual-file-explorer-llimphi`](nahual-file-explorer-llimphi/README.md) | Lógica de exploración de directorios (lista virtualizada). |
| [`nahual-text-viewer-llimphi`](nahual-text-viewer-llimphi/README.md) | Visor de texto (fallback universal, syntax por extensión). |
| [`nahual-image-viewer-llimphi`](nahual-image-viewer-llimphi/README.md) | Visor de imagen (PNG/JPEG/WebP; pan/zoom, EXIF) sobre `llimphi-image`. |
| [`nahual-video-viewer-llimphi`](nahual-video-viewer-llimphi/README.md) | Reproductor de video (AV1 puro-Rust: WebM/MKV/IVF + GIF animado). |
| `nahual-audio-viewer-llimphi` | Reproductor de audio (WAV/MP3/FLAC/Opus/Vorbis; espectro 48 bandas). |
| `nahual-card-viewer-llimphi` | Visor estructurado de Cards (`shared/card`). |
| `nahual-tree-viewer-llimphi` | Árbol JSON/TOML indentado. |
| `nahual-hex-viewer-llimphi` | Volcado hex/ASCII para binarios (ELF/wasm). |
| `nahual-table-viewer-llimphi` | Tabla CSV/TSV con columnas alineadas. |
| `nahual-markdown-viewer-llimphi` | Markdown renderizado (pulldown-cmark). |
| `nahual-map-viewer-llimphi` | Mapa: GeoJSON/GPX/KML, inspección, choropleth, búsqueda, ruteo. |
| `nahual-archive-viewer-llimphi` | Listado de comprimidos (ZIP/jar/apk/epub/OOXML, tar, tar.gz). |
| `nahual-font-viewer-llimphi` | Fuentes TTF/OTF: metadatos + muestra dibujada con los contornos. |
| `nahual-viewer-core` | Núcleos agnósticos de GUI de los visores simples (parseo/decode + tipos de preview). |
| `nahual-geo-core` | Núcleo geoespacial agnóstico: parseo, proyección, hit-test, A*, basemap PMTiles v3 + MVT. |
| `nahual-source-core` | Trait `Source` + adapters (POSIX, wawa `.img`, nouser, minga). |
| `nahual-thumb-core` | Pipeline de miniaturas: generación, cache, cola priorizada al viewport. |
| [`meta-schema`](libs/meta-schema/README.md) | Schema declarativo de UIs data-driven. |
| [`meta-runtime`](libs/meta-runtime/README.md) | Helpers puros sobre el schema (parseo tipado, validación, delta). |

## Consideraciones

- **Visualizadores, no editores.** Si querés editar el archivo, `nada`. Si querés editar la imagen, `pineal` o un editor externo.
- Los visores nuevos se registran in-process en `viewer_registry`; la costura open-with (`external_handler_for` sobre `shared/app-bus`) resuelve apps externas registradas por mime/lens.
- Las libs `meta-schema`/`meta-runtime` apuntan a **definir un viewer en JSON** sin código; hoy las consumen otros dominios (nakui), todavía no el shell.
