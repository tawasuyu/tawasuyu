# wawa-explorer

> Visor host-side del DAG de Wawa.

Corre en un host Linux y lee el filesystem de Wawa **sin montar nada**: abre el `.img`, recorre el DAG content-addressed y muestra el árbol con detalle en Llimphi. Cliente Akasha (raw sockets) para inspeccionar Wawa corriendo. Útil para debugging, forensics y educación.

## Instalación

```sh
# leer una imagen .img
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img

# conectar a un Wawa corriendo (vía Akasha)
cargo run --release -p wawa-explorer-llimphi -- akasha://<host>:<port>
```

## Compatibilidad

- **Linux** — raw sockets requieren `CAP_NET_RAW` o `setcap`.
- **macOS** — sólo modo `.img` (raw sockets bloqueado por OS).
- **Windows** — sólo modo `.img`.

## Crates

| Crate | Rol |
|---|---|
| [`wawa-explorer-core`](wawa-explorer-core/README.md) | Lectura del `.img`, decode del DAG. |
| [`wawa-explorer-aoe`](wawa-explorer-aoe/README.md) | Cliente Akasha (raw sockets). |
| [`wawa-explorer-llimphi`](wawa-explorer-llimphi/README.md) | UI: árbol + panel de detalle. |

## Consideraciones

- **Read-only.** No muta el DAG ni el sistema en vivo.
- Akasha es un protocolo propio; raw sockets requieren permisos elevados o `cap_net_raw=p`.
- Útil para validar lo que `wawa-fs` materializa cuando algo no cuadra.
