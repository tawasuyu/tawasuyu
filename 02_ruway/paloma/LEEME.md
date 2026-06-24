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

- **Fase 9 (2026-06-01):** interfaz pulida + acciones + ganchos del roadmap.
  - **Pulido**: avatares con iniciales (color estable por correo) en lista,
    lectura y resultados; estrella clicable por hilo; barra de acento en
    selección; fechas cortas en listas; estados vacíos; placeholder de lectura.
  - **Acciones reales** (al backend + caché): ★ destacar / ✓ leído-no-leído
    (`set_flags`), 🗑 papelera (`\Deleted`, oculto vía `store.threads`), ↪
    reenviar (`OutgoingMessage::forward`), Cc en redacción. Barra de acciones en
    lectura; atajos `f`=reenviar, `Supr`=borrar.
  - **Ganchos del roadmap** (UI lista, backend pendiente y rotulado como tal):
    nav lateral Calendario/Contactos (chip "pronto"); checkbox **Firmar
    (Ed25519)** en redacción (estado "enviado · firmado"); toggle de búsqueda
    **Exacta | Semántica** (semántica avisa que cae a exacta hasta integrar
    `rimay`); botón **Ver HTML enriquecido** en mensajes con `body_html` (avisa
    que el render rico vía puriy está pendiente).

- **Fase 10 (2026-06-24):** búsqueda **por significado** (`paloma-semantic`).
  - Crate nuevo `paloma-semantic`: índice de embeddings (`rimay-verbo`) sobre
    los mensajes cacheados. El **cómputo es async** (`embed_messages`/
    `embed_query`, fuera del hilo de UI); el **ranking es síncrono y puro**
    (`SemanticIndex::search`, coseno) para el bucle Elm. Persistencia postcard +
    embedding **incremental** (`missing`) + purga (`retain`). Agnóstico al
    proveedor (mock/fastembed/Cohere). 6 tests verde (incl. el pipeline e2e).
  - El puente sync↔async vive en `paloma-app::semantic::DaemonSemantic`: runtime
    tokio + `rimay_verbo::conectar()` (daemon) o, con `PALOMA_SEMANTIC=mock`, el
    mock determinista. `paloma-llimphi` define el trait `SemanticEngine` y el
    `Msg::SemanticResults`; el motor embebe/rankea y despacha los ids por
    `Handle`. Sin daemon, el modo 🧠 cae a la búsqueda exacta avisándolo.
  - UI: en modo semántico se escribe y se presiona **Enter**; el panel muestra
    los resultados rankeados (o "buscando…"/"presioná Enter").
  - Requiere un `rimay-verbo-daemon` corriendo para ser útil de verdad
    (`cargo run -p rimay-verbo-daemon-bin -- --provider fastembed`).

- **Fase 11 (2026-06-24):** correo **LLM-nativo** (Eje 2).
  - `paloma-llimphi` define el trait `LlmAssistant` (`summarize` / `draft_reply`)
    + los `Msg::{Summarize,LlmSummary,DismissSummary,DraftReply,LlmDraft,LlmError}`.
    Mismo patrón async que el semántico: el asistente corre fuera del hilo de UI
    y despacha el resultado por `Handle`.
  - UI: botones **✨ Resumir** / **✨ Borrador IA** en la barra de acciones del
    hilo (sólo si hay asistente). El resumen aparece como banner descartable
    arriba del hilo; el borrador abre el compositor de respuesta con el cuerpo
    redactado (Para/Asunto/References ya puestos — listo para revisar y enviar).
  - El puente vive en `paloma-app::llm::LlmHelper`: runtime tokio + `pluma-llm`
    (`from_env`). **Local-first**: con `PLUMA_LLM_BACKEND=ollama` +
    `PLUMA_LLM_MODEL=...` el correo no sale de la máquina. Sin backend real (y
    sin opt-in) el asistente no se engancha y los botones ✨ no aparecen.
  - Certificado: 2 tests en `paloma-app` (camino de la request contra el mock +
    `truncar`); el despacho por `Handle` reusa el patrón probado del semántico.

- **Fase 12 (2026-06-24):** **firma/verificación Ed25519** real (Eje 3.A).
  - `paloma-core`: `MailSignature` + `canonical_signing_bytes` (versionado, cubre
    remitente/destinatarios/asunto/cuerpo, body normalizado CRLF→LF + trim para
    sobrevivir el ida-y-vuelta por SMTP/MIME). `OutgoingMessage.signature`.
  - `paloma-sign` (crate nuevo, sobre `agora-core`): `sign_outgoing` /
    `verify_message` + `encode/decode_signature` (formato del cable, base64).
    5 tests (roundtrip, cuerpo/asunto/remitente manipulado → Invalid, clave
    equivocada → Invalid).
  - `paloma-net`: SMTP emite los headers `X-Paloma-Pubkey` / `X-Paloma-Signature`;
    MIME los lee, recomputa los bytes canónicos y **verifica** → puebla el
    `SignatureStatus` (el badge de la UI ya lo pinta). Test e2e: firmar → cable →
    parsear → `Verified`; cuerpo alterado → `Invalid`.
  - `paloma-llimphi`: trait `MailSigner` inyectado; "Firmar" en el compositor
    ahora produce firma real (o avisa si no hay identidad).
  - `paloma-app::identity::AgoraSigner`: `Keypair` Ed25519 desde
    `~/.config/paloma/identity.seed` (0600, CSPRNG la 1ª vez).
  - **Alcance honesto**: `Verified` = integridad (la firma cierra sobre el
    contenido bajo la clave declarada). Falta atar `pubkey ↔ contacto` (red de
    confianza de `agora`) para que signifique "y la clave es de quien dice ser".

- **Fase 13 (2026-06-24):** **rail soberano** P2P — el protocolo (Eje 3.B).
  - `paloma-rail` (crate nuevo): correo suite-a-suite **sin SMTP**. La unidad es
    el `RailEnvelope` — el `Message` nativo (postcard) + identidades emisor/
    receptor + firma Ed25519 sobre todo. La dirección **es la clave pública**
    (`agora`), no un `usuario@dominio`: no hay "From spoofing".
    - `seal(keypair, to, msg)` / `open(env, me)` — sellar/abrir+verificar; el
      mensaje llega `Verified` (el sobre firmado lo autentica).
    - trait `RailTransport` (enviar a una identidad) — la implementación concreta
      va sobre chasqui; `MockTransport` corre el rail sin red. `RailInbox`
      acumula lo recibido (el futuro buzón "Suyu").
    - 7 tests: roundtrip, sobre para otra identidad rechazado, payload
      manipulado → BadSignature, **reenvío a otro receptor no cuela** (la firma
      ata `to`), bytes del cable, dirección `<hex>@rail.suyu` ↔ identidad, y
      **rail completo de punta a punta** sobre el transporte.
  - **Integración en la app (viva):** `paloma-llimphi` define el trait `RailLink`
    + `Msg::RailReceived`; al adjuntarlo se **fija el buzón local "Suyu"**
    (sobrevive a los syncs IMAP via `MailStore::pin_mailbox`). `send_compose`
    **enruta por destinatario**: las direcciones `@rail.suyu` van por el rail
    (selladas, sin SMTP), el resto por SMTP — mixto soportado. Botón "🛰 Mi
    dirección Suyu" en la toolbar. `paloma-app::rail::RailHost` sella con la misma
    identidad Ed25519 que firma el SMTP; **loopback en proceso** (enviarte a vos
    mismo entrega a tu Suyu y la marca Verified — ejercita el rail completo sin
    red). 2 tests de enrutado en `paloma-llimphi` + 1 de buzón pinned en core.
  - **Pendiente (requiere 2 nodos):** el **transporte de red real** (chasqui
    request-response / canal akasha) que implemente `RailTransport` y dispare
    `Msg::RailReceived` desde su loop de recepción — hoy `RailHost` usa
    `MockTransport` (los envíos a peers remotos se encolan). + resolución
    `contacto ↔ identidad` (libreta). El protocolo, el enrutado y el buzón están
    cerrados y certificados; falta el salto de red entre máquinas.

- **Probador de conexión (2026-06-24):** binario `paloma-test` (en `paloma-app`)
  verifica IMAP+SMTP reales sin GUI. Gmail-aware (defaults `imap/smtp.gmail.com`).
  `PALOMA_EMAIL` + `PALOMA_PASSWORD` (contraseña de **aplicación** en Gmail);
  `PALOMA_SEND_TEST=1` manda un correo de prueba a uno mismo.

## Pendiente (orden sugerido)

1. **Verificar contra un servidor real** (laptop, con credenciales) los caminos
   IMAP (TLS/STARTTLS/plano) y SMTP — sólo testeados por tipos. Usar `paloma-test`.
2. **Bandeja-canvas por significado** — sobre `paloma-semantic`, ordenar la
   bandeja por tema/atención (estilo `khipu`) en vez de lista cronológica.
3. **LLM-nativo — triage** — sobre `LlmAssistant`, falta el triage/importancia
   automático y la extracción de pendientes a una lista (resumen + borrador ✅).
4. **Confianza** — red de confianza `agora` (`pubkey ↔ contacto`) para que
   `Verified` signifique identidad, no sólo integridad; + seed cifrada
   (`agora-keystore`) en vez del `identity.seed` en claro (firma básica ✅).
5. **Rail soberano** `chasqui`/`ayni` — correo suite-a-suite sin SMTP (Eje 3.B).
6. **Multilienzo** (como `pluma`) — escribir una vez, leer en otro idioma/tono.
7. **Calendario/Contactos** (CalDAV/CardDAV) compartiendo la capa de cuentas.
8. **HTML rico vía puriy** cuando el usuario lo pida (hoy: texto despojado).
