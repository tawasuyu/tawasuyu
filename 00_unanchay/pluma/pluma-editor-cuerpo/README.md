# pluma-editor-cuerpo

> Text ↔ atoms editor with diff (greedy) for [pluma](../README.md).

`EditorCuerpo { texto, atom_ids }`. `from_cuerpo(c, atoms)` concatenates with `SEPARADOR = "\n\n"`. `parrafos()` retrieves the current split. `diff(atoms_originales) -> Vec<CambioAtom>` with greedy content-matching: matching paragraphs are skipped, differing ones emit `Mutar` reusing the `Uuid` (live threads), excess emits `Crear` or `Eliminar`. `aplicar_cambios(cambios, nuevos_ids)` extends/removes `atom_ids` after persisting.

## API

```rust
use pluma_editor_cuerpo::EditorCuerpo;

let mut e = EditorCuerpo::from_cuerpo(&c, &atoms);
e.set_texto(nuevo);
let cambios = e.diff(&atoms_originales);
```

## Deps

- [`pluma-core`](../pluma-core/README.md), [`pluma-cuerpo`](../pluma-cuerpo/README.md)
- `serde`, `uuid`
