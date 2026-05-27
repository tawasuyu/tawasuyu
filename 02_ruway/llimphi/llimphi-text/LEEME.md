# llimphi-text

> Shaping + fonts de [llimphi](../README.md).

Capa de tipografía. Fontdue para subset minimal; HarfBuzz cuando se requiere shaping complejo (árabe, devanagari, ligaduras). Cache de glyphs rasterizados; medición precisa para layout (`measure(text, font, size) → (w, h)`).

## Deps

- `fontdue`, `harfbuzz_rs` (feature)
