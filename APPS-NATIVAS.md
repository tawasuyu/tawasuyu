# APPS-NATIVAS.md — utilidades nativas que reemplazan a las web-apps con JIT

> Snapshot: 2026-06-01. Doc estratégico, denso para IA y para el autor.
> Cuando otra doc contradiga sobre "qué app nativa reemplaza a qué web-app", esta gana.

## Tesis

`puriy-js` **no tiene JIT a propósito** (intérprete + caché de bytecode, nunca cranelift/V8).
Es una elección de soberanía y simplicidad: sin JIT, las SPAs pesadas (Gmail, Google Docs,
Figma, YouTube web) van a ir lentas dentro de puriy. La estrategia ganadora **no** es acelerar
puriy hasta poder correrlas — es **no necesitarlas**: para cada web-app exigente construimos
una app **nativa Llimphi** que pega a los **mismos backends** (IMAP/SMTP, CalDAV, API de
YouTube/Invidious, Matrix/XMPP…) usando la red real de `puriy-net`, pero renderiza nativo.

Reparto de roles:
- **puriy** = la web "de documentos" (HTML/CSS, JS liviano, formularios, sitios estáticos).
- **apps nativas de la suite** = la web "de aplicación" (correo, video, chat, mapas…).

Si una tarea cotidiana necesita un navegador con JIT, eso es una **señal de que falta una app
nativa**, no de que falte JIT.

## Ya cubierto en el repo

| Reemplaza a | Utilidad nativa | Dominio |
|---|---|---|
| Google Docs / Word | **pluma** (multilienzo) | `00_unanchay/pluma` |
| Sheets / Excel | **nakui-sheet** | `01_yachay/nakui/nakui-sheet` |
| Slides / PowerPoint | **pluma-deck** (modo Recorrido) | `00_unanchay/pluma` |
| YouTube | **media** + plan **FreeTube** | `02_ruway/media` |
| Spotify / audio | **takiy** + media | `02_ruway/takiy` |
| ChatGPT | **pluma-llm** | `00_unanchay/pluma` |
| Photoshop | **tullpu** (+ `foreign-psd` planificado) | `02_ruway/tullpu` |
| Notion / notas | **khipu** + pluma notebook | `00_unanchay/khipu` |
| Drive / archivos | **nahual** + CAS de wawa + **minga** (P2P) | `02_ruway/nahual` |
| Passwords / login / social | **agora** + `shared/auth` + `card` | `03_ukupacha/agora` |
| Translate | **rimay** + pluma multilienzo | `00_unanchay/rimay` |
| Lector PDF/imagen | visores **nahual** | `02_ruway/nahual` |

## Huecos por construir — prioridad recomendada

### Tanda 1 — el "Google Workspace" diario (máximo ROI)
1. **Correo** — cliente IMAP/SMTP/JMAP nativo. *La* app que reemplaza a Gmail.
   Reusa `puriy-net` (TCP/TLS), `rimay` para búsqueda semántica de mensajes, `agora` para
   identidad/firma (PGP/Ed25519). Núcleo agnóstico `*-core` + frontend Llimphi, como el resto.
2. **Calendario + Contactos** — CalDAV/CardDAV. Se construye junto al correo; juntos cierran
   el reemplazo de Workspace. Comparte la capa de cuentas/credenciales con correo.

### Tanda 2 — tiempo real (apalanca P2P + WebRTC que ya existen)
3. **Chat / mensajería** — cliente Matrix/XMPP, o nativo sobre BrahmanNet/`chasqui` P2P.
   Reemplaza Slack/WhatsApp/Discord; la identidad de `agora` ya está.
4. **Videollamadas** — **ARRANCADA** (`02_ruway/uya`, "cara" en quechua). MVP andando
   end-to-end: identidad BLAKE3, presencia, video en ambos sentidos (RGBA sobre TCP) +
   cámara/micrófono/colgar, sobre `media-core` (captura) y un frontend Llimphi de rejilla.
   El stack WebRTC de `puriy-js` resultó ser sólo bindings JS (sin transporte nativo
   reusable), así que `uya` monta señalización+media propios. Pendiente: audio, mudar el
   transporte a card-net (P2P soberano, akasha/minga) y compresión de cuadros. Reemplaza Zoom/Meet.

### Tanda 3 — descubrimiento e información
5. **Mapas / navegación** — tiles OSM + routing. Caro pero alto valor; reemplaza Google Maps.
   Encaja como canvas Llimphi (GPU directo, ver bench Iris Xe).
6. **Lector RSS / noticias** — agregador de feeds. Combina con `khipu` (gravedad temporal) y
   `rimay` (clustering semántico). Reemplaza Feedly y los portales pesados.
7. **Buscador / meta-search nativo** — front que consulta varios motores (o índice local) sin
   cargar el JS de google.com. Pega con puriy + `rimay`.

### Tanda 4 — segunda línea
8. **Lector PDF/ePub nativo** dedicado (hoy parcial vía nahual/pluma-deck).
9. **Cliente Git/forge** (GitHub/GitLab vía su API) + `shuma` + `nada` + pluma notebook.
10. **Dashboard de finanzas** — la vertical Fintech que ya contempla el meta-modelo de `nakui`.

## Principio de diseño común

Toda app nueva sigue las reglas duras del repo:
- **Un dominio = un crate raíz `*-core` agnóstico + frontend(s) Llimphi intercambiables.**
- La lógica de dominio no sabe quién la pinta.
- Formatos/protocolos ajenos entran por puentes (`shared/foreign-*` o un `*-net` propio), nunca
  al núcleo.
- Identidad y transporte compartidos: `agora` (quién), BrahmanNet/`card-net` (cómo viaja).
- Integración al escritorio: registrarse como **visor/Card** en el registry de `nahual`
  (`(lens, mime, priority)`) para el "open-with universal".

## Primer paso concreto sugerido

Arrancar el **dominio de correo**: `core` (cuentas, IMAP/SMTP, modelo de mensaje/hilo,
índice) + frontend Llimphi (lista de hilos + lectura + redacción). Es el de mayor impacto diario
y siembra la capa de cuentas que después reusan calendario y contactos.
