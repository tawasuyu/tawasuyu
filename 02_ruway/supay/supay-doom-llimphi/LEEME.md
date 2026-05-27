# supay-doom-llimphi

> Driver: enlaza motor + atlas + UI de [supay](../README.md).

Carga `doom1.wad`, construye el atlas, en cada `Msg::Tick` recorre sectores/mobjs del snapshot y registra los `pic_idx`/`spritenum` nuevos. Costo: O(unique pics) acumulado por proceso.

## Deps

- [`supay-core`](../supay-core/README.md), [`supay-scene`](../supay-scene/README.md), [`supay-render-llimphi`](../supay-render-llimphi/README.md)
