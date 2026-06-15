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

## Después de la familia clip-path (no determinado aún)

Cuando 7.1222–7.1225 cierren, el siguiente bloque cohesivo a determinar es la
**familia `mask`** (`mask-image` hoy sólo parsea `url()`; falta pintarla,
`mask-mode`, `mask-repeat`, `mask-position`, `mask-size`, `mask-composite`,
`-webkit-mask-*`). No la detallo todavía: requiere decidir cómo el compositor
aplica una máscara de luminancia/alpha (capa `Mix::*` o sampling), que es una
bifurcación de diseño — se especifica cuando lleguemos.
