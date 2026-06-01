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
paloma-net         — puente IMAP/SMTP: implementa MailBackend contra
                     servidores reales (TLS/STARTTLS).                  [pendiente]
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

## Pendiente (orden sugerido)

1. **`paloma-net`** — IMAP fetch (LIST/SELECT/FETCH/STORE) + SMTP send sobre
   TLS/STARTTLS; mapea fallos a `MailError`. Credenciales vía un proveedor
   aparte (a futuro `agora`/`shared/auth`).
2. **Parser MIME** — `Date`/`From`/`To` + cuerpos `multipart/alternative` y
   nombres codificados `=?utf-8?…?=` (entran por un puente, no al núcleo).
3. **`paloma-llimphi`** — tres-paneles (buzones · hilos · lectura) + redacción.
4. **`paloma-store`** — persistencia nativa + búsqueda semántica (`rimay`).
5. **Calendario/Contactos** (CalDAV/CardDAV) compartiendo la capa de cuentas.
