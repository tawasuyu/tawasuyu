# khipu

> `khipu` (quechua: nudos de cuerdas para registrar memoria). Notas con gravedad temporal.

![el mapa mental de khipu: nodos que respiran por masa, filamentos de afinidad del nodo seleccionado, topónimos de regiones bautizadas y el chip de bautizo de un clúster emergente](https://tawasuyu.net/00_unanchay/khipu/pantallazo.png)

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
| `khipu-share` | Sobres de notas firmados (Ed25519) y direccionados por contenido (BLAKE3) sobre agora; transporte TCP/LAN + descubrimiento UDP + identidad cifrada. |
| `khipu-brahman` | Transporte de sobres sobre libp2p (BrahmanNet): stream cifrado + descubrimiento por DHT. |
| [`khipu-app`](khipu-app/README.md) | UI Llimphi sobre el core: el mapa mental es la interfaz. |

## El mapa es la interfaz (mapa mental)

El lienzo de pensamientos ocupa toda la ventana; lista y editor son overlays que aparecen sólo cuando hacen falta (cajón «☰ notas» a la izquierda, editor flotante a la derecha; `Esc` cierra en cascada: bautizo → editor → cajón → foco). Lo que lo hace habitable:

- **Ancla persistida**: cada nota se domicilia una sola vez (`Note.pos`) en el baricentro de su parentela semántica (afinidad coseno) con separación mínima — los clústeres emergen y nada se reacomoda solo, así la memoria espacial puede recordar dónde vive cada cosa. Cámara con pan/zoom sobre lienzo infinito.
- **El mapa respira**: el render usa la masa viva (`khipu-gravity`) para tamaño y brillo — lo reciente arde, lo abandonado se apaga. Al seleccionar, filamentos de activación por difusión.
- **Zoom semántico**: cerca (zoom ≥ 1.6) el nodo deja de ser un punto y se abre *en su lugar del mapa* como tarjeta editable que viaja con pan/zoom; lejos, el editor cae al panel lateral.
- **Regiones emergentes**: cuando un clúster denso junta ≥3 notas visibles sin topónimo cerca, el mapa ofrece un chip «✛ nombrar zona» en su centroide; el nombre queda como landmark tenue detrás de los nodos (pertenencia por vecindad, no carpeta). Se bautiza *después* de ver el patrón.

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

Para compartir en vivo sin copiar archivos: `publicar` levanta un servidor TCP que sirve el cuaderno (puerto `KHIPU_BIND`, default `127.0.0.1:7700`) **y anuncia una baliza UDP** para que lo descubran en la LAN. `recibir` abre un panel con un **campo de dirección** (`host:puerto`, prellenado y editable) y, debajo, los **pares descubiertos en la LAN** (nombre · autor · dirección): click en uno para jalarle el cuaderno, o escribí una dirección y «jalar». El transporte es `std::net` puro y **no necesita ser confiable** — el receptor verifica firma + hash antes de ingerir; la baliza sólo dice *dónde*, no *qué*.

**WAN / libp2p**: el campo de dirección de `recibir` acepta dos formas, autodetectadas: `host:puerto` (TCP directo) o una **multiaddr libp2p** —directa `/ip4/…/p2p/<id>` o de circuito `/ip4/…/p2p/<relay>/p2p-circuit/p2p/<id>`. Al `publicar`, khipu sirve por libp2p (stream cifrado Noise sobre `BrahmanNet`, protocolo `/khipu/sobre/1.0.0`, vía `khipu-brahman`) y muestra tu dirección de marcado.

**NAT traversal**: `BrahmanNet` ahora trae **Circuit Relay v2 + DCUtR** (`card-net`). Un nodo alcanzable hace de relay; uno detrás de NAT reserva un circuito ahí y queda accesible vía la dirección de circuito. Configurá `KHIPU_RELAY=/ip4/…/tcp/…/p2p/<relay-id>` antes de `publicar` y khipu reserva el circuito y muestra la dirección para compartir. Las direcciones externas no se confían a ciegas: **AutoNAT** las confirma pidiendo dial-backs a otros peers, y sólo las confirmadas se anuncian (y entran en las reservas de relay). En la malla Brahman AutoNAT corre con `only_global_ips=false` para confirmar también en LAN/loopback.

**Descubrimiento por DHT**: con `KHIPU_BOOTSTRAP=/ip4/…/p2p/<id>` (un nodo de la malla), khipu se une a la DHT Kademlia al arrancar; `publicar` se anuncia bajo la clave khipu y `recibir` lista —además de los pares LAN— los pares hallados por DHT (filas `DHT · …<id>`), que se jalan por peer-id. Así dos khipu se encuentran sin compartir IP ni multiaddr a mano, sólo conociendo un bootstrap común. Verificado end-to-end en localhost (rendezvous + publicador + receptor; 4 tests en `khipu-brahman`).

La lógica vive en `khipu-share`: `net` (transporte TCP), `discovery` (baliza UDP) e `identity` (keystore). 19 tests + un test de integración que recorre la cadena completa descubrir→jalar→verificar en loopback.

## Estado (2026-06-10)

### Hecho

- `khipu-core` (modelo de nota + store) + `khipu-gravity` (masa/decay con refuerzo por acceso).
- `khipu-app`: UI Llimphi sobre el core, con menú principal y contextual; partida en módulos (`main` / `map` / `panels` / `net`).
- Rediseño mapa mental completo: canvas-raíz (lista/editor flotan), ancla persistida con cámara pan/zoom, masa viva como tamaño/brillo, zoom semántico in-situ y regiones emergentes bautizables.
- Gravedad semántica: clustering por embeddings del `verbo-daemon` (rimay), con fallback trigram 16d offline; cálculo en worker que no bloquea la UI.
- Compartir vía agora (`khipu-share`): sobres firmados Ed25519 + direccionados BLAKE3, identidad cifrada (Argon2id + ChaCha20-Poly1305 en keystore), compartir selectivo + procedencia del autor; transporte TCP/LAN + descubrimiento por baliza UDP (15 tests + integración loopback).
- WAN/P2P (`khipu-brahman` sobre libp2p/BrahmanNet): stream cifrado Noise, NAT traversal (Circuit Relay v2 + DCUtR), AutoNAT, descubrimiento por DHT Kademlia (4 tests e2e localhost).

### Pendiente

- Sincronización bidireccional / resolución de conflictos entre cuadernos (hoy es import unidireccional de sobres).
- Transferir física temporal opcional al compartir (hoy el contenido nace fresco en el receptor — decisión de diseño, no bug).
- Endurecimiento de la malla DHT en WAN real (probado en localhost/LAN).

## Consideraciones

- **No es un sistema de "todo"** — no hay due-dates ni recordatorios; es un cuaderno con física propia.
- El decay es transparente: cada nota expone su masa actual; el usuario decide si la salva.
- La gravedad es local y no transferible: compartir mueve el contenido, no la atención.
