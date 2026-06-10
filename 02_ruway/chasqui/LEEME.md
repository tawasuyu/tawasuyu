# chasqui

> `chasqui` (quechua: *mensajero del camino del inca*). Broker de tipos + Mónadas semánticas.

Dominio dual (ver `ARQUITECTURA.md`, el doc técnico autoritativo). **Brahman**: broker de tipos determinista — los módulos (Cards) declaran flujos tipados de entrada/salida y el broker matchea consumer↔producer (matching exact + structural, prioridades, sesgos por contexto) sin mover datos. **Nouser**: inteligencia de datos — escanea directorios en Mónadas (clusters semánticos de archivos) enriquecidas por proveedores de embeddings intercambiables tras el contrato `nous`: `mock` (32d determinista) y `real` (ONNX 384d). El discovery de consumidores es local-first con fallback DHT (`card-sidecar::discovery::resolve_provider`), más connect-and-consume remoto por libp2p (`consume_remote`).

## Instalación

```sh
cargo run --release -p chasqui-core --bin chasqui        # CLI Nouser: scan|show|json|daemon|attract
cargo run --release -p chasqui-broker-explorer-llimphi   # UI probe del broker
cargo run --release -p chasqui-explorer-llimphi          # UI explorador de Mónadas
```

El broker Brahman es una librería hospedada por el init de arje (`arje-zero`), no un binario standalone.

## Compatibilidad

- **Linux / macOS** — Rust nativo; handshake sobre sockets Unix.
- **Remoto** — handshake sobre stream libp2p (`card-net`): relay + dcutr + autonat, el NAT ya no bloquea.
- El broker vive en la memoria del Init — efímero por diseño, sin snapshot/recover.

## Crates

| Crate | Rol |
|---|---|
| [`chasqui-broker`](chasqui-broker/README.md) | Brahman: librería de matching de tipos (Exact/Structural, prioridades, contextos). |
| [`card-handshake`](card-handshake/) | Handshake Init↔módulo: Unix socket local, stream libp2p remoto. |
| [`card-sidecar`](card-sidecar/) | Mantiene viva la sesión + discovery (`resolve_provider`, `consume_remote`). |
| [`card-admin`](card-admin/) | Snapshot del estado del broker (sesiones + matches) — `brahman-status`. |
| [`chasqui-core`](chasqui-core/README.md) | Nouser: scanner, clustering determinista, MonadDb, CLI `chasqui`. |
| [`chasqui-card`](chasqui-card/README.md) | Manifiesto de Mónada + cliente de query (`resolve_monad`). |
| [`chasqui-nous`](chasqui-nous/README.md) | Contrato Nous: JSON line-delimited sobre Unix socket. |
| [`chasqui-nous-mock`](chasqui-nous-mock/README.md) | Proveedor de pseudo-embeddings 32d determinista. |
| [`chasqui-nous-real`](chasqui-nous-real/README.md) | Proveedor de embeddings 384d ONNX (feature `embeddings`). |
| [`chasqui-broker-explorer-llimphi`](chasqui-broker-explorer-llimphi/README.md) | UI probe del broker: estado + timeline de matches. |
| [`chasqui-explorer-llimphi`](chasqui-explorer-llimphi/README.md) | UI exploradora de Mónadas con búsqueda semántica. |

## Consideraciones

- **El broker matchea tipos, no mueve datos.** Cada módulo abre su propio data plane (`service_socket`).
- **Efímero por diseño.** El broker es el registro en memoria del Init de qué está vivo *ahora* — no es deuda de persistencia.
- **No es un bus pub/sub.** Esa aspiración original migró al dominio Ayni; chasqui no transporta mensajes app↔app en tiempo real.
