# supay-doom-llimphi

> Driver: wires engine + atlas + UI of [supay](../README.md).

Loads `doom1.wad`, builds the atlas, on every `Msg::Tick` walks the snapshot's sectors/mobjs and registers new `pic_idx`/`spritenum`. Cost: O(unique pics) accumulated per process.

## Deps

- [`supay-core`](../supay-core/README.md), [`supay-scene`](../supay-scene/README.md), [`supay-render-llimphi`](../supay-render-llimphi/README.md)
