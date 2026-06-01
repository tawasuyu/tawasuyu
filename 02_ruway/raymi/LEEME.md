# raymi

> `raymi` (los festivales del calendario andino: Inti Raymi, Qhapaq Raymi…).
> Tipo: **calendario + contactos nativos sobre Llimphi** (CalDAV/CardDAV).

El compañero de `paloma`: cierra el reemplazo de Google Workspace con agenda y
libreta de direcciones nativas, sin navegador con JIT (Tanda 1 #2 de
`/APPS-NATIVAS.md`). **Reusa la capa de cuentas de paloma** (`Account` /
`ServerConfig` / `Address`): una sola identidad de cuenta para correo, calendario
y contactos. Habla CalDAV (eventos) y CardDAV (contactos) y renderiza nativo.

## Anatomía (objetivo)

```
raymi-core         — modelo agnóstico: eventos, recurrencia (RRULE),
                     calendarios, contactos, el trait de transporte.    [HECHO]
raymi-net          — puente CalDAV/CardDAV: iCalendar (VEVENT) + vCard
                     (VCARD) + REPORT/PUT; implementa los traits.        [pendiente]
raymi-store        — persistencia nativa (postcard) + sync incremental.  [pendiente]
raymi-llimphi      — frontend: vista mes/semana/día + lista de contactos. [pendiente]
raymi-app          — binario lanzable.                                    [pendiente]
```

Un dominio = un crate raíz `*-core` agnóstico + frontends Llimphi
intercambiables, como el resto de la suite.

## Estado

- **Fase 1 (2026-06-01):** `raymi-core` — núcleo agnóstico y `cargo test`-eable.
  - `time` — aritmética de fecha civil sobre timestamps Unix UTC sin crate de
    tiempo (Hinnant): `days_from_civil`/`civil_from_days`, `weekday`, `to_civil`/
    `to_unix`, `add_months`/`add_years` (recorte de fin de mes), `is_leap`.
  - `Event` — `VEVENT` nativo: instantes en s Unix UTC, `all_day`, `rrule` cruda,
    organizador/invitados (reusa `paloma_core::Address`), `overlaps`/`duration`.
  - `recur` — parseo y **expansión de RRULE** (subconjunto práctico de RFC 5545):
    `FREQ=DAILY|WEEKLY|MONTHLY|YEARLY` + `INTERVAL` + `COUNT` + `UNTIL` + `BYDAY`
    (semanal). `occurrences(start, rrule, from, to)` con tope anti-cuelgue.
  - `Calendar` + `CalendarRole` (infiere rol del nombre) + color hex opcional.
  - `Contact` + `AddressBook` — `VCARD` plano: nombre, correos, teléfonos, org,
    nota; `initials`, `matches`, `primary_email`.
  - `CalendarBackend` + `ContactsBackend` (traits) + `MockBackend` que implementa
    ambos (put/delete/fetch in-memory).
  - `CalStore` — caché local: calendarios → eventos, libretas → contactos; expande
    recurrencias a `Occurrence`s (`occurrences_in(from, to)`, capta eventos en
    curso), busca contactos y cruza por correo con paloma.
  - 25 tests verde.

## Pendiente (orden sugerido)

1. **`raymi-llimphi`** — frontend sobre `MockBackend`: vista mes (grilla) +
   agenda del día + lista/detalle de contactos, reusando avatares y estética de
   paloma.
2. **`raymi-net`** — puente CalDAV/CardDAV (REPORT time-range, PUT, ETag) +
   parser iCalendar/vCard. Reusa `ServerConfig`/`Security`.
3. **`raymi-store`** — persistencia nativa (postcard) + sync incremental.
4. **`raymi-app`** — binario lanzable, comparte `cuenta.json` con paloma.
5. **Cruce con paloma**: invitar contactos a eventos; “crear evento desde correo”.
