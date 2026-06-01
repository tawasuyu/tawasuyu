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
                     (VCARD) + REPORT/PUT; implementa los traits.        [HECHO]
raymi-store        — persistencia nativa (postcard) + sync incremental.  [HECHO]
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

- **Fase 3 (2026-06-01):** `raymi-net` — puente CalDAV/CardDAV.
  - `ical` — iCalendar (RFC 5545) ↔ `Event`: parsea `VEVENT` (UID/SUMMARY/
    DTSTART/DTEND con `VALUE=DATE` para día completo/DATE-TIME UTC, DESCRIPTION/
    LOCATION/RRULE/ORGANIZER/ATTENDEE), desdobla líneas plegadas, escapa/desescapa;
    `write_event` para `PUT`. `vcard` — vCard ↔ `Contact` (FN/N/EMAIL/TEL/ORG/
    NOTE/UID). `text` — helpers compartidos (unfold/split/escape).
  - `dav` — cliente HTTP sobre `ureq`: `REPORT` (calendar-query/addressbook-query),
    `PUT`, `DELETE`, Basic auth; parseo de `multistatus` con `roxmltree` por nombre
    local de etiqueta. `NetBackend` implementa ambos traits (colecciones por URL;
    autodescubrimiento pendiente). **17 tests offline**; los caminos HTTP se
    verifican contra un servidor real (Nextcloud/Radicale) en la laptop.

- **Fase 4 (2026-06-01):** `raymi-store` — caché en disco (postcard + BLAKE3).
  - `CalDb` — raíz de disco con un directorio por cuenta (id saneado). Persiste
    calendarios (`calendarios.pc`) y sus eventos (`eventos-<blake3>.pc`), libretas
    (`libretas.pc`) y sus contactos (`contactos-<blake3>.pc`). El hash del id de
    colección (una URL CalDAV/CardDAV con `/`, espacios y mayúsculas) evita rutas
    rotas. Escritura **atómica** (`.tmp` + `rename`); lectura best-effort (blob
    corrupto/viejo → vacío). Espeja a `paloma-store` (mismo patrón).
  - **Sync incremental** sobre el mismo snapshot: `upsert_event`/`delete_event` y
    `upsert_contact`/`delete_contact` aplican el delta por UID sin reescribir lo
    que no cambió a nivel lógico.
  - **Puente con `CalStore`**: `snapshot(account, &CalStore)` vuelca la caché en
    memoria entera a disco; `hydrate(account) -> CalStore` la reconstruye
    (offline-first). Añadido accesor `CalStore::contacts(book)` en el núcleo.
  - 7 tests verde (roundtrips, id con URL, miss vacío, upsert/delete idempotente,
    snapshot↔hydrate, cuentas aisladas).

## Pendiente (orden sugerido)

1. **Autodescubrimiento DAV** (PROPFIND `calendar-home-set`/`addressbook-home-set`)
   + verificar `raymi-net` contra servidor real.
2. **`raymi-app`** — binario lanzable, comparte `cuenta.json` con paloma; hidrata
   desde `raymi-store` al arrancar y refresca contra la red.
3. **Crear/editar eventos y contactos** desde la UI (put/delete ya en el trait).
4. **Cruce con paloma**: invitar contactos a eventos; “crear evento desde correo”.
