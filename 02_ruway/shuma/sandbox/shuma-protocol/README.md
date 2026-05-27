# shuma-protocol

> Wire protocol (SSH/mosh replacement) of [shuma](../../README.md).

Binary over TCP/TLS (with `rustls`). Resilient reconnect, native multiplexing, integrated file transfer. **No SSH server required**: the daemon speaks this protocol.

## Deps

- `serde`, `postcard`, `rustls`
