# minga

> `minga` (quechua: *trabajo comunitario voluntario*). Colaboración entre nodos.

Red de pares para el monorepo. DHT + P2P + VFS distribuido. Cualquier nodo puede aportar storage o cómputo a la red de `minga`; los protocolos garantizan que la identidad de los bytes (BLAKE3) es la verdad, no el path. Compatible con la ingesta de `wawa` y con el gossip de `agora`.

## Instalación

```sh
# nodo standalone
cargo run --release -p minga-cli

# explorer (ver peers + content)
cargo run --release -p minga-explorer-llimphi
```

## Compatibilidad

- **Linux / macOS / Windows / Wawa** — Rust nativo + tokio para I/O.

## Crates

| Crate | Rol |
|---|---|
| [`minga-core`](minga-core/README.md) | Modelo: peer, chunk, address. |
| [`minga-dht`](minga-dht/README.md) | DHT (Kademlia adaptado). |
| [`minga-p2p`](minga-p2p/README.md) | Capa P2P (libp2p o propio). |
| [`minga-vfs`](minga-vfs/README.md) | VFS distribuido. |
| [`minga-store`](minga-store/README.md) | Storage local. |
| [`minga-cli`](minga-cli/README.md) | CLI. |
| [`minga-explorer-llimphi`](minga-explorer-llimphi/README.md) | UI: peers, content, tráfico. |

## Consideraciones

- **No es BitTorrent**: el modelo es content-addressed BLAKE3 (matchea wawa), no hash de torrent.
- **Privacidad por defecto**: nada se publica sin que el usuario lo marque como compartible.
- Diseñado para latencias domésticas/comunitarias, no para CDN globales.
