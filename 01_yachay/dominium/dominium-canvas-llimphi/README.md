# dominium-canvas-llimphi

> Llimphi (vello) backend for [dominium](../README.md).

Converts the `Vec<Quad>` from [`dominium-render-plan`](../dominium-render-plan/README.md) into `vello::Scene` operations inside a Llimphi `View::paint_with(...)`. Single-pass; zero allocations per frame (reuses the quad buffer).

## Deps

- [`dominium-render-plan`](../dominium-render-plan/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/) (vello)
