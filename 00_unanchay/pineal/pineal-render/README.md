# pineal-render

> Llimphi render of [pineal](../README.md)'s model.

Converts a `Scene` from [`pineal-core`](../pineal-core/README.md) into `vello` operations inside a Llimphi `SceneCanvas`. Handles antialiasing, composited layers (alpha blending), exports to PNG via `pineal-export` when snapshot is requested.

## Deps

- [`pineal-core`](../pineal-core/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) (vello + scene)
