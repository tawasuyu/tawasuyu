# khipu

> `khipu` (quechua: nudos de cuerdas para registrar memoria). Notas con gravedad temporal.

Captura de notas rápidas donde el olvido es parte del modelo: cada nota tiene una masa que decae con el tiempo y se refuerza con cada acceso. Lo recurrente queda visible; lo que no se vuelve a tocar se va difuminando hasta caer del horizonte.

## Instalación

```sh
cargo run --release -p khipu-app
```

## Compatibilidad

- **Linux / macOS / Windows** — UI Llimphi (Wayland/X11/Win32 vía `winit`).
- Persistencia local en `$XDG_DATA_HOME/khipu/`.

## Crates

| Crate | Rol |
|---|---|
| [`khipu-core`](khipu-core/README.md) | Modelo de nota + store; sin UI. |
| [`khipu-gravity`](khipu-gravity/README.md) | Algoritmo de masa/decay; refuerzo por acceso. |
| [`khipu-app`](khipu-app/README.md) | UI Llimphi sobre el core. |

## Gravedad semántica (embeddings)

El canvas de la derecha agrupa las notas por afinidad. Los vectores salen del `verbo-daemon` si está corriendo; si no, de un hash-trigram local.

```sh
# Embeddings reales (clústeres y vecinos semánticos de verdad):
cargo run -p rimay-verbo-daemon-bin -- --provider fastembed   # escucha en $XDG_RUNTIME_DIR/verbo.sock
cargo run --release -p khipu-app                              # lo detecta solo al arrancar
```

Sin daemon, khipu cae al embebedor trigram de 16d — determinista y offline, idéntico al comportamiento histórico. El cálculo nunca bloquea la UI: viaja a un worker y reentra al bucle cuando termina. Si el espacio vectorial cambia entre dos arranques (arrancó/cayó el daemon, otro modelo), los vectores se recalculan automáticamente.

## Consideraciones

- **No es un sistema de "todo"** — no hay due-dates ni recordatorios; es un cuaderno con física propia.
- El decay es transparente: cada nota expone su masa actual; el usuario decide si la salva.
- Compatible con la red `agora` (03_ukupacha): notas pueden compartirse sin perder su gravedad local.
