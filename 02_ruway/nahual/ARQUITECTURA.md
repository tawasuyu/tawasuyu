# ARQUITECTURA.md — nahual

> Descripción técnico-arquitectónica densa, optimizada para consumo por IA.
> Snapshot: 2026-05-30. Fuente autoritativa cuando difiera con la prosa de los READMEs.

```yaml
DOMINIO: nahual
CUADRANTE: 02_ruway (HACER)
NOMBRE: nahuatl "espíritu acompañante"
TESIS: meta-app "open-with universal" — abre cualquier dato heterogéneo despachando al visor correcto
       según la NATURALEZA DISCERNIDA del contenido, NO la extensión del archivo
PARADIGMA: shell + file-explorer + preview-pane; despacho por contenido (shuma-discern → viewer_registry::pick)
PRINCIPIO: VIEWERS, NO EDITORS — visualización read-only; para editar se usa otra app (nada/pineal)
TAMAÑO: ~9.5 KLoC; 2 libs + 11 visores productivos (+1 stub SVG)
```

## Flujo central (el corazón del dominio)

```
Archivo → read(8 KB header) → shuma-discern (pipeline de probes) → Discernment{ty, mime, lens, confidence}
        → viewer_registry::pick(Discernment) → ViewerKind → Shell::load_for → PreviewPane::<X>(state) → <X>_viewer_view()

EJEMPLO CLAVE: un .png renombrado a .txt → MagicBytes detecta 0x89PNG (conf 0.99) → ViewerKind::Image.
Acierta pese a la extensión mentirosa. Esa es la tesis entera: contenido > extensión.
```

## Detección: `shuma-discern` (en `02_ruway/shuma/sandbox/shuma-discern`)

```
DiscernPipeline::default = [MagicBytes, CardProbe, JsonProbe, TomlProbe, TabularProbe, Utf8Probe]
ESTRATEGIA: primer discerner con confidence ≥ 0.9 gana; si ninguno, el de mayor confidence.

MagicBytes (0.99): PNG/JPEG/PDF/ELF/WASM/gzip/ZIP/tar(ustar@257)/GIF/WEBP/WAVE/FLAC/Ogg/MP3/EBML/IVF/TTF/OTF/ttcf.
CardProbe (0.97): JSON con keys {schema_version,id,payload} → TypeRef::Wit{package:"brahman:card"}, lens "card". ANTES que JsonProbe.
JsonProbe (0.95): serde_json parsea → lens "tree".
TomlProbe (0.93): secciones [..]/clave=valor → lens "tree".
TabularProbe (0.93): hint path .csv/.tsv + delimitador en 1ª línea → lens "table".
Utf8Probe (0.5): UTF-8 válido, controles <5%; lens por extensión (.md→markdown, .rs/.py/.go/.js/.ts→code). Fallback universal.

Discernment{ ty: TypeRef, confidence: f32, mime: Option<String>, lens: Option<String> }   // TypeRef viene de card-core
```

## Despacho: `viewer_registry::pick` (nahual-shell-llimphi/src/viewer_registry.rs, 200 LOC, 11 tests)

```
PRECEDENCIA:
  1. lens explícito  gallery→Image · video→Video · audio→Audio · card→Card · tree→Tree · table→Table · markdown→Markdown · font→Font
     EXCEPCIÓN: image/gif SIEMPRE → Video (anima aunque el lens sea gallery)
  2. mime prefix     image/* → Image · video/* → Video · audio/* → Audio
  3. mime contenedor application/{zip,x-tar,gzip} → Archive
  4. mime binario    application/{x-executable,wasm} → Hex
  5. fallback        Text (nunca falla feo)
```

## Visores implementados (11) — cada uno 200–600 LOC, bajo acoplamiento

```
Text     fallback UTF-8 + syntax por extensión (read-only sobre widget text-editor)
Image    PNG/JPEG/WebP + GIF estático; pan/zoom, fit, magnifier, EXIF
Video    WebM/MKV(AV1 nativo puro-Rust)/IVF + GIF animado; demuxer por extensión
Audio    WAV/MP3/FLAC/Opus/Vorbis; espectro 48 bandas (Goertzel); sink cpal !Send vive en Model
Card     shared/card (schema_version/id/payload) como campos legibles
Tree     JSON/TOML indentado (legible aun minificado)
Hex      ELF/wasm/binarios: dump offset+hex+ascii, 16 B/fila, sin deps
Table    CSV/TSV columnas alineadas
Markdown .md vía pulldown-cmark 0.12 (encabezados h1..h6, código, listas, citas)
Archive  ZIP/jar/apk/epub/OOXML (zip 2.4) · tar(ustar@257) · tar.gz(flate2 streaming)
Font     TTF/OTF: metadatos + MUESTRA DIBUJADA — ttf_parser::OutlineBuilder → kurbo::BezPath → paint_with
         (showcase de render vectorial directo; se ve aunque la fuente no esté instalada)
```

## Libs declarativas (presentes, aún NO usadas por el shell)

```
meta-schema  (1172) schema declarativo de UIs data-driven: Module · EntitySpec · View{List,Form,Detail,Dashboard,Report,Graph}
meta-runtime (2360) helpers puros: parseo tipado, validación, delta, formato/presentación
USO ACTUAL: consumidos por Nakui/otros, NO por nahual-shell. Reservados para definir visores en JSON sin código Rust (futuro).
```

## Cómo registrar un visor nuevo (procedimiento real)

```
1. crate nahual-<x>-viewer-llimphi: struct <X>Preview + fn load_<x>(path) + fn <x>_viewer_view()
2. shell main.rs: import + variante en enum PreviewPane + arm en load_for
3. viewer_registry: variante ViewerKind::<X> + fila en pick() (lens/mime) + test
4. shuma-discern: nuevo Discerner si hay magic-bytes/heurística nueva, o reusar lens existente
5. (futuro) meta-schema: cuando exista AppBus, registro dinámico en tabla en vez de hardcode in-process
```

## Faltantes y limitaciones conocidas

```
PDF      shuma lo detecta (%PDF-, lens "reader") pero NO hay rasterizador PDF puro-Rust en el workspace ⇒ cae a Text. BLOQUEADO.
SVG      stub (1 LOC), en construcción por otro agente.
Deck     presentaciones — futuro.
seek/scrub  video/audio sólo Space play/pausa; audio estima playhead con reloj (AudioSource type-erased, falta exponer Seekable).
AppBus   NO existe aún: el registro de visores es HARDCODE in-process. Sin visores out-of-process ni registro dinámico.
```

## Relaciones inter-dominio

```
shuma    : shuma-discern es el cerebro de detección (pipeline de probes por magic-bytes + heurística).
llimphi  : toda la UI — llimphi-ui/theme/layout(taffy)/text/raster(kurbo,peniko) + widgets {list,splitter,text-editor,tree}.
card     : card-core aporta TypeRef/Discernment; shared/card es el formato del Card viewer. "brahman:card" es nombre legacy.
media    : media-source-{av1,webm,gif} + media-audio-cpal + media-core(AudioProbe, Spectrum) para video/audio.
wawa-config: preferencias (theme/lang) compartidas con el monorepo; watcher reactivo sin reinicio.
minga    : card-discovery (en minga) es el widget de descubrimiento de Cards consumido por nahual-shell.   ← NEXO BRAHMAN
```

## Estado (2026-05-31)

### Hecho
- 11 visores en-proceso (Text/Image/Video/Audio/Card/Tree/Hex/Table/Markdown/Archive/Font), cada uno 200–600 LOC, bajo acoplamiento.
- Shell con split draggable, navegación por teclado (↑↓ Enter Backspace Space) y despacho por contenido (no extensión) vía `shuma-discern` → `viewer_registry::pick`.
- Galería de miniaturas (`nahual-gallery-llimphi`): `thumb-core` (generación + cache en disco + planificador), fast-path EXIF embebido, orientación EXIF, zoom de grilla, eviction RAM, badge de tamaño, slideshow, ordenamiento, preview a tamaño completo.
- Espina del front universal: trait `Source` + 4 adapters (POSIX, wawa-image, NouserSource/Mónadas, MingaSource/DAG de AST); el shell monta nouser y minga por atajo.
- Render vectorial directo en el font viewer; GIF reusa el video viewer sin crate nuevo. Menú principal + contextual en las vitrinas.

### Pendiente
- PDF: detectado por `shuma` pero sin rasterizador puro-Rust en el workspace → cae a Text (BLOQUEADO).
- SVG: stub (1 LOC), en construcción.
- Deck (presentaciones), seek/scrub real en video/audio (hoy sólo play/pausa), play por click.
- AppBus / registro dinámico: hoy el `viewer_registry` es hardcode in-process; sin visores out-of-process registrados por `(lens, mime, priority)`.
- Libs `meta-schema`/`meta-runtime` presentes pero aún NO consumidas por el shell (reservadas para visores data-driven en JSON).

## Estado vs aspiración

```
ASPIRA_A:
  - PDF (bloqueado por rasterizador) · SVG (WIP) · Deck · seek/scrub · play por click.
  - AppBus / EntityType: visores FUERA-DE-PROCESO que se registran con (lens, mime, priority);
    shell publica EntitySelected, viewers suscriben; registro pasa de hardcode a TABLA DINÁMICA.
  - mover viewer_registry a crate compartido si otra app además del shell lo necesita.

NORTE_ARQUITECTÓNICO:
  nahual es el "abridor universal" de la suite: dado cualquier byte, discierne su naturaleza y lo muestra bien.
  Hoy el registro de visores es estático e in-process. Su destino declarado es el AppBus: que cualquier app/servicio
  registre un visor por (lens, mime, priority) y el shell despache out-of-process — exactamente el patrón de discovery
  tipado que Brahman ya implementa para datos. nahual+AppBus ES el caso de uso natural de las Cards a nivel de UI.
```

---

**Síntesis de una línea para otra IA:** nahual es una meta-app "open-with universal" que lee los primeros 8 KB de
cualquier archivo, discierne su naturaleza por contenido (no extensión) vía un pipeline de probes (`shuma-discern`), y
despacha al visor correcto de entre 11 (`viewer_registry::pick`) — read-only por principio, con detección robusta ante
archivos mal renombrados, cuyo norte es reemplazar el registro estático in-process por un AppBus de visores
out-of-process registrados por `(lens, mime, priority)`, que es justamente el patrón de discovery tipado de Brahman llevado a la UI.
