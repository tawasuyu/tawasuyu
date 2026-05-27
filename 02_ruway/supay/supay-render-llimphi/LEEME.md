# supay-render-llimphi

> [`scene_view`](../supay-scene/README.md) → polígonos vello + atlas de [supay](../README.md).

`WadAtlas` con interior mutability (Mutex) sobre cache lazy de flat colors + sprite patches. `scene_view(pair, last_tick_at, tick_period, config) -> View<Msg>` produce el nodo Llimphi cuyo `paint_with` proyecta el snapshot interpolado.

## Deps

- [`supay-scene`](../supay-scene/README.md), [`supay-wad`](../supay-wad/README.md), [`llimphi-ui`](../../llimphi/)
