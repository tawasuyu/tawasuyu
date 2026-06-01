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
raymi-llimphi      — frontend: vista mes + agenda del día + contactos.   [HECHO]
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

- **Fase 2 (2026-06-01):** `raymi-llimphi` — frontend sobre Llimphi.
  - Dos modos en la barra superior: **Calendario** y **Contactos**.
  - Calendario: grilla del mes (6×7) con chips de eventos coloreados por
    calendario, día de hoy con disco de acento, navegación ‹ Mes Año › + "Hoy"
    (←/→ y rueda cambian de mes); a la derecha, **agenda del día** seleccionado
    (instancias con hora/“todo el día”, color y lugar). Recurrencias expandidas
    por `CalStore::occurrences_in`.
  - Contactos: lista buscable (avatar con iniciales + correo) + ficha (avatar
    grande, organización, correos/teléfonos/nota).
  - `DavBackend` (supertrait Calendar+Contacts, blanket impl) para llevar un solo
    backend. `resync` (F5) re-sincroniza. Crate **agnóstico**: `Model`/`Msg` +
    funciones libres; el anfitrión arma su `impl App`. El frontend sí lee el reloj
    (`SystemTime`) para “hoy”, a diferencia del núcleo.
  - Demo: `cargo run -p raymi-llimphi --example agenda_demo --release`
    (2 calendarios, eventos recurrentes anclados a hoy, 3 contactos).

## Pendiente (orden sugerido)

1. **`raymi-net`** — puente CalDAV/CardDAV (REPORT time-range, PUT, ETag) +
   parser iCalendar/vCard. Reusa `ServerConfig`/`Security`.
2. **`raymi-store`** — persistencia nativa (postcard) + sync incremental.
3. **`raymi-app`** — binario lanzable, comparte `cuenta.json` con paloma.
4. **Crear/editar eventos y contactos** desde la UI (put/delete ya en el trait).
5. **Cruce con paloma**: invitar contactos a eventos; “crear evento desde correo”.
