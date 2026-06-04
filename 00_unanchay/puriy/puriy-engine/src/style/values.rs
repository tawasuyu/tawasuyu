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
    /// `text-box-edge` (Fase 7.353). Default `Auto`. **Heredable**.
    /// Plumb.
    pub text_box_edge: TextBoxEdge,
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

/// `writing-mode`: orientación de bloque. Default `HorizontalTb`
/// (occidental). Heredable. Fase 7.249. Plumb: el resto de los modos
/// quedan parseados pero el shaper no rota glifos todavía.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingMode {
    #[default]
    HorizontalTb,
    VerticalRl,
    VerticalLr,
    SidewaysRl,
    SidewaysLr,
}

/// `direction`: dirección base del texto del elemento. Heredable.
/// Default `Ltr`. Fase 7.250. Plumb: sin BiDi runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// `unicode-bidi`: trato del elemento por el algoritmo BiDi. NO heredable.
/// Default `Normal`. Fase 7.251. Plumb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}

/// CSS `filter` / `backdrop-filter` function-list item. CSS Filter
/// Effects 1, subset. Cada variante guarda el argumento ya parseado.
/// `none` = lista vacía (no se modela acá). Fase 7.264.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterFn {
    /// `blur(<length>)` en px.
    Blur(f32),
    /// `brightness(<number>|<percentage>)`. 1.0 = sin cambio.
    Brightness(f32),
    /// `contrast(<number>|<percentage>)`. 1.0 = sin cambio.
    Contrast(f32),
    /// `grayscale(<number>|<percentage>)`. 0 = sin cambio, 1 = full.
    Grayscale(f32),
    /// `hue-rotate(<angle>)` en grados.
    HueRotate(f32),
    /// `invert(<number>|<percentage>)`.
    Invert(f32),
    /// `opacity(<number>|<percentage>)`. 1 = sin cambio.
    Opacity(f32),
    /// `saturate(<number>|<percentage>)`. 1 = sin cambio.
    Saturate(f32),
    /// `sepia(<number>|<percentage>)`.
    Sepia(f32),
    /// `drop-shadow(offset-x offset-y [blur] [color])`. Reusa el
    /// `BoxShadow` (inset=false).
    DropShadow(BoxShadow),
}

/// `text-orientation` (CSS Writing Modes 3). Heredable. Default `Mixed`.
/// Fase 7.266.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOrientation {
    #[default]
    Mixed,
    Upright,
    Sideways,
    /// Legacy `sideways-right` (deprecado, alias de `Sideways`).
    SidewaysRight,
}

/// `overscroll-behavior-x` / `-y`. Default `Auto`. NO heredable.
/// Fase 7.267.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverscrollBehavior {
    #[default]
    Auto,
    Contain,
    None,
}

/// `scroll-snap-type`. Default `None`. NO heredable. Fase 7.268.
/// El axis + strictness se modela como struct (None = sin snap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapAxis {
    X,
    Y,
    Block,
    Inline,
    #[default]
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapStrictness {
    #[default]
    Proximity,
    Mandatory,
}

/// `scroll-snap-type`. `None` = sin snap. Some((axis, strictness)) si
/// se declaró. Fase 7.268.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollSnapType(pub Option<(ScrollSnapAxis, ScrollSnapStrictness)>);

/// `scroll-snap-align` (CSS Scroll Snap 1). Default `None` (no snap).
/// El shorthand acepta 1 o 2 valores; con 1 se aplica a block + inline.
/// Fase 7.269.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapAlign {
    #[default]
    None,
    Start,
    End,
    Center,
}

/// `scroll-snap-stop` (CSS Scroll Snap 1). Default `Normal` (el snap
/// puede saltearse). `Always` fuerza parar en cada snap point. Fase 7.270.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSnapStop {
    #[default]
    Normal,
    Always,
}

/// `touch-action` (CSS Pointer Events 2). Default `Auto`. `Pan { ... }`
/// modela la combinación `pan-x` / `pan-y` / `pinch-zoom`; al menos uno
/// debe estar en `true` (validado por el parser). Fase 7.273.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TouchAction {
    #[default]
    Auto,
    None,
    Manipulation,
    Pan {
        pan_x: bool,
        pan_y: bool,
        pinch_zoom: bool,
    },
}

/// `clip-path` (CSS Masking 1). Subset: `inset()`, `circle()`, `ellipse()`.
/// `polygon()` y `path()` quedan fuera por ahora — la mayoría del wild
/// usa formas básicas. `None` (afuera del enum) = `clip-path: none`.
/// Fase 7.274.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipPath {
    /// `inset(<top> [<right> [<bottom> [<left>]]] [round <r>])`. Los
    /// offsets en px desde cada borde; `radius` (opcional) curva las
    /// esquinas. La spec acepta `<length-percentage>`; acá guardamos px.
    Inset { top: f32, right: f32, bottom: f32, left: f32, radius: f32 },
    /// `circle(<radius> [at <x> <y>])`. Radio en px; centro en px desde
    /// el origen del box (default centro = 50% del box, no resuelto acá).
    Circle { radius: f32, cx: LengthVal, cy: LengthVal },
    /// `ellipse(<rx> <ry> [at <x> <y>])`.
    Ellipse { rx: f32, ry: f32, cx: LengthVal, cy: LengthVal },
}

/// `mask-image` (CSS Masking 1). Subset: `url(...)`. `image()`,
/// `linear-gradient()` y demás quedan fuera. `None` = `mask-image: none`.
/// Fase 7.275.
#[derive(Debug, Clone, PartialEq)]
pub enum MaskImage {
    Url(String),
}

/// `content-visibility` (CSS Containment 2). Default `Visible`. `Auto`
/// permite al UA skipear el render fuera de viewport (no implementado);
/// `Hidden` lo skipea siempre. NO heredable. Fase 7.276.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentVisibility {
    #[default]
    Visible,
    Auto,
    Hidden,
}

/// `contain` (CSS Containment 2). Bitset — el shorthand `strict` y
/// `content` se expanden a combinaciones de bits. `none` = todos los
/// bits a 0. Fase 7.277.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContainFlags {
    pub size: bool,
    pub inline_size: bool,
    pub layout: bool,
    pub style: bool,
    pub paint: bool,
}

/// `column-fill` (CSS Multi-column 1). Default `Balance`. Fase 7.281.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnFill {
    Auto,
    #[default]
    Balance,
    BalanceAll,
}

/// `column-span` (CSS Multi-column 1). Default `None`. `All` saca el
/// elemento del flujo multicol. Fase 7.282.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnSpan {
    #[default]
    None,
    All,
}

/// `break-inside` (CSS Fragmentation 3). Default `Auto`. Las variantes
/// `AvoidPage`/`AvoidColumn`/`AvoidRegion` son hints más finos que `Avoid`.
/// Fase 7.283.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakInside {
    #[default]
    Auto,
    Avoid,
    AvoidPage,
    AvoidColumn,
    AvoidRegion,
}

/// `table-layout` (CSS Tables 3). Default `Auto`. Fase 7.284.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableLayout {
    #[default]
    Auto,
    Fixed,
}

/// `border-collapse` (CSS Tables 3). Default `Separate`. Heredable.
/// Fase 7.285.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

/// `caption-side` (CSS Tables 3). Default `Top`. Heredable.
/// Las variantes `inline-start`/`inline-end` (logical) caen a Top/Bottom
/// según `direction`; por simplicidad las aceptamos pero las aplastamos
/// a Top/Bottom. Fase 7.287.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptionSide {
    #[default]
    Top,
    Bottom,
}

/// `empty-cells` (CSS Tables 3). Default `Show`. Heredable. Fase 7.288.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmptyCells {
    #[default]
    Show,
    Hide,
}

/// `break-before` / `break-after` (CSS Fragmentation 3). Default `Auto`.
/// Comparten dominio de valores. Fase 7.289 / 7.290.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakBetween {
    #[default]
    Auto,
    Avoid,
    /// Forzar break (genérico).
    Always,
    /// Variantes específicas por tipo de break.
    AvoidPage,
    Page,
    Left,
    Right,
    Recto,
    Verso,
    AvoidColumn,
    Column,
    AvoidRegion,
    Region,
}

/// `color-scheme` (CSS Color Adjustment 1). Default `Normal` (sin
/// compromiso). El valor `Only(...)` marca el `only` opt-in (un browser
/// fuera del set no puede caer a otro). Heredable. Fase 7.293.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorScheme {
    pub light: bool,
    pub dark: bool,
    /// `only` marca que el UA no debe caer al esquema opuesto.
    pub only: bool,
}

impl ColorScheme {
    /// `normal` = light=false, dark=false, only=false.
    pub const NORMAL: Self = Self { light: false, dark: false, only: false };
}

/// `list-style-position` (CSS Lists 3). Default `Outside`. Fase 7.294.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

/// `quotes` (CSS Generated Content 3). Default `Auto` — la UA elige.
/// `None` (afuera del enum, `Quotes::None`) deja los `open-quote`/
/// `close-quote` mudos. `Pairs(vec)` fija pares concretos por nivel
/// de anidamiento. Fase 7.298.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Quotes {
    #[default]
    Auto,
    None,
    /// Lista `(open, close)` por nivel — el último par se recicla.
    Pairs(Vec<(String, String)>),
}

/// `text-underline-position` (CSS Text Decoration 4). Default `Auto`.
/// Heredable. `Left`/`Right` aplican sólo a writing-modes verticales.
/// Fase 7.299.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextUnderlinePosition {
    #[default]
    Auto,
    FromFont,
    Under,
    Left,
    Right,
}

/// `text-justify` (CSS Text 3). Default `Auto`. Heredable. Fase 7.300.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextJustify {
    #[default]
    Auto,
    None,
    InterWord,
    InterCharacter,
    /// Alias legacy de `InterCharacter`.
    Distribute,
}

/// `print-color-adjust` (CSS Color Adjustment 1). Alias `color-adjust`.
/// Default `Economy`. Heredable. Fase 7.301.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintColorAdjust {
    #[default]
    Economy,
    Exact,
}

/// `forced-color-adjust` (CSS Color Adjustment 1). Default `Auto`.
/// Heredable. Fase 7.302.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForcedColorAdjust {
    #[default]
    Auto,
    None,
    /// Hint moderno (subset opt-in).
    Preserve,
}

/// `font-variant-caps` (CSS Fonts 4). Default `Normal`. Heredable.
/// Fase 7.304.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantCaps {
    #[default]
    Normal,
    SmallCaps,
    AllSmallCaps,
    PetiteCaps,
    AllPetiteCaps,
    Unicase,
    TitlingCaps,
}

/// `font-variant-numeric` (CSS Fonts 4). Bitset libre — los valores
/// `normal` (todos false) y los individuales se acumulan. Heredable.
/// Fase 7.305.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontVariantNumeric {
    pub lining_nums: bool,
    pub oldstyle_nums: bool,
    pub proportional_nums: bool,
    pub tabular_nums: bool,
    pub diagonal_fractions: bool,
    pub stacked_fractions: bool,
    pub ordinal: bool,
    pub slashed_zero: bool,
}

/// `font-variant-ligatures` (CSS Fonts 4). `None` (variante) = todas las
/// ligaduras off; `Normal` (todos false, no_* false) = defaults de la
/// font. Fase 7.306.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantLigatures {
    #[default]
    Normal,
    None,
    /// Combinación de habilitaciones/deshabilitaciones explícitas.
    Custom(LigatureSet),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LigatureSet {
    pub common_ligatures: bool,
    pub no_common_ligatures: bool,
    pub discretionary_ligatures: bool,
    pub no_discretionary_ligatures: bool,
    pub historical_ligatures: bool,
    pub no_historical_ligatures: bool,
    pub contextual: bool,
    pub no_contextual: bool,
}

/// `font-variant-east-asian` (CSS Fonts 4). Bitset libre — `normal` =
/// todos false. Las variantes JIS78/JIS83/.../Simplified/Traditional son
/// mutuamente excluyentes, igual que `full-width`/`proportional-width`;
/// el parser rechaza combinaciones inválidas. Heredable. Fase 7.307.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontVariantEastAsian {
    pub jis78: bool,
    pub jis83: bool,
    pub jis90: bool,
    pub jis04: bool,
    pub simplified: bool,
    pub traditional: bool,
    pub full_width: bool,
    pub proportional_width: bool,
    pub ruby: bool,
}

/// `font-variant-position` (CSS Fonts 4). Default `Normal`. Heredable.
/// Fase 7.308.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontVariantPosition {
    #[default]
    Normal,
    Sub,
    Super,
}

/// `text-emphasis-style` (CSS Text Decoration 4). Default `None` (sin
/// marca). `Mark` modela `[filled|open] && [dot|circle|...]`. `Custom`
/// guarda el string literal (sólo 1 grapheme válido según spec, pero
/// no validamos). Fase 7.309.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TextEmphasisStyle {
    #[default]
    None,
    Mark {
        fill: TextEmphasisFill,
        shape: TextEmphasisShape,
    },
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEmphasisFill {
    #[default]
    Filled,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEmphasisShape {
    #[default]
    Dot,
    Circle,
    DoubleCircle,
    Triangle,
    Sesame,
}

/// `text-emphasis-position` (CSS Text Decoration 4). Default `Over Right`.
/// Combina eje (over/under) + lado (right/left). Fase 7.311.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEmphasisPosition {
    pub over: bool,
    /// `right` (true) o `left` (false).
    pub right: bool,
}

impl Default for TextEmphasisPosition {
    fn default() -> Self {
        Self { over: true, right: true }
    }
}

/// `ruby-position` (CSS Ruby 1). Default `Alternate` (over normalmente,
/// under cuando hay dos anotaciones). Heredable. Fase 7.313.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyPosition {
    Over,
    Under,
    InterCharacter,
    #[default]
    Alternate,
}

/// `transform-origin` (CSS Transforms 1). Punto pivote para `transform`.
/// `x`/`y` en `LengthVal` (`Px(n)` u `Pct(p)`) — el chrome resolvería el
/// % contra el border-box del elemento. `z` en px (`Pct` no se permite
/// en el eje Z). Default CSS: `50% 50% 0`. Fase 7.314.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOrigin {
    pub x: LengthVal,
    pub y: LengthVal,
    pub z: f32,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self { x: LengthVal::Pct(50.0), y: LengthVal::Pct(50.0), z: 0.0 }
    }
}

/// `transform-style` (CSS Transforms 2). Define si los hijos viven en su
/// propio plano (Flat) o componen en 3D con sus padres (Preserve3d).
/// Default `Flat`. Fase 7.315.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformStyle {
    #[default]
    Flat,
    Preserve3d,
}

/// `perspective-origin` (CSS Transforms 2). Punto desde el que se mira
/// a los hijos cuando hay `perspective: <length>`. Default `50% 50%`.
/// Fase 7.317.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveOrigin {
    pub x: LengthVal,
    pub y: LengthVal,
}

impl Default for PerspectiveOrigin {
    fn default() -> Self {
        Self { x: LengthVal::Pct(50.0), y: LengthVal::Pct(50.0) }
    }
}

/// `backface-visibility` (CSS Transforms 2). `Hidden` esconde el elemento
/// cuando una rotación 3D lo voltea (cara mirando para atrás). Default
/// `Visible`. Fase 7.318.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackfaceVisibility {
    #[default]
    Visible,
    Hidden,
}

/// `scrollbar-width` (CSS Scrollbars 1). Heredable. Default `Auto`.
/// `None` = sin barra. Fase 7.319.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollbarWidth {
    #[default]
    Auto,
    Thin,
    None,
}

/// `scrollbar-color: <thumb> <track>` (CSS Scrollbars 1). El valor `auto`
/// se modela como `Option::None` del field padre — esta struct sólo
/// existe cuando ambos colores fueron declarados. Fase 7.320.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarColorPair {
    pub thumb: Color,
    pub track: Color,
}

/// `scrollbar-gutter` (CSS Overflow 3). NO hereda. Default `Auto`.
/// `Stable` reserva el espacio aunque la barra no esté montada;
/// `stable_both_edges` además duplica el gutter en el lado opuesto.
/// Fase 7.321.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollbarGutter {
    pub stable: bool,
    pub both_edges: bool,
}

impl ScrollbarGutter {
    pub const AUTO: Self = Self { stable: false, both_edges: false };
    pub const STABLE: Self = Self { stable: true, both_edges: false };
    pub const STABLE_BOTH: Self = Self { stable: true, both_edges: true };
}

/// `overflow-anchor` (CSS Scroll Anchoring 1). NO hereda. Default
/// `Auto` (el browser decide). `None` desactiva el reanclaje en este
/// subárbol. Fase 7.322.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowAnchor {
    #[default]
    Auto,
    None,
}

/// `overflow-clip-margin` (CSS Overflow 4). Extiende el clip de
/// `overflow: clip` afuera del padding-box. `length` siempre en px no
/// negativo; `visual_box` indica la caja desde la que se mide
/// (default `PaddingBox` en CSS). Fase 7.323.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverflowClipMargin {
    pub visual_box: VisualBox,
    pub length: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualBox {
    ContentBox,
    #[default]
    PaddingBox,
    BorderBox,
}

/// `text-align-last` (CSS Text 4). Alineación de la última línea de un
/// bloque (la única para `text-align: justify`). Heredable. Default
/// `Auto`. Fase 7.324.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlignLast {
    #[default]
    Auto,
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}

/// `text-wrap` (CSS Text 4). Heredable. Default `Wrap`. Fase 7.325.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrap {
    #[default]
    Wrap,
    Nowrap,
    Balance,
    Pretty,
    Stable,
}

/// `line-break` (CSS Text 3). Estrictez del breaker para CJK y
/// puntuación pegada. Heredable. Default `Auto`. Fase 7.326.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineBreak {
    #[default]
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

/// `hanging-punctuation` (CSS Text 4). Combinación de flags. La spec
/// permite `none | [first || force-end | allow-end || last]`. `force_end`
/// y `allow_end` son mutuamente excluyentes (modelado: bool `force_end`
/// + bool `allow_end`; sólo uno puede ser true a la vez). Heredable.
/// Fase 7.327.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HangingPunctuation {
    pub first: bool,
    pub force_end: bool,
    pub allow_end: bool,
    pub last: bool,
}

impl HangingPunctuation {
    pub const fn is_none(self) -> bool {
        !self.first && !self.force_end && !self.allow_end && !self.last
    }
}

/// `text-decoration-skip-ink` (CSS Text Decoration 4). Heredable.
/// Default `Auto`. Fase 7.328.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationSkipInk {
    #[default]
    Auto,
    None,
    All,
}

/// `font-optical-sizing` (CSS Fonts 4). `Auto` deja que el shaper
/// setee el axis `opsz` según el tamaño; `None` lo fija al default.
/// Heredable. Fase 7.329.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontOpticalSizing {
    #[default]
    Auto,
    None,
}

/// `font-synthesis-*` (CSS Fonts 4). Cada eje permite la síntesis
/// (true, default) o la desactiva (false). El shorthand `font-synthesis`
/// (Fase 7.333) setea los tres a la vez. Heredable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontSynthesis {
    pub weight: bool,
    pub style: bool,
    pub small_caps: bool,
}

impl Default for FontSynthesis {
    fn default() -> Self {
        Self { weight: true, style: true, small_caps: true }
    }
}

impl FontSynthesis {
    /// `font-synthesis: none` apaga los tres.
    pub const NONE: Self = Self { weight: false, style: false, small_caps: false };
}

/// `font-size-adjust` (CSS Fonts 5). `None` = sin ajuste; `Value(metric,
/// number)` = ajustar la métrica del fallback al `number * font-size`
/// del referente; `FromFont(metric)` = usar el valor que provee la
/// fuente para esa métrica. Heredable. Fase 7.334.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSizeAdjust {
    None,
    Value(FontMetric, f32),
    FromFont(FontMetric),
}

impl Default for FontSizeAdjust {
    fn default() -> Self {
        FontSizeAdjust::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontMetric {
    /// Métrica default cuando no se da una explícitamente.
    #[default]
    ExHeight,
    CapHeight,
    ChWidth,
    IcWidth,
    IcHeight,
}

/// `image-orientation` (CSS Images 3). `FromImage` rota según EXIF;
/// `None` ignora EXIF; `Angle(deg, flip)` aplica un ángulo + flip
/// horizontal opcional. Heredable. Fase 7.335.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageOrientation {
    FromImage,
    None,
    /// `<angle> [flip]?` — el flip se aplica antes de la rotación.
    Angle { degrees: f32, flip: bool },
}

impl Default for ImageOrientation {
    fn default() -> Self {
        ImageOrientation::FromImage
    }
}

/// `animation-timeline` (CSS Animations 2 / Scroll-driven Animations 1).
/// `Auto` usa el monotonic timer del documento (el default
/// implícito); `None` desactiva la animación; `Named(s)` la enlaza a
/// un scroll/view-timeline declarado en otro lado. NO hereda.
/// Fase 7.339.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TimelineRef {
    #[default]
    Auto,
    None,
    Named(String),
}

/// `scroll-timeline-axis` / `view-timeline-axis` (CSS Scroll-driven
/// Animations 1). Default `Block` (el eje block del writing-mode).
/// `X`/`Y` son aliases físicos. Fases 7.341, 7.343.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineAxis {
    #[default]
    Block,
    Inline,
    X,
    Y,
}

/// `white-space-collapse` (CSS Text 4). Heredable. Default `Collapse`.
/// Fase 7.344.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpaceCollapse {
    #[default]
    Collapse,
    Preserve,
    PreserveBreaks,
    BreakSpaces,
}

/// `text-wrap-mode` (CSS Text 4). Heredable. Default `Wrap`.
/// Fase 7.345.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrapMode {
    #[default]
    Wrap,
    Nowrap,
}

/// `text-wrap-style` (CSS Text 4). Heredable. Default `Auto`.
/// Fase 7.346.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrapStyle {
    #[default]
    Auto,
    Balance,
    Pretty,
    Stable,
}

/// `text-spacing-trim` (CSS Text 4). Heredable. Default `Normal`.
/// Fase 7.347.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSpacingTrim {
    #[default]
    Normal,
    SpaceAll,
    SpaceFirst,
    TrimStart,
}

/// `text-box-trim` (CSS Inline Layout 3). Heredable. Default `None`.
/// Fase 7.348.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBoxTrim {
    #[default]
    None,
    TrimStart,
    TrimEnd,
    TrimBoth,
}

/// `math-style` (CSS MathML 3 Core). Heredable. Default `Normal`.
/// Fase 7.349.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathStyle {
    #[default]
    Normal,
    Compact,
}

/// `math-depth` (CSS MathML 3 Core). `Auto` = el browser ajusta; `Add(n)`
/// suma `n` al heredado; `Value(n)` lo fija absoluto. Heredable.
/// Default `Auto`. Fase 7.350. NOTA: el `add` con signo se modela con
/// `i32` (negativo permitido).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathDepth {
    Auto,
    Add(i32),
    Value(i32),
}

impl Default for MathDepth {
    fn default() -> Self {
        MathDepth::Auto
    }
}

/// `math-shift` (CSS MathML 3 Core). Heredable. Default `Normal`.
/// Fase 7.351.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathShift {
    #[default]
    Normal,
    Compact,
}

/// `field-sizing` (CSS Basic UI 4). NO hereda. Default `Fixed`.
/// `Content` permite que `<input>`/`<textarea>` se autoencojan al
/// contenido (caso de uso: `textarea` sin scroll horizontal). Fase 7.352.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldSizing {
    #[default]
    Fixed,
    Content,
}

/// `text-box-edge` (CSS Inline Layout 3). `Auto` deja al browser elegir
/// según script/fuente. Caso con 1 o 2 keywords (`<text-edge> [<text-edge>]?`).
/// `Edge { over, under }` cubre el caso de 1 keyword (over==under) o 2
/// keywords explícitos. Heredable. Default `Auto`. Fase 7.353.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBoxEdge {
    Auto,
    Edge { over: TextEdge, under: TextEdge },
}

impl Default for TextBoxEdge {
    fn default() -> Self {
        TextBoxEdge::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEdge {
    #[default]
    Text,
    Cap,
    Ex,
    Ideographic,
    IdeographicInk,
    /// Sólo válido en el lado bajo (`under`); sintetizado por el parser.
    Alphabetic,
}

impl ContainFlags {
    /// `strict` = `size layout style paint`.
    pub const STRICT: Self = Self {
        size: true,
        inline_size: false,
        layout: true,
        style: true,
        paint: true,
    };
    /// `content` = `layout style paint` (sin `size`).
    pub const CONTENT: Self = Self {
        size: false,
        inline_size: false,
        layout: true,
        style: true,
        paint: true,
    };
    /// `true` si NINGÚN bit está activo (equiv. `contain: none`).
    pub const fn is_none(self) -> bool {
        !self.size && !self.inline_size && !self.layout && !self.style && !self.paint
    }
}

/// `font-kerning`. Heredable. Default `Auto`. Fase 7.259.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontKerning {
    #[default]
    Auto,
    Normal,
    None,
}

/// Un entry de `font-feature-settings`: tag de 4 bytes + valor entero
/// (0 = off, 1 = on, N = índice de variante). Fase 7.260.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFeatureSetting {
    /// 4 ASCII chars (case-sensitive por OpenType). Sin validar contra
    /// `[a-zA-Z0-9]` por simplicidad — el shaper hace la verificación final.
    pub tag: [u8; 4],
    pub value: i32,
}

/// Un entry de `font-variation-settings`: tag de 4 bytes + valor
/// número (`wght 700`, `wdth 100`, `slnt -15`...). Fase 7.261.
#[derive(Debug, Clone, PartialEq)]
pub struct FontVariationSetting {
    pub tag: [u8; 4],
    pub value: f32,
}

/// `text-rendering`. Heredable. Default `Auto`. Fase 7.263.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextRendering {
    #[default]
    Auto,
    OptimizeSpeed,
    OptimizeLegibility,
    GeometricPrecision,
}

/// `mix-blend-mode` / `background-blend-mode`. Subset Compositing &
/// Blending 1. Default `Normal`. Plumb. Fase 7.254/7.255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
}

/// `isolation`. NO heredable. `Isolate` fuerza un nuevo stacking context.
/// Fase 7.256.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Isolation {
    #[default]
    Auto,
    Isolate,
}

/// `will-change`: hint individual. `Auto` cuando la lista es vacía.
/// Subset: `scroll-position`, `contents`, o nombre arbitrario de
/// propiedad (almacenado como `Property(String)`). Fase 7.257.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WillChangeHint {
    ScrollPosition,
    Contents,
    /// Nombre de propiedad CSS (ej. `transform`, `opacity`). Se almacena
    /// tal cual lo escribió el autor, en lowercase.
    Property(String),
}

/// `appearance` (CSS UI 4). Default `Auto`. NO heredable. Fase 7.258.
/// El subset cubre los valores de compat más usados; cualquier otro
/// keyword cae a `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Auto,
    None,
    /// Hints de compat conservados.
    Textfield,
    MenulistButton,
    Button,
    Checkbox,
    Radio,
}

/// `image-rendering`: hint del sampler al pintar `<img>` y backgrounds.
/// Heredable. Default `Auto`. Fase 7.253. Plumb: el chrome aún no elige
/// `nearest` vs `linear` en función de este flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageRendering {
    #[default]
    Auto,
    /// CSS Images 3 `smooth` — bilinear/trilinear (lo que el GPU haga).
    Smooth,
    /// CSS Images 3 `crisp-edges` — sin antialiasing en escala (ideal pixel art).
    CrispEdges,
    /// CSS Images 4 `pixelated` — nearest-neighbour explícito.
    Pixelated,
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
            writing_mode: WritingMode::HorizontalTb,
            direction: Direction::Ltr,
            unicode_bidi: UnicodeBidi::Normal,
            font_stretch: 1.0,
            image_rendering: ImageRendering::Auto,
            mix_blend_mode: BlendMode::Normal,
            background_blend_mode: Vec::new(),
            isolation: Isolation::Auto,
            will_change: Vec::new(),
            appearance: Appearance::Auto,
            font_kerning: FontKerning::Auto,
            font_feature_settings: Vec::new(),
            font_variation_settings: Vec::new(),
            font_language_override: None,
            text_rendering: TextRendering::Auto,
            filter: Vec::new(),
            backdrop_filter: Vec::new(),
            text_orientation: TextOrientation::Mixed,
            overscroll_behavior_x: OverscrollBehavior::Auto,
            overscroll_behavior_y: OverscrollBehavior::Auto,
            scroll_snap_type: ScrollSnapType(None),
            scroll_snap_align_block: ScrollSnapAlign::None,
            scroll_snap_align_inline: ScrollSnapAlign::None,
            scroll_snap_stop: ScrollSnapStop::Normal,
            scroll_padding: Sides {
                top: LengthVal::Auto,
                right: LengthVal::Auto,
                bottom: LengthVal::Auto,
                left: LengthVal::Auto,
            },
            scroll_margin: Sides::all(0.0),
            touch_action: TouchAction::Auto,
            clip_path: None,
            mask_image: None,
            content_visibility: ContentVisibility::Visible,
            contain: ContainFlags::default(),
            column_count: None,
            column_width: LengthVal::Auto,
            column_rule_width: 0.0,
            column_rule_color: None,
            column_rule_style: BorderLineStyle::Solid,
            column_rule_style_active: false,
            column_fill: ColumnFill::Balance,
            column_span: ColumnSpan::None,
            break_inside: BreakInside::Auto,
            table_layout: TableLayout::Auto,
            border_collapse: BorderCollapse::Separate,
            border_spacing_h: 0.0,
            border_spacing_v: 0.0,
            caption_side: CaptionSide::Top,
            empty_cells: EmptyCells::Show,
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            orphans: 2,
            widows: 2,
            color_scheme: ColorScheme::NORMAL,
            list_style_position: ListStylePosition::Outside,
            list_style_image: None,
            counter_set: Vec::new(),
            quotes: Quotes::Auto,
            text_underline_position: TextUnderlinePosition::Auto,
            text_justify: TextJustify::Auto,
            print_color_adjust: PrintColorAdjust::Economy,
            forced_color_adjust: ForcedColorAdjust::Auto,
            line_clamp: None,
            font_variant_caps: FontVariantCaps::Normal,
            font_variant_numeric: FontVariantNumeric::default(),
            font_variant_ligatures: FontVariantLigatures::Normal,
            font_variant_east_asian: FontVariantEastAsian::default(),
            font_variant_position: FontVariantPosition::Normal,
            text_emphasis_style: TextEmphasisStyle::None,
            text_emphasis_color: None,
            text_emphasis_position: TextEmphasisPosition::default(),
            ruby_position: RubyPosition::Alternate,
            transform_origin: TransformOrigin::default(),
            transform_style: TransformStyle::Flat,
            perspective: None,
            perspective_origin: PerspectiveOrigin::default(),
            backface_visibility: BackfaceVisibility::Visible,
            scrollbar_width: ScrollbarWidth::Auto,
            scrollbar_color: None,
            scrollbar_gutter: ScrollbarGutter::AUTO,
            overflow_anchor: OverflowAnchor::Auto,
            overflow_clip_margin: None,
            text_align_last: TextAlignLast::Auto,
            text_wrap: TextWrap::Wrap,
            line_break: LineBreak::Auto,
            hanging_punctuation: HangingPunctuation::default(),
            text_decoration_skip_ink: TextDecorationSkipInk::Auto,
            font_optical_sizing: FontOpticalSizing::Auto,
            font_synthesis: FontSynthesis::default(),
            font_size_adjust: FontSizeAdjust::None,
            image_orientation: ImageOrientation::FromImage,
            animation_timeline: TimelineRef::Auto,
            scroll_timeline_name: None,
            scroll_timeline_axis: TimelineAxis::Block,
            view_timeline_name: None,
            view_timeline_axis: TimelineAxis::Block,
            white_space_collapse: WhiteSpaceCollapse::Collapse,
            text_wrap_mode: TextWrapMode::Wrap,
            text_wrap_style: TextWrapStyle::Auto,
            text_spacing_trim: TextSpacingTrim::Normal,
            text_box_trim: TextBoxTrim::None,
            math_style: MathStyle::Normal,
            math_depth: MathDepth::Auto,
            math_shift: MathShift::Normal,
            field_sizing: FieldSizing::Fixed,
            text_box_edge: TextBoxEdge::Auto,
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
