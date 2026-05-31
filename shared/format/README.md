# format — el formato nativo de gioser

Tipos canónicos del **DAG direccionado por contenido** (BLAKE3 + postcard),
compartidos entre host y kernel `wawa`. `#![no_std]` — cruza la frontera al
kernel bare-metal por `path`. Es el formato en el que TODO el suite trabaja en
nativo (los formatos ajenos entran por `shared/foreign-*` y se convierten a
esto).

## Módulos

- `tipos` — objetos, hashes, identidades de contenido.
- `cable` — referencias entre objetos (aristas del DAG).
- `firma` — firmas Ed25519 y verificación.
- `pruebas` — pruebas de revocación de capacidades (WAWA.md §14.1.3).
- `grafo` — construcción/recorrido del DAG.
- `constantes` — parámetros del formato (tamaños, versiones).

## Estado (2026-05-31)

### Hecho
- Tipos canónicos del DAG (objetos, cables, hashes) en `no_std`, validados en
  `wasm32-unknown-unknown` por `scripts/check-shared-cores.sh`.
- Firma/verificación Ed25519 (`firma`) y pruebas de revocación (`pruebas`),
  canónicos compartidos kernel↔host para el enforcement §14.1.3.
- `lib.rs` (2327 LOC) **dividido en módulos temáticos** (cable/firma/grafo/…).
- Suite amplia (~52 tests).

### Pendiente
- Versionado/migración del formato en disco (campo de versión existe; políticas
  de upgrade aún por definir).
- Más cobertura de los caminos de revocación end-to-end.

## Lugar en el repo

`shared/format` — núcleo `no_std` compartido. Lo consumen apps, `agora` y el
kernel `wawa`.
