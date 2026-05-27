# supay-render-llimphi

> [`scene_view`](../supay-scene/README.md) → vello polygons + atlas of [supay](../README.md).

`WadAtlas` with interior mutability (Mutex) over lazy cache of flat colors + sprite patches. `scene_view(pair, last_tick_at, tick_period, config) -> View<Msg>` produces the Llimphi node whose `paint_with` projects the interpolated snapshot.

## Deps

- [`supay-scene`](../supay-scene/README.md), [`supay-wad`](../supay-wad/README.md), [`llimphi-ui`](../../llimphi/)
