# supay-wad

> Parser WAD (lumps, patches, flats, sprites) de [supay](../README.md).

Lee el formato WAD original (header + directory + lumps). Resuelve PNAMES, TEXTURES, FLATS, SPRITES. Sprite lookup con fallbacks: `<NAME><F><angle>` → `<NAME><F>0` (omnidireccional) → escaneo entre `S_START..S_END` con espejado horizontal.

## Deps

- `byteorder`, `serde`
