# FreeTube — features y cómo mapean a tawasuyu

> Estado: **plan vivo / exploratorio**. FreeTube no es un reproductor de
> archivos sino un **cliente de plataforma de video** (YouTube) con foco en
> privacidad. Este doc inventaría sus features y propone cómo se traducen a
> la suite. No bloquea el trabajo de `PARIDAD.md` (reproductor); es el frente
> "navegador de plataforma" que **usa** `media` para reproducir.

## Qué es FreeTube

Cliente de escritorio (Electron + Vue) que reproduce YouTube **sin cuenta de
Google, sin anuncios y con tracking reducido**. Dos backends intercambiables:

- **Local API** (`youtubei.js` / Innertube): habla directo con YouTube sin
  cuenta.
- **Invidious API**: proxy a través de instancias Invidious (más privacidad,
  depende de la salud de la instancia).

## Inventario de features

### Cuenta-less / privacidad (su razón de ser)
- Suscripciones **locales** sin cuenta (feed de suscripciones).
- Historial de reproducción **local**.
- Playlists **locales** (múltiples) + favoritos.
- **Perfiles** múltiples para organizar suscripciones.
- Import/Export: suscripciones, historial, playlists; importa de YouTube
  (Google Takeout), NewPipe, OPML/RSS.
- Soporte de **proxy** (p. ej. enrutar por Tor).
- Sin anuncios.

### Reproductor (video.js / Shaka)
- Selección de **calidad** y de **formato** (DASH vs legacy), modo **sólo audio**.
- Velocidad de reproducción, volumen, **captions/subtítulos**.
- **Autoplay** (siguiente), **loop**, screenshot.
- **Theatre mode**, **fullscreen**, **picture-in-picture**, **mini-player** al scrollear.
- Atajos de teclado.
- Defaults configurables (calidad/volumen/velocidad/autoplay por defecto).

### Integraciones de comunidad
- **SponsorBlock**: saltar/silenciar segmentos (sponsor, intro, outro,
  autopromo, recordatorio de interacción) + mostrar capítulos.
- **DeArrow**: des-clickbait colaborativo de títulos y miniaturas.
- **Return YouTube Dislike**: muestra el conteo de dislikes.

### Navegación / descubrimiento
- **Páginas de canal**: videos, shorts, live, playlists, posts de comunidad,
  about, búsqueda dentro del canal.
- **Búsqueda** con filtros (orden, fecha, duración, tipo) + sugerencias.
- **Trending** (con categorías) y **Popular** (Invidious).
- **Comentarios** (con respuestas, orden top/nuevos).
- **Lives** + live chat (limitado).

### Distraction-free / personalización
- Mostrar/ocultar elementos (comentarios, recomendados, live chat, trending,
  popular, miniaturas, etc.).
- Temas (claro/oscuro, color base/principal, custom), región/locale.
- **Reproductor externo** (abrir en mpv/VLC) y **descarga** de video (yt-dlp).
- Enlaces "abrir en YouTube/Invidious".

## Cómo mapea a tawasuyu

FreeTube sería un **frontend Llimphi nuevo** que compone piezas existentes en
vez de reimplementar un reproductor. Encaja casi natural con la filosofía del
repo (local-first, soberano, direccionado por contenido):

| Feature FreeTube | Pieza tawasuyu |
|---|---|
| Reproducir el stream | `media` (`FrameSource`/`AudioSource`, decoders nativos + `foreign-av`) — **R1/R2 de `PARIDAD.md` ✅** (URL + yt-dlp vía `shared/foreign-ytdlp`) |
| Backend YouTube/Invidious | **nuevo puente `shared/foreign-youtube`** (regla #4: formato/protocolo ajeno por puente) — Innertube/Invidious client |
| Suscripciones/historial/playlists locales | almacenamiento local direccionado por contenido (BLAKE3 + DAG + postcard), **sin cuenta** — ya es el modelo nativo |
| Perfiles + identidad | **`agora`** (identidad Ed25519 + grafo de confianza): perfiles soberanos, y un modelo de recomendación/feed federado sobre el grafo en vez de un DB central |
| SponsorBlock (saltar segmentos) | feature genérica de **segmentos** sobre el timeline de `media` (ya hay `SeekTo` + `llimphi-widget-timeline`); el catálogo de segmentos puede venir de SponsorBlock o de la red tawasuyu |
| DeArrow / Return Dislike | overlays de metadata opcionales sobre las tarjetas de video |
| Búsqueda/trending/canales/comentarios | vistas Llimphi sobre el puente; subtítulos ya los entiende `media` (SRT/WebVTT) |
| Proxy/Tor | a nivel del puente de red |
| Descarga | `foreign-av::transcode_a_av1` ya ingesta al formato nativo (BLAKE3) |

### Decisiones abiertas (no bloquean el arranque)
1. **Nombre y cuadrante** del app nuevo (semántico quechua/español; un
   "navegador de plataforma de video" podría caer en PERCIBIR o como app de
   HACER sobre `media`). **Pendiente de confirmar.**
2. **Prioridad de plataforma**: la ética del repo favorece lo federado
   (**PeerTube**/ActivityPub) e **Invidious** sobre YouTube directo. Propuesta:
   diseñar el puente con backends intercambiables y arrancar por el menos
   frágil. **Pendiente de confirmar.**
3. Qué integraciones de comunidad (SponsorBlock/DeArrow) entran y si su
   catálogo se centraliza o se federa sobre `agora`/`minga`.

### Prerrequisitos
De `PARIDAD.md` ya están cerrados los que bloqueaban este frente: **R1**
(URL/HLS/RTSP) ✅, **R2** (yt-dlp/plataformas vía `shared/foreign-ytdlp`,
formato muxeado) ✅ y **M1** (sync A/V) ✅ para que el video de red no derive.
Es decir: `media` ya **reproduce** desde una URL de plataforma. Lo que falta
para un FreeTube útil es la capa de **navegación/descubrimiento** (búsqueda,
canales, suscripciones, comentarios), que es el puente `shared/foreign-youtube`
(Innertube/Invidious) + un frontend Llimphi — no el reproductor.

Calidad alta (DASH A/V separados, YouTube > 720p) **✅ (2026-06-02)**:
`foreign_ytdlp::resolve_best` (`-f bv*+ba/b`) devuelve video y audio en URLs
separadas y `foreign_av::probe_dash` los muxea con dos entradas de ffmpeg, así
que ya no quedamos topados en 720p. (Falta validar a oído con red real.)
