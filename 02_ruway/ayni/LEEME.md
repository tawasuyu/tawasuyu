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
| `ayni-core`    | DAG de mensajes firmados, direccionado por contenido (no_std)  | ✅ P0  |
| `ayni-crypto`  | firma Ed25519 sobre agora ✅ + E2EE 1:1 (X25519/HKDF/ChaCha) ✅ | ✅ P2  |
| `ayni-sync`    | `Transporte` + `EnlaceTcp` + anti-entropía (diff Merkle)       | ✅ P3  |
| `ayni-minga`   | `EnlaceMinga`: transporte P2P sobre libp2p (mismo trait `Transporte`) | ✅ P3 |
| `ayni-store`   | persistencia del DAG + blobs de adjuntos (dedup) sobre sled    | ✅ P3/P5 |
| `ayni-cli`     | chat headless de terminal (bin `ayni`)                         | ✅ P1  |
| `ayni-llimphi` | UI Llimphi (frontend intercambiable sobre `ayni-core`)         | ✅ P1  |
| `ayni-index`   | búsqueda semántica local (rimay embeddings + coseno)           | ✅ P4  |
| `ayni-ai`      | multilienzo: traducir/resumir/tono vía pluma-llm (máquina propone) | ✅ P4 |

La app de wawa (P6) vive aparte, en `03_ukupacha/wawa/apps/ayni` (módulo WASM
sobre akasha), porque cruza la frontera del workspace bare-metal; reusa
`ayni-core` por path, como `format`.

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
- **P7 — confianza/UX**: grafo agora, membresía firmada, recibos simétricos.

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
