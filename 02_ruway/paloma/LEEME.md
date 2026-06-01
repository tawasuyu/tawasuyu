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
paloma-store       — persistencia nativa (postcard + BLAKE3), offline-first. [HECHO]
paloma-llimphi     — frontend: lista de hilos + lectura + redacción.   [HECHO]
paloma-app         — binario lanzable (`paloma`): NetBackend + fallback demo. [HECHO]
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

- **Fase 4 (2026-06-01):** `paloma-app` — el binario lanzable (`paloma`).
  - Arma el `impl App` sobre `paloma-llimphi`, delegando en sus funciones libres.
  - Cuenta en JSON (`~/.config/paloma/cuenta.json` o `PALOMA_CONFIG`), plana y
    editable a mano (`CuentaFile` → `Account`). Contraseñas por entorno
    (`PALOMA_PASSWORD` o `PALOMA_IMAP_PASSWORD`/`PALOMA_SMTP_PASSWORD`) — nunca
    en el archivo.
  - Sin config/credenciales o si falla la conexión IMAP → **fallback a demo**
    (`paloma_llimphi::demo::backend()`), con el motivo en la barra de estado.
  - El seed de demostración se movió a `paloma-llimphi::demo` (única fuente,
    compartida con `examples/buzon_demo`).
  - Lanzar: `cargo run -p paloma-app --release` (o `-p paloma-app --bin paloma`).

- **Fase 5 (2026-06-01):** `paloma-store` — caché en disco **offline-first**.
  - Persiste buzones y mensajes por cuenta con **postcard**; el archivo por
    buzón se nombra por **BLAKE3** del nombre (tolera `/`, espacios, mayúsculas);
    escritura atómica (`.tmp` + rename). 5 tests.
  - `MailStore::ingest_mailboxes` precarga desde caché. El `Model` gana
    `with_persistence`: pinta lo cacheado, refresca de red y persiste; sin red,
    abre desde disco. `paloma-app` cablea un `MailDb` en `~/.cache/paloma`.

- **Fase 6 (2026-06-01):** búsqueda de texto local.
  - `paloma-core::search` (AND de términos; peso asunto>remitente>cuerpo) +
    `MailStore::search` cruza buzones y ordena. 7 tests.
  - Frontend: caja en la toolbar (tecla `/`); con consulta, el panel central
    muestra resultados planos; click o Enter abren el mensaje en su hilo; Esc
    limpia. (Semántica con `rimay` queda como extensión: su daemon es async y
    opt-in; no encaja en el modelo síncrono sin un runtime — ver Pendiente.)

- **Fase 7 (2026-06-01):** IMAP STARTTLS/plano + fetch de los últimos N.
  - `imap_client` soporta los tres transportes (TLS implícito, STARTTLS 143→TLS,
    plano) vía enum de sesión; métodos genéricos sobre el stream. Fetch acotado
    a los últimos N (rango desde el `EXISTS` del `SELECT`), N configurable
    (default 200) — `NetBackend::set_fetch_limit`, env `PALOMA_FETCH_LIMIT`.

- **Fase 8 (2026-06-01):** lectura — scroll + cuerpo HTML.
  - `Message::display_body` cae a `strip_html` cuando el mensaje vino sólo en
    HTML (despoja etiquetas, salta `<style>`/`<script>`, decodifica entidades,
    respeta saltos de bloque). 2 tests. Panel de lectura con scroll en píxeles
    (margen negativo + viewport clip); la rueda elige panel por la X del cursor.

## Pendiente (orden sugerido)

1. **Verificar contra un servidor real** (laptop, con credenciales) los caminos
   IMAP (TLS/STARTTLS/plano) y SMTP — sólo testeados por tipos hasta ahora.
2. **Búsqueda semántica con `rimay`** — exige un puente sync↔async al
   `rimay-verbo-daemon` (embeddings) + índice persistido; hoy la búsqueda es
   exacta. Es el gancho que falta para "buscar por significado".
3. **Firma/verificación con `agora`** (Ed25519) — firmar salientes y verificar
   entrantes; necesita keystore de `agora` + un header propio.
4. **Calendario/Contactos** (CalDAV/CardDAV) compartiendo la capa de cuentas.
5. **HTML rico vía puriy** cuando el usuario lo pida (hoy: texto despojado).
