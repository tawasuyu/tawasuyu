# rimay-localize — declarative localization (i18n)

Per-language string catalogs + a minimalist lookup API for tawasuyu's
apps. The catalogs are **data** (JSON) loadable at runtime; the
code asks for a key and a language and gets the string, with interpolation of
`{nombre}` placeholders.

## What it exposes

- `Lang` — language tag (simplified BCP-47).
- `Catalog` — key → string map for one language.
- `Localizer` — collection of catalogs + active language + fallback.
- Lookup with placeholder substitution by named arguments.

## Naming note

`rimay` is the language/voice domain; this crate is its cross-cutting
localization utility (it is not part of `rimay`'s core).

## Status (2026-05-31)

### Done
- Loading of JSON catalogs per language.
- Lookup with active language + fallback and tests.
- Interpolation of `{nombre}` placeholders.

### Pending
- Complex pluralization (ICU) and per-language rules.
- Detection of the system language (today the app decides it).
- Tool for extracting/validating missing keys.

## Place in the repo

`shared/rimay-localize` — cross-cutting i18n utility for tawasuyu apps.
