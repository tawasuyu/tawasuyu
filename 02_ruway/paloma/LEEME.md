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
paloma-llimphi     — frontend: lista de hilos + lectura + redacción.   [pendiente]
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

## Pendiente (orden sugerido)

1. **`paloma-llimphi`** — tres-paneles (buzones · hilos · lectura) + redacción,
   sobre `MockBackend` primero, luego `NetBackend`.
2. **Verificar `paloma-net` contra un servidor real** (laptop, con credenciales).
3. **STARTTLS/plain en IMAP** + límite de fetch a los últimos N (sync incremental).
4. **`paloma-store`** — persistencia nativa (BLAKE3 + postcard) + búsqueda (`rimay`).
5. **Calendario/Contactos** (CalDAV/CardDAV) compartiendo la capa de cuentas.
