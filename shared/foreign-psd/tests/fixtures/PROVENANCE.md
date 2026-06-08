# Fixtures PSD — procedencia

Los archivos `.psd` de esta carpeta vienen del corpus de tests del crate
[`psd`](https://github.com/chinedufn/psd) v0.3.5 (`tests/fixtures/`), publicado
bajo licencia dual **MIT / Apache-2.0**, ambas compatibles con MPL-2.0 (la
licencia de tawasuyu). Se redistribuyen aquí sin modificación.

| archivo                               | tamaño  | descripción                                              |
| ------------------------------------- | ------- | -------------------------------------------------------- |
| `green-1x1.psd`                       | 22 483  | 1×1 píxel, una capa verde opaca                          |
| `two-layers-red-green-1x1.psd`        | 22 385  | 1×1 píxel, dos capas: roja y verde                       |
| `group-one-layer.psd`                 | 23 269  | un grupo con una capa adentro (`green-1x1-one-group-one-layer-inside`) |
| `groups-nested.psd`                   | 24 045  | grupo anidado en otro grupo, una capa al fondo (`green-1x1-one-group-inside-another`) |
| `groups-siblings.psd`                 | 24 377  | dos grupos hermanos, una capa en cada uno (`green-1x1-two-groups-two-layers-inside`) |

Sirven para los unit tests de `foreign-psd::importar_psd`. No son producto del
proyecto: si necesitamos un PSD propio para algún caso, lo agregamos aparte y
notamos su origen aquí.
