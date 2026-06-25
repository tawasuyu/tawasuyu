# SDD — willay · centro de eventos

> **Estado:** v1 en construcción (2026-06-25). Nombre `willay` **provisional**
> (quechua: *avisar / dar noticia*) — renombrable mientras los crates sean
> nuevos. Esta es la fuente autoritativa cuando difiera con CLAUDE.md.

## 0. Qué es

Un **centro de eventos**: la generalización de las notificaciones de escritorio
a un *timeline histórico, faceteable y buscable* de varios tipos de cosas que
pasan en el escritorio — notificaciones, **capturas** (hapiy), **clipboard**, y
más adelante correo (paloma), unidades (sandokan), sistema. Hoy `pata-notify` ya
es esto **para un solo tipo**: persiste cada notificación a un sled append-only
ordenado por tiempo (`pata-notify-panel` lo lista, `pata-notify-triage` lo
agrupa por embeddings + LLM). willay sube ese patrón a *eventos heterogéneos*.

## 1. Decisión de arquitectura: **federado con índice compartido**

No un store único monolítico ni un merge puro en tiempo de consulta, sino el
punto medio:

- **El dato nativo/pesado se queda en su productor.** Las notificaciones siguen
  en el sled de `pata-notify`; los PNG de hapiy siguen como archivos; un clip de
  texto largo vive en su store. willay **no** centraliza payloads grandes.
- **Un índice liviano compartido** (`willay-store`) guarda una entrada [`Evento`]
  por cada cosa que pasó: identidad direccionada por contenido (BLAKE3),
  timestamp, clase, origen, un texto-para-buscar/embeber, y una **referencia** al
  payload (ruta de archivo, o texto corto inline). Es lo que el centro lista,
  facetea, busca y embebe.

Por qué: encaja con la filosofía del repo (`*-core` agnóstico, los frontends son
lectores desacoplados — el panel ya lee al daemon, paloma-rag lee la caché de
paloma) y evita el sistema paralelo. El índice es chico y uniforme → ordenar por
tiempo y buscar cross-tipo es trivial; los payloads no se duplican.

### 1.1 Escritor único (daemon), por sled

`sled` **no es multi-proceso** (lockea la DB a un proceso). Igual que
`pata-notify`, las escrituras se embudan por un **daemon willay** de escritor
único; los productores *emiten* un `Evento` al daemon y el daemon hace el
`append`. Los lectores (panel, rag) consultan al daemon o leen la DB en modo
read-only. **El daemon es un bloque posterior**; v1 arranca por el backbone
in-proceso (`willay-core` + `willay-store`), ejercitado por tests.

**Transporte: socket Unix propio** (decidido 2026-06-25, por desacople). El
daemon escucha en `$XDG_RUNTIME_DIR/willay.sock`; `willay-emit` es un cliente
fino (postcard sobre el socket). Independiente de D-Bus — mismo patrón que
`rimay-verbo-daemon`. `pata-notify` emite un espejo al socket *además* de su
propio store (no se absorbe ni se reescribe pata-notify).

### 1.2 Compatibilidad con Wawa (no_std)

El `Evento` es direccionado por contenido, así que por la ley de Wawa (todo lo
que vive en disco por hash o cruza al kernel compila sin std) **`willay-core` es
`#![no_std]` sobre `alloc`** y está en el guardián `scripts/check-shared-cores.sh`
(compila a `wasm32-unknown-unknown`, BLAKE3 escalar puro como `format`). El
índice `willay-store` (sled) sí es std, pero no cruza la frontera. Esto deja la
puerta abierta a eventos del lado wawa sin reescribir el esquema.

## 2. Crates (federado = pocas piezas compartidas + emisión por productor)

```
shared/willay/
  willay-core    — esquema agnóstico: Evento, Clase, Payload, id BLAKE3. (este SDD)
  willay-store   — índice sled append-only: append/listar/recientes/por_clase/buscar/rango.
  willay-daemon  — (futuro) escritor único; recibe emisiones, dueño del sled.
  willay-emit    — (futuro) cliente fino que usa cada productor para emitir.
  willay-panel-llimphi — (futuro) el feed heterogéneo (generaliza pata-notify-panel).
  willay-triage  — (futuro) clustering + resumen (generaliza pata-notify-triage).
```

La búsqueda semántica **no es un crate nuevo**: willay se vuelve otro `source`
del widget `rag` existente (junto a `paloma`), embebiendo `Evento::cuerpo` por
`rimay-verbo`. "Convivir en un solo rag/app" = misma UI, distinto corpus.

## 3. Esquema del evento (`willay-core`)

```rust
EventoId = [u8; 32]           // BLAKE3 del contenido canónico → id estable y dedup
enum Clase { Notificacion, Captura, Clip }   // v2: Correo, Unidad, Sistema, Nota…
enum Payload {
    Nada,
    Texto(String),                    // clip/cuerpo corto, inline
    Archivo { ruta: String, mime: String },   // captura PNG por ruta (federación)
}
struct Evento { id, clase, ts_usec, origen, titulo, cuerpo, payload }
```

- `origen`: quién lo emitió — `app_name` de la notif, `"hapiy"`, el conector del
  monitor capturado, la app que copió al clipboard.
- `titulo`: la línea principal (summary / "Captura DP-1" / primeras palabras del clip).
- `cuerpo`: el texto que se busca/embebe (body de la notif, texto del clip, OCR futuro).
- `id`: BLAKE3 sobre `(clase, ts_usec, origen, titulo, cuerpo, payload)` — mismo
  contenido ⇒ mismo id (dedup natural de re-emisiones).

## 4. Tipos de evento del v1

Los tres que el usuario nombró, por leverage:

1. **Notificacion** — ya hay productor (`pata-notify`). Emite un `Evento` espejo
   compacto al índice; la `Notificacion` completa se queda en su sled.
2. **Captura** — `hapiy` gana un punto de emisión: al guardar el PNG, emite
   `Evento{clase: Captura, payload: Archivo{ruta, "image/png"}}`.
3. **Clip** — el clipboard hoy es stub (no persiste). Gana una historia mínima y
   emite `Evento{clase: Clip, payload: Texto|Archivo}` por cada copia.

v2 (no en este SDD): Correo (paloma), Unidad (sandokan start/fail), Sistema
(batería/red/dispositivos), Nota (khipu), efeméride (cosmos).

## 5. Cómo se ordena/presenta (resumen; detalle en willay-panel)

Columna vertebral cronológica descendente con separadores de fecha (Hoy/Ayer/…),
**facetas** por clase/origen/importancia, **agrupación semántica** (triage) para
ráfagas, **búsqueda** en dos registros (literal/filtro instantáneo + RAG
semántico), y **acciones por tipo** (recopiar clip, abrir captura en tullpu,
accionar notif, fijar, borrar).

## 6. Plan de bloques

1. **(este) backbone** — `willay-core` (esquema + id) + `willay-store` (índice
   sled + consultas) + tests. Sin daemon, sin UI.
2. emisión — `willay-daemon` (escritor único) + `willay-emit`, y enganchar los
   tres productores (notify espejo, hapiy, clipboard con historia).
3. lectura — `willay-panel-llimphi` (feed heterogéneo, generaliza notify-panel).
4. semántica — willay como `source` del widget `rag`; generalizar el triage.
