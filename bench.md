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
| 25_000 | 10.18 | 3.19 | 3.19× | <5× |
| 50_000 | 15.10 | 6.18 | 2.44× | <5× |
| 100_000 | 29.10 | 5.25 | 5.54× | ≥5× |
| 200_000 | 30.97 | 9.36 | 3.31× | <5× |
| 500_000 | 67.95 | 32.11 | 2.12× | <5× |
| 1_000_000 | 126.17 | 43.64 | 2.89× | <5× |

**Veredicto Fase 0:** factor a 500K = 2.12× < 5 → **ABORTAR** según criterio literal del SDD.
Pero ver si vello revienta a tamaños mayores — eso cambia el veredicto cualitativamente.

## Escalado GPU directo

API real (`GpuPipelines` + `GpuBatch::add_rect`). Sólo se mide el lado GPU directo — vello no llega acá.

| N | ms / frame | fps (1000/ms) | Mprim/s |
|---:|---:|---:|---:|
| 100_000 | 5.80 | 172.5 | 17.25 |
| 500_000 | 29.52 | 33.9 | 16.94 |
| 1_000_000 | 35.60 | 28.1 | 28.09 |
| 2_000_000 | 51.67 | 19.4 | 38.71 |
| 5_000_000 | 139.65 | 7.2 | 35.80 |
| 10_000_000 | 301.87 | 3.3 | 33.13 |

**Veredicto Fase 0 (objetivo 60 fps @ 1M):** 28.1 fps < 60 → marginal. ¿Es CPU-bound el bench (write_buffer de 12-20 MB por frame)? Probar también con `mapped_at_creation` para sacar el camino más rápido.

## Validación visual

- vello 100K   → `bench_vello_100k.png` (1024×1024)
- directo 100K → `bench_directo_100k.png` (1024×1024)

Las dos imágenes deben mostrar la misma constelación de puntos (LCG determinista).
Mirar en visor: si vello tiene halo AA suave y directo tiene pixeles hard-edged, todo bien.

## Resumen

Copiar lo que sigue al chat:

```
vello vs directo:
    25_000  vello=    10.2ms  directo=    3.2ms  factor=3.19x
    50_000  vello=    15.1ms  directo=    6.2ms  factor=2.44x
   100_000  vello=    29.1ms  directo=    5.3ms  factor=5.54x
   200_000  vello=    31.0ms  directo=    9.4ms  factor=3.31x
   500_000  vello=    68.0ms  directo=   32.1ms  factor=2.12x
  1_000_000  vello=   126.2ms  directo=   43.6ms  factor=2.89x

escalado directo:
    100_000      5.8ms  172.5fps  17.25Mprim/s
    500_000     29.5ms   33.9fps  16.94Mprim/s
  1_000_000     35.6ms   28.1fps  28.09Mprim/s
  2_000_000     51.7ms   19.4fps  38.71Mprim/s
  5_000_000    139.7ms    7.2fps  35.80Mprim/s
  10_000_000    301.9ms    3.3fps  33.13Mprim/s
```
