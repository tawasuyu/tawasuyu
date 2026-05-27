# pluma-editor-cuerpo

> Editor texto ↔ átomos con diff (greedy) para [pluma](../README.md).

`EditorCuerpo { texto, atom_ids }`. `from_cuerpo(c, atoms)` concatena con `SEPARADOR = "\n\n"`. `parrafos()` recupera el split actual. `diff(atoms_originales) -> Vec<CambioAtom>` con greedy por contenido: párrafos que coinciden se saltan, los que difieren emiten `Mutar` reusando el `Uuid` (hebras vivas), sobrantes emiten `Crear` o `Eliminar`. `aplicar_cambios(cambios, nuevos_ids)` extiende/remueve el `atom_ids` tras persistir.

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
