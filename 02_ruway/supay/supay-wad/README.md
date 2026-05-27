# supay-wad

> WAD parser (lumps, patches, flats, sprites) of [supay](../README.md).

Reads the original WAD format (header + directory + lumps). Resolves PNAMES, TEXTURES, FLATS, SPRITES. Sprite lookup with fallbacks: `<NAME><F><angle>` → `<NAME><F>0` (omnidirectional) → scan between `S_START..S_END` with horizontal mirroring.

## Deps

- `byteorder`, `serde`
