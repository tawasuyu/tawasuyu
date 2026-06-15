//! Estructura `ComputedStyle` (el estilo computado por nodo).
//! Tipos de valores CSS extraídos de `values.rs` (regla #1). Sin cambios de lógica.
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
    /// hereda; default `false`.
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    /// `margin-top/bottom: auto` — centrado/empuje vertical. Sólo tiene efecto
    /// cuando el padre es flex/grid (block flow → 0); la resolución contra el
    /// contexto se hace al construir el box. No hereda; default `false`.
    pub margin_top_auto: bool,
    pub margin_bottom_auto: bool,
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
    /// `writing-mode` (Fase 7.249). Heredable. Sólo `HorizontalTb` se
    /// renderiza con layout real — los vertical-* y sideways-* quedan
    /// parseados pero el shaper no rota glifos todavía.
    pub writing_mode: WritingMode,
    /// `direction` (Fase 7.250). Heredable. Plumb: el shaper no reordena
    /// bidi todavía; sólo afecta cómo se interpreta `text-align: start`.
    pub direction: Direction,
    /// `unicode-bidi` (Fase 7.251). NO heredable. Plumb: sin runtime BiDi.
    pub unicode_bidi: UnicodeBidi,
    /// `font-stretch` (Fase 7.252). Heredable. Sin axis variable wired al
    /// shaper — se almacena como porcentaje (50%-200%) normalizado a 1.0.
    pub font_stretch: f32,
    /// `image-rendering` (Fase 7.253). Heredable. Plumb: el chrome no
    /// elige el sampler GPU a partir de este flag aún.
    pub image_rendering: ImageRendering,
    /// `mix-blend-mode` (Fase 7.254). Default `Normal`. NO heredable.
    /// Plumb: vello no expone el blend mode todavía como composite del nodo.
    pub mix_blend_mode: BlendMode,
    /// `background-blend-mode` (Fase 7.255). Lista paralela a las capas
    /// de background (de la 0 hacia arriba). Vacío = todas `Normal`. NO
    /// heredable. Plumb: pendiente integrar al pintor de capas.
    pub background_blend_mode: Vec<BlendMode>,
    /// `isolation` (Fase 7.256). NO heredable. `Isolate` crea un nuevo
    /// stacking context que aísla el subárbol del blending del padre.
    pub isolation: Isolation,
    /// `will-change` (Fase 7.257). Lista de hints. NO heredable. Plumb:
    /// el chrome aún no promueve a capa GPU separada por este hint.
    pub will_change: Vec<WillChangeHint>,
    /// `appearance` (Fase 7.258). NO heredable. CSS UI 4. El chrome aún
    /// no remueve el render UA al ver `appearance: none`.
    pub appearance: Appearance,
    /// `font-kerning` (Fase 7.259). Heredable. Plumb: el shaper no
    /// togglea el kerning por flag aún.
    pub font_kerning: FontKerning,
    /// `font-feature-settings` (Fase 7.260). Lista parseada. Vacío =
    /// `normal`. Heredable.
    pub font_feature_settings: Vec<FontFeatureSetting>,
    /// `font-variation-settings` (Fase 7.261). Lista parseada. Vacío =
    /// `normal`. Heredable.
    pub font_variation_settings: Vec<FontVariationSetting>,
    /// `font-language-override` (Fase 7.262). `None` = `normal`. El tag
    /// se guarda tal cual lo escribió el autor (uppercase recomendado
    /// por OpenType). Heredable.
    pub font_language_override: Option<String>,
    /// `text-rendering` (Fase 7.263). Heredable. Plumb: el shaper no
    /// elige entre legibility/speed/precision aún.
    pub text_rendering: TextRendering,
    /// `filter` (Fase 7.264). Cadena de funciones de filtro aplicadas
    /// al nodo. Vacío = `none`. NO heredable. Plumb: vello no expone
    /// los filter ops como composite todavía.
    pub filter: Vec<FilterFn>,
    /// `backdrop-filter` (Fase 7.265). Mismo modelo que `filter`,
    /// aplicado al fondo detrás del nodo. NO heredable. Plumb.
    pub backdrop_filter: Vec<FilterFn>,
    /// `text-orientation` (Fase 7.266). Heredable. Sólo aplica si
    /// `writing-mode` es vertical-*; el chrome sólo soporta horizontal
    /// todavía, así que es plumb.
    pub text_orientation: TextOrientation,
    /// `overscroll-behavior` (Fase 7.267). Tupla X/Y. NO heredable.
    /// Plumb: el chrome todavía no captura el overflow rebote.
    pub overscroll_behavior_x: OverscrollBehavior,
    pub overscroll_behavior_y: OverscrollBehavior,
    /// `scroll-snap-type` (Fase 7.268). NO heredable. Plumb.
    pub scroll_snap_type: ScrollSnapType,
    /// `scroll-snap-align` (Fase 7.269). Tupla block/inline. NO heredable.
    /// Plumb: el chrome no resuelve el snap.
    pub scroll_snap_align_block: ScrollSnapAlign,
    pub scroll_snap_align_inline: ScrollSnapAlign,
    /// `scroll-snap-stop` (Fase 7.270). NO heredable. Plumb.
    pub scroll_snap_stop: ScrollSnapStop,
    /// `scroll-padding` (Fase 7.271). Sides T/R/B/L con `LengthVal`
    /// (acepta `auto` + px + %). NO heredable. Plumb.
    pub scroll_padding: Sides<LengthVal>,
    /// `scroll-margin` (Fase 7.272). Sides T/R/B/L en px. NO heredable.
    /// Plumb.
    pub scroll_margin: Sides<f32>,
    /// `touch-action` (Fase 7.273). NO heredable. CSS Pointer Events 2.
    /// Plumb: el chrome no rutea pointer events según este hint.
    pub touch_action: TouchAction,
    /// `clip-path` (Fase 7.274). `None` = sin recorte. NO heredable.
    /// Plumb: vello no aplica el recorte a la sub-scene del nodo aún.
    pub clip_path: Option<ClipPath>,
    /// `mask-image` (Fase 7.275). `None` = sin máscara. NO heredable.
    /// Plumb: subset url(...) — no se baja ni se aplica todavía.
    pub mask_image: Option<MaskImage>,
    /// `content-visibility` (Fase 7.276). NO heredable. Plumb: el chrome
    /// no skipea el render de subtrees con `auto`/`hidden`.
    pub content_visibility: ContentVisibility,
    /// `contain` (Fase 7.277). CSS Containment 2. Bitset de tipos.
    /// `None` (todos los bits a 0) = sin containment. NO heredable. Plumb.
    pub contain: ContainFlags,
    /// `column-count` (Fase 7.278). `None` = `auto`. NO heredable. Plumb:
    /// no hay layout multicol todavía.
    pub column_count: Option<u32>,
    /// `column-width` (Fase 7.279). `LengthVal::Auto` = `auto`. NO heredable.
    /// Plumb.
    pub column_width: LengthVal,
    /// `column-rule` (Fase 7.280). Subset: width + style + color, igual
    /// shape que `border`. `style_active` togglea el dibujo. NO heredable.
    /// Plumb.
    pub column_rule_width: f32,
    pub column_rule_color: Option<Color>,
    pub column_rule_style: BorderLineStyle,
    pub column_rule_style_active: bool,
    /// CSS Gap Decorations 1 (Fase 7.920) — `row-rule-*`, espejo del eje de
    /// columnas. NO heredable. Plumb.
    pub row_rule_width: f32,
    pub row_rule_color: Option<Color>,
    pub row_rule_style: BorderLineStyle,
    pub row_rule_style_active: bool,
    /// `column-fill` (Fase 7.281). Default `Balance`. NO heredable. Plumb.
    pub column_fill: ColumnFill,
    /// `column-span` (Fase 7.282). Default `None`. NO heredable. Plumb.
    pub column_span: ColumnSpan,
    /// `break-inside` (Fase 7.283). Default `Auto`. NO heredable. Plumb.
    pub break_inside: BreakInside,
    /// `table-layout` (Fase 7.284). Default `Auto`. NO heredable. Plumb:
    /// el chrome aún no diferencia layout fixed vs auto en `display: table`.
    pub table_layout: TableLayout,
    /// `border-collapse` (Fase 7.285). Default `Separate`. **Heredable**.
    /// Plumb.
    pub border_collapse: BorderCollapse,
    /// `border-spacing` (Fase 7.286). Tupla h/v en px (sólo aplica si
    /// `border-collapse: separate`). Default 0/0. **Heredable**. Plumb.
    pub border_spacing_h: f32,
    pub border_spacing_v: f32,
    /// `caption-side` (Fase 7.287). Default `Top`. **Heredable** (sólo
    /// hereda en `<caption>`). Plumb.
    pub caption_side: CaptionSide,
    /// `empty-cells` (Fase 7.288). Default `Show`. **Heredable**. Plumb.
    pub empty_cells: EmptyCells,
    /// `break-before` (Fase 7.289). Default `Auto`. NO heredable. Plumb.
    pub break_before: BreakBetween,
    /// `break-after` (Fase 7.290). Default `Auto`. NO heredable. Plumb.
    pub break_after: BreakBetween,
    /// `orphans` (Fase 7.291). Default 2. **Heredable**. Plumb.
    pub orphans: u32,
    /// `widows` (Fase 7.292). Default 2. **Heredable**. Plumb.
    pub widows: u32,
    /// `color-scheme` (Fase 7.293). Default `Normal` (sin compromiso).
    /// **Heredable**. Plumb: el chrome no toggea UA defaults dark vs light.
    pub color_scheme: ColorScheme,
    /// `list-style-position` (Fase 7.294). Default `Outside`. **Heredable**.
    /// Plumb: el chrome pinta el marker siempre afuera.
    pub list_style_position: ListStylePosition,
    /// `list-style-image` (Fase 7.295). `None` = `none`. **Heredable**.
    /// Plumb: el marker no se reemplaza por la imagen aún.
    pub list_style_image: Option<String>,
    /// `counter-set: name [N] ...` (Fase 7.297). Vacío = sin counter-set.
    /// Idéntico shape a `counter-reset` (default 0). NO heredable.
    pub counter_set: Vec<(String, i32)>,
    /// `quotes` (Fase 7.298). `Auto` (default) deja la UA elegir; vacío
    /// = `none` (los `open-quote`/`close-quote` no insertan nada); con
    /// pares concretos, el (open, close) por nivel de anidamiento se
    /// recicla en el último par si se profundiza más allá. **Heredable**.
    /// Plumb: el `content: open-quote` no se resuelve contra esta tabla.
    pub quotes: Quotes,
    /// `text-underline-position` (Fase 7.299). Default `Auto`. **Heredable**.
    /// Plumb: el shaper no mueve el underline a posición alternativa aún.
    pub text_underline_position: TextUnderlinePosition,
    /// `text-justify` (Fase 7.300). Default `Auto`. **Heredable**. Sólo
    /// aplica si `text-align: justify`. Plumb.
    pub text_justify: TextJustify,
    /// `print-color-adjust` (Fase 7.301). Default `Economy`. **Heredable**.
    /// Plumb: el chrome no decide cuándo simplificar colores para imprimir.
    pub print_color_adjust: PrintColorAdjust,
    /// `forced-color-adjust` (Fase 7.302). Default `Auto`. **Heredable**.
    /// Plumb: el chrome no entra en modo forced-colors.
    pub forced_color_adjust: ForcedColorAdjust,
    /// `-webkit-line-clamp` / `line-clamp` (Fase 7.303). `None` = sin
    /// truncado. NO heredable. Plumb: el layout no recorta a N líneas.
    pub line_clamp: Option<u32>,
    /// `font-variant-caps` (Fase 7.304). Default `Normal`. **Heredable**.
    /// Plumb: el shaper no aplica caps variants.
    pub font_variant_caps: FontVariantCaps,
    /// `font-variant-numeric` (Fase 7.305). Bitset. **Heredable**. Plumb.
    pub font_variant_numeric: FontVariantNumeric,
    /// `font-variant-ligatures` (Fase 7.306). Bitset + `None` (todas off)
    /// vs `Normal` (defaults). **Heredable**. Plumb.
    pub font_variant_ligatures: FontVariantLigatures,
    /// `font-variant-east-asian` (Fase 7.307). Bitset. **Heredable**. Plumb.
    pub font_variant_east_asian: FontVariantEastAsian,
    /// `font-variant-position` (Fase 7.308). Default `Normal`. **Heredable**.
    /// Plumb.
    pub font_variant_position: FontVariantPosition,
    /// `text-emphasis-style` (Fase 7.309). Default `None`. **Heredable**.
    /// Plumb: el shaper no dibuja la marca encima/debajo de cada char.
    pub text_emphasis_style: TextEmphasisStyle,
    /// `text-emphasis-color` (Fase 7.310). `None` = `currentColor`.
    /// **Heredable**. Plumb.
    pub text_emphasis_color: Option<Color>,
    /// `text-emphasis-position` (Fase 7.311). Default `Over Right`.
    /// **Heredable**. Plumb.
    pub text_emphasis_position: TextEmphasisPosition,
    /// `ruby-position` (Fase 7.313). Default `Alternate`. **Heredable**.
    /// Plumb: no hay layout de `<ruby>` propio aún.
    pub ruby_position: RubyPosition,
    /// `transform-origin` (Fase 7.314). Default `50% 50% 0`. NO hereda.
    /// Plumb: el chrome no ancla las transforms a este punto todavía
    /// (rota/escala alrededor del centro fijo).
    pub transform_origin: TransformOrigin,
    /// `transform-style` (Fase 7.315). Default `Flat`. NO hereda. Plumb:
    /// no hay composición 3D entre hijos.
    pub transform_style: TransformStyle,
    /// `perspective` (Fase 7.316). `None` = sin proyección. NO hereda.
    /// Plumb: el chrome no proyecta a partir de los hijos.
    pub perspective: Option<f32>,
    /// `perspective-origin` (Fase 7.317). Default `50% 50%`. NO hereda.
    /// Plumb.
    pub perspective_origin: PerspectiveOrigin,
    /// `backface-visibility` (Fase 7.318). Default `Visible`. NO hereda.
    /// Plumb: el chrome siempre pinta la cara, incluso cuando una
    /// `rotateY(180deg)` la voltearía.
    pub backface_visibility: BackfaceVisibility,
    /// `scrollbar-width` (Fase 7.319). Default `Auto`. **Heredable**
    /// (CSS Scrollbars 1). Plumb: la UA scrollbar es la única — no
    /// ajustamos su grosor.
    pub scrollbar_width: ScrollbarWidth,
    /// `scrollbar-color` (Fase 7.320). `None` = `auto`. **Heredable**.
    /// Plumb: no pintamos el thumb/track con estos colores.
    pub scrollbar_color: Option<ScrollbarColorPair>,
    /// `scrollbar-gutter` (Fase 7.321). Default `Auto`. NO hereda.
    /// Plumb: no reservamos un canal cuando la barra no está montada.
    pub scrollbar_gutter: ScrollbarGutter,
    /// `overflow-anchor` (Fase 7.322). Default `Auto`. NO hereda.
    /// Plumb: no hay scroll anchoring real (no reanclamos al
    /// reflowear contenido encima del viewport).
    pub overflow_anchor: OverflowAnchor,
    /// `overflow-clip-margin` (Fase 7.323). `None` = sin extensión.
    /// NO hereda. Plumb: el chrome usa el rect normal de clipping.
    pub overflow_clip_margin: Option<OverflowClipMargin>,
    /// `text-align-last` (Fase 7.324). Default `Auto`. **Heredable**.
    /// Plumb: no se distingue la última línea de un párrafo justificado.
    pub text_align_last: TextAlignLast,
    /// `text-wrap` (Fase 7.325). Default `Wrap`. **Heredable**.
    /// Plumb: el line-breaker no implementa balance/pretty/stable.
    pub text_wrap: TextWrap,
    /// `line-break` (Fase 7.326). Default `Auto`. **Heredable**.
    /// Plumb: el line-breaker usa siempre Unicode default.
    pub line_break: LineBreak,
    /// `hanging-punctuation` (Fase 7.327). Default `None`. **Heredable**.
    /// Plumb: no se cuelga puntuación fuera del box.
    pub hanging_punctuation: HangingPunctuation,
    /// `text-decoration-skip-ink` (Fase 7.328). Default `Auto`.
    /// **Heredable**. Plumb: no se saltean descendientes en underline.
    pub text_decoration_skip_ink: TextDecorationSkipInk,
    /// `font-optical-sizing` (Fase 7.329). Default `Auto`. **Heredable**.
    /// Plumb: el shaper no setea el axis `opsz` de fuentes variables.
    pub font_optical_sizing: FontOpticalSizing,
    /// `font-synthesis-{weight,style,small-caps}` (Fases 7.330–7.332) +
    /// shorthand `font-synthesis` (Fase 7.333). Cada flag = `auto`
    /// (true, default) o `none` (false). Si toda la struct está en
    /// `none`, equivale al keyword `font-synthesis: none`. **Heredable**.
    /// Plumb: el shaper hace synthesis siempre si la fuente no provee
    /// la variante.
    pub font_synthesis: FontSynthesis,
    /// `font-size-adjust` (Fase 7.334). Default `None` (sin ajuste).
    /// **Heredable**. Plumb: el shaper no escala glifos contra la
    /// métrica del fallback.
    pub font_size_adjust: FontSizeAdjust,
    /// `image-orientation` (Fase 7.335). Default `FromImage` (rota
    /// según EXIF). NO hereda en el grafo de imágenes pero el property
    /// `image-orientation` SÍ hereda al estilo (los `<img>` lo leen).
    /// Plumb: el chrome no aplica rotación a `<img>`/`background-image`.
    pub image_orientation: ImageOrientation,
    /// `animation-timeline` (Fase 7.339). Default `Auto`. NO hereda.
    /// Plumb: no hay runtime de animación (B4), así que la línea de
    /// tiempo nunca se consume.
    pub animation_timeline: TimelineRef,
    /// `scroll-timeline-name` (Fase 7.340). `None` = sin timeline.
    /// NO hereda. Plumb.
    pub scroll_timeline_name: Option<String>,
    /// `scroll-timeline-axis` (Fase 7.341). Default `Block`. NO hereda.
    /// Plumb.
    pub scroll_timeline_axis: TimelineAxis,
    /// `view-timeline-name` (Fase 7.342). `None` = sin timeline.
    /// NO hereda. Plumb.
    pub view_timeline_name: Option<String>,
    /// `view-timeline-axis` (Fase 7.343). Default `Block`. NO hereda.
    /// Plumb.
    pub view_timeline_axis: TimelineAxis,
    /// `white-space-collapse` (Fase 7.344). Default `Collapse`.
    /// **Heredable**. Plumb: `white-space` clásico sigue mandando en
    /// el layout; este axis no se consume.
    pub white_space_collapse: WhiteSpaceCollapse,
    /// `text-wrap-mode` (Fase 7.345). Default `Wrap`. **Heredable**.
    /// Plumb.
    pub text_wrap_mode: TextWrapMode,
    /// `text-wrap-style` (Fase 7.346). Default `Auto`. **Heredable**.
    /// Plumb.
    pub text_wrap_style: TextWrapStyle,
    /// `wrap-before` / `wrap-after` (CSS Text 4). Default `Auto`. NO hereda.
    /// Plumb: no se consume en el quiebre de línea. Fase 7.927.
    pub wrap_before: WrapBetween,
    pub wrap_after: WrapBetween,
    /// `wrap-inside` (CSS Text 4). Default `Auto`. NO hereda. Plumb. Fase 7.927.
    pub wrap_inside: WrapInside,
    /// `text-spacing-trim` (Fase 7.347). Default `Normal`.
    /// **Heredable**. Plumb: el shaper no recorta puntuación CJK.
    pub text_spacing_trim: TextSpacingTrim,
    /// `text-box-trim` (Fase 7.348). Default `None`. **Heredable**.
    /// Plumb: el chrome no recorta el leading/trailing del text-box.
    pub text_box_trim: TextBoxTrim,
    /// `math-style` (Fase 7.349). Default `Normal`. **Heredable**.
    /// Plumb: no hay rendering MathML propio.
    pub math_style: MathStyle,
    /// `math-depth` (Fase 7.350). Default `Auto`. **Heredable**.
    /// Plumb.
    pub math_depth: MathDepth,
    /// `math-shift` (Fase 7.351). Default `Normal`. **Heredable**.
    /// Plumb.
    pub math_shift: MathShift,
    /// `field-sizing` (Fase 7.352). Default `Fixed`. NO hereda.
    /// Plumb: `<input>`/`<textarea>` siempre fixed-size.
    pub field_sizing: FieldSizing,
    /// `overlay` (Fase 7.905). Default `None`. NO hereda. Plumb opaco.
    pub overlay: Overlay,
    /// `dynamic-range-limit` (Fase 7.905). Default `NoLimit`. **Heredable**.
    /// Plumb opaco (sin tone-mapping HDR).
    pub dynamic_range_limit: DynamicRangeLimit,
    /// `text-box-edge` (Fase 7.353). Default `Auto`. **Heredable**.
    /// Plumb.
    pub text_box_edge: TextBoxEdge,
    /// `anchor-name` (Fase 7.354). Vacío = `none`. NO hereda.
    /// Plumb: el chrome no implementa anchor positioning.
    pub anchor_name: Vec<String>,
    /// `position-anchor` (Fase 7.355). `None` = `auto`. NO hereda.
    /// Plumb.
    pub position_anchor: Option<String>,
    /// `anchor-scope` (Fase 7.356). Default `None` (=`none`). `All`
    /// extiende a todos los anchors. `Names(v)` limita por lista.
    /// **Heredable**. Plumb.
    pub anchor_scope: AnchorScope,
    /// `view-transition-name` (Fase 7.357). `None` = `none`. NO hereda.
    /// Plumb.
    pub view_transition_name: Option<String>,
    /// `view-transition-class` (Fase 7.358). Vacío = `none`. NO hereda.
    /// Plumb.
    pub view_transition_class: Vec<String>,
    /// `font-palette` (Fase 7.359). Default `Normal`. **Heredable**.
    /// Plumb: el shaper usa la paleta default.
    pub font_palette: FontPalette,
    /// `font-variant-alternates` (Fase 7.360). Default `Normal`.
    /// **Heredable**. Plumb: no se aplican alternates.
    pub font_variant_alternates: FontVariantAlternates,
    /// `background-attachment` (Fase 7.362). Vec paralelo a las capas
    /// de background. Por defecto `[Scroll]` (1 capa). NO hereda.
    /// Plumb: el chrome no implementa `fixed`/`local`.
    pub background_attachment: Vec<BackgroundAttachment>,
    /// `caret-shape` (Fase 7.363). Default `Auto`. **Heredable**.
    /// Plumb: el caret se pinta siempre como bar.
    pub caret_shape: CaretShape,
    /// `baseline-source` (Fase 7.364). Default `Auto`. NO hereda.
    /// Plumb: el inline-flow usa siempre la baseline del primer hijo.
    pub baseline_source: BaselineSource,
    /// `alignment-baseline` (Fase 7.365). Default `Baseline`. NO hereda.
    /// Plumb: SVG no implementado, el text-anchor lo ignora.
    pub alignment_baseline: AlignmentBaseline,
    /// `dominant-baseline` (Fase 7.366). Default `Auto`. **Heredable**.
    /// Plumb.
    pub dominant_baseline: DominantBaseline,
    /// `paint-order` (Fase 7.367). Default `Normal` (= `fill stroke
    /// markers`). **Heredable**. Plumb.
    pub paint_order: PaintOrder,
    /// `marker-side` (Fase 7.368). Default `MatchSelf`. **Heredable**.
    /// Plumb.
    pub marker_side: MarkerSide,
    /// `fill` (Fase 7.369). Default `Color(black)` (la spec SVG).
    /// **Heredable**. Plumb: SVG no implementado.
    pub fill: SvgPaint,
    /// `stroke` (Fase 7.370). Default `None`. **Heredable**. Plumb.
    pub stroke: SvgPaint,
    /// `fill-opacity` (Fase 7.371). Default `1.0`. **Heredable**. Plumb.
    pub fill_opacity: f32,
    /// `stroke-opacity` (Fase 7.372). Default `1.0`. **Heredable**. Plumb.
    pub stroke_opacity: f32,
    /// `stroke-width` (Fase 7.373). Default `Px(1.0)`. **Heredable**.
    /// Plumb.
    pub stroke_width: LengthVal,
    /// `stroke-linecap` (Fase 7.374). Default `Butt`. **Heredable**.
    /// Plumb.
    pub stroke_linecap: StrokeLinecap,
    /// `stroke-linejoin` (Fase 7.375). Default `Miter`. **Heredable**.
    /// Plumb.
    pub stroke_linejoin: StrokeLinejoin,
    /// `stroke-miterlimit` (Fase 7.376). Default `4.0`. **Heredable**.
    /// Plumb.
    pub stroke_miterlimit: f32,
    /// `stroke-dasharray` (Fase 7.377). Vec vacío = `none`. **Heredable**.
    /// Plumb.
    pub stroke_dasharray: Vec<LengthVal>,
    /// `stroke-dashoffset` (Fase 7.378). Default `Px(0.0)`. **Heredable**.
    /// Plumb.
    pub stroke_dashoffset: LengthVal,
    /// `fill-rule` (Fase 7.379). Default `Nonzero`. **Heredable**. Plumb.
    pub fill_rule: FillRule,
    /// `clip-rule` (Fase 7.380). Default `Nonzero`. **Heredable**. Plumb.
    pub clip_rule: FillRule,
    /// `color-interpolation` (Fase 7.381). Default `SRgb`. **Heredable**.
    /// Plumb.
    pub color_interpolation: ColorInterpolation,
    /// `shape-rendering` (Fase 7.382). Default `Auto`. **Heredable**. Plumb.
    pub shape_rendering: ShapeRendering,
    /// `vector-effect` (Fase 7.383). Default `None`. NO hereda. Plumb.
    pub vector_effect: VectorEffect,
    /// `flood-color` (Fase 7.384). `None` = `currentColor`. NO hereda.
    /// Plumb.
    pub flood_color: Option<Color>,
    /// `flood-opacity` (Fase 7.385). Default `1.0`. NO hereda. Plumb.
    pub flood_opacity: f32,
    /// `lighting-color` (Fase 7.386). `None` = `currentColor`. NO hereda.
    /// Plumb.
    pub lighting_color: Option<Color>,
    /// `stop-color` (Fase 7.387). `None` = `currentColor`. NO hereda.
    /// Plumb.
    pub stop_color: Option<Color>,
    /// `stop-opacity` (Fase 7.388). Default `1.0`. NO hereda. Plumb.
    pub stop_opacity: f32,
    /// `text-anchor` (Fase 7.389). Default `Start`. **Heredable**. Plumb.
    pub text_anchor: TextAnchor,
    /// `color-rendering` (Fase 7.390). Default `Auto`. **Heredable**. Plumb.
    pub color_rendering: ColorRendering,
    /// `color-interpolation-filters` (Fase 7.391). Default `LinearRgb`
    /// (la spec lo separa de `color-interpolation`). **Heredable**. Plumb.
    pub color_interpolation_filters: ColorInterpolationFilters,
    /// `glyph-orientation-vertical` (Fase 7.392). Default `Auto`.
    /// **Heredable**. Plumb (SVG 1.1 deprecated, sólo parseo).
    pub glyph_orientation_vertical: GlyphOrientationVertical,
    /// `transform-box` (Fase 7.393). Default `ViewBox`. NO hereda. Plumb.
    pub transform_box: TransformBox,
    /// `marker-start` (Fase 7.394). `None` = `none`. **Heredable**. Plumb.
    pub marker_start: MarkerRef,
    /// `marker-mid` (Fase 7.395). `None` = `none`. **Heredable**. Plumb.
    pub marker_mid: MarkerRef,
    /// `marker-end` (Fase 7.396). `None` = `none`. **Heredable**. Plumb.
    /// El shorthand `marker` (Fase 7.397) setea los tres a la vez.
    pub marker_end: MarkerRef,
    /// `mask-type` (Fase 7.398). Default `Luminance`. NO hereda. Plumb.
    pub mask_type: MaskType,
    /// `mask-mode` (Fase 7.399). Default `MatchSource`. NO hereda. Plumb.
    pub mask_mode: MaskMode,
    /// `mask-clip` (Fase 7.400). Default `BorderBox`. NO hereda. Plumb.
    pub mask_clip: MaskClip,
    /// `mask-composite` (Fase 7.401). Default `Add`. NO hereda. Plumb.
    pub mask_composite: MaskComposite,
    /// `mask-origin` (Fase 7.402). Default `BorderBox`. NO hereda. Plumb.
    pub mask_origin: MaskOrigin,
    /// `mask-repeat` (Fase 7.403). Default `Repeat`. NO hereda. Plumb.
    /// Reusa `BackgroundRepeat` (mismas formas).
    pub mask_repeat: BackgroundRepeat,
    /// `mask-position` (Fase 7.404). Default `(Pct(0), Pct(0))` — esquina
    /// superior-izquierda. NO hereda. Plumb. Reusa `BackgroundPosition`.
    pub mask_position: BackgroundPosition,
    /// `mask-size` (Fase 7.405). Default `Auto`. NO hereda. Plumb. Reusa
    /// `BackgroundSize`.
    pub mask_size: BackgroundSize,
    /// `container-name` (Fase 7.406). Vec vacío = `none`. NO hereda. Plumb.
    pub container_name: Vec<String>,
    /// `container-type` (Fase 7.407). Default `Normal`. NO hereda. Plumb.
    /// El shorthand `container` (Fase 7.408) setea name + type.
    pub container_type: ContainerType,
    /// `offset-path` (Fase 7.427). `None` = `none`; `Some(s)` guarda la
    /// cadena cruda (sin parsear `path(...)` / `ray(...)` / `<basic-shape>`).
    /// NO hereda. Plumb.
    pub offset_path: Option<String>,
    /// `offset-distance` (Fase 7.428). Distancia recorrida a lo largo del
    /// `offset-path`. Default `Px(0)`. NO hereda. Plumb.
    pub offset_distance: LengthVal,
    /// `hyphenate-character` (Fase 7.429). `None` = `auto` (motor elige el
    /// carácter del idioma — típicamente U+2010); `Some(s)` = string literal.
    /// HEREDA. Plumb.
    pub hyphenate_character: Option<String>,
    /// `hyphenate-limit-chars` (Fase 7.430). Triple `<total> <start> <end>`
    /// con `auto` por cada uno (`None`). HEREDA. Plumb.
    pub hyphenate_limit_chars: HyphenateLimitChars,
    /// `text-size-adjust` (Fase 7.431). Default `Auto`. HEREDA. Plumb.
    pub text_size_adjust: TextSizeAdjust,
    /// `line-height-step` (Fase 7.432). Tamaño de la cuadrícula vertical
    /// (px). `0` = sin cuadrícula. HEREDA. Plumb.
    pub line_height_step: f32,
    /// `font-variant-emoji` (Fase 7.433). Default `Normal`. HEREDA. Plumb.
    pub font_variant_emoji: FontVariantEmoji,
    /// `contain-intrinsic-width` (Fase 7.434). Default `None`. NO hereda. Plumb.
    pub contain_intrinsic_width: ContainIntrinsicSize,
    /// `contain-intrinsic-height` (Fase 7.435). Default `None`. NO hereda. Plumb.
    pub contain_intrinsic_height: ContainIntrinsicSize,
    /// `grid-auto-flow` (Fase 7.441). Default `Row`. NO hereda. Plumb.
    pub grid_auto_flow: GridAutoFlow,
    /// `grid-auto-columns` (Fase 7.442). Lista de tracks implícitos
    /// (CSS Grid 1). Vacío = `auto`. NO hereda. Plumb.
    pub grid_auto_columns: Vec<GridTrackSize>,
    /// `grid-auto-rows` (Fase 7.443). Lista de tracks implícitos
    /// (CSS Grid 1). Vacío = `auto`. NO hereda. Plumb.
    pub grid_auto_rows: Vec<GridTrackSize>,
    /// `shape-outside` (Fase 7.444). `None` = `none`; `Some(s)` guarda
    /// el valor crudo (parse opaco, igual que `offset-path`). NO hereda. Plumb.
    pub shape_outside: Option<String>,
    /// `shape-margin` (Fase 7.445). `<length-or-pct>` no-negativo. Default
    /// `Px(0)`. NO hereda. Plumb.
    pub shape_margin: LengthVal,
    /// `shape-image-threshold` (Fase 7.446). `<alpha-value>` clamp [0..1].
    /// Default `0.0`. NO hereda. Plumb.
    pub shape_image_threshold: f32,
    /// `text-combine-upright` (Fase 7.447). Default `None`. NO hereda. Plumb.
    pub text_combine_upright: TextCombineUpright,
    /// `ruby-align` (Fase 7.448). Default `SpaceAround`. HEREDA. Plumb.
    pub ruby_align: RubyAlign,
    /// `offset-rotate` (Fase 7.449). Default `auto`. NO hereda. Plumb.
    pub offset_rotate: OffsetRotate,
    /// `offset-anchor` (Fase 7.450). `None` = `auto` (espejo de
    /// `transform-origin`). NO hereda. Plumb.
    pub offset_anchor: Option<BackgroundPosition>,
    /// `offset-position` (Fase 7.451). `None` = `auto` (usa la posición
    /// del box). `Some(p)` = punto en el contenedor. NO hereda. Plumb.
    pub offset_position: Option<BackgroundPosition>,
    /// `object-view-box` (Fase 7.452). `None` = `none`; `Some(s)` guarda
    /// el valor crudo (parse opaco). NO hereda. Plumb.
    pub object_view_box: Option<String>,
    /// `ruby-overhang` (Fase 7.453). Default `Auto`. HEREDA. Plumb.
    pub ruby_overhang: RubyOverhang,
    /// `block-step-size` (Fase 7.454). Default `None`. NO hereda. Plumb.
    pub block_step_size: BlockStepSize,
    /// `block-step-insert` (Fase 7.455). Default `MarginBox`. NO hereda. Plumb.
    pub block_step_insert: BlockStepInsert,
    /// `block-step-align` (Fase 7.456). Default `Auto`. NO hereda. Plumb.
    pub block_step_align: BlockStepAlign,
    /// `block-step-round` (Fase 7.457). Default `Up`. NO hereda. Plumb.
    pub block_step_round: BlockStepRound,
    /// `position-visibility` (Fase 7.459). Default `Always`. NO hereda. Plumb.
    pub position_visibility: PositionVisibility,
    /// `position-try-order` (Fase 7.460). Default `Normal`. NO hereda. Plumb.
    pub position_try_order: PositionTryOrder,
    /// `position-try-fallbacks` (Fase 7.461). Vec vacío = `none`. NO hereda. Plumb.
    pub position_try_fallbacks: Vec<String>,
    /// `position-area` (Fase 7.463). `None` = `none`; `Some(s)` guarda el
    /// valor crudo (parse opaco). NO hereda. Plumb.
    pub position_area: Option<String>,
    /// `animation-range-start` (Fase 7.464). Default `Normal`. NO hereda. Plumb.
    pub animation_range_start: AnimationRange,
    /// `animation-range-end` (Fase 7.465). Default `Normal`. NO hereda. Plumb.
    pub animation_range_end: AnimationRange,
    /// `transition-behavior` (Fase 7.467). Default `Normal`. NO hereda. Plumb.
    pub transition_behavior: TransitionBehavior,
    /// `interpolate-size` (Fase 7.468). Default `NumericOnly`. **HEREDA**. Plumb.
    pub interpolate_size: InterpolateSize,
    /// `view-timeline-inset` (Fase 7.469). Par `(start, end)` — `LengthVal::Auto`
    /// (= cero) por default. NO hereda. Plumb.
    pub view_timeline_inset_start: LengthVal,
    pub view_timeline_inset_end: LengthVal,
    /// `interactivity` (Fase 7.473). Default `Auto`. **HEREDA** (CSS UI 4).
    /// Plumb.
    pub interactivity: Interactivity,
    /// `cx` (Fase 7.474). Geometría SVG `<circle>`/`<ellipse>`. Default
    /// `LengthVal::Px(0.0)`. NO hereda. Plumb.
    pub cx: LengthVal,
    /// `cy` (Fase 7.475). Geometría SVG `<circle>`/`<ellipse>`. Default
    /// `LengthVal::Px(0.0)`. NO hereda. Plumb.
    pub cy: LengthVal,
    /// `r` (Fase 7.476). Radio de `<circle>`. Default `LengthVal::Px(0.0)`.
    /// NO hereda. Plumb.
    pub r: LengthVal,
    /// `rx` (Fase 7.477). Radio elipse eje X (`<ellipse>`/`<rect>`).
    /// Default `LengthVal::Auto`. NO hereda. Plumb.
    pub rx: LengthVal,
    /// `ry` (Fase 7.478). Radio elipse eje Y (`<ellipse>`/`<rect>`).
    /// Default `LengthVal::Auto`. NO hereda. Plumb.
    pub ry: LengthVal,
    /// `x` (SVG 2). Posición SVG como prop CSS. Default `Px(0)`. NO hereda.
    pub x: LengthVal,
    /// `y` (SVG 2). Posición SVG como prop CSS. Default `Px(0)`. NO hereda.
    pub y: LengthVal,
    /// `baseline-shift` (SVG / CSS Inline 3). Default `Baseline`. NO hereda.
    pub baseline_shift: BaselineShift,
    /// `solid-color` (SVG 2 `<solidcolor>`). Default negro. NO hereda. Plumb.
    pub solid_color: Color,
    /// `solid-opacity` (SVG 2 `<solidcolor>`). Default `1.0`. NO hereda. Plumb.
    pub solid_opacity: f32,
    /// `order` (Fase 7.479). Reordena ítems en flex/grid sin alterar el DOM.
    /// Default `0`. Negativos = antes del bloque. NO hereda. Plumb.
    pub order: i32,
    /// `path-length` (Fase 7.480). SVG: longitud "lógica" del path para
    /// dasharray. `None` = `none` (usar la real). NO hereda. Plumb.
    pub path_length: Option<f32>,
    /// `animation-composition` (Fase 7.481). Cómo se combinan los efectos
    /// concurrentes sobre una misma propiedad. Default `Replace`. NO hereda.
    /// Plumb.
    pub animation_composition: AnimationComposition,
    /// `timeline-scope` (Fase 7.482). Lista de nombres de timeline que este
    /// elemento expone hacia descendientes. Vec vacío = `none`. NO hereda.
    /// Plumb.
    pub timeline_scope: Vec<String>,
    /// `reading-order` (Fase 7.483). CSS Inline 3: orden lógico para AT
    /// que difiere del orden visual. Default `0`. NO hereda. Plumb.
    pub reading_order: i32,
    /// `reading-flow` (Fase 7.484). CSS Display 4: cómo recorrer el
    /// contenido focalizable de un contenedor (lectura/tab). Default
    /// `Normal`. NO hereda. Plumb.
    pub reading_flow: ReadingFlow,
    /// `image-resolution` (Fase 7.485). CSS Images 4. Default
    /// `FromImage`. **HEREDA**. Plumb.
    pub image_resolution: ImageResolution,
    /// `bookmark-level` (Fase 7.486). CSS GCPM. Profundidad del marcador
    /// PDF. `None` = `none` (no genera bookmark). NO hereda. Plumb.
    pub bookmark_level: Option<u32>,
    /// `bookmark-state` (Fase 7.487). CSS GCPM. Default `Open`. NO hereda.
    /// Plumb.
    pub bookmark_state: BookmarkState,
    /// `bookmark-label` (Fase 7.488). CSS GCPM. `None` = `content(text)`
    /// (default — toma el texto del elemento). NO hereda. Plumb.
    pub bookmark_label: Option<String>,
    /// `string-set` (Fase 7.489). CSS GCPM: define strings nombradas que
    /// luego `content: string()` consume en headers/footers paginados.
    /// `None` = `none`. Parse opaco. NO hereda. Plumb.
    pub string_set: Option<String>,
    /// `footnote-display` (Fase 7.490). CSS GCPM 4: cómo se renderiza la
    /// nota al pie. Default `Block`. NO hereda. Plumb.
    pub footnote_display: FootnoteDisplay,
    /// `footnote-policy` (Fase 7.491). CSS GCPM 4: cuándo desplazar una
    /// nota al pie a la siguiente página. Default `Auto`. NO hereda. Plumb.
    pub footnote_policy: FootnotePolicy,
    /// `marker-knockout-left` (Fase 7.492). CSS GCPM 4: cómo el marker
    /// del list-item evita la regla de margen izquierda. Default `Auto`.
    /// NO hereda. Plumb.
    pub marker_knockout_left: MarkerKnockout,
    /// `marker-knockout-right` (Fase 7.493). Espejo del anterior para el
    /// margen derecho. Default `Auto`. NO hereda. Plumb.
    pub marker_knockout_right: MarkerKnockout,
    /// `leading-trim` (Fase 7.494). CSS Inline 3: recorta la half-leading
    /// del bloque. Default `Normal`. **HEREDA**. Plumb.
    pub leading_trim: LeadingTrim,
    /// `initial-letter-align` (Fase 7.495). CSS Inline 3: cómo alinear
    /// el drop-cap respecto al texto adyacente. Default `Auto`. **HEREDA**.
    /// Plumb.
    pub initial_letter_align: InitialLetterAlign,
    /// `text-autospace` (Fase 7.496). CSS Text 4: espaciado automático
    /// entre scripts (CJK ↔ latin/digit). Parse opaco — `None` = `normal`.
    /// **HEREDA**. Plumb.
    pub text_autospace: Option<String>,
    /// `white-space-trim` (Fase 7.497). CSS Text 4: recorta whitespace en
    /// los bordes del bloque. Parse opaco — `None` = `none`. **HEREDA**.
    /// Plumb.
    pub white_space_trim: Option<String>,
    /// `view-transition-group` (Fase 7.498). CSS View Transitions 2:
    /// nombre del grupo donde el elemento participa. `None` = `normal`.
    /// NO hereda. Plumb.
    pub view_transition_group: Option<String>,
    /// `inset-area` (Fase 7.499). CSS Anchor Positioning 1: alias legacy
    /// de `position-area`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub inset_area: Option<String>,
    /// `view-transition-image-pair` (Fase 7.500). CSS View Transitions 2:
    /// nombre del par de imagen para la animación. `None` = `auto`. NO
    /// hereda. Plumb.
    pub view_transition_image_pair: Option<String>,
    /// `animation-trigger` (Fase 7.501). CSS Animations 2: trigger
    /// scroll-driven. Shorthand opaco — `None` = sin trigger. NO hereda.
    /// Plumb.
    pub animation_trigger: Option<String>,
    /// `border-image-source` (Fase 7.502). `None` = `none` (renderer cae
    /// al border tradicional). NO hereda. Plumb.
    pub border_image_source: Option<String>,
    /// `border-image-repeat` (Fase 7.503). Par (horizontal, vertical).
    /// Default `(Stretch, Stretch)`. NO hereda. Plumb.
    pub border_image_repeat_h: BorderImageRepeat,
    pub border_image_repeat_v: BorderImageRepeat,
    /// `border-image-slice` (Fase 7.504). Parse opaco — la gramática
    /// (`<number-percentage>{1,4} && fill?`) se evalúa cuando un
    /// renderer lo necesite. `None` = default (`100%`). NO hereda. Plumb.
    pub border_image_slice: Option<String>,
    /// `border-image-width` (Fase 7.505). Parse opaco. `None` = default
    /// (`1`). NO hereda. Plumb.
    pub border_image_width: Option<String>,
    /// `border-image-outset` (Fase 7.506). Parse opaco. `None` = default
    /// (`0`). NO hereda. Plumb.
    pub border_image_outset: Option<String>,
    /// `border-image` shorthand (Fase 7.507). Parse opaco. `None` = `none`.
    /// NO hereda. Plumb.
    pub border_image: Option<String>,
    /// `grid-template-areas` (Fase 7.508). Parse opaco hasta que la
    /// resolución de áreas con nombre se necesite. `None` = `none`. NO
    /// hereda. Plumb.
    pub grid_template_areas: Option<String>,
    /// `grid-row-start` (Fase 7.509). Parse opaco — la gramática
    /// `<grid-line>` se resuelve cuando un resolver de grid lo necesite.
    /// `None` = `auto`. NO hereda. Plumb.
    pub grid_row_start: Option<String>,
    /// `grid-row-end` (Fase 7.510). Parse opaco. `None` = `auto`. NO
    /// hereda. Plumb.
    pub grid_row_end: Option<String>,
    /// `grid-column-start` (Fase 7.511). Parse opaco. `None` = `auto`. NO
    /// hereda. Plumb.
    pub grid_column_start: Option<String>,
    /// `grid-column-end` (Fase 7.512). Parse opaco. `None` = `auto`. NO
    /// hereda. Plumb.
    pub grid_column_end: Option<String>,
    /// `text-emphasis-skip` (Fase 7.513). CSS Text Decoration 4: qué
    /// caracteres saltea la marca de énfasis. Default `Spaces`. **HEREDA**.
    /// Plumb.
    pub text_emphasis_skip: TextEmphasisSkip,
    /// `float` (CSS2.1 §9.5). Saca la caja del flujo a un lado. Default
    /// `None`. NO hereda. Plumb (la maquinaria de layout de floats no está).
    pub float: Float,
    /// `clear` (CSS2.1 §9.5.2). Baja el borde de margen por debajo de los
    /// floats del lado pedido. Default `None`. NO hereda. Plumb.
    pub clear: Clear,
    /// `clip` (CSS2.1, deprecada): rect de recorte para cajas posicionadas.
    /// Default `Auto`. NO hereda. Plumb (el recorte de render no está).
    pub clip: Clip,
    /// `page` (CSS Paged Media 3): el `@page` con nombre al que pertenece el
    /// elemento al fragmentar para impresión. `None` = `auto`. NO hereda
    /// (pero se propaga al árbol de fragmentación, fuera de alcance). Plumb.
    pub page: Option<String>,
    /// `d` (SVG 2 §6) como prop CSS. `None` = `none`; `Some(raw)` guarda el
    /// `path(...)` crudo (parse opaco). Default `None`. NO hereda. Plumb.
    pub d: Option<String>,
    /// `masonry-auto-flow` (CSS Grid 3 draft). Default `pack definite-first`.
    /// NO hereda. Plumb.
    pub masonry_auto_flow: MasonryAutoFlow,
    /// `justify-tracks` (CSS Grid 3 draft). Default vacío. NO hereda. Plumb.
    pub justify_tracks: Vec<JustifyContent>,
    /// `align-tracks` (CSS Grid 3 draft). Default vacío. NO hereda. Plumb.
    pub align_tracks: Vec<AlignContent>,
    /// `grid-template-columns: subgrid` (CSS Grid 2). Default `false`. NO
    /// hereda. Plumb (sin maquinaria de layout subgrid).
    pub grid_template_columns_subgrid: bool,
    /// `grid-template-rows: subgrid` (CSS Grid 2). Default `false`. NO hereda.
    pub grid_template_rows_subgrid: bool,
    /// `float-defer` (Fase 7.519). CSS Page Floats 3: cuántas regiones
    /// difiere el flotador. Default `None`. NO hereda. Plumb.
    pub float_defer: FloatDefer,
    /// `float-reference` (Fase 7.520). CSS Page Floats 3: contexto de
    /// flotación. Default `Inline`. NO hereda. Plumb.
    pub float_reference: FloatReference,
    /// `float-offset` (Fase 7.521). CSS Page Floats 3: desplazamiento en
    /// px del flotador. Default `0`. NO hereda. Plumb.
    pub float_offset: f32,
    /// `box-decoration-break` (Fase 7.522). CSS Fragmentation 4: cómo se
    /// trozan borde/padding/etc. en saltos. Default `Slice`. NO hereda.
    /// Plumb.
    pub box_decoration_break: BoxDecorationBreak,
    /// `line-snap` (Fase 7.523). CSS Line Grid: cómo se alinean las
    /// líneas a la grilla baseline. Default `None`. **HEREDA**. Plumb.
    pub line_snap: LineSnap,
    /// `line-grid` (Fase 7.524). CSS Line Grid: si el contenedor crea
    /// nueva grilla o se acopla a la heredada. Default `Match`. **HEREDA**.
    /// Plumb.
    pub line_grid: LineGrid,
    /// `initial-letter` shorthand (Fase 7.525). CSS Inline 3.
    /// Parse opaco — `None` = `normal` (sin drop-cap). **HEREDA**. Plumb.
    pub initial_letter: Option<String>,
    /// `highlight` (Fase 7.526). CSS Highlight API: nombre del highlight
    /// custom que se aplica. `None` = `none`. **HEREDA**. Plumb.
    pub highlight: Option<String>,
    /// `ruby-merge` (Fase 7.527). CSS Ruby 1: cómo se fusionan ruby
    /// adyacentes. Default `Separate`. **HEREDA**. Plumb.
    pub ruby_merge: RubyMerge,
    /// `text-spacing` shorthand (Fase 7.528). CSS Text 4. Parse opaco —
    /// `None` = `normal`. **HEREDA**. Plumb.
    pub text_spacing: Option<String>,
    /// `speak-as` (Fase 7.529). CSS Speech 1: cómo se vocaliza el texto.
    /// Default `Normal`. **HEREDA**. Plumb.
    pub speak_as: SpeakAs,
    /// `voice-balance` (Fase 7.530). CSS Speech 1: paneo estéreo de la
    /// voz, -100 (izq) a 100 (der). Default `0.0` (centro). **HEREDA**.
    /// Plumb.
    pub voice_balance: f32,
    /// `voice-pitch` (Fase 7.531). CSS Speech 1. Parse opaco — `None` =
    /// `medium`. **HEREDA**. Plumb.
    pub voice_pitch: Option<String>,
    /// `voice-rate` (Fase 7.532). CSS Speech 1. Parse opaco — `None` =
    /// `normal`. **HEREDA**. Plumb.
    pub voice_rate: Option<String>,
    /// `voice-volume` (Fase 7.533). CSS Speech 1. Parse opaco — `None` =
    /// `medium`. **HEREDA**. Plumb.
    pub voice_volume: Option<String>,
    /// `voice-family` (Fase 7.919). CSS Speech 1. Parse opaco — `None` =
    /// `preserve`. Plumb.
    pub voice_family: Option<String>,
    /// `voice-stress` (Fase 7.919). CSS Speech 1. Parse opaco — `None` =
    /// `normal`. Plumb.
    pub voice_stress: Option<String>,
    /// `voice-duration` (Fase 7.919). CSS Speech 1. Parse opaco — `None` =
    /// `auto`. Plumb.
    pub voice_duration: Option<String>,
    /// `pause-before` (Fase 7.534). CSS Speech 1: pausa antes del
    /// elemento. Parse opaco — `None` = `none`. **HEREDA**. Plumb.
    pub pause_before: Option<String>,
    /// `pause-after` (Fase 7.535). Análogo a `pause-before`. **HEREDA**.
    /// Plumb.
    pub pause_after: Option<String>,
    /// `rest-before` (Fase 7.536). CSS Speech 1: silencio antes/después
    /// del contenido (sin pausa fonética). Parse opaco — `None` = `none`.
    /// **HEREDA**. Plumb.
    pub rest_before: Option<String>,
    /// `rest-after` (Fase 7.537). Análogo a `rest-before`. **HEREDA**.
    /// Plumb.
    pub rest_after: Option<String>,
    /// `cue-fade-duration` (Fase 7.538). CSS Speech 1: duración del
    /// fade-in/out del cue audible en segundos. Default `0.0`. NO hereda.
    /// Plumb.
    pub cue_fade_duration: f32,
    /// `cue-before` (Fase 7.539). CSS Speech 1: sonido de cue antes del
    /// elemento. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub cue_before: Option<String>,
    /// `cue-after` (Fase 7.540). Análogo a `cue-before`. NO hereda. Plumb.
    pub cue_after: Option<String>,
    /// `cue` shorthand (Fase 7.541). CSS Speech 1. Parse opaco — `None` =
    /// `none`. NO hereda. Plumb.
    pub cue: Option<String>,
    /// `navigation-up` (Fase 7.542). CSS UI 3 legacy: cuál elemento
    /// recibe foco al presionar la flecha arriba. Parse opaco — `None`
    /// = `auto`. NO hereda. Plumb.
    pub navigation_up: Option<String>,
    /// `glyph-orientation-horizontal` (Fase 7.543). SVG 1.1 legacy: ángulo
    /// (0/90/180/270) que rota glyphs en bloques horizontales. Default
    /// `0.0`. **HEREDA**. Plumb.
    pub glyph_orientation_horizontal: f32,
    /// `navigation-down` (Fase 7.544). Análogo a `navigation-up`. NO
    /// hereda. Plumb.
    pub navigation_down: Option<String>,
    /// `navigation-left` (Fase 7.545). Análogo a `navigation-up`. NO
    /// hereda. Plumb.
    pub navigation_left: Option<String>,
    /// `navigation-right` (Fase 7.546). Análogo a `navigation-up`. NO
    /// hereda. Plumb.
    pub navigation_right: Option<String>,
    /// `counter-increment-style` (Fase 7.547). CSS Lists 4: estilo de
    /// numeración usado al incrementar el counter. Parse opaco — `None` =
    /// `decimal`. NO hereda. Plumb.
    pub counter_increment_style: Option<String>,
    /// `overflow-clip-box` (Fase 7.548). CSS Overflow legacy: en qué caja
    /// se recorta el contenido cuando hay overflow. Default `PaddingBox`.
    /// NO hereda. Plumb.
    pub overflow_clip_box: OverflowClipBox,
    /// `mask-border-source` (Fase 7.549). CSS Masking 1: imagen-fuente del
    /// borde-máscara. `None` = `none`. NO hereda. Plumb.
    pub mask_border_source: Option<String>,
    /// `mask-border-slice` (Fase 7.550). Recorte de la fuente en 9 zonas.
    /// `None` = `0`. NO hereda. Plumb.
    pub mask_border_slice: Option<String>,
    /// `mask-border-width` (Fase 7.551). Ancho de las zonas del borde.
    /// `None` = `auto`. NO hereda. Plumb.
    pub mask_border_width: Option<String>,
    /// `mask-border-outset` (Fase 7.552). Cuánto sobresale el borde de la
    /// caja. `None` = `0`. NO hereda. Plumb.
    pub mask_border_outset: Option<String>,
    /// `mask-border-repeat` (Fase 7.553). Cómo se ajustan los bordes/centro.
    /// Default `Stretch`. NO hereda. Plumb.
    pub mask_border_repeat: MaskBorderRepeat,
    /// `mask-border-mode` (Fase 7.554). CSS Masking 1: si la fuente se
    /// interpreta por luminancia o por alpha. Default `Alpha`. NO hereda.
    /// Plumb.
    pub mask_border_mode: MaskBorderMode,
    /// `mask-border` shorthand (Fase 7.909). Opaco. Default `None`. NO hereda.
    pub mask_border: Option<String>,
    /// `caret-animation` (Fase 7.555). CSS UI 4: si el caret parpadea
    /// (`auto`) o queda fijo (`manual`). Default `Auto`. HEREDA. Plumb.
    pub caret_animation: CaretAnimation,
    /// `scroll-marker-group` (Fase 7.556). CSS Overflow 5: dónde se ubica
    /// el grupo de marcadores de scroll. Default `None`. NO hereda. Plumb.
    pub scroll_marker_group: ScrollMarkerGroup,
    /// `scroll-initial-target` (Fase 7.557). CSS Overflow 5: si el elemento
    /// es el target inicial de scroll del contenedor. Default `None`. NO
    /// hereda. Plumb.
    pub scroll_initial_target: ScrollInitialTarget,
    /// `corner-shape` (Fase 7.558). CSS Borders 4: forma de las esquinas
    /// redondeadas (round/bevel/notch/scoop/squircle…). Parse opaco —
    /// `None` = `round`. NO hereda. Plumb.
    pub corner_shape: Option<String>,
    /// `hyphenate-limit-lines` (Fase 7.559). CSS Text 4: máx. de líneas
    /// consecutivas terminadas en guion. `None` = `no-limit`. HEREDA. Plumb.
    pub hyphenate_limit_lines: Option<u32>,
    /// `hyphenate-limit-last` (Fase 7.560). CSS Text 4: restringe el guion
    /// en la última línea de un bloque/columna/página. Default `None`.
    /// HEREDA. Plumb.
    pub hyphenate_limit_last: HyphenateLimitLast,
    /// `hyphenate-limit-zone` (Fase 7.561). CSS Text 4: ancho máx. de la
    /// zona sin justificar antes de guionar. `None` = `0`. HEREDA. Plumb.
    pub hyphenate_limit_zone: Option<String>,
    /// `interest-target` (Fase 7.562). HTML/CSS interest invokers: id del
    /// elemento que recibe el interés. `None` = sin target. NO hereda. Plumb.
    pub interest_target: Option<String>,
    /// `scroll-start` + longhands lógicos (CSS Scroll Snap 2). Posición inicial
    /// del scroll. `None` = `auto`. NO hereda. Plumb. Fase 7.928.
    pub scroll_start: Option<String>,
    pub scroll_start_block: Option<String>,
    pub scroll_start_inline: Option<String>,
    /// `scroll-start-target` + longhands lógicos. `None` = `none`. Fase 7.928.
    pub scroll_start_target: Option<String>,
    pub scroll_start_target_block: Option<String>,
    pub scroll_start_target_inline: Option<String>,
    /// `interest-delay-start` (Fase 7.563). Retardo antes de mostrar el
    /// interés. `None` = `normal`. NO hereda. Plumb.
    pub interest_delay_start: Option<String>,
    /// `interest-delay-end` (Fase 7.564). Retardo antes de ocultar el
    /// interés. `None` = `normal`. NO hereda. Plumb.
    pub interest_delay_end: Option<String>,
    /// `azimuth` (Fase 7.565). CSS 2.1 aural: posición horizontal de la
    /// fuente sonora. `None` = `center`. HEREDA. Plumb.
    pub azimuth: Option<String>,
    /// `elevation` (Fase 7.566). CSS 2.1 aural: posición vertical de la
    /// fuente sonora. `None` = `level`. HEREDA. Plumb.
    pub elevation: Option<String>,
    /// `richness` (Fase 7.567). CSS 2.1 aural: brillo/riqueza de la voz
    /// (0–100). Default `50.0`. HEREDA. Plumb.
    pub richness: f32,
    /// `speak-header` (Fase 7.930). CSS 2.1 aural: cómo se anuncian las
    /// cabeceras de tabla. `None` = `once`. HEREDA. Plumb.
    pub speak_header: Option<String>,
    /// `pitch-range` (Fase 7.930). CSS 2.1 aural: variación de tono 0–100
    /// (50 = normal). HEREDA. Plumb.
    pub pitch_range: f32,
    /// `margin-trim` (CSS Box 4, Fase 7.931). `None` = `none`. NO hereda. Plumb.
    pub margin_trim: Option<String>,
    /// `margin-break` (CSS Fragmentation 4, Fase 7.931). `None` = `auto`. Plumb.
    pub margin_break: Option<String>,
    /// `input-security` (CSS UI 4, Fase 7.931). `None` = `auto`. NO hereda. Plumb.
    pub input_security: Option<String>,
    /// `border-boundary` (CSS Round Display 1, Fase 7.931). `None` = `none`. Plumb.
    pub border_boundary: Option<String>,
    /// `shape-inside` (CSS Shapes 2, Fase 7.932). `None` = `auto`. NO hereda. Plumb.
    pub shape_inside: Option<String>,
    /// `speak-punctuation` (CSS 2.1 aural, Fase 7.932). `None` = `none`. HEREDA. Plumb.
    pub speak_punctuation: Option<String>,
    /// `speak-numeral` (CSS 2.1 aural, Fase 7.932). `None` = `continuous`. HEREDA. Plumb.
    pub speak_numeral: Option<String>,
    /// `stress` (Fase 7.568). CSS 2.1 aural: énfasis de la entonación
    /// (0–100). Default `50.0`. HEREDA. Plumb.
    pub stress: f32,
    /// `pitch` (Fase 7.569). CSS 2.1 aural: tono medio de la voz. `None` =
    /// `medium`. HEREDA. Plumb.
    pub pitch: Option<String>,
    /// `speech-rate` (Fase 7.570). CSS 2.1 aural: velocidad del habla.
    /// `None` = `medium`. HEREDA. Plumb.
    pub speech_rate: Option<String>,
    /// `volume` (Fase 7.571). CSS 2.1 aural: volumen medio. `None` =
    /// `medium`. HEREDA. Plumb.
    pub volume: Option<String>,
    /// `speak` (Fase 7.572). CSS 2.1 aural: si el contenido se renderiza
    /// auditivamente y cómo. Default `Normal`. HEREDA. Plumb.
    pub speak: Speak,
    /// `play-during` (Fase 7.573). CSS 2.1 aural: sonido de fondo durante
    /// el elemento. `None` = `auto`. NO hereda. Plumb.
    pub play_during: Option<String>,
    /// `text-decoration-skip` (Fase 7.574). CSS Text Decor 4: qué partes
    /// salta la línea de decoración (shorthand legacy). Parse opaco —
    /// `None` = `auto`. HEREDA. Plumb.
    pub text_decoration_skip: Option<String>,
    /// `text-decoration-skip-box` (Fase 7.575). Si la decoración salta el
    /// margen de la caja. Default `None`. HEREDA. Plumb.
    pub text_decoration_skip_box: TextDecorationSkipBox,
    /// `text-decoration-skip-self` (Fase 7.576). Si el elemento salta su
    /// propia decoración heredada. Parse opaco — `None` = `auto`. HEREDA.
    /// Plumb.
    pub text_decoration_skip_self: Option<String>,
    /// `text-decoration-skip-spaces` (Fase 7.577). Si se saltan los
    /// espacios. Parse opaco — `None` = `start end`. HEREDA. Plumb.
    pub text_decoration_skip_spaces: Option<String>,
    /// `text-decoration-skip-inset` (Fase 7.578). Si la decoración se
    /// recorta hacia adentro. Default `None`. HEREDA. Plumb.
    pub text_decoration_skip_inset: TextDecorationSkipInset,
    /// `-webkit-text-stroke-width` (Fase 7.579). Ancho del trazo del texto,
    /// px. Default `0.0`. HEREDA. Plumb.
    pub webkit_text_stroke_width: f32,
    /// `-webkit-text-stroke-color` (Fase 7.580). Color del trazo. Parse
    /// opaco — `None` = `currentColor`. HEREDA. Plumb.
    pub webkit_text_stroke_color: Option<String>,
    /// `-webkit-text-fill-color` (Fase 7.581). Color de relleno del texto.
    /// Parse opaco — `None` = `currentColor`. HEREDA. Plumb.
    pub webkit_text_fill_color: Option<String>,
    /// `font-smooth` (Fase 7.582). Control no estándar del antialiasing de
    /// fuentes. Parse opaco — `None` = `auto`. HEREDA. Plumb.
    pub font_smooth: Option<String>,
    /// `text-group-align` (Fase 7.583). CSS Text 4: alineación compartida
    /// de un grupo de líneas. Default `None`. NO hereda. Plumb.
    pub text_group_align: TextGroupAlign,
    /// `continue` (Fase 7.584). CSS Overflow 4: qué pasa con el contenido
    /// que no cabe (fragmenta vs descarta). Default `Auto`. NO hereda.
    /// Reservado en Rust → campo `continue_`. Plumb.
    pub continue_: Continue,
    /// `block-ellipsis` (Fase 7.585). CSS Overflow 4: cadena que marca el
    /// truncado por bloque. Parse opaco — `None` = `none`. HEREDA. Plumb.
    pub block_ellipsis: Option<String>,
    /// `max-lines` (Fase 7.586). CSS Overflow 4: máx. de líneas antes de
    /// fragmentar/recortar. `None` = `none`. NO hereda. Plumb.
    pub max_lines: Option<u32>,
    /// `region-fragment` (Fase 7.587). CSS Regions 1: cómo se rompe la
    /// última región. Default `Auto`. NO hereda. Plumb.
    pub region_fragment: RegionFragment,
    /// `overflow-style` (Fase 7.588). CSS Marquee/Basic UI legacy: mecanismo
    /// preferido de scroll del overflow (scrollbar/panner/move/marquee).
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub overflow_style: Option<String>,
    /// `marquee-style` (Fase 7.589). CSS Marquee: modo de desplazamiento
    /// (scroll/slide/alternate). Default `Scroll`. NO hereda. Plumb.
    pub marquee_style: MarqueeStyle,
    /// `marquee-direction` (Fase 7.590). Sentido del desplazamiento.
    /// Default `Forward`. NO hereda. Plumb.
    pub marquee_direction: MarqueeDirection,
    /// `marquee-speed` (Fase 7.591). Velocidad. Default `Normal`. NO
    /// hereda. Plumb.
    pub marquee_speed: MarqueeSpeed,
    /// `marquee-loop` (Fase 7.592). Nº de repeticiones. `None` = `infinite`.
    /// NO hereda. Plumb.
    pub marquee_loop: Option<i32>,
    /// `marquee-increment` (Fase 7.593). Distancia por paso. Parse opaco —
    /// `None` = `6px`. NO hereda. Plumb.
    pub marquee_increment: Option<String>,
    /// `nav-index` (Fase 7.594). CSS UI 3 legacy: orden de navegación
    /// secuencial. Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub nav_index: Option<String>,
    /// `nav-up` (Fase 7.595). CSS UI 3 legacy (nombre viejo de
    /// `navigation-up`): target al navegar hacia arriba. `None` = `auto`.
    /// NO hereda. Plumb.
    pub nav_up: Option<String>,
    /// `nav-down` (Fase 7.596). Análogo a `nav-up`. `None` = `auto`. NO
    /// hereda. Plumb.
    pub nav_down: Option<String>,
    /// `nav-left` (Fase 7.597). Análogo a `nav-up`. `None` = `auto`. NO
    /// hereda. Plumb.
    pub nav_left: Option<String>,
    /// `nav-right` (Fase 7.598). Análogo a `nav-up`. `None` = `auto`. NO
    /// hereda. Plumb.
    pub nav_right: Option<String>,
    /// `-webkit-box-orient` (Fase 7.599). Flexbox viejo: eje del box.
    /// Parse opaco — `None` = `inline-axis`. NO hereda. Plumb.
    pub webkit_box_orient: Option<String>,
    /// `-webkit-box-direction` (Fase 7.600). Sentido del eje. Parse opaco —
    /// `None` = `normal`. NO hereda. Plumb.
    pub webkit_box_direction: Option<String>,
    /// `-webkit-box-align` (Fase 7.601). Alineación transversal. Parse
    /// opaco — `None` = `stretch`. NO hereda. Plumb.
    pub webkit_box_align: Option<String>,
    /// `-webkit-box-pack` (Fase 7.602). Alineación principal. Parse opaco —
    /// `None` = `start`. NO hereda. Plumb.
    pub webkit_box_pack: Option<String>,
    /// `-webkit-box-flex` (Fase 7.603). Factor de crecimiento. Default
    /// `0.0`. NO hereda. Plumb.
    pub webkit_box_flex: f32,
    /// `-webkit-box-ordinal-group` (Fase 7.604). Orden visual del ítem en
    /// el box viejo. `None` = `1`. NO hereda. Plumb.
    pub webkit_box_ordinal_group: Option<u32>,
    /// `-webkit-font-smoothing` (Fase 7.605). Antialiasing no estándar
    /// (WebKit). Parse opaco — `None` = `auto`. HEREDA. Plumb.
    pub webkit_font_smoothing: Option<String>,
    /// `-moz-osx-font-smoothing` (Fase 7.606). Antialiasing no estándar
    /// (Gecko/macOS). Parse opaco — `None` = `auto`. HEREDA. Plumb.
    pub moz_osx_font_smoothing: Option<String>,
    /// `-webkit-tap-highlight-color` (Fase 7.607). Color del flash al tocar
    /// en móviles. Parse opaco. NO hereda. Plumb.
    pub webkit_tap_highlight_color: Option<String>,
    /// `zoom` (Fase 7.608). Factor de escala no estándar (en vías de
    /// estandarización). Parse opaco — `None` = `normal`. NO hereda. Plumb.
    pub zoom: Option<String>,
    /// `column-break-before` (Fase 7.614). CSS Multicol legacy (alias viejo
    /// de `break-before`). Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub column_break_before: Option<String>,
    /// `column-break-after` (Fase 7.615). Análogo. `None` = `auto`. NO
    /// hereda. Plumb.
    pub column_break_after: Option<String>,
    /// `column-break-inside` (Fase 7.616). Análogo. `None` = `auto`. NO
    /// hereda. Plumb.
    pub column_break_inside: Option<String>,
    /// `user-modify` (Fase 7.617). No estándar: si el usuario puede editar
    /// el contenido. Parse opaco — `None` = `read-only`. HEREDA. Plumb.
    pub user_modify: Option<String>,
    /// `-webkit-touch-callout` (Fase 7.618). iOS: muestra/oculta el callout
    /// al mantener pulsado. Parse opaco — `None` = `default`. HEREDA. Plumb.
    pub webkit_touch_callout: Option<String>,
    /// `-webkit-user-drag` (Fase 7.619). Si el elemento es arrastrable.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_user_drag: Option<String>,
    /// `-webkit-rtl-ordering` (Fase 7.620). Orden lógico vs visual en RTL.
    /// Parse opaco — `None` = `logical`. HEREDA. Plumb.
    pub webkit_rtl_ordering: Option<String>,
    /// `-webkit-text-security` (Fase 7.621). Glifo que enmascara el texto
    /// (disc/circle/square/none). Parse opaco — `None` = `none`. HEREDA.
    /// Plumb.
    pub webkit_text_security: Option<String>,
    /// `-webkit-nbsp-mode` (Fase 7.622). Tratamiento de los espacios
    /// duros. Parse opaco — `None` = `normal`. HEREDA. Plumb.
    pub webkit_nbsp_mode: Option<String>,
    /// `-webkit-locale` (Fase 7.623). Locale para reglas dependientes del
    /// idioma. Parse opaco — `None` = `auto`. HEREDA. Plumb.
    pub webkit_locale: Option<String>,
    /// `-webkit-column-axis` (Fase 7.624). Eje de flujo de columnas
    /// (horizontal/vertical/auto). Parse opaco — `None` = `auto`. NO
    /// hereda. Plumb.
    pub webkit_column_axis: Option<String>,
    /// `-webkit-column-progression` (Fase 7.625). Sentido de avance de las
    /// columnas (normal/reverse). Parse opaco — `None` = `normal`. NO
    /// hereda. Plumb.
    pub webkit_column_progression: Option<String>,
    /// `-webkit-app-region` (Fase 7.626). Chrome/Electron: zona arrastrable
    /// de la ventana (drag/no-drag). Parse opaco — `None` = `none`. NO
    /// hereda. Plumb.
    pub webkit_app_region: Option<String>,
    /// `-webkit-highlight` (Fase 7.627). Nombre de highlight personalizado.
    /// Parse opaco — `None` = `none`. HEREDA. Plumb.
    pub webkit_highlight: Option<String>,
    /// `-webkit-box-reflect` (Fase 7.628). Reflejo del elemento
    /// (dirección + offset + máscara). Parse opaco — `None` = `none`. NO
    /// hereda. Plumb.
    pub webkit_box_reflect: Option<String>,
    /// `-webkit-mask-composite` (Fase 7.644). Modo de composición de las
    /// capas de máscara (add/subtract/intersect/exclude). Parse opaco —
    /// `None` = `add`. NO hereda. Plumb.
    pub webkit_mask_composite: Option<String>,
    /// `-webkit-mask-position-x` (Fase 7.645). Longhand horizontal de la
    /// posición de máscara. Parse opaco — `None` = `center`. NO hereda. Plumb.
    pub webkit_mask_position_x: Option<String>,
    /// `-webkit-mask-position-y` (Fase 7.646). Longhand vertical de la
    /// posición de máscara. Parse opaco — `None` = `center`. NO hereda. Plumb.
    pub webkit_mask_position_y: Option<String>,
    /// `-webkit-mask-repeat-x` (Fase 7.647). Longhand horizontal del repeat
    /// de máscara. Parse opaco — `None` = `repeat`. NO hereda. Plumb.
    pub webkit_mask_repeat_x: Option<String>,
    /// `-webkit-mask-repeat-y` (Fase 7.648). Longhand vertical del repeat
    /// de máscara. Parse opaco — `None` = `repeat`. NO hereda. Plumb.
    pub webkit_mask_repeat_y: Option<String>,
    /// `-webkit-margin-start` (Fase 7.649). Alias legacy de
    /// `margin-inline-start`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_margin_start: Option<String>,
    /// `-webkit-margin-end` (Fase 7.650). Alias legacy de
    /// `margin-inline-end`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_margin_end: Option<String>,
    /// `-webkit-margin-before` (Fase 7.651). Alias legacy de
    /// `margin-block-start`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_margin_before: Option<String>,
    /// `-webkit-margin-after` (Fase 7.652). Alias legacy de
    /// `margin-block-end`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_margin_after: Option<String>,
    /// `-webkit-padding-start` (Fase 7.653). Alias legacy de
    /// `padding-inline-start`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_padding_start: Option<String>,
    /// `-webkit-padding-end` (Fase 7.654). Alias legacy de
    /// `padding-inline-end`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_padding_end: Option<String>,
    /// `-webkit-padding-before` (Fase 7.655). Alias legacy de
    /// `padding-block-start`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_padding_before: Option<String>,
    /// `-webkit-padding-after` (Fase 7.656). Alias legacy de
    /// `padding-block-end`. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_padding_after: Option<String>,
    /// `-webkit-logical-width` (Fase 7.657). Alias legacy de `inline-size`.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_logical_width: Option<String>,
    /// `-webkit-logical-height` (Fase 7.658). Alias legacy de `block-size`.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_logical_height: Option<String>,
    /// `-webkit-transform-origin-x` (Fase 7.664). Longhand por-eje del origen
    /// de transform. Parse opaco — `None` = `50%`. NO hereda. Plumb.
    pub webkit_transform_origin_x: Option<String>,
    /// `-webkit-transform-origin-y` (Fase 7.665). Longhand vertical del origen
    /// de transform. Parse opaco — `None` = `50%`. NO hereda. Plumb.
    pub webkit_transform_origin_y: Option<String>,
    /// `-webkit-transform-origin-z` (Fase 7.666). Longhand de profundidad del
    /// origen de transform. Parse opaco — `None` = `0`. NO hereda. Plumb.
    pub webkit_transform_origin_z: Option<String>,
    /// `-webkit-perspective-origin-x` (Fase 7.667). Longhand horizontal del
    /// origen de perspectiva. Parse opaco — `None` = `50%`. NO hereda. Plumb.
    pub webkit_perspective_origin_x: Option<String>,
    /// `-webkit-perspective-origin-y` (Fase 7.668). Longhand vertical del
    /// origen de perspectiva. Parse opaco — `None` = `50%`. NO hereda. Plumb.
    pub webkit_perspective_origin_y: Option<String>,
    /// `-webkit-min-logical-width` (Fase 7.669). Alias legacy de
    /// `min-inline-size`. Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_min_logical_width: Option<String>,
    /// `-webkit-max-logical-width` (Fase 7.670). Alias legacy de
    /// `max-inline-size`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_max_logical_width: Option<String>,
    /// `-webkit-min-logical-height` (Fase 7.671). Alias legacy de
    /// `min-block-size`. Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_min_logical_height: Option<String>,
    /// `-webkit-max-logical-height` (Fase 7.672). Alias legacy de
    /// `max-block-size`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_max_logical_height: Option<String>,
    /// `-webkit-background-composite` (Fase 7.673). Modo de composición de las
    /// capas de fondo. Parse opaco — `None` = `source-over`. NO hereda. Plumb.
    pub webkit_background_composite: Option<String>,
    /// `-webkit-border-before` (Fase 7.674). Shorthand legacy de
    /// `border-block-start`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_before: Option<String>,
    /// `-webkit-border-after` (Fase 7.675). Shorthand legacy de
    /// `border-block-end`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_after: Option<String>,
    /// `-webkit-border-start` (Fase 7.676). Shorthand legacy de
    /// `border-inline-start`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_start: Option<String>,
    /// `-webkit-border-end` (Fase 7.677). Shorthand legacy de
    /// `border-inline-end`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_end: Option<String>,
    /// `-webkit-border-horizontal-spacing` (Fase 7.678). Longhand legacy del
    /// eje horizontal de `border-spacing`. Parse opaco — `None` = `0`.
    /// HEREDA (como border-spacing). Plumb.
    pub webkit_border_horizontal_spacing: Option<String>,
    /// `-webkit-flow-into` (Fase 7.679). CSS Regions (spec abandonado): manda
    /// el elemento a un named flow. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_flow_into: Option<String>,
    /// `-webkit-flow-from` (Fase 7.680). CSS Regions: la región consume un
    /// named flow. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_flow_from: Option<String>,
    /// `-webkit-region-break-before` (Fase 7.681). Quiebre de región antes.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_region_break_before: Option<String>,
    /// `-webkit-region-break-after` (Fase 7.682). Quiebre de región después.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_region_break_after: Option<String>,
    /// `-webkit-region-break-inside` (Fase 7.683). Quiebre de región adentro.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_region_break_inside: Option<String>,
    /// `-webkit-border-before-color` (Fase 7.698). Longhand legacy de
    /// `border-block-start-color`. Parse opaco — `None` = `currentcolor`. NO hereda. Plumb.
    pub webkit_border_before_color: Option<String>,
    /// `-webkit-border-before-style` (Fase 7.699). Longhand legacy de
    /// `border-block-start-style`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_before_style: Option<String>,
    /// `-webkit-border-before-width` (Fase 7.700). Longhand legacy de
    /// `border-block-start-width`. Parse opaco — `None` = `medium`. NO hereda. Plumb.
    pub webkit_border_before_width: Option<String>,
    /// `-webkit-border-after-color` (Fase 7.701). Longhand legacy de
    /// `border-block-end-color`. Parse opaco — `None` = `currentcolor`. NO hereda. Plumb.
    pub webkit_border_after_color: Option<String>,
    /// `-webkit-border-after-style` (Fase 7.702). Longhand legacy de
    /// `border-block-end-style`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_after_style: Option<String>,
    /// `-webkit-border-after-width` (Fase 7.703). Longhand legacy de
    /// `border-block-end-width`. Parse opaco — `None` = `medium`. NO hereda. Plumb.
    pub webkit_border_after_width: Option<String>,
    /// `-webkit-border-start-color` (Fase 7.704). Longhand legacy de
    /// `border-inline-start-color`. Parse opaco — `None` = `currentcolor`. NO hereda. Plumb.
    pub webkit_border_start_color: Option<String>,
    /// `-webkit-border-start-style` (Fase 7.705). Longhand legacy de
    /// `border-inline-start-style`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_start_style: Option<String>,
    /// `-webkit-border-start-width` (Fase 7.706). Longhand legacy de
    /// `border-inline-start-width`. Parse opaco — `None` = `medium`. NO hereda. Plumb.
    pub webkit_border_start_width: Option<String>,
    /// `-webkit-border-end-color` (Fase 7.707). Longhand legacy de
    /// `border-inline-end-color`. Parse opaco — `None` = `currentcolor`. NO hereda. Plumb.
    pub webkit_border_end_color: Option<String>,
    /// `-webkit-border-end-style` (Fase 7.708). Longhand legacy de
    /// `border-inline-end-style`. Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_border_end_style: Option<String>,
    /// `-webkit-border-end-width` (Fase 7.709). Longhand legacy de
    /// `border-inline-end-width`. Parse opaco — `None` = `medium`. NO hereda. Plumb.
    pub webkit_border_end_width: Option<String>,
    /// `-webkit-margin-top-collapse` (Fase 7.730). Control no estándar del
    /// colapso del margen superior. Parse opaco — `None` = `collapse`. NO hereda. Plumb.
    pub webkit_margin_top_collapse: Option<String>,
    /// `-webkit-margin-bottom-collapse` (Fase 7.731). Ídem margen inferior.
    /// Parse opaco — `None` = `collapse`. NO hereda. Plumb.
    pub webkit_margin_bottom_collapse: Option<String>,
    /// `-webkit-margin-collapse` (Fase 7.732). Shorthand de top/bottom-collapse.
    /// Parse opaco — `None` = `collapse`. NO hereda. Plumb.
    pub webkit_margin_collapse: Option<String>,
    /// `-webkit-border-vertical-spacing` (Fase 7.733). Longhand legacy del eje
    /// vertical de `border-spacing`. Parse opaco — `None` = `0`. HEREDA. Plumb.
    pub webkit_border_vertical_spacing: Option<String>,
    /// `-webkit-mask-source-type` (Fase 7.734). Interpretación de la máscara
    /// (alpha/luminance). Parse opaco — `None` = `alpha`. NO hereda. Plumb.
    pub webkit_mask_source_type: Option<String>,
    /// `-webkit-marquee-direction` (Fase 7.750). Eje/sentido del marquee legacy.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_marquee_direction: Option<String>,
    /// `-webkit-marquee-increment` (Fase 7.751). Salto por iteración del marquee.
    /// Parse opaco — `None` = `6px`. NO hereda. Plumb.
    pub webkit_marquee_increment: Option<String>,
    /// `-webkit-marquee-repetition` (Fase 7.752). Repeticiones del marquee.
    /// Parse opaco — `None` = `infinite`. NO hereda. Plumb.
    pub webkit_marquee_repetition: Option<String>,
    /// `-webkit-marquee-speed` (Fase 7.753). Velocidad del marquee.
    /// Parse opaco — `None` = `normal`. NO hereda. Plumb.
    pub webkit_marquee_speed: Option<String>,
    /// `-webkit-marquee-style` (Fase 7.754). Modo del marquee (scroll/slide/alternate).
    /// Parse opaco — `None` = `scroll`. NO hereda. Plumb.
    pub webkit_marquee_style: Option<String>,
    /// `-webkit-overflow-scrolling` (Fase 7.755). Inercia de scroll táctil legacy.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_overflow_scrolling: Option<String>,
    /// `-webkit-line-grid` (Fase 7.756). Grilla de línea base nombrada.
    /// Parse opaco — `None` = `none`. NO hereda. Plumb.
    pub webkit_line_grid: Option<String>,
    /// `-webkit-cursor-visibility` (Fase 7.757). Auto-ocultado del cursor.
    /// Parse opaco — `None` = `auto`. NO hereda. Plumb.
    pub webkit_cursor_visibility: Option<String>,
    /// `-webkit-border-fit` (Fase 7.758). Ajuste del borde al contenido.
    /// Parse opaco — `None` = `border`. NO hereda. Plumb.
    pub webkit_border_fit: Option<String>,
    /// `-webkit-color-correction` (Fase 7.759). Corrección de color (default/sRGB).
    /// Parse opaco — `None` = `default`. HEREDA. Plumb.
    pub webkit_color_correction: Option<String>,
    pub text_shadows: Vec<TextShadow>,
    /// Cadena de transformaciones (translate/scale/rotate) aplicadas
    /// en orden. Vacío = identidad. Las props individuales `translate`/
    /// `rotate`/`scale` (Fase 7.826-7.828) se prependean acá al cierre del
    /// compute, en el orden CSS Transforms 2 (translate→rotate→scale→list).
    pub transforms: Vec<Transform>,
    /// Prop individual `translate` (CSS Transforms 2). `None` = sin
    /// traslación. Se compone en `transforms` al cierre del compute.
    pub translate: Option<Transform>,
    /// Prop individual `rotate` (CSS Transforms 2).
    pub rotate: Option<Transform>,
    /// Prop individual `scale` (CSS Transforms 2).
    pub scale: Option<Transform>,
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
    // === Fase 7.966-7.985 — propiedades estándar legacy/de nicho (plumb opaco) ===
    /// `spatial-navigation-action` (CSS Spatial Navigation 1). `None` = `auto`.
    /// NO hereda. Plumb. Fase 7.966.
    pub spatial_navigation_action: Option<String>,
    /// `spatial-navigation-contain` (CSS Spatial Navigation 1). `None` = `auto`.
    /// NO hereda. Plumb. Fase 7.967.
    pub spatial_navigation_contain: Option<String>,
    /// `spatial-navigation-function` (CSS Spatial Navigation 1). `None` =
    /// `normal`. NO hereda. Plumb. Fase 7.968.
    pub spatial_navigation_function: Option<String>,
    /// `wrap-flow` (CSS Exclusions 1). `None` = `auto`. NO hereda. Plumb. Fase 7.969.
    pub wrap_flow: Option<String>,
    /// `wrap-through` (CSS Exclusions 1). `None` = `wrap`. NO hereda. Plumb. Fase 7.970.
    pub wrap_through: Option<String>,
    /// `flow-into` (CSS Regions 1). `None` = `none`. NO hereda. Plumb. Fase 7.971.
    pub flow_into: Option<String>,
    /// `flow-from` (CSS Regions 1). `None` = `none`. NO hereda. Plumb. Fase 7.972.
    pub flow_from: Option<String>,
    /// `mark-before` (CSS Speech, draft aural). `None` = `none`. NO hereda. Plumb. Fase 7.973.
    pub mark_before: Option<String>,
    /// `mark-after` (CSS Speech, draft aural). `None` = `none`. NO hereda. Plumb. Fase 7.974.
    pub mark_after: Option<String>,
    /// `text-align-all` (CSS Text 4). Alinea todas las líneas (incluida la
    /// última). `None` = `start`. HEREDA. Plumb. Fase 7.975.
    pub text_align_all: Option<String>,
    /// `min-zoom` (CSS Device Adaptation, `@viewport`). `None` = `auto`.
    /// NO hereda. Plumb. Fase 7.976.
    pub min_zoom: Option<String>,
    /// `max-zoom` (CSS Device Adaptation, `@viewport`). `None` = `auto`.
    /// NO hereda. Plumb. Fase 7.977.
    pub max_zoom: Option<String>,
    /// `user-zoom` (CSS Device Adaptation, `@viewport`). `None` = `zoom`.
    /// NO hereda. Plumb. Fase 7.978.
    pub user_zoom: Option<String>,
    /// `viewport-fit` (CSS Round Display 1 / `@viewport`). `None` = `auto`.
    /// NO hereda. Plumb. Fase 7.979.
    pub viewport_fit: Option<String>,
    /// `ime-mode` (CSS UI 3, deprecated). `None` = `auto`. NO hereda. Plumb. Fase 7.980.
    pub ime_mode: Option<String>,
    /// `kerning` (SVG 1.1 presentation attr, deprecated). `None` = `auto`.
    /// HEREDA (propiedad de texto SVG). Plumb. Fase 7.981.
    pub kerning: Option<String>,
    /// `enable-background` (SVG 1.1, deprecated). `None` = `accumulate`.
    /// NO hereda. Plumb. Fase 7.982.
    pub enable_background: Option<String>,
    /// `color-profile` (SVG 1.1, deprecated). `None` = `auto`. HEREDA. Plumb. Fase 7.983.
    pub color_profile: Option<String>,
    /// `voice-range` (CSS Speech 1). `None` = `medium`. HEREDA. Plumb. Fase 7.984.
    pub voice_range: Option<String>,
    /// `text-security` (proposed; `-webkit-text-security`). `None` = `none`.
    /// NO hereda. Plumb. Fase 7.985.
    pub text_security: Option<String>,
    // === Fase 7.986-7.1005 — props de nicho (CSS Shapes/Inline/Line-Layout, plumb opaco) ===
    /// `shape-padding` (CSS Shapes 2). `None` = `0`. NO hereda. Plumb. Fase 7.986.
    pub shape_padding: Option<String>,
    /// `line-fit-edge` (CSS Inline 3). `None` = `leading`. NO hereda. Plumb. Fase 7.987.
    pub line_fit_edge: Option<String>,
    /// `inline-sizing` (CSS Inline 3). `None` = `normal`. NO hereda. Plumb. Fase 7.988.
    pub inline_sizing: Option<String>,
    /// `box-snap` (CSS Line Grid 1). `None` = `none`. NO hereda. Plumb. Fase 7.989.
    pub box_snap: Option<String>,
    /// `copy-into` (CSS GCPM 3). `None` = `none`. NO hereda. Plumb. Fase 7.990.
    pub copy_into: Option<String>,
    /// `line-stacking` shorthand (CSS Line Layout 3). `None` = initial.
    /// HEREDA. Plumb. Fase 7.991.
    pub line_stacking: Option<String>,
    /// `line-stacking-ruby` (CSS Line Layout 3). `None` = `exclude-ruby`.
    /// HEREDA. Plumb. Fase 7.992.
    pub line_stacking_ruby: Option<String>,
    /// `line-stacking-shift` (CSS Line Layout 3). `None` = `consider-shifts`.
    /// HEREDA. Plumb. Fase 7.993.
    pub line_stacking_shift: Option<String>,
    /// `line-stacking-strategy` (CSS Line Layout 3). `None` = `inline-line-height`.
    /// HEREDA. Plumb. Fase 7.994.
    pub line_stacking_strategy: Option<String>,
    /// `inline-box-align` (CSS Line Layout 3). `None` = `last`. NO hereda. Plumb. Fase 7.995.
    pub inline_box_align: Option<String>,
    /// `alignment-adjust` (CSS Line Layout 3 / SVG 1.2). `None` = `auto`.
    /// NO hereda. Plumb. Fase 7.996.
    pub alignment_adjust: Option<String>,
    /// `text-height` (CSS Line Layout 3). `None` = `auto`. HEREDA. Plumb. Fase 7.997.
    pub text_height: Option<String>,
    /// `drop-initial-size` (CSS Line Layout 3). `None` = `auto`. NO hereda. Plumb. Fase 7.998.
    pub drop_initial_size: Option<String>,
    /// `drop-initial-value` (CSS Line Layout 3). `None` = `initial`. NO hereda. Plumb. Fase 7.999.
    pub drop_initial_value: Option<String>,
    /// `drop-initial-before-align` (CSS Line Layout 3). `None` = `caps-height`.
    /// NO hereda. Plumb. Fase 7.1000.
    pub drop_initial_before_align: Option<String>,
    /// `drop-initial-after-align` (CSS Line Layout 3). `None` = `baseline`.
    /// NO hereda. Plumb. Fase 7.1001.
    pub drop_initial_after_align: Option<String>,
    /// `drop-initial-before-adjust` (CSS Line Layout 3). `None` = `before-edge`.
    /// NO hereda. Plumb. Fase 7.1002.
    pub drop_initial_before_adjust: Option<String>,
    /// `drop-initial-after-adjust` (CSS Line Layout 3). `None` = `after-edge`.
    /// NO hereda. Plumb. Fase 7.1003.
    pub drop_initial_after_adjust: Option<String>,
    /// `block-progression` (MS/SVG Tiny legacy, predecesor de `writing-mode`).
    /// `None` = `tb`. HEREDA. Plumb. Fase 7.1004.
    pub block_progression: Option<String>,
    /// `snap-height` (CSS Rhythmic Sizing, draft temprano). `None` = `none`.
    /// HEREDA. Plumb. Fase 7.1005.
    pub snap_height: Option<String>,
    // === Fase 7.1031-7.1034 — CSS Scroll Snap v0 (deprecado, shipped) ===
    /// `scroll-snap-points-x` (CSS Scroll Snap v0, 2016). `None` = `none`.
    /// NO hereda. Plumb. Fase 7.1031.
    pub scroll_snap_points_x: Option<String>,
    /// `scroll-snap-points-y` (CSS Scroll Snap v0). `None` = `none`.
    /// NO hereda. Plumb. Fase 7.1032.
    pub scroll_snap_points_y: Option<String>,
    /// `scroll-snap-destination` (CSS Scroll Snap v0). `None` = `0px 0px`.
    /// NO hereda. Plumb. Fase 7.1033.
    pub scroll_snap_destination: Option<String>,
    /// `scroll-snap-coordinate` (CSS Scroll Snap v0). `None` = `none`.
    /// NO hereda. Plumb. Fase 7.1034.
    pub scroll_snap_coordinate: Option<String>,
    // === Fase 7.1035-7.1042 — Gecko -moz- propiedades reales (plumb opaco) ===
    /// `-moz-orient` (Gecko). `None` = `inline`. NO hereda. Plumb. Fase 7.1035.
    pub moz_orient: Option<String>,
    /// `-moz-user-focus` (Gecko). `None` = `none`. HEREDA. Plumb. Fase 7.1036.
    pub moz_user_focus: Option<String>,
    /// `-moz-user-input` (Gecko). `None` = `auto`. HEREDA. Plumb. Fase 7.1037.
    pub moz_user_input: Option<String>,
    /// `-moz-window-dragging` (Gecko chrome). `None` = `default`. NO hereda.
    /// Plumb. Fase 7.1038.
    pub moz_window_dragging: Option<String>,
    /// `-moz-float-edge` (Gecko). `None` = `content-box`. NO hereda. Plumb. Fase 7.1039.
    pub moz_float_edge: Option<String>,
    /// `-moz-force-broken-image-icon` (Gecko). `None` = `0`. NO hereda.
    /// Plumb. Fase 7.1040.
    pub moz_force_broken_image_icon: Option<String>,
    /// `-moz-image-region` (Gecko XUL). `None` = `auto`. HEREDA. Plumb. Fase 7.1041.
    pub moz_image_region: Option<String>,
    /// `-moz-binding` (Gecko XBL, removido). `None` = `none`. NO hereda.
    /// Plumb. Fase 7.1042.
    pub moz_binding: Option<String>,
    // === Fase 7.1043-7.1047 — Gecko -moz-outline-radius (plumb opaco) ===
    /// `-moz-outline-radius` shorthand (Gecko). `None` = `0`. NO hereda.
    /// Plumb. Fase 7.1043.
    pub moz_outline_radius: Option<String>,
    /// `-moz-outline-radius-topleft` (Gecko). `None` = `0`. NO hereda.
    /// Plumb. Fase 7.1044.
    pub moz_outline_radius_topleft: Option<String>,
    /// `-moz-outline-radius-topright` (Gecko). `None` = `0`. NO hereda.
    /// Plumb. Fase 7.1045.
    pub moz_outline_radius_topright: Option<String>,
    /// `-moz-outline-radius-bottomleft` (Gecko). `None` = `0`. NO hereda.
    /// Plumb. Fase 7.1046.
    pub moz_outline_radius_bottomleft: Option<String>,
    /// `-moz-outline-radius-bottomright` (Gecko). `None` = `0`. NO hereda.
    /// Plumb. Fase 7.1047.
    pub moz_outline_radius_bottomright: Option<String>,
}

