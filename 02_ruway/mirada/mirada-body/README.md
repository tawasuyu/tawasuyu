# mirada-body

> Estado físico del display (monitors, modes) de [mirada](../README.md).

Inventario de outputs (HDMI/DP/eDP/...) y sus modos (resolution + refresh + scale). El operador puede cambiar layout/scale en runtime sin reiniciar el compositor.

## Deps

- [`mirada-protocol`](../mirada-protocol/README.md)
- `drm`, `gbm` (Linux)
