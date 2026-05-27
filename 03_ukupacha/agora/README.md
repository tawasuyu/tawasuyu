# agora

> Plaza pública. Foro, conversación, deliberación con identidad mínima.

`agora` es el espacio donde nodos del monorepo hablan en abierto. Gossip protocol sobre `chasqui` para distribución; identidad por clave pública (ed25519); grafo de hilos persistido localmente. Diseñado para sobrevivir sin servidor central, sin cuentas corporativas y sin moderación algorítmica.

## Instalación

```sh
cargo run --release -p agora-app
```

## Compatibilidad

- **Linux / macOS / Windows / Wawa** — todos los crates son puro Rust.
- Persistencia local; sincronización opcional con peers de la red `minga`.

## Crates

| Crate | Rol |
|---|---|
| [`agora-core`](agora-core/README.md) | Modelo: hilo, mensaje, autor, firma. |
| [`agora-graph`](agora-graph/README.md) | Grafo de hilos + relaciones. |
| [`agora-store`](agora-store/README.md) | Persistencia local. |
| [`agora-gossip`](agora-gossip/README.md) | Gossip protocol sobre chasqui. |
| [`agora-app`](agora-app/README.md) | UI Llimphi. |

## Consideraciones

- **Identidad por clave**, no por email ni teléfono.
- Sin algoritmo de feed: el orden es cronológico o por hilo; el usuario decide qué seguir.
- Compatible con `minga`: agora corre encima de la red de pares de `minga` cuando ambos están activos.
