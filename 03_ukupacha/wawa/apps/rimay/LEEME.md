# rimay (app wawa)

> Reflejo bare-metal del verbo de [rimay](../../../../00_unanchay/rimay/). Embedding determinista + coseno, pintado sobre un framebuffer de 480×560 dentro de wawa OS.

El subdominio host `00_unanchay/rimay/` corre un `verbo-daemon` por socket Unix y sirve embeddings ONNX reales vía fastembed. Nada de eso entra en wawa: no hay socket, no hay `~/.cache/fastembed`, no hay ONNX. Esta app conserva el *contrato* (Provider → vector → coseno) y descarta el modelo: corre el mismo FNV-1a + LCG que el `rimay-verbo-mock` del host, de modo que el mismo texto produce el mismo vector y dos textos distintos producen ruido ~ortogonal.

Demo honesto, no similitud semántica: cosine(A, A) = 1.000, cosine(A, B) ≈ 0 para cadenas distintas.

## Build

```sh
./scripts/build-rimay.sh        # cargo build → wasm-opt → wawa-kernel/assets/rimay.wasm
./scripts/build-rimay.sh --debug  # build crudo, sin wasm-opt, sin consolidación
```

Salida: `~3 KiB` sellado (sobrado bajo el techo nominal de 10 KiB del manifiesto).

## Interacción

| Tecla | Acción |
|---|---|
| `SPACE` (0x39) | Avanza al siguiente par de textos |
| `ENTER` (0x1C) | Vuelve al primer par |

Cinco pares pre-baked rotan, incluyendo un par idéntico (`RIMAY / RIMAY`) para hacer visible el contrato: coseno = 1.000.

## Por qué no comparte crate

`rimay-verbo-core` arrastra `async_trait`, `tokio` y `serde` con feature `std` — ninguno cruza la jaula de wasmi. El cuerpo del FNV+LCG son ~30 líneas y se reimplementó inline en `#![no_std]`. Si `rimay-verbo-core` algún día se vuelve `no_std`-compatible tras una feature flag, esta app debería depender de él directamente.

## Permisos

`permisos: 0` — ninguna capacidad más allá del par universal `sys_render_frame` + `sys_get_scancode`. No toca el grafo de objetos, no habla por red, no necesita raíz.
