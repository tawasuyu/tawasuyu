# paloma

> `paloma` (la paloma mensajera). Tipo: **cliente de correo nativo sobre Llimphi**.

El correo de la suite, nativo y sin navegador. Reemplaza a Gmail/Outlook sin
depender de una web-app con JIT — es la primera utilidad de la Tanda 1 de
`/APPS-NATIVAS.md` (el "Google Workspace" diario). Habla IMAP (entrada) y SMTP
(salida) contra cualquier servidor, y renderiza nativo en Llimphi; el HTML de
los mensajes lo puede pintar puriy cuando haga falta.

## Anatomía (objetivo)

```
paloma-core        — modelo agnóstico: direcciones, mensajes, buzones, hilos,
                     cuentas, el trait de transporte. Sin red ni UI.   [HECHO]
paloma-net         — puente MIME + IMAP (fetch) + SMTP (envío);
                     implementa MailBackend contra servidores reales.  [HECHO]
paloma-store       — persistencia nativa (BLAKE3 + postcard) + sync
                     incremental + búsqueda (rimay).                    [pendiente]
paloma-llimphi     — frontend: lista de hilos + lectura + redacción.   [HECHO]
paloma-app         — binario lanzable.                                  [pendiente]
```

Un dominio = un crate raíz `*-core` agnóstico + frontends Llimphi
intercambiables, como el resto de la suite.

## Estado

- **Fase 1 (2026-06-01):** `paloma-core` — núcleo agnóstico y `cargo test`-eable.
  - `Address` (parse/display de `Nombre <correo>` + listas con comas).
  - `Message` + `Flags` + `MessageId`; `snippet`, `reply_subject`.
  - `Mailbox` + `MailboxRole` (infiere rol del nombre: Inbox/Sent/Drafts/…).
  - `thread::build_threads` — hilos por `References`/`In-Reply-To` (union-find,
    JWZ simplificado; une respuestas a un ancestro ausente; no mezcla por asunto).
  - `Account` + `ServerConfig` + `Security` (sin contraseña: el secreto va aparte).
  - `MailBackend` (trait de transporte) + `MockBackend` in-memory.
  - `MailStore` — caché local: buzones → mensajes → hilos, flags, no-leídos.
  - 27 tests verde.

- **Fase 2 (2026-06-01):** `paloma-net` — puente de red.
  - `mime::parse_message` — RFC 822 → `Message` (mail-parser): headers, hilado,
    cuerpos text+html, encoded-words. 4 tests offline.
  - `imap_client` — fetch síncrono (`imap`+native-tls, TLS implícito): buzones,
    mensajes, set de flags por Message-ID (UID SEARCH+STORE).
  - `smtp` — envío (`lettre`): RFC 822 desde `OutgoingMessage`, multipart alt.
  - `NetBackend` — IMAP+SMTP tras el trait `MailBackend`. MIME testeado; los
    caminos IMAP/SMTP compilan, se verifican contra servidor real en la laptop.

- **Fase 3 (2026-06-01):** `paloma-llimphi` — frontend tres-paneles + redacción.
  - Paneles: **buzones** (rol + no-leídos) · **hilos** (asunto, remitente,
    extracto, fecha, punto de no-leído, contador) · **lectura** (mensajes del
    hilo apilados, de · para · fecha · cuerpo).
  - Selección: click en buzón sincroniza y abre; click en hilo lo marca leído.
  - Redacción: modal (scrim + tarjeta) con Para/Asunto/Cuerpo + Enviar/Cancelar;
    `c` redacta, `r` responde (prellena Re: + References), Tab cicla, Esc cierra.
    Envía por el backend y refleja en `Sent`.
  - Scroll de la lista de hilos por rueda; fecha formateada sin crate de tiempo
    (civil-from-days). Crate **agnóstico al backend**: `lib` expone `Model`/`Msg`
    + funciones libres; el anfitrión arma su `impl App` e inyecta el backend.
  - Demo: `cargo run -p paloma-llimphi --example buzon_demo --release`
    (MockBackend sembrado: un hilo de 3, suelto sin leer, boletín).

## Pendiente (orden sugerido)

1. **`paloma-app`** — binario lanzable: arma el `impl App` sobre `NetBackend`
   (lee cuenta/credenciales de config) con fallback a `MockBackend` sin red.
2. **Verificar `paloma-net` contra un servidor real** (laptop, con credenciales).
3. **STARTTLS/plain en IMAP** + límite de fetch a los últimos N (sync incremental).
4. **`paloma-store`** — persistencia nativa (BLAKE3 + postcard) + búsqueda (`rimay`).
5. **Calendario/Contactos** (CalDAV/CardDAV) compartiendo la capa de cuentas.
6. **Scroll del panel de lectura** + cuerpo HTML vía puriy cuando haga falta.
