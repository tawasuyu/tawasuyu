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
raymi-llimphi      — frontend: vista mes/semana + agenda + contactos +
                     editor (crear/editar/borrar, recurrencia, invitados). [HECHO]
raymi-app          — binario lanzable.                                    [HECHO]
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
    local de etiqueta. `NetBackend` implementa ambos traits (colecciones por URL).
    **17 tests offline**; los caminos HTTP se verifican contra un servidor real
    (Nextcloud/Radicale) en la laptop.

- **Fase 5 (2026-06-01):** autodescubrimiento DAV en `raymi-net`.
  - `dav` gana `PROPFIND` + `DavClient::discover(base_url)`: principal del usuario
    (`current-user-principal`) → home-sets (`calendar-home-set`/
    `addressbook-home-set`, ambos en un viaje) → enumeración `Depth: 1` de
    colecciones (`resourcetype` + `displayname` + `calendar-color` de Apple).
    Tolerante: si falta principal o home-set, cae a la base/principal. `resolve`
    arma URLs absolutas a partir de los `href` del servidor; el color `#rrggbbaa`
    se recorta a `#rrggbb`.
  - `NetBackend::discover(user, pass, base_url)` mapea las colecciones a
    `Calendar`/`AddressBook` nativos (rol inferido del nombre) y queda listo para
    `sync_*`. **+5 tests offline (22)**; los viajes HTTP se verifican en la laptop.

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

- **Fase 6 (2026-06-01):** `raymi-app` — el binario lanzable (`cargo run -p
  raymi-app`, binario `raymi`), offline-first.
  - Comparte la cuenta con paloma: lee el mismo `~/.config/paloma/cuenta.json`
    (campo extra `dav_url`, que paloma ignora) y acepta `PALOMA_PASSWORD`.
    Envs propios: `RAYMI_PASSWORD`, `RAYMI_DAV_URL`, `RAYMI_CONFIG`.
  - `try_net`: parsea la cuenta → `NetBackend::discover(user, pass, dav_url)`
    (autodescubrimiento de Fase 5) → `Box<dyn DavBackend>`.
  - **Offline-first** vía `raymi-store`: `Model::with_persistence` hidrata el
    `CalStore` desde `~/.cache/raymi` antes del primer viaje de red (pinta lo
    cacheado al instante), sincroniza y vuelca el snapshot fresco a disco. Si la
    red falla, conserva lo hidratado y el status lo dice (“sin red · N en caché”).
  - Sin `dav_url`/contraseña → modo demo (`raymi-llimphi::demo::backend`), así
    siempre arranca. `cargo check --workspace` verde.
  - Añadidos en `raymi-llimphi`: `Model::with_persistence(backend, theme, CalDb,
    account_id)` + constructor común `build`; `resync` persiste tras éxito.

- **Fase 7 (2026-06-01):** crear / editar / borrar **eventos y contactos** desde
  la UI (`raymi-llimphi`), escribiendo a través del backend (`put_*`/`delete_*`
  ya en el trait) y a la caché.
  - `raymi-core::CalStore` gana `upsert_event`/`remove_event` y
    `upsert_contact`/`remove_contact`: mantienen la caché en memoria consistente
    sin re-sincronizar la colección entera (+2 tests).
  - `raymi-llimphi::editor` — borradores (`EventDraft`/`ContactDraft`) con campos
    de texto, ciclo de foco con **Tab**, parseo `AAAA-MM-DD` / `HH:MM` y
    conversión borrador → modelo nativo. Día completo ancla a medianoche y dura
    un día; si el fin ≤ inicio se corrige a +1h; sin título → “(sin título)”;
    contacto sin nombre no se guarda. Correos/teléfonos se editan en una línea
    separada por comas. Al editar un evento existente se **preservan** los campos
    no editables (recurrencia, organizador, invitados). +6 tests.
  - **Modal** (`view_overlay`): selector de calendario (clic cicla), asunto,
    checkbox día completo, fecha, horas (inicio/fin lado a lado), lugar,
    descripción; abajo Eliminar · Cancelar · Guardar. Backdrop oscuro cierra; la
    tarjeta absorbe el click (`Msg::Noop`).
  - **Disparadores**: botones “＋ Evento” / “＋ Contacto” en la barra; clic en una
    fila de agenda edita ese evento; “✎ Editar” en la ficha de contacto; tecla
    `n` abre el editor según el modo. Con el editor abierto, el teclado es suyo
    (Esc cierra, Tab cicla foco, el resto escribe).
  - `save_*`/`delete_*` envían al backend y, si lo acepta, aplican a la caché y la
    persisten (offline-first); en error dejan el editor abierto y avisan en la
    barra de estado. `raymi-app` cablea `view_overlay`. En modo demo todo funciona
    (el `MockBackend` ya guarda). `cargo check --workspace` verde.

- **Fase 8 (2026-06-01):** **recurrencia editable** en el modal de evento.
  - `raymi-core::Recurrence::to_rrule` — serializador canónico (inverso práctico
    de `parse`): omite `INTERVAL=1`, lista `BYDAY` en orden, `COUNT` sobre `UNTIL`
    (excluyentes), `UNTIL` en forma de fecha `AAAAMMDD`. +1 test de roundtrip.
  - `EventDraft` gana controles de repetición: `Repeat` (No se repite / Diaria /
    Semanal / Mensual / Anual), intervalo, días `BYDAY` (semanal), y término
    `RepeatEnd` (Sin fin / Tras N veces / Hasta fecha). `from_event` **descompone**
    la `RRULE` en los controles si parsea; si no la sabemos representar, se
    preserva cruda y el formulario muestra “No se repite”. `build` recompone la
    regla (la compuesta gana sobre la preservada). +3 tests.
  - **UI**: selector de cadencia (chip que cicla ⟳), “cada N <unidad>”, 7 toggles
    de día (L M X J V S D) sólo en semanal, y la condición de término con su campo
    contextual (N veces / fecha). Las nuevas ocurrencias se expanden de inmediato
    en la grilla y la agenda (vía `CalStore::occurrences_in`). `cargo check
    --workspace` verde.

- **Fase 9 (2026-06-01):** **invitar contactos a eventos** (cruce con la libreta).
  - `EventDraft` expone `attendees: Vec<Address>` editable (+ `add_attendee`/
    `remove_attendee`, dedup por correo sin distinguir mayúsculas) y una caja
    `invitee`; los `ATTENDEE` ya no sólo se preservan, se editan. +1 test.
  - **UI** en el modal de evento, sección **Invitados**: pills removibles (✕) de
    los actuales, caja “Nombre &lt;correo&gt; · Enter” (parsea con
    `Address::parse`) y **sugerencias** en vivo desde `CalStore::search_contacts`
    (hasta 4, con correo, que no estén ya invitados) → clic los suma con nombre.
  - La agenda del día muestra “👤 N” cuando el evento tiene invitados; el demo
    siembra “Reunión con clientes” con Ana y Bruno para verlo de una.
  - `cargo check --workspace` verde.

- **Fase 10 (2026-06-01):** **editar serie vs. instancia** de un recurrente, sin
  `RECURRENCE-ID` multi-VEVENT — sólo `EXDATE` + `UNTIL` + uids distintos.
  - `Event` gana `exdates: Vec<i64>` (instancias excluidas, `EXDATE`); `serde`
    `default` para no romper la caché. `CalStore::occurrences_in` salta las
    excluidas (+1 test). `raymi-net::ical` escribe/parsea `EXDATE` (+1 roundtrip).
  - **Tres alcances** (chip “Aplicar a”, visible sólo al abrir una instancia de un
    recurrente): *Toda la serie* (edita la base, como antes), *Esta instancia*
    (excluye la instancia en la base vía `EXDATE`; al editar, además crea un evento
    **suelto** con lo cambiado), *Esta y siguientes* (corta la base con
    `UNTIL = instancia−1` y abre una **serie nueva** desde la instancia con la
    misma cadencia). Borrar respeta el mismo alcance.
  - La agenda pasa el `start` de la instancia clickeada (`EditEvent.occ_start`);
    `EventDraft::focus_instance` ancla el formulario a ese día. `until_before`
    recorta la `RRULE` (limpia `COUNT`, excluyente con `UNTIL`). +3 tests de
    integración en `raymi-llimphi` (13 en total). `cargo check --workspace` verde.
  - **Límite conocido:** “Esta instancia” usa un evento de uid propio en vez de un
    `RECURRENCE-ID` en el mismo recurso; internamente consistente, pero un servidor
    CalDAV real preferiría el override en el mismo `.ics` (lo afinará el puente).

- **Fase 11 (2026-06-01):** **vista semana** (rejilla horaria) y **vista día**.
  - Conmutador **Mes / Semana / Día** en la barra (teclas `m`/`w`/`d`); la
    navegación ‹ ›, la rueda y ←/→ se vuelven **período-aware** (mes/semana/día).
    “Hoy” y la etiqueta (mes, rango “1–7 Junio 2026”, o “Lun 1 Junio 2026”) siguen
    la vista. La vista día reusa la rejilla de la semana con una columna ancha.
  - `week_grid`: cabecera de 7 días (hoy con disco de acento, clic selecciona el
    día), franja de **día completo** arriba, y **rejilla horaria** 07:00–22:00 con
    medidor a la izquierda. Los eventos con hora se **posicionan por hora**
    (bloques `Position::Absolute`, `top`/`height` derivados de `start`/`end`,
    recortados al rango); líneas horarias de fondo. Clic en un bloque lo edita
    (pasa `occ_start`, así enchufa con el alcance serie/instancia de la Fase 10).
  - Sin núcleo nuevo: todo sale de `CalStore::occurrences_in`. `cargo check
    --workspace` verde.

## Pendiente (orden sugerido)

1. **Verificar `raymi-net` contra servidor real** (Nextcloud/Radicale) en la
   laptop: discover + sync + put/delete + `EXDATE`/`UNTIL` end-to-end.
2. **“Crear evento desde correo”** en paloma (la otra mitad del cruce): un mensaje
   con fecha/hora detectada o un `.ics` adjunto → evento en raymi.
3. **Overrides fieles con `RECURRENCE-ID`** (mismo UID, multi-VEVENT por recurso)
   para que “esta instancia” viaje 1:1 a un servidor real.
