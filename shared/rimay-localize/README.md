# rimay-localize — localización (i18n) declarativa

Catálogos de cadenas por idioma + una API de lookup minimalista para las apps de
gioser. Los catálogos son **datos** (JSON) cargables en tiempo de ejecución; el
código pide una clave y un idioma y obtiene la cadena, con interpolación de
placeholders `{nombre}`.

## Qué expone

- `Lang` — etiqueta de idioma (BCP-47 simplificado).
- `Catalog` — mapa clave → cadena para un idioma.
- `Localizer` — colección de catálogos + idioma activo + fallback.
- Lookup con sustitución de placeholders por argumentos con nombre.

## Nota de naming

`rimay` es el dominio de lenguaje/voz; este crate es su utilidad de localización
transversal (no es parte del núcleo de `rimay`).

## Estado (2026-05-31)

### Hecho
- Carga de catálogos JSON por idioma.
- Lookup con idioma activo + fallback y tests.
- Interpolación de placeholders `{nombre}`.

### Pendiente
- Pluralización compleja (ICU) y reglas por idioma.
- Detección del idioma del sistema (hoy lo decide la app).
- Herramienta de extracción/validación de claves faltantes.

## Lugar en el repo

`shared/rimay-localize` — utilidad i18n transversal para apps gioser.
