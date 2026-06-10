# nahual

> `nahual` (Nahuatl: *companion spirit*). Everyday viewers over Llimphi.

The suite's universal "open-with": a file shell that discerns any file **by content** (`shuma-discern` → `viewer_registry::pick`) and dispatches it to one of 12 in-process viewers — text, image (pan/zoom, EXIF), video (AV1/WebM/GIF), audio (with live spectrum), card, tree (JSON/TOML), hex, table (CSV/TSV), markdown, map (GeoJSON/GPX/KML, A* routing, PMTiles/MVT basemap), archive (zip/tar), font — plus a web handoff (HTML launches `puriy`). A thumbnail gallery and a `Source` trait (POSIX · wawa `.img` · nouser · minga) round out the universal front. Built on the same UI framework; preferences shared via `wawa-config`.

## Install

```sh
cargo run --release -p nahual-shell-llimphi     # shell + the 12 viewers
cargo run --release -p nahual-gallery-llimphi   # thumbnail gallery
```

The viewer crates are libraries the shell mounts; only the shell and the gallery are binaries.

## Compatibility

- **Linux / macOS / Windows** — native Llimphi UI.
- **Wawa** — the shell navigates wawa `.img` images (content-addressed objects) through the `Source` adapter in `nahual-source-core`, host-side over `wawa-explorer-core`.

Crate table in [LEEME.md](LEEME.md); detection/dispatch design and viewer registry in [ARQUITECTURA.md](ARQUITECTURA.md).

## Considerations

- **Viewers, not editors.** Edit the file → `nada`. Edit the image → `pineal` or external.
- New viewers register in-process in `viewer_registry`; the open-with seam (`external_handler_for` over `shared/app-bus`) resolves external registered apps by mime/lens.
- The `meta-schema`/`meta-runtime` libs aim at **defining a viewer in JSON** without code; today they're consumed by other domains (nakui), not yet by the shell.
