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
| [`minga-core`](minga-core/README.md) | Núcleo: AST semántico, direccionamiento por contenido, Merkle Search Tree, α-hashing por lenguaje. Lógica pura, sin IO. |
| [`minga-store`](minga-store/README.md) | Almacenamiento persistente sobre sled: nodos, atestaciones, MST. |
| [`minga-dht`](minga-dht/README.md) | Discovery typed (`DhtKey`: Code/Card/Persona/Service) sobre el Kademlia compartido vía `card-net`. |
| [`minga-p2p`](minga-p2p/README.md) | Protocolo de sync entre repos + `MingaPeer` sobre libp2p (`card-net`: relay + DCUtR + AutoNAT heredados). |
| [`minga-vfs`](minga-vfs/README.md) | Proyecta el repo direccionado por contenido como filesystem FUSE de sólo lectura. |
| [`minga-cli`](minga-cli/README.md) | CLI: init, ingest, log/show/diff/blame, sign, verify, prune, sync, listen, mount, bundles. |
| [`minga-explorer-llimphi`](minga-explorer-llimphi/README.md) | Dashboard Llimphi del repo (stat cards sobre sled). |
| [`card-discovery`](card-discovery/) | Búsqueda de Cards: índice local + escaneo de directorios + discovery P2P sobre `minga-dht`. |

## Consideraciones

- **No es BitTorrent**: el modelo es content-addressed BLAKE3 (matchea wawa), no hash de torrent.
- **Privacidad por defecto**: nada se publica sin que el usuario lo marque como compartible.
- Diseñado para latencias domésticas/comunitarias, no para CDN globales.
