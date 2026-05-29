# Fixtures PSD — procedencia

Los archivos `.psd` de esta carpeta vienen del corpus de tests del crate
[`psd`](https://github.com/chinedufn/psd) v0.3.5 (`tests/fixtures/`), publicado
bajo licencia dual **MIT / Apache-2.0**, ambas compatibles con MPL-2.0 (la
licencia de gioser). Se redistribuyen aquí sin modificación.

| archivo                               | tamaño  | descripción                                |
| ------------------------------------- | ------- | ------------------------------------------ |
| `green-1x1.psd`                       | 22 483  | 1×1 píxel, una capa verde opaca            |
| `two-layers-red-green-1x1.psd`        | 22 385  | 1×1 píxel, dos capas: roja y verde         |

Sirven para los unit tests de `foreign-psd::importar_psd`. No son producto del
proyecto: si necesitamos un PSD propio para algún caso, lo agregamos aparte y
notamos su origen aquí.
