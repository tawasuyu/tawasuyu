# Tullpu — editor de imágenes por capas, IA-able

> Tullpu (quechua: *teñir, dar color, pigmento*). Tipo: **Layered image editor over Llimphi**, edición no destructiva con operaciones IA como nodos del DAG.

> Estado: **propuesta de diseño** (2026-05-29). Nada en disco todavía salvo este SDD. Referenciado en `PLAN.md` §6.ter (tabla office/PSD) y §6.quinquies (multimedia).

## Tesis

Un editor de imágenes donde **la pila de capas ES un DAG content-addressed** y **cada operación (filtro, ajuste, op IA) es un nodo derivado**, no un píxel pisado. Edición no destructiva por construcción: deshacer es navegar el grafo, no un buffer de undo. La parte "IA-able" no acopla tullpu a ningún modelo — las ops IA hablan con un daemon de modelos de píxel por socket (calco de `rimay-verbo-daemon`).

El insight central: **es el mismo patrón que `pluma` ya inventó para texto** (haz de cuerpos, ver `PLAN.md` §11). En pluma, una traducción/resumen es un *cuerpo hijo* derivado de un *cuerpo madre* por una `Transformacion`; si la madre cambia, la hija queda *stale* y la UI pinta la hebra punteada con botón "regenerar". En tullpu, un inpaint/upscale/segmentación es una *capa hija* derivada de una *capa madre* por la misma maquinaria. Reuso conceptual total, distinta carga útil (píxeles en vez de párrafos).

## Por qué encaja en gioser sin fricción

- **Capas = DAG.** Una pila de capas con sus modos de fusión es literalmente un grafo dirigido. Se mapea sobre `format::Objeto{datos, hijos}` (BLAKE3 + postcard) sin modelo nuevo.
- **Dedup automática.** Dos capas con contenido idéntico (o dos versiones de un proyecto que comparten un fondo) comparten hash y se almacenan una sola vez en `almacen.rs`.
- **No destructivo gratis.** Cada ajuste es un nodo. Cambiar un parámetro aguas arriba marca *stale* el cono descendiente; la UI lo regenera bajo demanda.
- **UI = frontend intercambiable.** `tullpu-core` no sabe quién lo pinta. El frontend Llimphi se monta sobre `llimphi-widget-nodegraph` (ya pensado para tullpu, ver su doc) + un lienzo de pintura custom.
- **Formatos ajenos por puente.** PSD entra por `shared/foreign-psd`, nunca al núcleo. La app trabaja siempre en formato nativo.

## Anatomía — crates

```
[ CUADRANTE III · 0x02 RUWAY — HACER ]

4. tullpu-app-llimphi   — Binario lanzable (en mirada o en wawa)
   │                      Lienzo + panel de capas + grafo de ops + inspector
   │                      Sobre llimphi-widget-nodegraph + color-canvas
   ▼
3. tullpu-render        — Compositor: recorre el DAG top-down, fusiona capas
   │                      (modos de blend) → buffer Rgba8 → llimphi-surface
   │                      Acelerable por compute shader (vello/wgpu) a futuro
   ▼
2. tullpu-ops           — Catálogo de operaciones (nodos del DAG)
   │                      Locales: brush, ajustes (curvas, niveles, HSL),
   │                      filtros (blur, sharpen), máscaras, transformaciones.
   │                      IA (vía pixel-verbo-daemon): segmentar, inpaint,
   │                      upscale, restyle, generar. Cada op = TransformacionPixel
   ▼
1. tullpu-core          — Modelo agnóstico
   │                      Capa, Lienzo, GrafoDeCapas, modos de fusión,
   │                      estado stale/fresh, serialización a Objeto BLAKE3
   │                      Sin deps de Llimphi ni de modelos IA
   ▼
[ Estado puro · DAG content-addressed ]
```

Más:
- `shared/foreign-psd` — puente psd ↔ grafo de capas tullpu (import; export lossy a futuro).
- `pixel-verbo-daemon` — sirve modelos de imagen (ONNX) por socket Unix; consumidores cambian `Mock` por `DaemonClient::connect(...)`. Ver `PLAN.md` §6.quinquies.

## Modelo de datos (borrador)

```rust
// tullpu-core
struct Capa {
    id: Uuid,                 // estable a través de regeneraciones
    contenido: Hash,          // BLAKE3 del buffer Rgba8 (Objeto en el grafo)
    blend: ModoFusion,        // Normal, Multiplicar, Pantalla, Superponer, ...
    opacidad: f32,
    mascara: Option<Hash>,    // máscara alfa, otro Objeto
    visible: bool,
    origen: OrigenCapa,       // Raster | Derivada(TransformacionPixel)
}

enum OrigenCapa {
    Raster,                                  // pintada/importada a mano
    Derivada { madre: Uuid, op: TransformacionPixel, estado: Frescura },
}

enum Frescura { Fresca, Stale }              // stale ⇒ la UI ofrece "regenerar"

enum TransformacionPixel {
    Local(OpLocal),                          // determinista, en proceso
    Ia { modelo: String, prompt: Option<String>, params: Value },  // vía daemon
}
```

Una capa derivada que queda *stale* (su madre cambió) pinta la conexión punteada en el grafo — idéntico al *stale* de `pluma-notebook` y al haz de cuerpos. La frescura se propaga por el cono descendiente.

## Decisiones abiertas

- **Compositing CPU vs GPU.** Arranca en CPU (`image` crate ya en el stack); migrar a compute shader vía wgpu cuando el lienzo lo pida. `tullpu-render` aísla la decisión.
- **Resolución de escala.** Buffers grandes no caben cómodos en un solo `Objeto`; evaluar tiling (capa = grafo de tiles content-addressed → dedup por tile, ideal para fondos repetidos).
- **Export.** Reusar `pineal-export` (ya hace PNG/SVG/GIF) para la salida; PSD de salida es post-MVP.

## Fases de forja

1. **`tullpu-core`** ✓ — Capa, GrafoDeCapas, modos de fusión, serialización a `Objeto`. Testeable con `cargo test` sin gráficos.
2. **`tullpu-render`** ✓ — compositor top-down CPU → buffer Rgba8. Hito: cargar 3 capas PNG y componerlas a un PNG de salida correcto.
3. **`tullpu-app-llimphi`** ✓ (MVP) — lienzo + panel de capas + paleta. Botón clicable por capa; lienzo central pinta `peniko::Image`. Pendiente: nodegraph sobre `llimphi-widget-nodegraph` cuando llegue `llimphi-surface`.
4. **`tullpu-ops` locales** ✓ — invertir/brillo/contraste/niveles/blur/opacidad/saturación/tonalidad como nodos derivados con *stale tracking*.
5. **`pixel-verbo-daemon` + `tullpu-ops` IA** ✓ — `pixel-verbo-{core,mock,daemon,daemon-bin}`; `regenerar_stale_con_ia` cablea `TransformacionPixel::Ia` al `Proveedor`. Mock determinista (segmentar/inpaint/restyle/generar). Daemon `std::thread`-por-conexión, cliente bloqueante. La app resuelve daemon→mock al arranque y lo muestra en el header. Pendiente: proveedor ONNX real (segment-anything, restyle, upscale), tiling para imágenes grandes.
6. **`shared/foreign-psd`** ✓ — `importar_psd(bytes) -> DocumentoPsdImportado{lienzo, buffers, informe}`. Una capa PSD → una `Capa` raster con buffer Rgba8 hasheado BLAKE3 (dedup automática). Blend modes mapeados al catálogo de `ModoFusion`; los no soportados (Soft Light, HSL, Burn…) caen a `Normal` y se anotan en el informe. Sin máscaras / grupos / clipping / ajustes (post-MVP). Ejemplo `psd_a_png` cierra el loop: PSD real → tullpu-render → PNG.
7. **Blend modes faltantes en `tullpu-render`** ✓ — `ModoFusion` extendido con 12 variantes Photoshop por-canal: `SubExpQuemado` (Color Burn), `SubLinealQuemado` (Linear Burn), `SobreExpAclarado` (Color Dodge), `LuzFuerte` (Hard Light), `LuzSuave` (Soft Light, fórmula W3C), `LuzViva` (Vivid Light), `LuzLineal` (Linear Light), `LuzPunto` (Pin Light), `MezclaDura` (Hard Mix), `Exclusion`, `Resta` (Subtract), `Division` (Divide). `mezclar_canal` cubre cada uno con clamps explícitos para evitar NaN en bordes (`src=0` en burn, `src=1` en dodge). `foreign-psd::mapear_blend` ahora mapea esos 12 discriminantes PSD directo (antes caían a Normal con `degradado=true`). App: `siguiente_blend` y `etiqueta_blend` cubren las nuevas variantes; el ciclador del botón "blnd" recorre las 20.
8. **Blends HSL (no-separables)** ✓ — `ModoFusion` cierra la familia con `HslTono` (Hue), `HslSaturacion` (Saturation), `HslColor` (Color), `HslLuminosidad` (Luminosity). Implementación via W3C Compositing & Blending L1 §10.3: luminosidad ponderada `Lum = 0.3R + 0.59G + 0.11B`, `Sat = max−min`, `SetLum` con `ClipColor` (proyecta al cubo `[0,1]³` preservando el matiz), `SetSat` por reordenamiento de canales. `mezclar_canal` cortocircuita antes del despacho per-channel — los HSL operan sobre el triple, no por canal. `foreign-psd::mapear_blend` ahora cablea los 4 discriminantes HSL directo. Tests cubren: matiz preservado al colorizar grayscale, luminosidad pasada desde src (blanco/negro), saturación cero anula a gris uniforme, lum de dst preservada al cambiar tono.
9. **Comparativos por luminosidad** ✓ — `ModoFusion::ColorMasOscuro` (Photoshop "Darker Color") y `ColorMasClaro` ("Lighter Color"). No factorizan por canal: la decisión es por píxel completo — `Lum(src) ⋚ Lum(dst)` selecciona el triple ganador y se devuelve entero. `mezclar_canal` los cortocircuita junto a los HSL antes del despacho per-channel; convención de empate: gana `dst`. `foreign-psd::mapear_blend` cablea los discriminantes 7 (DARKER_COLOR) y 12 (LIGHTER_COLOR) directo. App: el ciclador `blnd` ahora recorre 22 modos.
10. **Dissolve — catálogo Photoshop cerrado** ✓ — `ModoFusion::Disolver` umbraliza el alfa efectivo (`sa * opacidad * mascara`) contra un ruido PRNG estable por píxel; si gana el alfa, el píxel sale 100% src opaco, si no, el dst se mantiene. Semilla: primeros 8 bytes del `Capa.id` (Uuid es estable a través de regeneraciones, así que el patrón acompaña a la capa aunque cambie su contenido). PRNG: splitmix64 mezclando `seed + i * 0x9E3779B97F4A7C15` (golden ratio), mantissa de 24 bits para evitar artefactos en el borde 1.0. Vive en rama propia `fundir_disolver` dentro de `fundir_capa` — no encaja en `mezclar_canal` porque la decisión es per-píxel binaria sobre el alfa, no una mezcla `(s,d)`. `foreign-psd::mapear_blend` ahora cablea DISSOLVE directo: **el catálogo Photoshop completo mapea sin degradado** (los 28 discriminantes upstream cubiertos). Tests cubren: alfa=1 pinta todo src; alfa=0 deja todo dst; alfa=0.5 da ~50% mix (tolerancia ±10% en 4096 píxeles); render es determinista (a==b bit a bit entre composiciones del mismo lienzo); el patrón cambia con `Capa.id` distinto. App: ciclador a 23 modos.
11. **Export — guardar lienzo como PNG** ✓ — `tullpu-render::exportar_png(lienzo, fuente, ruta) -> Result<RgbaImage, Error>` envuelve `componer` + `RgbaImage::save`; el formato lo deduce `image` por extensión (PNG, JPEG, etc.). Nuevo variant `Error::Imagen(image::ImageError)` propaga fallas de codificación; si `componer` falla antes, el archivo no se crea. App: variante `Msg::ExportarPng` + botón `💾 exportar PNG` arriba en el panel ops (sección "salida", sin lock por selección — exportar es global). Path generado: `tullpu-export-<unix_ts>.png` en CWD (sin file picker todavía); el header muestra el path final o el error. Tests: roundtrip (lienzo 4×3 con dos capas → PNG → re-decode → píxeles idénticos al compuesto en RAM), y propagación de error (si compose falla, el archivo no se toca). Pendiente: file picker en la app, exports a JPEG/WebP con parámetros (calidad), export a PSD (post-MVP), máscaras de capa PSD (bloqueado upstream), jerarquías de grupos PSD.

**Estimación gruesa** (de `PLAN.md` §6.ter): tullpu base 3-4 meses · foreign-psd 2 sem post-tullpu.

## Relación con otros dominios

- **[[pluma]]** — el haz de cuerpos es el patrón madre/hija/stale que tullpu calca sobre píxeles.
- **[[pineal]]** — `pineal-export` para la salida; `pineal` es visualización de datos, tullpu es edición de imagen (no se solapan).
- **[[nahual]]** — `nahual-image-viewer-llimphi` es *viewer* (solo lectura); tullpu es *editor*. nahual decodifica PNG/JPEG con el crate `image`, mismo punto de entrada que tullpu para raster.
- **[[llimphi]]** — `llimphi-widget-nodegraph` (grafo de capas/ops) + `llimphi-surface` (lienzo vivo, primitivo nuevo del motor).
