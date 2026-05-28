# llimphi-gpu-bench

Validación de Fase 0 del SDD `02_ruway/llimphi/SDD.md` §"GPU directo wgpu".
Criterio: factor ≥ 5× a 500K Y ≥ 60 fps @ 1M en GPU mid (Radeon 5500M, Iris Xe).

- crate version: 0.1.0
- host OS: linux
- host arch: x86_64

## Adapter wgpu

- backend: `Vulkan`
- device name: `Intel(R) Iris(R) Xe Graphics (TGL GT2)`
- vendor: `0x8086`
- device id: `0x9a49`
- device type: `IntegratedGpu`
- driver: `Intel open-source Mesa driver`
- driver info: `Mesa 26.1.1-arch1.2`

Limits relevantes:

- max texture 2D: 16384
- max buffer size: 2047 MB
- max storage buffer binding: 2047 MB

## Spike vello vs GPU directo

Target: 1024×1024 Rgba8Unorm, headless. Cada N corre 5 warmup + 15 medidos, reporta mediana.

| N | vello ms | directo ms | factor | nota |
|---:|---:|---:|---:|---|
| 25_000 | 7.33 | 1.21 | 6.05× | ≥5× |
| 50_000 | 12.88 | 1.44 | 8.94× | ≥5× |
| 100_000 | 21.67 | 3.25 | 6.67× | ≥5× |
| 200_000 | 26.07 | 6.06 | 4.30× | <5× |
| 500_000 | 94.36 | 17.96 | 5.25× | ≥5× |
| 1_000_000 | 202.38 | 49.02 | 4.13× | <5× |

**Veredicto Fase 0:** factor a 500K = 5.25× ≥ 5 → **PASA** (criterio SDD cumplido).

## Escalado GPU directo

API real (`GpuPipelines` + `GpuBatch::add_rect`). Sólo se mide el lado GPU directo — vello no llega acá.

| N | ms / frame | fps (1000/ms) | Mprim/s |
|---:|---:|---:|---:|
| 100_000 | 5.51 | 181.4 | 18.14 |
| 500_000 | 34.48 | 29.0 | 14.50 |
| 1_000_000 | 45.22 | 22.1 | 22.12 |
| 2_000_000 | 76.87 | 13.0 | 26.02 |
| 5_000_000 | 185.07 | 5.4 | 27.02 |
| 10_000_000 | 348.72 | 2.9 | 28.68 |

**Veredicto Fase 0 (objetivo 60 fps @ 1M):** 22.1 fps < 60 → marginal. ¿Es CPU-bound el bench (write_buffer de 12-20 MB por frame)? Probar también con `mapped_at_creation` para sacar el camino más rápido.

## Persistente — datos fijos, sólo redraw por frame

Setup (LCG + write_buffer / Scene fill) fuera de la medición; el bucle medido sólo emite render_pass + draw + submit + wait.

### vello (Scene reutilizada sin reset)

| N | ms / frame | fps (1000/ms) |
|---:|---:|---:|
| 100_000 | 18.64 | 53.7 |
| 500_000 | 34.07 | 29.3 |
| 1_000_000 | 83.15 | 12.0 |
| 2_000_000 | 101.73 | 9.8 |
| 5_000_000 | crash | — |
| 10_000_000 | crash | — |

### GPU directo (buffer + bind group persistentes)

| N | ms / frame | fps (1000/ms) | Mprim/s |
|---:|---:|---:|---:|
| 100_000 | 0.83 | 1210.0 | 121.00 |
| 500_000 | 3.42 | 292.6 | 146.31 |
| 1_000_000 | 7.07 | 141.4 | 141.38 |
| 2_000_000 | 15.96 | 62.6 | 125.29 |
| 5_000_000 | 41.82 | 23.9 | 119.57 |
| 10_000_000 | 79.69 | 12.5 | 125.48 |

**Veredicto persistente @ 1M:** directo 141.4 fps ≥ 60 → **PASA**.
**Factor persistente @ 1M:** vello 83.1 ms / directo 7.1 ms = 11.76× (≥5×)

## Validación visual

- vello 100K   → `bench_vello_100k.png` (1024×1024)
- directo 100K → `bench_directo_100k.png` (1024×1024)

Las dos imágenes deben mostrar la misma constelación de puntos (LCG determinista).
Mirar en visor: si vello tiene halo AA suave y directo tiene pixeles hard-edged, todo bien.

## Resumen

Copiar lo que sigue al chat:

```
rebuild por frame — vello vs directo:
      25_000  vello=     7.3ms  directo=    1.2ms  factor=6.05x
      50_000  vello=    12.9ms  directo=    1.4ms  factor=8.94x
     100_000  vello=    21.7ms  directo=    3.2ms  factor=6.67x
     200_000  vello=    26.1ms  directo=    6.1ms  factor=4.30x
     500_000  vello=    94.4ms  directo=   18.0ms  factor=5.25x
   1_000_000  vello=   202.4ms  directo=   49.0ms  factor=4.13x

rebuild por frame — escalado directo:
     100_000      5.5ms  181.4fps  18.14Mprim/s
     500_000     34.5ms   29.0fps  14.50Mprim/s
   1_000_000     45.2ms   22.1fps  22.12Mprim/s
   2_000_000     76.9ms   13.0fps  26.02Mprim/s
   5_000_000    185.1ms    5.4fps  27.02Mprim/s
  10_000_000    348.7ms    2.9fps  28.68Mprim/s

persistente (datos fijos, sólo redraw):
     100_000  vello=   18.6ms  directo=    0.8ms  factor=22.55x  1210.0fps  121.00Mprim/s
     500_000  vello=   34.1ms  directo=    3.4ms  factor=9.97x  292.6fps  146.31Mprim/s
   1_000_000  vello=   83.1ms  directo=    7.1ms  factor=11.76x  141.4fps  141.38Mprim/s
   2_000_000  vello=  101.7ms  directo=   16.0ms  factor=6.37x   62.6fps  125.29Mprim/s
   5_000_000  vello=       —  directo=   41.8ms  factor=  —     23.9fps  119.57Mprim/s
  10_000_000  vello=       —  directo=   79.7ms  factor=  —     12.5fps  125.48Mprim/s
```
