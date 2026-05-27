# llimphi-text

> Shaping + fonts of [llimphi](../README.md).

Typography layer. Fontdue for minimal subset; HarfBuzz when complex shaping is required (Arabic, Devanagari, ligatures). Cache of rasterized glyphs; precise measurement for layout (`measure(text, font, size) → (w, h)`).

## Deps

- `fontdue`, `harfbuzz_rs` (feature)
