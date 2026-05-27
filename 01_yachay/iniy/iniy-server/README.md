# iniy-server

> HTTP server of [iniy](../README.md).

REST over [`iniy-store`](../iniy-store/README.md): endpoints `GET /documents`, `GET /affirms`, `POST /audit`, `GET /graph`. For integration with external frontends or review tools.

## Usage

```sh
cargo run --release -p iniy-server -- --port 7117
```

## Deps

- All `iniy-*` core
- `axum`, `tokio`, `serde_json`
