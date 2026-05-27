# cosmos-server

> REST HTTP server of [cosmos](../README.md).

Endpoints `/position`, `/rise-set`, `/eclipses`, `/transits`, `/sky`, `/observer`, etc. Returns JSON. Configurable CORS. Designed for integration with mobile apps, external control panels, dashboards.

## Usage

```sh
cargo run --release -p cosmos-server -- --port 7172
```

## Deps

- [`cosmos-engine`](../cosmos-engine/README.md), [`cosmos-model`](../cosmos-model/README.md)
- `axum`, `tokio`, `serde_json`
