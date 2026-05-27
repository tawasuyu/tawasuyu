# cosmos-server

> HTTP server REST de [cosmos](../README.md).

Endpoints `/position`, `/rise-set`, `/eclipses`, `/transits`, `/sky`, `/observer`, etc. Devuelve JSON. CORS configurable. Pensado para integrar con apps móviles, paneles de control externos, dashboards.

## Uso

```sh
cargo run --release -p cosmos-server -- --port 7172
```

## Deps

- [`cosmos-engine`](../cosmos-engine/README.md), [`cosmos-model`](../cosmos-model/README.md)
- `axum`, `tokio`, `serde_json`
