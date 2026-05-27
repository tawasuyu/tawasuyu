# shuma-protocol

> Protocolo wire (reemplazo de SSH/mosh) de [shuma](../../README.md).

Binario sobre TCP/TLS (con `rustls`). Reconnect resilient, multiplexing nativo, transferencia de archivos integrada. **No requiere SSH server**: el daemon habla este protocolo.

## Deps

- `serde`, `postcard`, `rustls`
