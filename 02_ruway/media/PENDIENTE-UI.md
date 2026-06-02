# media — pendientes de interfaz (wiring en `media-app`)

> Estado: **plan vivo**. Lista lo que ya tiene **núcleo puro cerrado y testeado
> en CI** pero **falta cablear en `media-app`** (o pintar con Llimphi). Pensado
> para retomar **con pantalla**: arrancá `cargo run -p media-app -- <archivo>` y
> andá indicándome los cambios de interfaz; cada ítem dice qué API de núcleo
> usar, qué hay que cablear y qué vas a tener que decidir/verificar a ojo/oído.
>
> Complementa a `PARIDAD.md` (qué falta y en qué orden) y `CONTROLES.md`
> (mapeo de entrada → acción, ya ✅). Cuando un ítem se cierre, moverlo a
> "Hecho" en `PARIDAD.md` y borrarlo de acá.

## Sesión 2026-06-02 — decisiones y avance

**Decisiones del usuario** (válidas para todo lo de abajo):
- **Config = ventana OS real** → hay que **extender Llimphi a multiventana**
  (hoy es monoventana). Es el cambio grande pendiente (ver más abajo).
- **Título de ventana dinámico**: ✅ hecho (`App::window_title`).
- **Visualizadores** ("cajas grandes"): ocultables, **default ocultos**. ✅ hecho.
- **Waveform tipo Audacity**: onda de la **pista completa** (decodificar/escanear
  todo el audio, con eje de tiempo + playhead). Pendiente.

**Hecho esta sesión** (en `origin/main`):
- ✅ **Seek en pausa salta de inmediato** al destino y sigue pausado (flag
  `SEEK_FORCE`, gate de pausa del video movido al render loop).
- ✅ **Título dinámico de ventana** (`llimphi-ui::App::window_title`) + media-app
  lo usa con el título del medio; se quitó el cartelón sobre el video.
- ✅ **Slider de volumen graduable** con el mouse en el medio (`−`[slider]`+`).
- ✅ **Visualizadores ocultables** (menú Ver, default ocultos).

**Pendiente de esta tanda de feedback** (en orden sugerido):
1. **Config con tabs de Llimphi + layout ordenado/delimitado** —
   `llimphi-widget-tabs` (reemplaza los `chip_button`); el contenido en un
   panel delimitado al tamaño. Reusable tal cual cuando la config pase a ventana
   real. *Independiente del multiventana; se puede hacer ya sobre el overlay.*
2. **Multiventana en Llimphi** (el grande) → config como ventana OS real con su
   barra de título. Refactor del runtime (`eventloop.rs`, 891 líneas, todo
   monoventana) + el trait `App` (retrocompatible, opt-in con defaults para no
   romper las ~decenas de apps). **Alto riesgo, no verificable sin pantalla**
   (además del bug de freeze abierto): hacerlo como paso dedicado y **correrlo**.
3. **Barras arriba/abajo del video** — hoy las barras van todas debajo del
   canvas. Agregar posición por barra (sobre/bajo el video) en
   `media-core::toolbar`/`layout` + respetarla en `toolbar_view`.
4. **Tercera barra** — el editor de barras (config → Barras) ya tiene
   `AddBar`; verificar que agregar una 3ª barra funcione end-to-end y, si hace
   falta, exponerlo mejor.
5. **Panel de playlist/cola** — vista de la cola (drawer o panel), reusando el
   modelo. Adoptar `media-core::playlist` (U1) de paso.
6. **Waveform de pista completa** (Audacity) — escaneo de picos del audio +
   caché + dibujo con eje de tiempo y playhead (reemplaza/complementa el visor
   en vivo). Necesita decodificar toda la pista (helper en `foreign-av`/fuentes).

## Cómo trabajar esto

El núcleo de cada ítem **ya existe y está testeado**: lo que sigue es trabajo
de UI, que sólo se valida corriendo la app (por eso no se hizo a ciegas). El
orden de abajo es sugerido (de menos a más invasivo), pero vos mandás: decime
"hagamos U4" o "primero el menú de pistas" y arrancamos por ahí.

Para cada ítem: **[núcleo]** lo que ya está listo · **[cablear]** lo que falta
en `media-app` · **[decidir/ver]** lo que necesito que mires o elijas.

---

## A — De bajo riesgo (overlays y datos, no tocan el pipeline)

### U4 — OSD (cartel de volumen/seek/velocidad)
- **[núcleo]** `media-core::osd`: `Osd` (mensaje transitorio con expiración +
  `alpha` de fade, tiempo inyectado) + `format_volume`/`format_speed`/
  `format_seek`/`format_hms`.
- **[cablear]** un `Osd` en el estado de `media-app`; en cada comando relevante
  (volumen, seek, velocidad, pista, etc.) llamar `osd.flash(texto, now)`; en el
  `view`/paint, si `osd.active(now)`, dibujar el texto con `osd.alpha_default(now)`
  como opacidad sobre el video. `now` = reloj de pared en segundos.
- **[decidir/ver]** posición del cartel (¿arriba-centro como mpv?), tipografía/
  tamaño, color/fondo, duración (`Osd::DEFAULT_SECS`).

### U5 — Metadata / carátula
- **[núcleo]** `media-core::metadata`: `Metadata` (título/artista/álbum/año/…) +
  `CoverArt {bytes, mime}` parseados de ID3v2 y FLAC.
- **[cablear]** al abrir un archivo de audio, parsear metadata; mostrar título/
  artista en la barra (hoy se muestra el nombre de archivo) y decodificar
  `CoverArt.bytes` con `peniko::Image` para pintar la carátula (usar
  `View::image`, ver MANUAL de Llimphi).
- **[decidir/ver]** dónde va la carátula (¿panel lateral?, ¿cuando no hay video?),
  qué campos mostrar.

### U2 — Resume / "continuar donde quedaste"
- **[núcleo]** `media-core::library`: `History` + `ResumePoint` (posición por
  medio, política "ya terminó", LRU), round-trip RON.
- **[cablear]** persistir un `historial.ron` (junto a `config`/`layout`);
  cargar al arrancar; al abrir un medio conocido con `ResumePoint`, ofrecer
  "continuar" (banner/diálogo) y hacer `SeekTo` a esa posición; ir guardando la
  posición periódicamente y al cerrar.
- **[decidir/ver]** dónde guardar el `.ron`, cómo ofrecer "continuar" (¿auto?,
  ¿botón?, ¿toast con timeout?), cada cuánto persistir.

### U6 — Bookmarks (marcas manuales)
- **[núcleo]** `media-core::library`: `Bookmarks` (varias marcas con etiqueta
  por medio, `next_after`/`prev_before`), round-trip RON.
- **[cablear]** comandos para poner/quitar marca y saltar a la próxima/anterior;
  pintar las marcas sobre el `llimphi-widget-timeline`; persistir en RON.
- **[decidir/ver]** glifo/color de la marca en el timeline, atajos, si comparte
  archivo con el historial o va aparte.

---

## B — Menús de pistas (tocan el spawn de ffmpeg)

### S2 / A2 — Selección de pista de audio y subtítulo embebidos
- **[núcleo]** `media-core::tracks`: `TrackSet` (selección/ciclado) +
  `foreign-av::probe` ya devuelve `MediaInfo.tracks: Vec<MediaTrack>`.
- **[cablear]** construir `TrackSet::from_tracks(info.tracks)` al abrir; menús
  "Pista de audio" / "Subtítulos" (con `MediaTrack::label()`); al elegir, pasar
  el `index` del stream al decoder. **Esto exige extender `foreign-av`**: el
  spawn hoy mapea `0:v:0`/`{input}:a:0` fijos — hay que poder mapear un `index`
  concreto (`-map 0:<index>`) y **respawnear** la sesión al cambiar (como el
  seek). Subtítulo embebido = extraerlo/renderizarlo (se cruza con S3).
- **[decidir/ver]** cómo se ven los menús (¿en una barra?, ¿command palette?,
  ¿ambos?), comportamiento del ciclo (tecla estilo VLC). **Necesita un archivo
  multi-pista real para verificar.**

---

## C — Pipeline de video (blit / render)

### V2 — Aspect ratio / crop / zoom / pan
- **[núcleo]** `media-core::viewport`: `ViewControl` (modo + aspecto + zoom +
  pan + crop, con mutadores clampeados) + `compute_layout(...) -> Layout{src,dst}`.
- **[cablear]** guardar un `ViewControl` en el estado; en el paint del video,
  calcular `Layout` con el tamaño del frame y del panel, y dibujar la textura
  en `dst` muestreando `src` (hoy se escala el frame al panel entero). Comandos
  en el palette: ciclar modo (`cycle_fit`), `zoom_by`, `pan_by`, `set_aspect`,
  `reset`. (Buen candidato a combinar con OSD: mostrar el modo al cambiar.)
- **[decidir/ver]** atajos, presets de aspecto (4:3, 16:9, 2.35:1), si el pan se
  hace con drag del mouse.

### S3 — Estilo de subtítulos ASS (color/fuente/posición)
- **[núcleo]** `parse_ass` ya llena `StyleSheet` + `cue.{style,align,pos}`;
  `SubtitleTrack::{style_for,align_for}` resuelven el estilo efectivo.
- **[cablear]** el render de subtítulos (hoy texto plano abajo-centro) debería
  usar `align_for(cue)` para posicionar (numpad → ancla), `style_for(cue)` para
  fuente/tamaño/color primario y `cue.pos` para posición absoluta. Color inline
  (`\c`) y karaoke (`\k`) siguen fuera de alcance.
- **[decidir/ver]** cuánto del estilo respetar (¿sólo posición+color?, ¿también
  outline/shadow?), fuente de fallback. **Necesita un .ass real con estilos.**

---

## D — Audio (necesitan oído / hardware)

### M5 — Velocidad con corrección de tono (atempo)
- **[núcleo]** `foreign-av::atempo_chain(speed)` + `setpts_for_speed(speed)`
  (filtros listos, encadenan fuera de `[0.5,2]`).
- **[cablear]** `MediaSession` necesita un `set_speed(speed)` que guarde la
  velocidad y respawnee ffmpeg agregando `-filter:a <atempo_chain>` y
  `-filter:v <setpts>`. Hoy la ruta ffmpeg no tiene varispeed (sólo las fuentes
  nativas, sin corrección de tono). Atar al comando de velocidad existente.
- **[decidir/ver]** verificar a oído que el pitch se mantiene; cómo convive con
  el varispeed nativo (¿unificar el control de velocidad?).

### A5 — Downmix/upmix y normalización automática
- **[núcleo]** `media-core::channels::remix_into` (matrices 5.1→estéreo, etc.) +
  `media-core::loudness` (medición EBU R128, ya wireada como `NormAuto`).
- **[cablear]** que las fuentes multicanal pasen por `remix_into` hacia el
  layout de salida real de cpal. Validar `NormAuto` a oído con material real.
- **[decidir/ver]** layout de salida objetivo (estéreo casi siempre), si el
  downmix es automático o opcional.

### A6 — Crossfade / gapless entre pistas
- **[núcleo]** `media-core::fade`: curvas + `crossfade_into` (mezcla por bloque).
- **[cablear]** la **máquina de transición** entre pistas vive en la capa de
  playlist de la app: detectar fin de pista, solapar el buffer de la siguiente,
  mezclar con `crossfade_into`. Necesita el modelo de playlist adoptado (U1).
- **[decidir/ver]** duración del crossfade, si es gapless puro o con solape.

### U1 — Editor de playlist (cola)
- **[núcleo]** `media-core::playlist`: modelo de orden + edición (drag-reorder,
  enqueue-next, repeat/shuffle determinista), round-trip RON.
- **[cablear]** que `media-app` adopte este modelo en vez de su `Playlist`
  acoplada a los decoders; UI editora (lista reordenable arrastrando, ver/guardar
  cola). Reusar patrones de reordenado de Llimphi.
- **[decidir/ver]** cómo se ve la cola (¿drawer?, ¿panel?), persistencia.

---

## E — Requieren hardware/pantalla desde cero (sin núcleo aún)

No tienen núcleo puro separable; hay que implementarlos contra la app/GPU/HW:

- **V1 — Fullscreen real** (necesita API de ventana de `llimphi-ui`).
- **V5 — Deinterlacing** · **V6 — Filtros/shaders** · **V8 — HDR/tone-mapping**
  (pipeline GPU).
- **U3 — Thumbnails en hover del timeline** (extraer frame por timestamp vía
  ffmpeg — `foreign-av` necesitaría un helper de extracción puntual + caché).
- **M2 — Decode por hardware** (VAAPI/NVDEC…) · **M3 — Seek frame-accurate** ·
  **M4 — Frame stepping** (ruta ffmpeg).
- **A3 — Selección de dispositivo de salida** (cpal enumera devices).
- **R3 — Servidor de streaming/transcoding** · **R4 — DLNA/Chromecast** (red).

---

## Verificaciones a ojo/oído acumuladas (lo ya cableado, sin confirmar)

- **R2 DASH** (yt-dlp `bv*+ba`): A/V separados muxeados por ffmpeg con dos
  entradas — falta confirmar con red real que sincroniza y supera 720p.
- **A5 NormAuto** (EBU R128): la medida sale bien en tests; falta validar que
  el ajuste de ganancia suena correcto con material real.
