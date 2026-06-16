# Puriy — backlog ejecutable (objetivos determinados, en serie)

Lista de objetivos **autocontenidos** para forjar sin preguntar ni desviarme.
Cada uno es una fase atómica con scope cerrado, encoding ya decidido y
criterio de aceptación. Se ejecutan **en orden**; cada fase termina commiteada
y verde antes de empezar la siguiente.

El changelog histórico vive en `SDD.md` §Estado. Este archivo es **futuro**:
cuando una fase se cierra, se tacha acá (`~~…~~ ✅ <commit>`) y su detalle pasa
al SDD si amerita.

## Reglas de ejecución (Definition of Done por fase)

Para CADA fase, en este orden:

1. **Parser** (`puriy-engine/src/style/parser/decls/effects.rs`) — acepta la
   sintaxis nueva; lo no soportado cae a `None` sin romper lo previo.
2. **Modelo de estilo** (`style/values/enums_text.rs`) — la variante/campo del
   enum `ClipPath` si cambia.
3. **BoxNode** (`boxes/model.rs` + resolución en `boxes/build/node.rs`, +
   `clip_*: None`/default en los otros 2 sitios de construcción: `node.rs`
   `empty_root`, `inline.rs`) — el dato resuelto que el chrome consume.
4. **Compositor** (`02_ruway/llimphi/llimphi-compositor/src/{lib.rs,view.rs,render.rs}`)
   — campo en `View`+`MountedNode`, builder, y el pintado (`push_layer` con la
   shape kurbo correcta). Destructuring en `render.rs` (2 sitios: mount + node).
5. **Wire** (`puriy-llimphi/src/render/mod.rs`) — `render_box` lee el campo del
   `BoxNode` y llama al builder del `View`.
6. **Tests** — parser (en `style/tests/group02.rs`), box-tree (en
   `boxes/tests/group01.rs`), builder del compositor (en `view.rs`
   `semantics_tests`). El render real a píxeles no se testea (no hay GPU en CI);
   se verifica que la cadena compone (engine computa → compositor almacena →
   geometría es aritmética simple → wiring compila).
7. **Gate**: `cargo test -p puriy-engine -p llimphi-compositor -p puriy-llimphi`
   verde + `cargo check --workspace` pasa.
8. **Commit + push**: `feat(puriy/llimphi): Fase 7.XXXX — <título>`, mensaje en
   español describiendo la cadena; `git pull --rebase origin main` antes del
   push (hay sesión paralela commiteando sobre el mismo repo).

**Cuándo SÍ frenar y preguntar** (lo único): (a) una fase no pasa el gate y la
causa es una decisión de producto, no un bug mío; (b) aparece una bifurcación
de diseño real no prevista acá. Si no, sigo a la próxima.

## Estado de partida (ya hecho)

- **7.1219** ✅ — `clip-path: inset()` se pinta (scissor rectangular).
- **7.1220** ✅ — `clip-path: circle()/ellipse()` se pinta (elipse real).
- **7.1221** ✅ — radios `%` en circle/ellipse (spec `clip_ellipse: [f32;12]` =
  centro `[cx_px,cx_pct,cy_px,cy_pct]` + 2 radios `[px,pct_w,pct_h,pct_diag]`).
- **7.1222** ✅ `1fcfd7f1` — closest-side/farthest-side (radio quint, spec
  `[f32;14]`; `circle()` vacío → closest-side).
- **7.1223** ✅ `b35b4109` — polygon() (ClipPath pierde Copy; `clip_polygon`).
- **7.1224** ✅ `79162621` — path() (kurbo BezPath::from_svg; `clip_path_svg`).
- **7.1225** ✅ `896d4cb3` — geometry-box de referencia (`clip_ref_inset`;
  `GeometryBox`). **Familia clip-path / basic-shape CERRADA.**

---

## Familia clip-path / basic-shape (CERRADA — detalle de cada fase abajo)

> Las 4 fases de esta sección están **hechas** (ver hashes arriba). Se deja el
> detalle como registro de lo planeado vs. lo construido. Desvíos respecto del
> plan original, anotados al implementar:
> - 7.1222: `side` se extendió a `{0,1,2,3,4}` (no `{0,1,2}`) para codificar la
>   BASE del lado (circle = 4 lados; ellipse = eje) — el compositor necesitaba
>   distinguirlas y el engine sabe cuál es al construir.
> - 7.1224 reparó además `View::lift` (sesión paralela) que no listaba los
>   campos nuevos de View — rebose silencioso por destructure exhaustivo.

### 7.1222 — `closest-side` / `farthest-side` en circle()/ellipse()

**Por qué**: hoy un radio keyword no parsea y `circle()` vacío cae a `0px`
(invisible) en vez del default spec `closest-side`. Son los radios implícitos
más comunes.

**Encoding**: extender cada radio del spec de quad `[px,pw,ph,pd]` a quint
`[px,pw,ph,pd,side]`, con `side ∈ {0=ninguno, 1=closest, 2=farthest}`. El spec
`clip_ellipse` crece `[f32;12]→[f32;14]` (centro 4 + 2 radios de 5). Cuando
`side≠0`, el compositor IGNORA px/pct y computa el radio desde el centro
resuelto `(cx,cy)` y el rect `(w,h)`:
- circle closest: `min(cx, w-cx, cy, h-cy)`; farthest: `max(…)`.
- ellipse rx closest: `min(cx, w-cx)`; farthest `max(cx, w-cx)`. ry: idem con
  `cy, h-cy`.

**Parser**: `parse_length_or_pct` o keyword. `closest-side`→side=1,
`farthest-side`→side=2 (px/pct = 0). `circle()`/`ellipse()` con radio ausente →
default `closest-side` (side=1), NO `0px`. El enum `ClipPath` guarda el radio
como un tipo que admita keyword: nuevo `enum ClipRadius { Len(LengthVal),
ClosestSide, FarthestSide }` reemplazando el `LengthVal` de radius/rx/ry.

**Aceptación**: parser (`circle(closest-side)`, `circle()`→closest, `ellipse(
farthest-side closest-side)`); box-tree (verifica side en el spec); builder.

### 7.1223 — `clip-path: polygon()`

**Por qué**: la otra basic-shape masiva (triángulos, recortes custom, flechas).

**Encoding**: `ClipPath` gana `Polygon { evenodd: bool, points: Vec<[f32;4]> }`
donde cada punto es `[x_px, x_pct, y_px, y_pct]` (resuelto contra ancho/alto en
el compositor). ⚠️ **`ClipPath` pierde `#[derive(Copy)]`** (un `Vec` no es Copy)
— pasar a `Clone`; corregir el único uso que lo asume: `style/decl.rs` (≈1492)
`s.clip_path = *c` → `c.clone()`. BoxNode: `clip_polygon: Option<(bool,
Vec<[f32;4]>)>` (campo nuevo, paralelo a `clip_ellipse`).

**Parser**: `polygon([<fill-rule>,]? <x> <y> [, <x> <y>]*)`. fill-rule opcional
`nonzero`(default)/`evenodd` antes de la lista. Cada coord `parse_length_or_pct`.

**Compositor**: campo `clip_polygon: Option<(bool, Vec<[f32;4]>)>` en
`View`/`MountedNode` + builder `clip_polygon`. Pintado: `BezPath` con `move_to`
al 1er punto resuelto, `line_to` al resto, `close_path`; `push_layer(if evenodd
{Fill::EvenOdd} else {Fill::NonZero}, …, &path)`. Prioridad de recorte por
nodo: polygon > elipse > inset > rect (una sola capa).

**Aceptación**: parser (con/sin fill-rule, %/px); box-tree (cuenta puntos +
evenodd); builder.

### 7.1224 — `clip-path: path()`

**Por qué**: forma arbitraria por path SVG; barata gracias a kurbo.

**Encoding**: `ClipPath::Path { evenodd: bool, d: String }`. BoxNode:
`clip_path_svg: Option<(bool, String)>`. Sin `%` (path() usa user units = px,
relativos al origen de la caja de referencia).

**Parser**: `path([<fill-rule>,]? "<svg-path-data>")`. Guardar el string crudo.

**Compositor**: en el pintado, `kurbo::BezPath::from_svg(&d)` (kurbo 0.11.3 lo
tiene — verificado), trasladar por `Affine::translate((r.x, r.y))`,
`push_layer`. Si `from_svg` falla, no recortar (log/skip silencioso). Campo
`clip_path_svg` + builder.

**Aceptación**: parser (con fill-rule, con comillas); box-tree (string + flag);
builder. Un caso compositor: `from_svg("M0 0 L10 0 L10 10 Z")` produce un path
no vacío.

### 7.1225 — geometry-box de referencia (`circle(50%) content-box`)

**Por qué**: `clip-path: <shape> <ref-box>` reposiciona la forma contra otra
caja (default `border-box`). Cierra la familia basic-shape.

**Dependencia**: el compositor sólo tiene el border-box rect; para
content/padding-box necesita los anchos de border (+padding). Por eso el
BoxNode debe llevar los insets de la caja de referencia ya resueltos por layout.

**Encoding**: `ClipPath` (todas las variantes) gana `ref_box: RefBox` (`enum
{MarginBox, BorderBox(default), PaddingBox, ContentBox}`). BoxNode:
`clip_ref_inset: Option<[f32;4]>` = `[top,right,bottom,left]` a restar del
border-box para llegar a la caja de referencia (border para padding-box;
border+padding para content-box; 0 para border-box; -margin para margin-box,
si hay margin disponible — si no, tratar margin-box como border-box). Se computa
en `build/node.rs` desde `style.border`/`style.padding` ya resueltos.

**Compositor**: antes de resolver centro/radios/puntos, encoger el rect base por
`clip_ref_inset` (igual patrón que `clip_inset`, pero como caja de referencia,
no como recorte final). Todas las shapes resuelven sus `%` contra ESE rect.

**Parser**: leer el keyword de caja al final de `clip-path` (puede venir antes o
después de la forma, p.ej. `content-box circle(50%)`). Forma sola → border-box;
caja sola (`clip-path: content-box`) → recorta a esa caja (rect).

**Aceptación**: parser (`circle(50%) content-box`, `padding-box`); box-tree
(insets correctos para content/padding/border); builder.

---

## Familia mask (DETERMINADA 2026-06-16)

**Bifurcación de diseño resuelta** (el usuario eligió, ver más abajo): el
compositor aplica `mask-image` como **máscara de luminancia nativa de vello**
(`push_luminance_mask_layer`, vello 0.7). El subárbol del nodo se aísla en una
capa (`push_layer`) y al cerrarla la luminancia de la imagen-máscara multiplica
el alpha del contenido ya pintado: blanco = visible, negro = oculto, gris =
semitransparente. Es lo más simple y correcto con la API actual.

**Estado del modelo de estilo**: los longhands `mask-mode`, `mask-type`,
`mask-repeat`, `mask-position`, `mask-size`, `mask-clip`, `mask-origin`,
`mask-composite`, `mask-source-type` y los `-webkit-mask-*` **ya parsean y
computan** (fases 7.275, 7.398–7.405, 7.1048–7.1051). Lo que faltaba era
**pintar**. Por eso estas fases son casi todas compositor + wire (el modelo de
estilo ya tiene el dato); sólo hay que llevar el longhand al `BoxNode` y
consumirlo en el pintado de la máscara.

### 7.1226 ✅ — `mask-image: url()` se pinta (máscara de luminancia)

Cadena completa forjada: parser (ya existía, 7.275) → `style.mask_image` →
`build/node.rs` decodifica el `url()` con `fetch_image_src` (misma cache que
`<img>`/`background-image`) → `BoxNode::mask_image: Option<ImageData>` →
`View`/`MountedNode::mask_image: Option<Image>` + builder `View::mask_image` →
`render.rs` aísla el subárbol y al cerrar la capa aplica `paint_mask_close`
(`push_luminance_mask_layer` + `draw_image` estirado al border-box) → wire
`puriy-llimphi` arma el `peniko::Image` y llama `.mask_image`. Tests: parser
(group02, ya estaba), box-tree (group03, data: percent-encoded para esquivar el
splitter `;`), builder (view.rs, ortogonal a `clip`). Fase 1: la máscara se
**estira al border-box** (sin size/position/repeat/mode/clip/origin/composite,
que vienen abajo). Default CSS para raster es `alpha`, no `luminance` — desvío
consciente: el modo alpha llega en 7.1228 vía `Compose::SrcIn`.

### 7.1227 ✅ — `mask-size` / `mask-position` / `mask-repeat` (encaje completo)

**Hecho** (`<pendiente hash>`). **Se folió 7.1229 (repeat) acá**: size y
position sin repeat producen un intermedio roto (una máscara intrínseca chica
con el default `repeat` mostraría un solo tile y ocultaría todo lo demás —
negro = oculto en luminancia), así que los tres van juntos para shippear algo
correcto. Reusa la aritmética EXACTA de `background-image`.

**Encoding** (sin sumar campos a los 4 sitios de construcción): el encaje viaja
**dentro** del campo `mask_image` del `BoxNode`, que pasó a
`Option<(ImageData, BackgroundSize, BackgroundPosition, BackgroundRepeat)>` —
sólo tiene sentido con imagen. El compositor gana tipos neutrales `MaskLen`
(`Auto`/`Px`/`Pct`), `MaskSize` (`Auto`/`Cover`/`Contain`/`Explicit`) y
`MaskPlacement` (size + pos_x/pos_y + repeat_x/repeat_y), más el campo
`View`/`MountedNode::mask_placement: Option<MaskPlacement>` (`None` = estirar al
border-box, Fase 7.1226) y builder `View::mask_placement`. `paint_mask_close`
resuelve size→tamaño de tile, position→offset del primero, repeat→tiling por
eje (mismo `axis()` con cap de 4096), dibujando N `draw_image` dentro de la capa
de luminancia. El wire (`mask_placement_de`) traduce los enums CSS → neutrales.

**Tests**: builder (`mask_placement_setea_encaje`), box-tree (group03 verifica
que el encaje por defecto `auto`/`repeat` llega al box).

### 7.1228 ✅ — `mask-mode` (luminance vs alpha)

**Hecho.** `mask-mode: alpha` (y el default `match-source` para raster) usa el
**canal alpha** de la máscara, no su luminancia. vello no expone capa de
alpha-mask directa → se compone con `push_layer(Fill::NonZero,
BlendMode::new(Mix::Normal, Compose::DestIn), ...)`: el subárbol ya pintado es
el **destino** y la máscara la **fuente**; `DestIn` mantiene el destino donde la
fuente tiene alpha (= alpha masking). `match-source` lo resuelve el wire a
`alpha` (las máscaras de puriy son raster `url()`); `luminance` explícito sigue
usando `push_luminance_mask_layer`. Efecto: el default CSS efectivo de una
`mask-image: url(raster.png)` pasó de luminancia (7.1226) a **alpha**.

**Encoding**: el `BoxNode::mask_image` sumó `MaskMode` a su tupla
`(ImageData, size, position, repeat, mode)`. El compositor ganó `enum MaskMode
{ Luminance(default), Alpha }` + campo `mode` en `MaskPlacement`;
`paint_mask_close` elige la apertura de capa según el modo y comparte la
aritmética de tiles. El wire (`mask_placement_de`) traduce `MaskMode` CSS →
neutral (`Alpha|MatchSource → Alpha`).

**Tests**: builder (`mask_placement_setea_encaje` incluye `mode` + default
`MaskMode::Luminance`), box-tree (group03 verifica que el modo por defecto
`match-source` llega al box). **NOTA**: el render real no se verifica a píxeles
(CI sin GPU) — la composición `DestIn` está validada por construcción/spec
Porter-Duff, no por captura. Conviene una verificación visual headless cuando
haya GPU disponible.

### 7.1229 ✅ — `mask-repeat` (tiling de la máscara) — FOLIADO EN 7.1227

Se hizo junto con size/position en 7.1227 (eran inseparables para no shippear un
intermedio roto). El tiling por eje reusa la lógica de `background-repeat`.

### 7.1230 ✅ — `mask-clip` / `mask-origin` (caja de referencia de la máscara)

**Hecho.** `mask-clip` recorta el efecto a border/padding/content-box;
`mask-origin` ancla el tiling/position. Análogo a `background-clip`/`-origin` +
al `clip_ref_inset` de clip-path: se computan en `build/node.rs` dos insets
`[t,r,b,l]` del border-box (padding-box = border; content-box = border+padding;
border/no-clip/cajas-SVG → `None`). En el compositor, `paint_mask_close` encoge
el border-box a `clip_rect` (recorte de la capa de máscara) y a `origin_rect`
(resolución de size/position/tiling). **Aproximación documentada**: `no-clip`
real (sin recorte) se trata como border-box; las cajas SVG (fill/stroke/view)
también caen a border-box en HTML.

**Refactor**: la tupla del `BoxNode::mask_image` (que ya tenía 5 elementos) pasó
a un struct `MaskSpec { image, size, position, repeat, mode, clip_inset,
origin_inset }` (re-exportado en `puriy_engine::MaskSpec`). El compositor sumó
`clip_inset`/`origin_inset` a `MaskPlacement`.

**Tests**: builder (`MaskPlacement` con clip/origin), box-tree (group03 verifica
que los defaults border-box no insetean). **La familia mask queda funcional**
salvo `mask-composite` (7.1231), que requiere modelar una lista de capas.

### 7.1231 ✅ — `mask-composite` + lista de capas de máscara

**Hecho.** `mask-image: url(a), url(b), …` ahora modela **varias capas** y
`mask-composite` (`add`/`subtract`/`intersect`/`exclude`) las combina.

**Modelado**: parser `parse_mask_image_layers` parte la lista por comas
top-level (paren-aware, así data: URLs no se rompen) → `style.mask_image` (capa
0) + `style.mask_extra_layers: Vec<MaskImage>` (nuevo decl `MaskImageLayers`).
El `MaskSpec` del box ganó `extra: Vec<(ImageData, MaskComposite)>` (las extras
viajan dentro del spec, sin sumar sitios de construcción al BoxNode). El
compositor ganó `enum MaskCompose { Add(default), Subtract, Intersect, Exclude }`
+ campo `View/MountedNode::mask_extra: Vec<(Image, MaskCompose)>` + builder.

**Pintado**: `paint_mask_close` pinta la capa 0 y luego cada extra; `add`
(default) se dibuja directo (source-over acumula), el resto compone vía
`Compose` Porter-Duff (`subtract→SrcOut`, `intersect→SrcIn`, `exclude→Xor`) en
una sub-capa. La aritmética de tiles se extrajo a `draw_mask_layer` (compartida
por todas las capas).

**Limitaciones documentadas** (scope acotado a propósito):
- Las capas extra **comparten** `mask-size`/`-position`/`-repeat`/`-mode`/
  `-clip`/`-origin` con la capa 0 (per-layer lists diferidas — un 7.1232 si hay
  apetito). Lo común (varias máscaras combinadas con un operador) sí anda.
- `mask-composite` es un único valor compartido entre todas las capas (no
  per-layer).
- **Sin verificación a píxeles** (CI sin GPU): `add` es correcto y de bajo
  riesgo (stacking); el mapeo de los otros operadores → `Compose` es el de la
  spec Porter-Duff pero NO está validado por captura. Para `mask-mode:
  luminance` multi-capa la combinación es aproximada (compone la imagen y luego
  toma su luminancia), exacta para `alpha`.

**Tests**: parser (`mask_image_lista_de_capas`, group02: lista, descarte de no-
url, stylesheet), box-tree (`mask_image_capas_multiples`, group03: 2 data: URLs
→ capa 0 + 1 extra con composite add), builder (`mask_extra_setea_capas`).

**La familia mask queda cerrada** (salvo refinamientos per-layer diferidos).

> **Bifurcación original (resuelta)**: cómo aplica el compositor la máscara —
> capa de luminancia nativa (`push_luminance_mask_layer`) vs. alpha vía
> `Compose::*`. Decisión 2026-06-16: **luminancia primero** (7.1226), alpha
> después (7.1228). Quedó así porque es la primitiva más simple y correcta de
> vello 0.7 y cubre el caso SVG `<mask>`; el alpha (default raster) se suma sin
> rehacer lo anterior.

---

## Familia mask CERRADA (2026-06-16)

Las fases 7.1226–7.1231 cierran `mask-*` (pintado luminance/alpha, size/
position/repeat, clip/origin, lista de capas + composite). Pendiente **menor y
diferido**: encaje/modo per-layer (hoy compartido) y verificación visual
headless de los compose Porter-Duff cuando haya GPU.

---

## Familia filter (DETERMINADA 2026-06-16) — bloque elegido por el usuario

**Estado de partida**: el engine ya **parsea** `filter`/`backdrop-filter` en
`BoxNode.{filter,backdrop_filter}: Vec<FilterFn>` (Fases 7.264/7.265; variantes
`Blur/Brightness/Contrast/Grayscale/HueRotate/Invert/Opacity/Saturate/Sepia/
DropShadow`). El compositor sólo pinta `View::backdrop_blur` (post-pasada Gauss
separable vía `BlurCompositor`). **Nada lee `BoxNode.filter` ni
`BoxNode.backdrop_filter` para pintar** — la cadena se corta tras el parseo.
Esta familia los cablea hasta el píxel.

**Arquitectura elegida**: post-pasada GPU sobre la intermediate, como
`backdrop_blur` — `collect_filters(mounted, computed)` recolecta `(rect, op)` y
el runtime los aplica tras la rasterización vello, restringidos al rect del
nodo. Limitación v1 idéntica a backdrop_blur: la post-pasada opera sobre los
píxeles finales del rect (no aísla el subárbol del fondo); aceptable y
documentada. Los filtros encadenan aplicándose en secuencia sobre el rect.

> **Bifurcación (resuelta)**: ¿layer vello (aislar subárbol) vs. post-pasada
> sobre la intermediate? Decisión 2026-06-16: **post-pasada**, reusando la
> infra de `backdrop_blur` y un `ColorFilterCompositor` nuevo en `llimphi-hal`
> espejo de `BlurCompositor`. vello 0.7 no expone color-matrix; el shader
> propio es la vía real (no stub) y CI no testea píxeles igual.

### 7.1232 ✅ — `filter: blur()` + `backdrop-filter: blur()` se pintan

**Spine de la familia.** Compositor: `View.filter: Vec<FilterOp>` +
`MountedNode.filter`; `enum FilterOp { Blur(f32) }` (crece por fase); builder
`View::filter(...)`; plumbing en `map_shared`/`mount_recursive`. Nuevo
`collect_filters(mounted, computed) -> Vec<FilterPass{rect, op}>` (salta el
subárbol al encontrar filtro, como `collect_backdrop_blurs`). Runtime
(`eventloop/redraw.rs`) aplica los `Blur` con `BlurCompositor` (mismo camino que
backdrop). Wire (`puriy-llimphi`): `b.filter` blur → `view.filter([Blur])`;
`b.backdrop_filter` blur → `view.backdrop_blur(sigma)`. CSS `blur(r)`: `r` es la
stdev → `sigma = r` directo; multi-blur suma. Tests: builder (view.rs
semantics_tests), `collect_filters` (mount+compute, sin GPU), box-tree
(`b.filter` carga el Blur).

### 7.1233 ✅ — filtros de color (color-matrix)

`brightness/contrast/grayscale/invert/sepia/saturate/hue-rotate/opacity`.
HAL: `ColorFilterCompositor` (WGSL color-matrix 4×5 RGBA+bias, dos pases
target→scratch (aplica) + scratch→target (copia), espejo de `BlurCompositor`).
`FilterOp::ColorMatrix([f32;20])`. Builders de matriz (**aritmética pura,
testeable**, en `puriy-llimphi`): brightness=diag k; contrast=k + bias
`(1-k)/2`; grayscale/saturate vía luminancia Rec.709 (`grayscale(g)=saturate(
1-g)`); invert=`(1-2a)·in+a`; sepia (matriz fija); hue-rotate (rotación
estándar SVG); opacity=fila alpha. `collect_filters` emite `ColorMatrix`;
runtime aplica con el compositor nuevo (state `color_filter_compositor`). Tests
(`puriy-llimphi::render::filter_tests`): neutros→identidad, grayscale total =
luminancia, invert total = negativo, brightness/opacity, mapeo+orden.

> **Desvío vs. plan**: `backdrop-filter` color **no** se cableó acá (sólo
> `filter` propio). El backdrop usa un camino distinto (`View::backdrop_blur`,
> pre-render del fondo); aplicarle color-matrix requiere extender ese mecanismo
> o conflar con la post-pasada de `filter`. Se difiere a 7.1235.

### 7.1234 — `filter: drop-shadow()`

Pinta sombra borroneada detrás del nodo reusando `draw_blurred_rounded_rect`
(primitiva de box-shadow). `FilterOp::DropShadow(...)`. v1: sombra del
border-box, no de la silueta alpha (misma aproximación que box-shadow). Se pinta
en `render.rs` antes del subárbol; wire desde `FilterFn::DropShadow(BoxShadow)`.
Tests: builder + box-tree.

### 7.1235 — cierre

Orden de aplicación de cadena (varios filtros en secuencia, verificado en
`collect_filters`), example headless `filter_demo` (evidencia PNG como
`backdrop_blur_demo`), doc de limitaciones (post-pasada sobre píxeles finales,
sin verificación GPU en CI). **Familia filter CERRADA.**

---

## Próximo bloque tras filter — a determinar

- **`background` per-layer avanzado** o gradientes cónicos/repeating que falten.
- Lo que marque el SDD §Estado como próximo hueco.
