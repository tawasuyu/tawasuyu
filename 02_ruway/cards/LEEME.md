# cards

**Una sola forma de leer cualquier tipo de Card.**

*Read this in English: [README.md](README.md).*

En tawasuyu varios dominios describen "cosas que corren o que agrupan
cosas" como documentos JSON llamados **Cards**: entidades de runtime,
agrupaciones semánticas, módulos de UI. Cada formato nació en su dominio
con su propio schema. Este crate es el brazo unificador: lee cualquiera de
ellos y lo proyecta a **una estructura `Card` canónica** que la UI, el
storage, el DHT y el wire pueden consumir sin importar de dónde vino el
documento.

## Cómo funciona

`load_card(path)` inspecciona la *forma* del JSON — sin flags, sin magia de
extensiones — y despacha al reader correcto:

- un **Ente** tiene `payload` + `supervision` → se lee vía `card-core`
  (`shared/card`), el schema de entidades de runtime;
- un **Monad** tiene `members` + `cardinality` → se lee vía `chasqui-card`,
  el schema de agrupaciones semánticas;
- un **UiModule** tiene `entities` + `views` + `menu` → se lee vía
  `nahual-meta-schema`, el schema de módulos de UI.

Cada formato sigue viviendo en su crate de origen con su propio schema; los
readers sólo deserializan y envuelven. La `Card` canónica lleva la
proyección compartida: un `id` string opaco, `schema_version`, un `label`
derivado, `lineage` opcional y un mapa `extensions` para compatibilidad
hacia adelante.

`load_cards_from_dir(dir)` recorre los subdirectorios inmediatos de una
raíz, cargando el card file convencional de cada uno (`card.ncl` con
preferencia sobre `card.json`; los subdirs sin card file se saltean en
silencio, los errores reales son ruidosos y cortan el recorrido).

## Templates Nickel (V2)

Además de JSON plano, las Cards pueden escribirse en
[Nickel](https://nickel-lang.org/) (`card.ncl`): templates con defaults
mergeados vía el `import` nativo de Nickel, evaluados al cargar. V1 (JSON)
y V2 (Nickel) conviven.

## Probalo

```bash
cargo test -p cards                      # suite completa
cargo test -p cards -- --test-threads=1  # los tests de templates mutan env → serial
```

Los tests de integración (`tests/integration.rs`) sirven también como
ejemplos de uso: detección de forma, dispatch, round-trips JSON→Card,
recorrido de directorios, restricción de readers.

## Estado

Funcionando: readers JSON para los tres formatos, dispatch por forma,
templates Nickel, carga por directorio. El conjunto de readers crece a
medida que aparecen nuevos formatos de Card en la suite.
