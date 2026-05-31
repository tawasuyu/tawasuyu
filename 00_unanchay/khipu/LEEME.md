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
| `khipu-share` | Sobres de notas firmados (Ed25519) y direccionados por contenido (BLAKE3) sobre agora. |
| [`khipu-app`](khipu-app/README.md) | UI Llimphi sobre el core. |

## Gravedad semántica (embeddings)

El canvas de la derecha agrupa las notas por afinidad. Los vectores salen del `verbo-daemon` si está corriendo; si no, de un hash-trigram local.

```sh
# Embeddings reales (clústeres y vecinos semánticos de verdad):
cargo run -p rimay-verbo-daemon-bin -- --provider fastembed   # escucha en $XDG_RUNTIME_DIR/verbo.sock
cargo run --release -p khipu-app                              # lo detecta solo al arrancar
```

Sin daemon, khipu cae al embebedor trigram de 16d — determinista y offline, idéntico al comportamiento histórico. El cálculo nunca bloquea la UI: viaja a un worker y reentra al bucle cuando termina. Si el espacio vectorial cambia entre dos arranques (arrancó/cayó el daemon, otro modelo), los vectores se recalculan automáticamente.

## Compartir (agora)

`exportar` sella en `compartido.khipu` un sobre firmado Ed25519 con la identidad del cuaderno y direccionado por su hash BLAKE3 de contenido.

La identidad vive **cifrada** (Argon2id + ChaCha20-Poly1305, vía `agora-keystore`) en `<datos>/keys/` — la semilla privada nunca queda en claro en disco. Al primer intento de compartir, khipu pide una passphrase: la primera vez la crea, después la usa para descifrar. `KHIPU_PASSPHRASE` en el entorno desbloquea sin prompt (headless). Una `identidad.seed` en claro de versiones viejas se migra al keystore (y se borra el claro) automáticamente. Comparte **lo que el buscador esté filtrando** (vacío = todo el cuaderno), así que escribir en la búsqueda y exportar manda sólo ese subconjunto. `importar` verifica firma + hash y, si cuadra, ingiere las notas, marcándolas con una etiqueta de procedencia `de:<autor>` (visible en el editor como «✎ de: …»).

Lo que viaja es el **contenido** (título, cuerpo, etiquetas), nunca la física temporal: al importar, cada nota nace fresca (masa plena, acceso = ahora) — su gravedad arranca en el cuaderno que la recibe. Los wiki-links `[[Título]]` se rearman solos porque khipu resuelve enlaces por título. Reimportar el mismo sobre no duplica (se omiten títulos ya presentes). Un sobre alterado o con firma ajena se rechaza entero, sin autoridad central.

Para compartir en vivo por la LAN sin copiar archivos: `publicar` levanta un servidor TCP que sirve el cuaderno (puerto `KHIPU_BIND`, default `127.0.0.1:7700`) **y anuncia una baliza UDP** para que lo descubran. `recibir` escucha la LAN unos segundos y **lista los pares hallados en el panel izquierdo** (nombre · autor · dirección); hacés click en uno para jalarle el cuaderno, o «cancelar». Si nadie anuncia, cae a un par explícito (`KHIPU_PEER`). El transporte es `std::net` puro y **no necesita ser confiable** — el receptor verifica firma + hash antes de ingerir; la baliza sólo dice *dónde*, no *qué*.

La lógica vive en `khipu-share`: `net` (transporte TCP) y `discovery` (baliza UDP). 15 tests + un test de integración que recorre la cadena completa descubrir→jalar→verificar en loopback.

## Consideraciones

- **No es un sistema de "todo"** — no hay due-dates ni recordatorios; es un cuaderno con física propia.
- El decay es transparente: cada nota expone su masa actual; el usuario decide si la salva.
- La gravedad es local y no transferible: compartir mueve el contenido, no la atención.
