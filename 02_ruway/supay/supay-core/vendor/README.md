# vendor/ — código C externo

Este directorio aloja **doomgeneric** vendoreado. No está versionado
porque no queremos arrastrar ~10k LOC C dentro de gioser; el `build.rs`
del crate lo busca acá y, si lo encuentra, lo compila.

## Cómo proveerlo

```sh
cd 02_ruway/supay/supay-core/vendor
git clone https://github.com/ozkl/doomgeneric.git
```

Esto crea `vendor/doomgeneric/doomgeneric/*.c` que `build.rs`
recoge automáticamente. La próxima `cargo build -p supay-core`
linkea el motor real.

## Si no está

El `build.rs` emite `cfg(doomgeneric_stub)` y `lib.rs` cae a un modo
sin Doom — la API expone las mismas funciones pero `DoomEngine::tick`
es un no-op y el framebuffer queda en negro. El workspace queda
verde para permitir desarrollo del resto sin doomgeneric.

## WAD

doomgeneric espera un WAD (datos del juego) en el cwd o por
`-iwad <path>`. `DOOM1.WAD` (shareware) se distribuye legalmente:

```sh
curl -O https://distro.ibiblio.org/slitaz/sources/packages/d/doom1.wad
```

Pongalo en el cwd desde donde corrás `supay-doom-llimphi`.
