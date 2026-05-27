# iniy-server

> HTTP server de [iniy](../README.md).

REST sobre [`iniy-store`](../iniy-store/README.md): endpoints `GET /documents`, `GET /affirms`, `POST /audit`, `GET /graph`. Para integración con frontends externos o herramientas de revisión.

## Uso

```sh
cargo run --release -p iniy-server -- --port 7117
```

## Deps

- Todos los `iniy-*` core
- `axum`, `tokio`, `serde_json`
