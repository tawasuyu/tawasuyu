# cosmos-app-llimphi

> App de escritorio de [cosmos](../README.md): mapa del cielo + ephemerides interactivas.

Binario para uso humano: mapa del cielo desde tu ubicación (auto-detect o manual), tabla de ephemerides en vivo (planetas, luna, sol), próximos eventos (eclipses, tránsitos, rise/set), búsqueda fuzzy de objetos del catálogo. Panel "esta noche" con la mejor hora de cada constelación visible.

## Uso

```sh
cargo run --release -p cosmos-app-llimphi
```

## Deps

- Todos los `cosmos-*` core + [`cosmos-canvas-llimphi`](../cosmos-canvas-llimphi/README.md), [`cosmos-engine`](../cosmos-engine/README.md), [`cosmos-skywatch`](../cosmos-skywatch/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/)
