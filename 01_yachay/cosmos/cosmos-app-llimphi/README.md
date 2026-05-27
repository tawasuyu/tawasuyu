# cosmos-app-llimphi

> Desktop app of [cosmos](../README.md): sky map + interactive ephemerides.

Binary for human use: sky map from your location (auto-detect or manual), live ephemerides table (planets, moon, sun), upcoming events (eclipses, transits, rise/set), fuzzy catalog search. "Tonight" panel with the best time for each visible constellation.

## Usage

```sh
cargo run --release -p cosmos-app-llimphi
```

## Deps

- All `cosmos-*` core + [`cosmos-canvas-llimphi`](../cosmos-canvas-llimphi/README.md), [`cosmos-engine`](../cosmos-engine/README.md), [`cosmos-skywatch`](../cosmos-skywatch/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/)
