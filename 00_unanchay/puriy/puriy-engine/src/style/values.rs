//! Tipos de valores CSS computados: `ComputedStyle` y todos los enums/structs
//! que la representan (longitudes, flex/grid, colores de gradiente, sombras,
//! transforms, animaciones, viewport, `Sides`/`Corners`), con sus `Default`.
//! Extraído de `style/mod.rs` (regla #1). Comparte los tipos del módulo `style`
//! y del crate vía `use super::*`.
use super::*;

/// Estilo computado por nodo. Defaults razonables — un nodo sin reglas
/// que matchen igual produce un box renderizable (texto negro sobre
/// transparente).
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub display: Display,
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub font_weight: u16,
    /// CSS `font-style`: normal vs italic/oblique. Heredable.
    pub font_style: FontStyle,
    /// CSS `font-family` como string crudo (acepta lista con fallbacks).
    /// `None` = sin override; usa la fuente default del runtime.
    pub font_family: Option<String>,
    pub margin: Sides<f32>,
    /// `margin-left/right: auto` — centrado horizontal en block flow. No
    /// hereda; default `false`. (El auto vertical no centra en block flow,
    /// se trata como 0 y por eso no se rastrea.)
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub padding: Sides<f32>,
    /// Ancho explícito. `Auto` = el default block-fills-parent.
    pub width: LengthVal,
    /// Alto explícito. `Auto` = lo dimensiona el contenido.
    pub height: LengthVal,
    /// Tope superior — útil para containers narrow ("max-width:800px").
    pub max_width: LengthVal,
    /// Alineación horizontal del texto dentro del box.
    pub text_align: TextAlign,
    /// Altura de línea como multiplicador del font-size. `None` =
    /// default razonable (1.4) en el caller.
    pub line_height: Option<f32>,
    /// Ancho del border en px por lado. `0` = ese lado sin border.
    /// El shorthand `border: 2px solid red` setea los 4 lados; las
    /// propiedades `border-top/right/bottom/left[-width]` los setean
    /// individualmente.
    pub border_widths: Sides<f32>,
    /// Color del border por lado. `None` = ese lado no se dibuja aunque
    /// `width > 0`. Mismo modelo que `border_widths`.
    pub border_colors: Sides<Option<Color>>,
    /// Radio del corner-radius en px por esquina (0 = esquina viva).
    /// El shorthand `border-radius: 8px` setea las 4; las propiedades
    /// `border-top-left-radius` etc. las setean individualmente.
    pub border_radii: Corners<f32>,
    /// `border-style` uniforme (`solid`/`dashed`/`dotted`/`double`). Se
    /// aplica a todos los lados que tengan border visible — el modelo
    /// per-lado del estilo no se distingue (sólo el ancho/color lo es).
    pub border_style: BorderLineStyle,
    /// `box-shadow`. Lista de sombras (cero o más) en orden de fuente:
    /// la PRIMERA capa pinta encima. `inset` se distingue por sombra.
    pub box_shadows: Vec<BoxShadow>,
    /// `z-index` aplicado al stacking order entre siblings positioned
    /// (absolute/fixed). Para nodos en flow normal (static), CSS spec
    /// dice que z-index no aplica y se ignora. `0` = default.
    pub z_index: i32,
    /// `content: ...` para pseudo-elementos `::before`/`::after`.
    /// `None` = no hay content (pseudo-element NO se materializa). Sólo
    /// se consulta en estilos computados para pseudo-elements; en el
    /// estilo del elemento real, content es no-op (matchea spec).
    ///
    /// Es un `Vec` porque `content:` admite concatenación de items:
    /// `content: "Sección " counter(sec) ": " attr(data-title)`.
    pub content: Option<Vec<ContentItem>>,
    /// `counter-reset: name [val] name2 [val2]...`. Cada par crea o
    /// resetea un contador en el scope del nodo. Se aplica antes que
    /// `counter-increment` al entrar al nodo en el DFS.
    pub counter_reset: Vec<(String, i32)>,
    /// `counter-increment: name [delta] name2 [delta2]...`. Cada par
    /// incrementa el contador correspondiente; si no existía, lo crea
    /// implícitamente (CSS spec: el reset implícito es 0).
    pub counter_increment: Vec<(String, i32)>,
    /// `text-decoration-line` reducido al subset que pintamos.
    /// `None` = sin decoración (default HTML, salvo `<a>`/`<u>`/`<s>`).
    pub text_decoration: TextDecorationLine,
    /// `text-decoration-color`. `None` = `currentColor` (sigue al `color`
    /// del texto, el default CSS). Se propaga junto a `text_decoration`.
    pub text_decoration_color: Option<Color>,
    /// `text-decoration-style` (`solid`/`double`/`dotted`/`dashed`/`wavy`).
    pub text_decoration_style: TextDecorationStyle,
    /// `text-decoration-thickness` en px. `None` = `auto`/`from-font` (el
    /// chrome deriva el grosor del font-size).
    pub text_decoration_thickness: Option<f32>,
    /// `text-underline-offset` en px. `None` = `auto` (posición default).
    pub text_underline_offset: Option<f32>,
    /// Marker que `<li>` pinta delante del contenido. Hereda (CSS spec).
    /// Default `Disc` (CSS default); UA stylesheet override en `<ol>` y
    /// `<ul>` por consistencia.
    pub list_style_type: ListStyleType,
    /// Solo relevante si `display` es `Flex`/`InlineFlex`. Default Row.
    pub flex_direction: FlexDirection,
    /// Distribución horizontal (eje principal) de los hijos flex.
    pub justify_content: JustifyContent,
    /// Alineación vertical (eje cruzado) de los hijos flex.
    pub align_items: AlignItems,
    /// Distribución de las líneas (flex multilínea) / pistas (grid) en el
    /// eje cruzado. `Normal` = default de taffy. No hereda.
    pub align_content: AlignContent,
    /// `justify-items` (grid): alineación por defecto de los items en el eje
    /// inline. `None` = default de taffy. No hereda.
    pub justify_items: Option<AlignItems>,
    /// `justify-self` (grid item): pisa el `justify-items` del contenedor
    /// para ese item. `Auto` = hereda del contenedor. No hereda.
    pub justify_self: AlignSelf,
    /// `nowrap` por default (CSS spec).
    pub flex_wrap: FlexWrap,
    /// Separación entre items en el eje principal (px). En CSS estándar,
    /// `column-gap` para row-direction, `row-gap` para column-direction.
    /// Acá los separamos para mapear directo a taffy.
    pub gap_row: f32,
    pub gap_column: f32,
    /// Cómo se cuentan padding/border dentro del width. Default
    /// `ContentBox` (CSS spec); los resets modernos lo fuerzan a
    /// BorderBox.
    pub box_sizing: BoxSizing,
    /// Ancho/alto mínimos.
    pub min_width: LengthVal,
    pub min_height: LengthVal,
    /// Alto máximo (max-width ya existe). `Auto` = sin tope.
    pub max_height: LengthVal,
    /// CSS `aspect-ratio` (relación ancho/alto preferida). `None` = `auto`
    /// (sin relación impuesta). El chrome lo pasa directo a taffy, que
    /// dimensiona el eje que quedó `auto` a partir del otro. No hereda.
    pub aspect_ratio: Option<f32>,
    /// Overflow del contenido. Default `Visible`.
    pub overflow: Overflow,
    /// Colapsado y wrap del texto.
    pub white_space: WhiteSpace,
    /// Transformación de texto pre-render.
    pub text_transform: TextTransform,
    /// 0..1. Multiplica alpha del background/border al pintar.
    /// `text` queda sin tocar (el spec exige multiplicar todo el
    /// subárbol, pero acá pragmaticamente sólo afecta el propio nodo —
    /// matchea el uso real donde opacity se aplica a overlays).
    pub opacity: f32,
    /// Item-side de flex.
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    /// `Auto` = el width del item; `Px/Pct` = base explícita.
    pub flex_basis: LengthVal,
    /// Outline (fuera del border, sin afectar layout).
    pub outline: Outline,
    /// `background-image: linear-gradient(...)`. Cuando es Some, el
    /// chrome lo pinta detrás (o encima del background sólido).
    pub background_gradient: Option<LinearGradient>,
    /// `background-image: url(...)` — URL sin resolver (puede ser
    /// relativa). El engine la resuelve y descarga en `build_node`; el
    /// chrome consume el resultado vía `BoxNode.background_image`.
    pub background_image_url: Option<String>,
    /// `background-size`. Default `Auto` (tamaño natural de la imagen).
    pub background_size: BackgroundSize,
    /// `background-position`. Default `0% 0%` (esquina superior-izquierda).
    pub background_position: BackgroundPosition,
    /// `background-repeat`. Default `Repeat` (tile en ambos ejes).
    pub background_repeat: BackgroundRepeat,
    /// Capas de background ADICIONALES (debajo de la capa 0, que vive en los
    /// campos `background_*` de arriba). Son las capas 2..N de una lista
    /// `background: a, b, c`. Default vacío. La shorthand siempre las setea
    /// (posiblemente vacías) para resetear las de una regla previa.
    pub background_extra_layers: Vec<BackgroundLayer>,
    /// `background-origin`. Default `PaddingBox`. Aplica a la capa 0 (las
    /// capas extra usan el default).
    pub background_origin: BackgroundOrigin,
    /// `background-clip`. Default `BorderBox`. Aplica a imágenes y gradientes
    /// (el color sólido sigue recortado al border-box, ver chrome).
    pub background_clip: BackgroundClip,
    /// CSS `position`. Default Static.
    pub position: Position,
    /// Insets (top/right/bottom/left). `Auto` por default.
    pub inset_top: LengthVal,
    pub inset_right: LengthVal,
    pub inset_bottom: LengthVal,
    pub inset_left: LengthVal,
    /// `vertical-align` para inline / inline-block / cells.
    pub vertical_align: VerticalAlign,
    /// `visibility: hidden` → ocupa espacio pero no se pinta.
    pub visibility: Visibility,
    /// `pointer-events: none` → ignora clics/hover.
    pub pointer_events: PointerEvents,
    /// `object-fit` de un `<img>`. `None` = no especificado (el chrome
    /// mantiene su encaje por defecto, contain responsivo). Fase 7.230.
    pub object_fit: Option<ObjectFit>,
    /// `object-position` de un `<img>`. `None` = default (centro 50% 50%).
    /// Fase 7.231.
    pub object_position: Option<BackgroundPosition>,
    /// Sangrado de primera línea de un bloque (en px).
    pub text_indent: f32,
    /// Espacio extra entre palabras (en px). Heredable.
    pub word_spacing: f32,
    /// Espacio extra entre letras (en px). Heredable. Espejo de
    /// `word_spacing`: hoy se parsea/hereda/almacena en el `BoxNode` pero el
    /// chrome todavía no lo pinta (la capa de texto compartida no expone
    /// tracking aún) — mismo estado que `word-spacing`.
    pub letter_spacing: f32,
    /// `caret-color` (Fase 7.238). `None` = `auto` (= currentColor). Heredable.
    /// Sólo parseado/propagado — el caret real lo pinta el widget de
    /// `<input>`/`<textarea>` aguas abajo, que aún no consume este campo.
    pub caret_color: Option<Color>,
    /// `accent-color` (Fase 7.239). `None` = `auto` (= color del tema UA).
    /// Heredable. Sólo parseado/propagado por ahora.
    pub accent_color: Option<Color>,
    /// `cursor` (Fase 7.240). Default `Auto`. Heredable. El chrome
    /// todavía no setea el cursor del mouse — sólo se almacena.
    pub cursor: Cursor,
    /// `text-overflow` (Fase 7.241). Default `Clip`. NO heredable. Sólo
    /// tiene efecto visual cuando el text node está en una caja con
    /// `overflow: hidden` + `white-space: nowrap` — el chrome aún no
    /// trunca con `…`, así que este campo sólo se propaga.
    pub text_overflow: TextOverflow,
    /// `scroll-behavior` (Fase 7.242). Default `Auto`. Heredable.
    /// Plumb: el scroll programático del chrome todavía es instantáneo.
    pub scroll_behavior: ScrollBehavior,
    /// `tab-size` (Fase 7.243) — ancho del carácter U+0009 dentro de
    /// `white-space: pre`. Default 8 chars. Heredable. Plumb: el text
    /// shaper aún no consume este campo (los `\t` se renderizan según
    /// el comportamiento default de parley).
    pub tab_size: TabSize,
    /// `user-select` (Fase 7.244). Heredable (CSS UI 4). Controla si el
    /// usuario puede seleccionar el texto del elemento. Sólo parseado/
    /// propagado — el chrome todavía no consulta este campo al construir
    /// las selecciones del text-input shared.
    pub user_select: UserSelect,
    /// `overflow-wrap` (Fase 7.245). Heredable. Controla si se permite
    /// quebrar palabras largas. Alias legacy `word-wrap`. Sólo plumb.
    pub overflow_wrap: OverflowWrap,
    /// `word-break` (Fase 7.246). Heredable. Controla cómo se quiebran
    /// palabras en el wrap. Subset (`break-word` se aplana a `Normal`
    /// por compat antigua de IE). Plumb.
    pub word_break: WordBreak,
    /// `hyphens` (Fase 7.247). Heredable. `auto` requeriría diccionarios
    /// de hyphenation por idioma — fuera de scope. Plumb.
    pub hyphens: Hyphens,
    /// `resize` (Fase 7.248). NO heredable. Sólo aplica a elementos con
    /// `overflow` distinto a `visible` (CSS UI 4); el chrome aún no pinta
    /// el grip ni el handle de drag. Plumb.
    pub resize: Resize,
    /// Sombras del texto. Vacío = ninguna.
    pub text_shadows: Vec<TextShadow>,
    /// Cadena de transformaciones (translate/scale/rotate) aplicadas
    /// en orden. Vacío = identidad.
    pub transforms: Vec<Transform>,
    /// Para `display: grid` — pistas de columnas y filas.
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    /// `animation: <name> <duration> ...` colapsado en una binding.
    /// `None` = sin animación. **Sólo parseado**: no hay runtime de tween
    /// todavía, así que esto no anima nada (ver Fase B4). El runtime
    /// futuro cruzaría `name` contra [`StyleEngine::keyframes`].
    pub animation: Option<AnimationBinding>,
    /// `transition: <prop> <duration> ...`. Lista separada por coma →
    /// varios bindings. Vacío = sin transición. **Sólo parseado** — sin
    /// runtime de tween no dispara nada (ver Fase B4).
    pub transitions: Vec<TransitionBinding>,
    /// Targets de `currentColor` pendientes de resolver. Transitorio: lo
    /// llena `Decl::apply` y lo vacía `compute_internal` resolviéndolo
    /// contra el `color` final del elemento (CSS: `currentColor` = used
    /// value de `color`). NUNCA se hereda ni viaja al `BoxNode` (se limpia
    /// antes de devolver el estilo). Ver Fase 7.210.
    pub current_color: Vec<ColorTarget>,
    /// `font-size` relativo pendiente (multiplicador `em`/`%`/`larger`/
    /// `smaller`) a resolver contra el font-size HEREDADO. Transitorio:
    /// `Decl::apply` lo setea, `compute_with_parent` lo resuelve al cierre
    /// y lo limpia. Un `font-size` absoluto posterior lo borra (cascada).
    /// Ver Fase 7.223.
    pub font_size_rel: Option<f32>,
}

/// Propiedad-destino de una declaración `currentColor`. Se resuelve al
/// `color` computado del elemento en una pasada final de la cascada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTarget {
    Background,
    BorderAll,
    BorderSide(BorderEdge),
    Outline,
}

/// Estilo del marker de `<li>`. Reducido al subset que el chrome puede
/// pintar como texto plano (sin imágenes ni cuadritos pintados a mano).
/// `Decimal`/`*Alpha`/`*Roman` requieren conocer la posición del `<li>`
/// entre sus hermanos — `boxes::build_node` la calcula y la sustituye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

/// Línea decorativa que el chrome dibuja sobre/atravesando/debajo del
/// texto del nodo. CSS spec dice que la propiedad NO se hereda — los
/// descendientes inline heredan la decoración por propagación visual,
/// no computacional. Acá la tratamos como heredable porque dibujamos
/// por leaf de texto: sin propagar, `<a>foo <b>bar</b></a>` rendearía
/// `foo` subrayado y `bar` sin subrayar. Override explícito a `None`
/// la suprime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecorationLine {
    None,
    Underline,
    LineThrough,
    Overline,
}

/// CSS `text-decoration-style`. El subset que el chrome sabe pintar:
/// `solid` (línea continua), `double` (dos líneas), `dotted`/`dashed`
/// (patrón de stroke) y `wavy` (aproximada como zig-zag). Default `Solid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// `cursor` CSS — subset reconocido. Otros valores (`url(...)` y
/// `<x> <y>` fallback, `none`, `progress`, `cell`, `vertical-text`,
/// `alias`, `copy`, `no-drop`, todas las flechas direccionales y los
/// resize compuestos) caen a `Auto` (= no cambia el cursor por
/// elemento). Heredable. Fase 7.240.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Auto,
    Default,
    Pointer,
    Text,
    Wait,
    Help,
    Crosshair,
    Move,
    NotAllowed,
    Grab,
    Grabbing,
    ZoomIn,
    ZoomOut,
    EResize,
    NResize,
    SResize,
    WResize,
    NsResize,
    EwResize,
    NeswResize,
    NwseResize,
    RowResize,
    ColResize,
}

/// `text-overflow` — qué hacer con el texto recortado por un padre
/// con `overflow: hidden` + `white-space: nowrap`. Sólo `Clip` y
/// `Ellipsis` por ahora (`fade` y string custom de CSS3 aparte).
/// Fase 7.241.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

/// `scroll-behavior` — animación del scroll programático
/// (`element.scrollTo`, jump por `#anchor`...). Fase 7.242.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollBehavior {
    #[default]
    Auto,
    Smooth,
}

/// `tab-size`: ancho del U+0009 expresado en caracteres o longitud px.
/// CSS permite ambos formatos (`tab-size: 4` o `tab-size: 32px`).
/// Default = 8 caracteres. Fase 7.243.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabSize {
    Chars(u16),
    Px(f32),
}

/// `user-select`: controla si el usuario puede seleccionar texto del
/// elemento. `Auto` = default del UA (texto seleccionable en bloque
/// de texto; no en widgets nativos). Heredable. Fase 7.244.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserSelect {
    #[default]
    Auto,
    None,
    Text,
    All,
    Contain,
}

/// `overflow-wrap` (alias legacy `word-wrap`): permite que el text shaper
/// quiebre dentro de una palabra cuando la línea no le alcanza. Default
/// `Normal` (sólo quiebra en oportunidades válidas del idioma).
/// Heredable. Fase 7.245.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

/// `word-break`: política de quiebre de palabra. `BreakAll` (CJK) y
/// `KeepAll` (sólo en separadores reales). Heredable. Fase 7.246.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordBreak {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
}

/// `hyphens`: control de hyphenation. `Auto` requeriría diccionarios por
/// idioma; quedó como acepto-pero-no-aplico. Heredable. Fase 7.247.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hyphens {
    #[default]
    Manual,
    None,
    Auto,
}

/// `resize`: el usuario puede arrastrar el borde del elemento para
/// redimensionarlo (típicamente `<textarea>`). Default `None`.
/// NO heredable. Fase 7.248.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resize {
    #[default]
    None,
    Both,
    Horizontal,
    Vertical,
    Block,
    Inline,
}

/// CSS `border-style` reducido al subset que el chrome pinta: `solid`
/// (línea continua), `dashed`/`dotted` (patrón de stroke) y `double` (dos
/// líneas). `none`/`hidden` se modelan aparte (color del lado = `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderLineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    Double,
    /// 3D "carved" — top+left dark, bottom+right light.
    Groove,
    /// 3D opuesto a `Groove` — top+left light, bottom+right dark.
    Ridge,
    /// 3D "hundido" — render como `Groove` (suficiente aprox sin
    /// gradiente real por dentro del lado).
    Inset,
    /// 3D opuesto a `Inset` — render como `Ridge`.
    Outset,
}

/// CSS `font-style`. Heredable. `Oblique` lo tratamos igual que
/// `Italic` (parley sintetiza si la fuente no tiene oblique nativo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// Sombra rectangular detrás del box. `blur_px` y `spread_px` se
/// combinan en una expansión efectiva del rect — gaussian blur real
/// queda para cuando el render-pipeline soporte multi-pass. `inset`
/// invierte el lado: en vez de pintar afuera, recorta una sombra
/// dentro del box (aproximada con un fill traslúcido del color sobre
/// el área interior).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub spread_px: f32,
    pub color: Color,
    pub inset: bool,
}

/// Valor longitud de CSS reducido al subset que soportamos: `auto`,
/// `Npx`, `N%`. `em`/`rem` se resuelven a px en parse time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthVal {
    Auto,
    Px(f32),
    Pct(f32),
}

/// 4 valores por lado (top/right/bottom/left). Lo usan `margin` y
/// `padding` para no perder información del shorthand CSS — un
/// `padding: 10px 20px` se queda con `top/bottom=10, right/left=20`
/// en vez de colapsarse a un único `f32`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sides<T: Copy> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

/// Eje principal de un contenedor `display: flex`. Default `Row`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// Distribución del espacio libre a lo largo del eje principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContent {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// Alineación de los items en el eje cruzado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItems {
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// Distribución de las *líneas* en el eje cruzado (flex multilínea) o de
/// las pistas en grid. CSS `align-content`. `Normal` (default) deja que
/// taffy use su comportamiento por defecto (stretch para flex). No hereda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignContent {
    Normal,
    Start,
    Center,
    End,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// ¿Hijos en una sola línea o wrap a múltiples?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

/// Modelo de caja CSS: cómo se cuentan `padding` y `border` dentro del
/// `width`/`height`. CSS default `ContentBox` (width = sólo contenido);
/// la mayoría de los resets modernos fuerzan `BorderBox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

/// `align-items` por item — pisa el del contenedor para ese hijo.
/// `Auto` significa heredar del padre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignSelf {
    Auto,
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// Comportamiento de overflow del contenido del box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
}

/// `white-space` controla colapsado de espacios y wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    /// Default: runs internos colapsan a un solo espacio, wrap libre.
    Normal,
    /// Sin wrap; runs internos colapsan.
    NoWrap,
    /// Preserva todo (espacios, tabs, newlines).
    Pre,
    /// Preserva espacios/newlines; wrap permitido en cualquier espacio.
    PreWrap,
    /// Colapsa runs internos a uno, pero preserva newlines.
    PreLine,
}

/// `text-transform` aplica una transformación al texto antes de
/// pintarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// `outline` se pinta fuera del border (sin ocupar layout). Útil para
/// focus rings y debug. `style_active=false` (CSS `none`/`hidden`) lo
/// desactiva aunque haya width/color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    pub width: f32,
    pub color: Option<Color>,
    pub style_active: bool,
    /// Patrón visual del outline (reusa el enum de border). Default `Solid`.
    pub style: BorderLineStyle,
    /// Distancia del border al outline. Default 0.
    pub offset: f32,
}

impl Default for Outline {
    fn default() -> Self {
        Self {
            width: 0.0,
            color: None,
            style_active: true,
            style: BorderLineStyle::Solid,
            offset: 0.0,
        }
    }
}

/// Un stop de gradiente. `pos` es la posición a lo largo del eje:
/// `Pct(n)` = fracción del eje (`n` en 0..100), `Px(n)` = distancia absoluta
/// (px en lineal/radial, grados en cónico). Si `None`, se distribuye
/// automáticamente entre los stops fijos adyacentes (interpolación CSS).
/// Fase 7.228 (antes era `Option<f32>` ya normalizado a 0..1, lo que perdía
/// los px reales que los `repeating-*` necesitan).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub pos: Option<LengthVal>,
}

/// Tamaño de un `radial-gradient` — qué borde/esquina toca el círculo en su
/// stop final. Default `FarthestCorner`. Fase 7.226.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialSize {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
}

/// Geometría de un `radial-gradient`. El render lo trata como círculo (peniko
/// `Radial` es circular): forma `circle`/`ellipse` no se distingue todavía.
/// `cx`/`cy` = centro (`at <position>`, default 50% 50%). Fase 7.226.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialSpec {
    pub size: RadialSize,
    pub cx: LengthVal,
    pub cy: LengthVal,
}

impl Default for RadialSpec {
    fn default() -> Self {
        Self {
            size: RadialSize::FarthestCorner,
            cx: LengthVal::Pct(50.0),
            cy: LengthVal::Pct(50.0),
        }
    }
}

/// Geometría de un gradiente CSS. Fase 7.227 (antes eran campos sueltos
/// `angle_deg` + `radial: Option`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientGeometry {
    /// `linear-gradient` — ángulo CSS en grados (0 = up, 90 = right, 180 =
    /// down, 270 = left).
    Linear { angle_deg: f32 },
    /// `radial-gradient` — forma/tamaño/centro.
    Radial(RadialSpec),
    /// `conic-gradient` — ángulo inicial `from <angle>` (grados, 0 = up) y
    /// centro (`at <position>`, default 50% 50%).
    Conic { from_deg: f32, cx: LengthVal, cy: LengthVal },
}

/// `background-image: {linear,radial,conic}-gradient(...)`. La `geometry`
/// discrimina el tipo; los `stops` (2+) son comunes a los tres. El nombre
/// histórico `LinearGradient` se conserva (deuda) para no propagar el rename
/// a ~9 archivos.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub geometry: GradientGeometry,
    pub stops: Vec<GradientStop>,
    /// `repeating-{linear,radial,conic}-gradient`: el patrón de stops se
    /// tilea a lo largo del eje en vez de extender el color de los extremos
    /// (peniko `Extend::Repeat`). Fase 7.228.
    pub repeating: bool,
}

impl LinearGradient {
    /// Ángulo del gradiente lineal en grados (0 si no es lineal).
    pub fn angle_deg(&self) -> f32 {
        match self.geometry {
            GradientGeometry::Linear { angle_deg } => angle_deg,
            _ => 0.0,
        }
    }

    /// La geometría radial si el gradiente es `radial-gradient`.
    pub fn radial(&self) -> Option<RadialSpec> {
        match self.geometry {
            GradientGeometry::Radial(spec) => Some(spec),
            _ => None,
        }
    }
}

/// CSS `position`. `Static` = el default (no position; los insets
/// se ignoran). `Fixed`/`Sticky` los fakeamos como Absolute/Relative en
/// el chrome — taffy 0.9 sólo expone esos dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

/// CSS `vertical-align` para inline / inline-block. Mapea a alignment
/// del item en el contexto del padre. `Super`/`Sub` los aproximamos
/// como Top/Bottom respectivamente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    Super,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvents {
    Auto,
    None,
}

/// `object-fit` de un reemplazado (`<img>`): cómo encaja la imagen en la
/// caja cuando el tamaño de la caja (CSS `width`/`height`) difiere del
/// intrínseco. `Fill` estira a la caja (default CSS), `Contain`/`Cover`
/// preservan aspecto (cabe / cubre), `None` usa el tamaño natural,
/// `ScaleDown` = el menor entre `None` y `Contain`. Fase 7.230.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

/// `background-size`. `Auto` = tamaño natural de la imagen; `Cover`/`Contain`
/// escalan preservando aspecto (la más grande / la más chica que cubre / cabe);
/// `Explicit` da ancho/alto, donde cada eje puede ser `Auto` (= derivado del
/// otro por aspecto). El chrome resuelve % y aspecto contra el rect del box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSize {
    Auto,
    Cover,
    Contain,
    Explicit { x: LengthVal, y: LengthVal },
}

/// `background-position`. `x`/`y` son el offset del origen del primer tile.
/// `Pct(p)` tiene semántica de alineación CSS (el punto `p%` de la imagen se
/// alinea con el `p%` del box) — la resuelve el chrome; `Px(n)` es un offset
/// directo desde la esquina superior-izquierda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundPosition {
    pub x: LengthVal,
    pub y: LengthVal,
}

/// `background-repeat`. `space`/`round` se aproximan a `Repeat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeat {
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

/// `background-origin`: el área de posicionamiento del background — contra qué
/// caja se anclan `background-position`, los `%` y `cover`/`contain`. Default
/// CSS `PaddingBox`. El chrome insetea el rect del border-box según el valor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundOrigin {
    BorderBox,
    PaddingBox,
    ContentBox,
}

/// `background-clip`: hasta qué caja se recorta el pintado del background.
/// Default CSS `BorderBox`. `Text` recorta el background a las glifos del
/// texto (Fase 7.208): el chrome lo propaga a las hojas de texto y rellena
/// los glifos con el gradiente en vez de pintar el fondo como rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundClip {
    BorderBox,
    PaddingBox,
    ContentBox,
    Text,
}

/// La imagen de una capa de background: o un gradiente, o una URL sin
/// resolver (el engine la descarga en `build_node`). Una capa siempre tiene
/// imagen — sin imagen no hay nada que pintar.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundImage {
    Url(String),
    Gradient(LinearGradient),
}

/// Una capa de background ADICIONAL (más allá de la capa 0, que vive en los
/// campos `background_*` sueltos de `ComputedStyle`). CSS pinta la PRIMERA
/// capa de la lista arriba; estas capas extra son las 2..N de una lista
/// `background: a, b, c` separada por coma y van por DEBAJO de la capa 0.
#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundLayer {
    pub image: BackgroundImage,
    pub size: BackgroundSize,
    pub position: BackgroundPosition,
    pub repeat: BackgroundRepeat,
}

/// Una sombra de texto. CSS permite varias separadas por coma.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub color: Color,
}

/// Una transformación CSS individual. Las cadenas `transform: rotate(45deg)
/// scale(2) translate(10px, 20px)` se aplican en orden de izquierda a
/// derecha como matrices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transform {
    /// Pixeles X/Y.
    Translate(f32, f32),
    /// Factores X/Y (uno solo si CSS da un valor).
    Scale(f32, f32),
    /// Grados (sentido horario en pantalla = sentido CSS).
    Rotate(f32),
    /// Sesgo X/Y en grados (`skew`/`skewX`/`skewY`).
    Skew(f32, f32),
    /// `matrix(a, b, c, d, e, f)` — afín 2D completa. `a..d` son unitless;
    /// `e`/`f` son la traslación en px (se escalan por zoom en el render).
    Matrix(f32, f32, f32, f32, f32, f32),
}

/// Tamaño de track para `display: grid`. `Fr(N)` = fracción del espacio
/// remanente (CSS unit `fr`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackSize {
    Auto,
    Px(f32),
    Pct(f32),
    Fr(f32),
}

/// Función de easing de una `transition`/`animation`. El runtime de
/// tween (Fase B4+, todavía NO implementado) la usaría para mapear el
/// progreso lineal `t∈[0,1]` al progreso efectivo. Por ahora sólo se
/// parsea y se guarda en `ComputedStyle` — no anima nada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EasingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// `step-start` ≡ `steps(1, start)`.
    StepStart,
    /// `step-end` ≡ `steps(1, end)`.
    StepEnd,
    /// `cubic-bezier(x1, y1, x2, y2)` — los dos puntos de control.
    CubicBezier(f32, f32, f32, f32),
    /// `steps(n, jump-term)`. `jump_start=true` ⇒ `steps(n, start)`
    /// (salto al inicio del intervalo); `false` ⇒ `steps(n, end)`.
    Steps(u32, bool),
}

impl Default for EasingFunction {
    fn default() -> Self {
        // CSS spec: el default de `transition-timing-function` y
        // `animation-timing-function` es `ease`.
        EasingFunction::Ease
    }
}

/// Número de iteraciones de una animación (`animation-iteration-count`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationIterations {
    Count(f32),
    Infinite,
}

/// `animation-direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// `animation-fill-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

/// `animation-play-state`. `Paused` congela el progreso de la animación en
/// el frame actual (lo consume el runtime de tween en `anim.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPlayState {
    Running,
    Paused,
}

/// `animation: <name> <duration> <timing> <delay> <iteration> <direction>
/// <fill> <play-state>` colapsado en una sola binding. Si el shorthand
/// lista varias animaciones separadas por coma nos quedamos con la primera.
/// El runtime de tween vive en `anim.rs` (rescatado del frente engine). Los
/// tokens se clasifican por forma, no por posición, así que el orden
/// laxo del wild (`animation: spin 2s linear infinite`) se tolera.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationBinding {
    pub name: String,
    /// Duración en segundos.
    pub duration_s: f32,
    pub timing: EasingFunction,
    /// Retardo en segundos.
    pub delay_s: f32,
    pub iterations: AnimationIterations,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

/// `transition: <property> <duration> <timing> <delay>`. Una lista
/// separada por coma produce varios bindings. `property` queda como
/// string cruda (`opacity`, `transform`, `all`...) — el matching contra
/// las propiedades animables real lo hará el runtime de tween (Fase B4+).
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionBinding {
    pub property: String,
    pub duration_s: f32,
    pub timing: EasingFunction,
    pub delay_s: f32,
}

/// Un paso de un `@keyframes`: el offset normalizado en el timeline
/// (`from` = 0.0, `to` = 1.0, `50%` = 0.5) + las declaraciones crudas
/// (`prop`, `value`) que aplican en ese punto. Guardamos los pares SIN
/// parsear porque el runtime de animación (Fase B4+) todavía no existe;
/// cuando llegue, los re-parseará con la maquinaria de `Decl` para
/// derivar el overlay interpolado entre pasos.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeStep {
    pub offset: f32,
    pub declarations: Vec<(String, String)>,
}

/// Definición de un `@keyframes name { ... }`. Los pasos quedan ordenados
/// por `offset` ascendente.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Keyframes {
    pub steps: Vec<KeyframeStep>,
}

/// Viewport asumido por el parser para resolver unidades `vw`/`vh`/
/// `vmin`/`vmax` y para evaluar `@media` queries. Por ahora es
/// constante (1280×800 — desktop típico). Cuando puriy soporte resize
/// dinámico del viewport, pasará a ser un parámetro de `StyleEngine`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    /// Factor de escala (DPI lógico) — `window.devicePixelRatio`. 1.0 normal,
    /// 2.0 HiDPI/Retina. Lo consume `evaluate_media_query` para las features
    /// `min/max-resolution` (`Ndppx` / `Ndpi`). Default 1.0.
    pub dpr: f32,
}

pub const DEFAULT_VIEWPORT: Viewport = Viewport { width: 1280.0, height: 800.0, dpr: 1.0 };

thread_local! {
    /// Viewport activo para resolver unidades `vw`/`vh`/`vmin`/`vmax` durante
    /// el parseo de un documento. `Engine::load_html` lo instala con el
    /// viewport real (vía [`ViewportScope`]) antes de parsear hojas y construir
    /// el box tree — incluido el `style="…"` inline que se parsea en
    /// `boxes::build`. Fuera de ese scope (tests que llaman parsers sueltos)
    /// cae a [`DEFAULT_VIEWPORT`], preservando el comportamiento previo.
    static RESOLVE_VIEWPORT: std::cell::Cell<Viewport> = const { std::cell::Cell::new(DEFAULT_VIEWPORT) };
}

/// Guard RAII que instala `vp` como viewport de resolución de longitudes
/// mientras viva, y restaura el anterior al dropear. Reentrante (anida bien).
/// Lo usa `Engine::load_html` para que `50vw`/`100vh` resuelvan contra el
/// tamaño real de la ventana en vez del viewport por defecto.
pub struct ViewportScope(Viewport);

impl ViewportScope {
    pub fn new(vp: Viewport) -> Self {
        let prev = RESOLVE_VIEWPORT.with(|c| c.replace(vp));
        ViewportScope(prev)
    }
}

impl Drop for ViewportScope {
    fn drop(&mut self) {
        RESOLVE_VIEWPORT.with(|c| c.set(self.0));
    }
}

/// Viewport contra el que se resuelven las unidades viewport ahora mismo.
/// `DEFAULT_VIEWPORT` salvo dentro de un [`ViewportScope`] activo.
pub(crate) fn resolve_viewport() -> Viewport {
    RESOLVE_VIEWPORT.with(|c| c.get())
}

impl<T: Copy> Sides<T> {
    pub const fn all(v: T) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }
}

impl Default for Sides<f32> {
    fn default() -> Self {
        Self::all(0.0)
    }
}

/// Valores por esquina (top-left, top-right, bottom-right, bottom-left)
/// — usado por `border-radius` per-corner. El shorthand `border-radius`
/// setea las 4; las longhand `border-{top|bottom}-{left|right}-radius`
/// las setean individualmente.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners<T: Copy> {
    pub top_left: T,
    pub top_right: T,
    pub bottom_right: T,
    pub bottom_left: T,
}

impl<T: Copy> Corners<T> {
    pub const fn all(v: T) -> Self {
        Self { top_left: v, top_right: v, bottom_right: v, bottom_left: v }
    }
}

impl Default for Corners<f32> {
    fn default() -> Self {
        Self::all(0.0)
    }
}

/// Lado de un border (`border-top-width: 2px` → `Top`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderEdge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Esquina de un border-radius (`border-top-left-radius` → `TopLeft`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

pub(crate) fn set_side<T: Copy>(sides: &mut Sides<T>, edge: BorderEdge, v: T) {
    match edge {
        BorderEdge::Top => sides.top = v,
        BorderEdge::Right => sides.right = v,
        BorderEdge::Bottom => sides.bottom = v,
        BorderEdge::Left => sides.left = v,
    }
}

pub(crate) fn set_side_f32(sides: &mut Sides<f32>, edge: BorderEdge, v: f32) {
    set_side(sides, edge, v)
}

pub(crate) fn set_corner(corners: &mut Corners<f32>, corner: BorderCorner, v: f32) {
    match corner {
        BorderCorner::TopLeft => corners.top_left = v,
        BorderCorner::TopRight => corners.top_right = v,
        BorderCorner::BottomRight => corners.bottom_right = v,
        BorderCorner::BottomLeft => corners.bottom_left = v,
    }
}

/// Alineación horizontal del contenido inline dentro de un bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            color: Color::BLACK,
            background: None,
            font_size: 16.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            font_family: None,
            margin: Sides::all(0.0),
            margin_left_auto: false,
            margin_right_auto: false,
            padding: Sides::all(0.0),
            width: LengthVal::Auto,
            height: LengthVal::Auto,
            max_width: LengthVal::Auto,
            text_align: TextAlign::Left,
            line_height: None,
            border_widths: Sides::all(0.0),
            border_colors: Sides::all(None),
            border_radii: Corners::all(0.0),
            border_style: BorderLineStyle::Solid,
            box_shadows: Vec::new(),
            z_index: 0,
            content: None,
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            text_decoration: TextDecorationLine::None,
            text_decoration_color: None,
            text_decoration_style: TextDecorationStyle::Solid,
            text_decoration_thickness: None,
            text_underline_offset: None,
            list_style_type: ListStyleType::Disc,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Stretch,
            align_content: AlignContent::Normal,
            justify_items: None,
            justify_self: AlignSelf::Auto,
            flex_wrap: FlexWrap::NoWrap,
            gap_row: 0.0,
            gap_column: 0.0,
            box_sizing: BoxSizing::ContentBox,
            min_width: LengthVal::Auto,
            min_height: LengthVal::Auto,
            max_height: LengthVal::Auto,
            aspect_ratio: None,
            overflow: Overflow::Visible,
            white_space: WhiteSpace::Normal,
            text_transform: TextTransform::None,
            opacity: 1.0,
            align_self: AlignSelf::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: LengthVal::Auto,
            outline: Outline::default(),
            background_gradient: None,
            background_image_url: None,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition {
                x: LengthVal::Pct(0.0),
                y: LengthVal::Pct(0.0),
            },
            background_repeat: BackgroundRepeat::Repeat,
            background_extra_layers: Vec::new(),
            background_origin: BackgroundOrigin::PaddingBox,
            background_clip: BackgroundClip::BorderBox,
            position: Position::Static,
            inset_top: LengthVal::Auto,
            inset_right: LengthVal::Auto,
            inset_bottom: LengthVal::Auto,
            inset_left: LengthVal::Auto,
            vertical_align: VerticalAlign::Baseline,
            visibility: Visibility::Visible,
            pointer_events: PointerEvents::Auto,
            object_fit: None,
            object_position: None,
            caret_color: None,
            accent_color: None,
            cursor: Cursor::Auto,
            text_overflow: TextOverflow::Clip,
            scroll_behavior: ScrollBehavior::Auto,
            tab_size: TabSize::Chars(8),
            user_select: UserSelect::Auto,
            overflow_wrap: OverflowWrap::Normal,
            word_break: WordBreak::Normal,
            hyphens: Hyphens::Manual,
            resize: Resize::None,
            text_indent: 0.0,
            word_spacing: 0.0,
            letter_spacing: 0.0,
            text_shadows: Vec::new(),
            transforms: Vec::new(),
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            animation: None,
            transitions: Vec::new(),
            current_color: Vec::new(),
            font_size_rel: None,
        }
    }
}
