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
| `ayni-crypto`  | MLS/OpenMLS + Ed25519/X25519 sobre identidad agora             | P2     |
| `ayni-sync`    | transporte: chasqui (LAN) + minga (P2P) + akasha (wawa)        | P1/P3  |
| `ayni-index`   | búsqueda semántica local (rimay embeddings)                    | P4     |
| `ayni-ai`      | multilienzo (pluma-transform + rimay-localize + pluma-llm)     | P4     |
| `ayni-llimphi` | UI (frontend intercambiable sobre `ayni-core`)                 | P1     |
| `ayni-app` / `ayni-cli` | binarios                                              | P1     |

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
- **P1 — primer lazo vivo**: chasqui LAN, 2 clientes, UI Llimphi MVP (fea).
- **P2 — E2EE**: MLS 1:1 (`ayni-crypto`).
- **P3 — sin servidor** *(HITO)*: sync P2P minga, DHT, store-and-forward.
- **P4 — inteligencia local**: búsqueda rimay + traducir-al-llegar / resumen.
- **P5 — cross-app**: adjuntar objetos del grafo (pluma/khipu/cosmos vivos).
- **P6 — Ayni en wawa**: app WASM/akasha reusando `ayni-core` no_std.
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
