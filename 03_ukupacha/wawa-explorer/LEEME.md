# wawa-explorer

> Visor host-side del DAG de Wawa.

Corre en un host Linux y lee el filesystem de Wawa **sin montar nada**: abre el `.img`, recorre el DAG content-addressed y muestra el árbol con detalle en Llimphi. Cliente Akasha (raw sockets) para traer objetos ausentes desde un Wawa vivo en la LAN. Útil para debugging, forensics y educación.

## Instalación

```sh
# leer una imagen .img (interfaz AoE auto-detectada)
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img

# interfaz AoE explícita para "fetch from peers"
cargo run --release -p wawa-explorer-llimphi -- /path/to/wawa.img eth0
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

## Estado (2026-06-09)

### Hecho
- `wawa-explorer-core`: lector de `.img` y decodificación del DAG direccionado por contenido (modo offline/forense en Linux/macOS/Windows), con ejemplo `dump`.
- `wawa-explorer-aoe`: cliente Akasha sobre raw sockets para inspeccionar un Wawa vivo — `anunciar_canal` + `servir` (el cable del lazo en vivo), con fragmentación de objetos grandes (Fase 65) y ejemplos `solicitar`/`servir_release`.
- `wawa-explorer-llimphi`: UI tree + panel de detalle; abre el `.img` y, para nodos referenciados pero ausentes, ofrece "fetch from peers" por AoE (interfaz pasada como segundo argumento o auto-detectada en `/sys/class/net/`; el payload llega verificado `blake3(payload) == id` y vive sólo en la sesión). Menús principal + contextuales (lote 4); chrome localizado con `rimay-localize`.

### Pendiente
- "Process monitor" de Wawa (censo de tareas del executor + balizas del compositor) — pieza futura del lado wawa, fuera de este crate (ver sandokan SDD §6.4).
- Capacidad de escritura/edición sigue deliberadamente ausente: el visor es read-only por diseño.
