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
6. **`shared/foreign-psd`** ✓ — `importar_psd(bytes) -> DocumentoPsdImportado{lienzo, buffers, informe}`. Una capa PSD → una `Capa` raster con buffer Rgba8 hasheado BLAKE3 (dedup automática). Blend modes mapeados al catálogo de `ModoFusion`; los no soportados (Soft Light, HSL, Burn…) caen a `Normal` y se anotan en el informe. Sin máscaras / grupos / clipping / ajustes (post-MVP). Ejemplo `psd_a_png` cierra el loop: PSD real → tullpu-render → PNG. Pendiente: máscaras de capa, jerarquías de grupos, blends faltantes en tullpu-render.

**Estimación gruesa** (de `PLAN.md` §6.ter): tullpu base 3-4 meses · foreign-psd 2 sem post-tullpu.

## Relación con otros dominios

- **[[pluma]]** — el haz de cuerpos es el patrón madre/hija/stale que tullpu calca sobre píxeles.
- **[[pineal]]** — `pineal-export` para la salida; `pineal` es visualización de datos, tullpu es edición de imagen (no se solapan).
- **[[nahual]]** — `nahual-image-viewer-llimphi` es *viewer* (solo lectura); tullpu es *editor*. nahual decodifica PNG/JPEG con el crate `image`, mismo punto de entrada que tullpu para raster.
- **[[llimphi]]** — `llimphi-widget-nodegraph` (grafo de capas/ops) + `llimphi-surface` (lienzo vivo, primitivo nuevo del motor).
