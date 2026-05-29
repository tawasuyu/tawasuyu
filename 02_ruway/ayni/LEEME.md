# Ayni — chat persona-a-persona soberano

> **Ayni** (quechua): reciprocidad. *Yo te doy, vos me das.* Un chat P2P donde
> los pares se custodian y retransmiten mensajes mutuamente **es** ayni —
> anti-extracción por diseño—. Hace par con [`minga`](../../03_ukupacha/minga/),
> el otro concepto andino de cooperación que le da transporte.

Chat humano↔humano **local-first** y soberano. No hay servidor, no hay número
de teléfono, no hay empresa que pueda morir y llevarse tus conversaciones: sos
dueño de tus bytes. Las innovaciones no son features añadidas — salen del
sustrato de gioser (BLAKE3 + DAG direccionado por contenido, identidad `agora`
Ed25519, transporte `chasqui`/`minga`/`akasha`).

No es "otro wasap": es la conversación tratada como un grafo criptográfico
reproducible.

## Tesis

1. **Conversación = DAG direccionado por contenido**, no un log lineal. Los
   hilos son ramas reales; el estado es reproducible por hash; reordenar el
   hilo de un autor invalida su firma. Dos pares que vieron los mismos mensajes
   calculan **el mismo grafo**, sin un servidor que asigne números de secuencia.
2. **Identidad sin servidor ni teléfono** (`agora`, Ed25519). Cada mensaje
   firmado ⇒ no-repudio; la confianza emerge del grafo de atestaciones.
3. **E2EE con MLS (RFC 9420)** vía OpenMLS — forward + post-compromise security,
   credencial MLS = identidad agora. *Nunca cripto a mano.*
4. **Transporte P2P sin servidor** por minga: diff de Merkle (sólo viaja lo que
   falta), DHT, store-and-forward offline (= ayni). Corre en Linux **y** en wawa.
5. Búsqueda semántica local (`rimay`), multilienzo en mensajes (traducción /
   resumen vivos, "máquina propone, humano firma"), adjuntos como objetos del
   grafo con dedup minga y referencias vivas cross-app. Cero telemetría;
   recibos/presencia opt-in y **simétricos** (ayni real).

## Arquitectura (crates planeados)

| crate          | rol                                                            | estado |
|----------------|----------------------------------------------------------------|--------|
| `ayni-core`    | DAG de mensajes firmados + membresía/confianza/recibos (no_std) | ✅ P0/P7 |
| `ayni-crypto`  | firma Ed25519 sobre agora ✅ + E2EE 1:1 (X25519/HKDF/ChaCha) ✅ | ✅ P2  |
| `ayni-sync`    | `Transporte` + `EnlaceTcp` + anti-entropía (diff Merkle)       | ✅ P3  |
| `ayni-minga`   | `EnlaceMinga`: transporte P2P sobre libp2p (mismo trait `Transporte`) | ✅ P3 |
| `ayni-store`   | persistencia del DAG + blobs de adjuntos (dedup) sobre sled    | ✅ P3/P5 |
| `ayni-app`     | núcleo de aplicación: transporte (TCP/minga) + store + cifrado + adjuntos + confianza | ✅ |
| `ayni-cli`     | chat de terminal (bin `ayni`), frontend delgado sobre `ayni-app` | ✅ P1+  |
| `ayni-llimphi` | UI Llimphi COMPLETA: charla + gente (membresía/confianza) + adjuntos + recibos | ✅ |
| `ayni-index`   | búsqueda semántica local (rimay embeddings + coseno)           | ✅ P4  |
| `ayni-ai`      | multilienzo: traducir/resumir/tono vía pluma-llm (máquina propone) | ✅ P4 |

La app de wawa (P6) vive aparte, en `03_ukupacha/wawa/apps/ayni` (módulo WASM
sobre akasha), porque cruza la frontera del workspace bare-metal; reusa
`ayni-core` por path, como `format`.

### Cierre de cabos (post-P7)

`ayni-app` recoge toda la lógica viva que no es modelo puro ni cara concreta, y
las dos UIs (`ayni-cli` y `ayni-llimphi`) son frontends delgados sobre él:

- **Transporte intercambiable EN LOS BINARIOS**: `--transporte tcp|minga`
  (env `AYNI_TRANSPORTE`). `Enlace` unifica `EnlaceTcp` y `EnlaceMinga` (libp2p)
  tras el trait `Transporte`; cambiar de cable no toca la lógica.
- **Persistencia local-first cableada**: cada binario abre un store sled
  (`--data`/`AYNI_DATA`, default `./ayni-<nombre>.db`) y CARGA la conversación
  al arrancar — el hilo sigue donde quedó entre sesiones.
- **Adjuntar (P5) con UX**: `/adjuntar <ruta>` lee el archivo, guarda el blob
  (dedup), difunde la referencia viva `Adjunto`, y el blob se pide/sirve por
  `Sobre::PedirBlob`/`Blob` automáticamente.
- **Recibos simétricos**: toggle `recibos` (opt-in en ambos lados); cuando está
  activo, el núcleo acusa los mensajes ajenos nuevos, y la UI muestra "✓N"
  (cuántos vieron cada nodo) y "✓rx" junto a quien reciproca.
- **GUI completa**: dos columnas (GENTE: miembros clicables, otros vistos,
  acciones admitir/atestar/expulsar sobre el seleccionado, grafo de confianza;
  CHARLA: hilo con scroll por rueda + recibos, compose con toggles de
  cifrado/recibos, adjuntar y enviar; barra `/` con los comandos).

**Genuinamente diferido** (deuda real, no fingida): **MLS de grupo** (forward/
post-compromise secrecy con OpenMLS — sincronizar estado de grupo es un
protocolo en sí; el canal de hoy es 1:1 sin PCS); **NAT traversal** (deuda de
`minga`, no de Ayni); y, dentro de la app de wawa, **anti-entropía completa
sobre L2** (hoy se difunde y absorbe en vivo, falta reconciliar historial) y
**cifrado de sesión en wawa**.

`ayni-core` es `#![no_std] + alloc` **desde el día cero** — no parcheado
después — para que el mismo núcleo viaje como app WASM dentro de wawa (P6) sin
reescribir el modelo. Por la misma razón es **cripto-agnóstico**: la firma entra
y se verifica por *closure*; las primitivas Ed25519/MLS viven en `ayni-crypto`.

## Roadmap por fases

- **P0 — `ayni-core`** ✅ *(hecho)*: DAG firmado local, sin red. Tipos
  (`Contenido`/`MensajeNodo`/`Conversacion`), id BLAKE3 = `hash(postcard(contenido))`,
  firma sobre el id, operaciones de DAG (cabezas, raíces, orden topológico
  determinista, verificación de firmas). 12 tests, incl. bifurcación/reconciliación
  y firma Ed25519 real.
- **P1 — primer lazo vivo** ✅ *(hecho)*: dos clientes chatean por LAN, mensajes
  firmados Ed25519, grafos que convergen sin servidor. `ayni-crypto` (identidad +
  firma), `ayni-sync` (trait `Transporte` + `EnlaceTcp` directo + `Fusionador`
  con búfer de fuera-de-orden), `ayni-cli` (chat de terminal, probado vivo) y
  `ayni-llimphi` (UI MVP). El transporte es TCP directo, no el daemon brahman de
  chasqui (matchmaking app↔app, desproporcionado para chat humano); minga/chasqui
  serán impls del mismo trait en P3.
- **P2 — E2EE 1:1** ✅ *(hecho)*: `ayni-crypto::canal` — `CanalSeguro` con
  X25519 (ECDH) + HKDF-SHA256 + ChaCha20-Poly1305, sólo primitivas auditadas
  (no cripto a mano). El par X25519 deriva de la **misma semilla agora** que la
  firma; la clave pública se intercambia al conectar (`Sobre::Hola`). El claro
  va en `Carga::Cifrado` *dentro* del contenido firmado → el **E2EE es ortogonal
  al transporte** (la red mueve ciphertext sin enterarse) y la autoría sigue
  siendo pública. Test prueba que el claro NO aparece en los bytes del cable;
  chat cifrado verificado vivo (CLI/UI con `--cifrar`).
  **Diferido a una fase posterior:** MLS (RFC 9420, OpenMLS) para chat de GRUPO
  y forward-secrecy/post-compromise — su valor exige sincronizar estado de grupo
  (Welcome/commits/epochs) sobre el transporte, que es un protocolo en sí mismo.
  `CanalSeguro` es el seam donde MLS entrará. El canal de hoy es static-static
  (estilo `crypto_box`): confidencialidad + integridad 1:1, sin PCS.
- **P3 — sin servidor** ✅ *(hecho)*: **anti-entropía** (diff de Merkle del DAG:
  sólo viaja lo que falta; la reconciliación camina el DAG hacia atrás),
  **persistencia** (`ayni-store` sobre sled; local-first + base del
  store-and-forward), y **`EnlaceMinga`** — transporte P2P real sobre el nodo
  libp2p de minga (`card_net::BrahmanNet`: TCP+Noise+yamux, Kademlia DHT,
  identify), tras el MISMO trait `Transporte` que `EnlaceTcp` (la app no cambia).
  Bridge async→sync: runtime tokio en hilo propio, protocolo `/ayni/transporte/1.0.0`.
  Test de 2 nodos libp2p convergiendo por anti-entropía, verde y estable. Lo
  único que falta es ajeno a Ayni: **NAT traversal**, que minga aún no implementa
  (hoy TCP directo + descubrimiento DHT en LAN).
- **P4 — inteligencia local** ✅ *(hecho)*: `ayni-index` (embeddings rimay +
  búsqueda coseno top-k sobre el historial, todo local; omite cifrados) y
  `ayni-ai` (multilienzo: traducir/resumir/tono vía la fachada `pluma-llm`,
  Mock determinista sin credenciales). "Máquina propone, humano firma": la IA
  redacta, no envía sola.
- **P5 — cross-app** ✅ *(modelo+protocolo+store; UX de adjuntar en CLI/UI pendiente)*:
  `Carga::Adjunto(Adjunto)` — una REFERENCIA VIVA por hash a un objeto del grafo
  (pluma/khipu/cosmos/archivo), no una copia muerta. La referencia viaja DENTRO
  del contenido firmado (intacta, infalsificable); los bytes viajan aparte como
  blob direccionado por hash, con **dedup** (mismo objeto = un solo blob) y
  **verificación** por contenido al recibir (`Adjunto::verifica`). `ayni-store`
  guarda blobs (árbol aparte); `ayni-sync` los reconcilia (`PedirBlob`/`Blob` +
  `blobs_faltantes`/`servir_blobs`/`blob_valido`). El mismo hash en el grafo de
  la app de origen y en el adjunto apuntan al mismo objeto — editar en origen da
  otro hash (otra versión): referencias vivas, no copias.
- **P6 — Ayni en wawa** ✅ *(hecho)*: el MISMO `ayni-core` (no_std + alloc) que
  corre el chat en Linux viaja, sin reescribir su modelo, a una app WASM dentro
  del SO bare-metal wawa — `03_ukupacha/wawa/apps/ayni`. Y ata dos grafos
  direccionados por contenido que comparten la misma `format::hash` (BLAKE3):
  cada nodo de la conversación —un mensaje firmado Ed25519 de verdad
  (`ed25519-compact`, el mismo del kernel)— se persiste como un OBJETO del grafo
  de akasha (`sys_object_put`), encadenado al anterior en una espina dorsal que
  el kernel custodia en disco. Al arrancar, la app recorre la espina, reconstruye
  la `Conversacion` con `desde_nodos`, añade el mensaje de este arranque, lo graba
  y corona la nueva cabeza como raíz: la conversación sobrevive a los reinicios
  porque vive en el disco de objetos, no en la RAM (local-first sobre el SO
  soberano, como la crónica de la `cronista`). Es la primera app de genesis que
  funda su propio heap (`linked_list_allocator`, el del kernel) — el grafo de la
  conversación necesita `alloc`. `ayni-core` ganó `MensajeNodo::serializar`/
  `deserializar` (el grano fino que el grafo de objetos y la anti-entropía piden).
  **P6+ — habla por akasha:** la app dejó de ser monólogo. Tecleás (teclado del
  kernel, `sys_get_scancode`), pulsás Enter, y el nodo firmado se persiste en el
  grafo Y se DIFUNDE por la red del SO en un frame Ethernet de EtherType propio
  (`0x88B7`), sin TCP/IP — akasha puro (`sys_net_*`, `PERMISO_RED`). Otra wawa en
  el segmento absorbe el frame, **verifica la firma** e integra el nodo: dos
  wawas convergen su conversación sin servidor. Pendiente: anti-entropía completa
  sobre L2 (hoy un peer recién arrancado ve lo NUEVO en vivo, no el historial
  hasta que alguien reemita) y cifrado de sesión.
- **P7 — confianza/UX** ✅ *(hecho)*: la dimensión SOCIAL del grafo, como cargas
  firmadas más (en `ayni-core`, módulo `confianza` — modelo puro `no_std`, viaja
  a wawa con el resto). Tres hechos que se DERIVAN plegando el DAG, sin autoridad
  central: **(1) membresía firmada** — `Carga::Membresia` (alta/baja); quién está
  en la sala se calcula con `Conversacion::membresia()`, regla de autoridad (sólo
  un miembro vigente admite/expulsa) + ancla (el autor del primer nodo funda y es
  inexpulsable); dos pares con el mismo grafo obtienen la misma membresía.
  **(2) grafo de confianza agora** — `Carga::Atestacion` es una arista firmada
  `autor → sujeto`; `confianza_desde(observador)` recorre en anchura y devuelve a
  quién alcanza y a cuántos saltos (fractal, transitiva, revocable con `nivel=0`).
  **(3) recibos simétricos** — `Carga::Recibo` (qué nodos vio el autor);
  `recibos()`/`acuses_de()` lo exponen verificable; la simetría (no acusar a quien
  no acusa) es política de la UX —ayni real, presencia recíproca, cero telemetría
  extractiva—. Constructores de conveniencia: `admitir`/`expulsar`/`atestar`/
  `acusar_recibo`. La UX llega a `ayni-cli` como comandos `/miembros` `/confianza`
  `/admitir` `/expulsar` `/atestar` `/recibo`, y `ayni-llimphi` pinta las cargas
  sociales como actos legibles. 17 tests verdes en `ayni-core`.

### Por qué `02_ruway` (HACER)

Ayni es una herramienta que el humano *usa para obrar* (comunicarse), no un
órgano de percepción ni de conocimiento. Vive junto a `chasqui` (transporte) y
`llimphi` (su UI). Defendible en `03_ukupacha` por su parentesco con
agora/minga, pero su naturaleza es de aplicación, no de raíz.

## El modelo de `ayni-core` en una imagen

```
        (raíz, sin padres)          cada nodo:
            ┌─────┐                   id   = BLAKE3(postcard(Contenido))
            │  R  │  "¿café?"         firma = Ed25519(autor, id)
            └──┬──┘                   padres = ids de nodos previos
          ┌────┴────┐                 → DAG acíclico POR CONSTRUCCIÓN
       ┌──▼──┐   ┌──▼──┐                (no podés referenciar un hash
       │  A  │   │  B  │                 antes de crear su contenido)
       │ "sí"│   │ "té"│  ← dos cabezas: la conversación bifurcó
       └──┬──┘   └──┬──┘
          └────┬────┘
            ┌──▼──┐
            │  U  │  "ok los dos"  ← un nodo con DOS padres: reconcilia
            └─────┘                   ← cabeza única otra vez
```

`cargo test -p ayni-core` ejercita exactamente este escenario.
