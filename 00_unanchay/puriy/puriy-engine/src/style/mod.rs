//! Style engine — parser CSS minimal sobre `cssparser`.
//!
//! Para Fase 2 soportamos sólo:
//! - selectores type (`p`, `div`, `h1`) y universal (`*`)
//! - propiedades `color`, `background-color`, `display`, `font-size`,
//!   `margin`, `padding`
//! - inline `style="..."` en cada elemento
//!
//! No hay cascada con especificidad real ni `!important`. Stylo entero
//! entra en Fase 3 cuando el chrome Llimphi consuma estilos jerárquicos
//! complejos. Por ahora, una pasada de regla→nodo con override por
//! inline style alcanza para renderizar páginas simples (example.com,
//! landing del propio repo).

use std::collections::HashMap;

use markup5ever_rcdom::Handle;

use crate::boxes::{Color, Display};
use crate::dom::{self, DomTree};

/// Tipos de valores CSS computados (`ComputedStyle` + enums/structs + Default).
mod values;
pub use values::*;
/// Selectores y matching (`Rule`/`Selector`/`Compound`/`Pseudo` + content items).
mod matching;
pub use matching::*;
/// Declaraciones CSS (`Decl`/`DeclKind` + aplicación sobre `ComputedStyle`).
mod decl;
pub(crate) use decl::*;
/// Parsing CSS (hoja/at-rules/keyframes, selectores, declaraciones, color).
mod parser;
pub use parser::*;

/// Almacena reglas parseadas + función de "computar para nodo".
#[derive(Debug, Clone)]
pub struct StyleEngine {
    rules: Vec<Rule>,
    /// CSS variables declaradas en `:root`/`html`/`*`. Se substituyen en
    /// los values en parse-time (y en values de `style="..."` inline en
    /// compute-time). Scope cascade real queda para una iteración futura
    /// — :root cubre el 80% de los usos en el wild.
    vars: HashMap<String, String>,
    /// Definiciones `@keyframes name { ... }` recogidas de todos los
    /// stylesheets. Las consumiría el runtime de animación (Fase B4+, aún
    /// no implementado) cruzando el `name` de un `AnimationBinding` con
    /// esta tabla. Hoy sólo se parsean y se exponen vía [`Self::keyframes`].
    keyframes: HashMap<String, Keyframes>,
    /// Definiciones `@font-face { ... }` recogidas de todos los stylesheets.
    /// Las consumiría el cargador de fuentes (aún no implementado) cruzando el
    /// `font-family` computado con `FontFaceRule::family`. Hoy sólo se parsean
    /// y se exponen vía [`Self::font_faces`].
    font_faces: Vec<FontFaceRule>,
    /// Definiciones `@property --name { ... }` (Houdini). Las consumiría la
    /// cascada de variables (valor inicial registrado + control de herencia);
    /// hoy sólo se parsean y se exponen vía [`Self::registered_properties`].
    registered_properties: Vec<PropertyRule>,
    /// Definiciones `@counter-style name { ... }`. Las consumiría
    /// `list-style-type: <name>` (trabajo futuro); hoy sólo se parsean y se
    /// exponen vía [`Self::counter_styles`].
    counter_styles: Vec<CounterStyleRule>,
    /// Definiciones `@page [<sel>] { ... }` (Paged Media). Las consumiría el
    /// pipeline de impresión/paginado (trabajo futuro); hoy sólo se parsean y
    /// se exponen vía [`Self::page_rules`].
    page_rules: Vec<PageRule>,
}

impl StyleEngine {
    /// Construye el engine desde el DOM: parsea cada `<style>` inline +
    /// inyecta el UA stylesheet (los defaults HTML que cssparser no
    /// conoce).
    /// Construye el motor de estilos resolviendo `@media` contra `DEFAULT_VIEWPORT`.
    /// El chrome usa [`Self::from_dom_with_viewport`] para el viewport real.
    pub fn from_dom(dom: &DomTree) -> Self {
        Self::from_dom_with_viewport(dom, DEFAULT_VIEWPORT)
    }

    /// Como [`Self::from_dom`] pero evalúa los `@media` del documento contra
    /// `vp` (el tamaño/DPR real de la ventana). Las queries que no matchean se
    /// descartan en el parse, así que la cascada sólo ve las reglas activas.
    /// Sólo ve los `<style>` inline — las hojas externas (`<link>`) las baja el
    /// `Engine` y entran por [`Self::from_sheets_with_viewport`].
    pub fn from_dom_with_viewport(dom: &DomTree, vp: Viewport) -> Self {
        Self::from_sheets_with_viewport(&dom.collect_inline_stylesheets(), vp)
    }

    /// Construye el motor desde una lista de hojas de estilo YA resueltas (su
    /// texto), en orden de cascada. Es el punto por el que el `Engine` mete
    /// tanto los `<style>` inline como los `<link rel="stylesheet">` externos
    /// (ya bajados), preservando el orden de documento. El UA stylesheet va
    /// siempre primero (menor prioridad).
    pub fn from_sheets_with_viewport(sheets: &[String], vp: Viewport) -> Self {
        let mut rules = ua_stylesheet();
        // Primera pasada: recoger `--name: value` de `:root` de todas las
        // hojas para que cualquier `var(--x)` se resuelva sin importar en qué
        // archivo se declaró.
        let mut vars: HashMap<String, String> = HashMap::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_root_vars(&cleaned, &mut vars);
        }
        // Segunda pasada: recoger `@keyframes` de todas las hojas. Son
        // globales (no caen en la cascada por selector), así que un mapa
        // name→def plano alcanza; conflictos los gana el último.
        let mut keyframes: HashMap<String, Keyframes> = HashMap::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_keyframes(&cleaned, &mut keyframes);
        }
        // Tercera pasada: recoger `@font-face` de todas las hojas. Como
        // `@keyframes`, son globales y no caen en la cascada por selector;
        // pero admiten duplicados de `family` (rangos distintos) → lista.
        let mut font_faces: Vec<FontFaceRule> = Vec::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_font_faces(&cleaned, &mut font_faces);
        }
        // Cuarta pasada: recoger `@property --name` (Houdini). Globales.
        let mut registered_properties: Vec<PropertyRule> = Vec::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_at_properties(&cleaned, &mut registered_properties);
        }
        // Quinta pasada: recoger `@counter-style`. Globales.
        let mut counter_styles: Vec<CounterStyleRule> = Vec::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_counter_styles(&cleaned, &mut counter_styles);
        }
        // Sexta pasada: recoger `@page`. Globales.
        let mut page_rules: Vec<PageRule> = Vec::new();
        for sheet in sheets {
            let cleaned = strip_comments(sheet);
            extract_page_rules(&cleaned, &mut page_rules);
        }
        for sheet in sheets {
            rules.extend(parse_stylesheet(sheet, &vars, vp));
        }
        Self {
            rules,
            vars,
            keyframes,
            font_faces,
            registered_properties,
            counter_styles,
            page_rules,
        }
    }

    /// Tabla de `@keyframes` parseados (name → definición). Vacía si el
    /// documento no declara animaciones. El runtime de tween (Fase B4+)
    /// la cruzará con `ComputedStyle::animation`; hoy es sólo lectura.
    pub fn keyframes(&self) -> &HashMap<String, Keyframes> {
        &self.keyframes
    }

    /// Lista de `@font-face` parseados, en orden de documento. Vacía si el
    /// documento no declara fuentes. El cargador de fuentes (trabajo futuro)
    /// la cruzará con `ComputedStyle::font_family`; hoy es sólo lectura.
    pub fn font_faces(&self) -> &[FontFaceRule] {
        &self.font_faces
    }

    /// Lista de `@property --name` registrados, en orden de documento. La
    /// cascada de variables (trabajo futuro) la cruzará con los `var(--name)`;
    /// hoy es sólo lectura.
    pub fn registered_properties(&self) -> &[PropertyRule] {
        &self.registered_properties
    }

    /// Lista de `@counter-style` definidos, en orden de documento. La
    /// resolución de `list-style-type: <name>` (trabajo futuro) la cruzará;
    /// hoy es sólo lectura.
    pub fn counter_styles(&self) -> &[CounterStyleRule] {
        &self.counter_styles
    }

    /// Lista de `@page` definidos, en orden de documento. El pipeline de
    /// paginado (trabajo futuro) los consumirá; hoy es sólo lectura.
    pub fn page_rules(&self) -> &[PageRule] {
        &self.page_rules
    }

    /// Computa el estilo de un nodo Element. Aplica en orden: UA →
    /// stylesheets del documento → atributo `style="..."`. El último
    /// gana (cascada simplificada). Sin inheritance — el caller debe
    /// usar [`Self::compute_with_parent`] si necesita propagación.
    pub fn compute(&self, node: &Handle) -> ComputedStyle {
        self.compute_with_parent(node, None)
    }

    /// Variante con inheritance CSS. Si `parent` está dado, las
    /// propiedades heredables (`color`, `font_size`, `font_weight`,
    /// `text_align`, `line_height`) se inicializan con el valor del
    /// padre antes de aplicar reglas y `style=`. Propiedades no
    /// heredables (`background`, `display`, `margin`, `padding`,
    /// `width`, `max_width`) siempre arrancan en el default.
    pub fn compute_with_parent(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        self.compute_with_parent_in_state(node, parent, false)
    }

    /// Variante con hover. Si `hover_active=true`, los selectores con
    /// `:hover` también matchean. Permite computar el "estilo bajo el
    /// mouse" sin un mouse real — el chrome lo usa para precalcular
    /// `hover_fill` en el render. Compat con la API anterior — para
    /// `:focus` usar [`compute_with_parent_for_state`].
    pub fn compute_with_parent_in_state(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
    ) -> ComputedStyle {
        self.compute_with_parent_for_state(node, parent, hover_active, false)
    }

    /// Computa el estilo del pseudo-element `::before` o `::after` del
    /// nodo. Sólo matchean selectores que terminan con ese pseudo;
    /// reglas para el elemento real se ignoran. Devuelve `None` si el
    /// pseudo no tiene `content` válido — CSS spec dice que un
    /// pseudo-element sin content no se materializa.
    pub fn compute_pseudo(
        &self,
        node: &Handle,
        pseudo: PseudoElement,
        parent: Option<&ComputedStyle>,
    ) -> Option<ComputedStyle> {
        let style = self.compute_internal(node, parent, false, false, Some(pseudo));
        // CSS spec: si `content` no se setea (None) o resuelve a `none`,
        // el pseudo-element NO se genera. Acá `content: None` cubre
        // ambos casos (el parser de content normaliza `none`/`normal` a
        // None, y la ausencia total también queda en None).
        style.content.is_some().then_some(style)
    }

    /// Variante con hover **y** focus. Cuando focus_active=true, los
    /// selectores `:focus` matchean. Útil para precalcular `focus_*`
    /// styles desde el chrome.
    pub fn compute_with_parent_for_state(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
        focus_active: bool,
    ) -> ComputedStyle {
        self.compute_internal(node, parent, hover_active, focus_active, None)
    }

    fn compute_internal(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
        focus_active: bool,
        target_pseudo: Option<PseudoElement>,
    ) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        if let Some(p) = parent {
            style.color = p.color;
            style.font_size = p.font_size;
            style.font_weight = p.font_weight;
            style.font_style = p.font_style;
            style.font_family = p.font_family.clone();
            style.text_align = p.text_align;
            style.line_height = p.line_height;
            // text-decoration: tratada heredable para que descendientes
            // inline (`<a>foo <b>bar</b></a>`) mantengan la línea.
            style.text_decoration = p.text_decoration;
            // list-style-type sí es heredable según CSS spec — un `<ol>`
            // con `list-style-type: decimal` debe propagarse a sus `<li>`.
            style.list_style_type = p.list_style_type;
            // white-space y text-transform son heredables (CSS spec).
            // Sin esto, `<p style="text-transform:uppercase">FOO <span>bar</span></p>`
            // dejaría "bar" en minúscula porque el text leaf vive en `<span>`.
            style.white_space = p.white_space;
            style.text_transform = p.text_transform;
            // text-shadow, word-spacing, text-indent, visibility,
            // pointer-events: heredables (CSS spec).
            style.text_shadows = p.text_shadows.clone();
            style.word_spacing = p.word_spacing;
            style.letter_spacing = p.letter_spacing;
            style.text_indent = p.text_indent;
            style.visibility = p.visibility;
            style.pointer_events = p.pointer_events;
            // caret-color, accent-color, cursor son heredables (CSS UI).
            style.caret_color = p.caret_color;
            style.accent_color = p.accent_color;
            style.cursor = p.cursor;
            // scroll-behavior (CSSOM-View) y tab-size (CSS Text 3) heredan.
            // text-overflow NO hereda (CSS UI 3).
            style.scroll_behavior = p.scroll_behavior;
            style.tab_size = p.tab_size;
            // user-select (CSS UI 4) hereda; overflow-wrap, word-break,
            // hyphens (CSS Text 3) también. resize (CSS UI 4) NO hereda.
            style.user_select = p.user_select;
            style.overflow_wrap = p.overflow_wrap;
            style.word_break = p.word_break;
            style.hyphens = p.hyphens;
            // writing-mode, direction (CSS Writing Modes 3) heredan;
            // unicode-bidi NO hereda. font-stretch (CSS Fonts 4) y
            // image-rendering (CSS Images 3) también heredan.
            style.writing_mode = p.writing_mode;
            style.direction = p.direction;
            style.font_stretch = p.font_stretch;
            style.image_rendering = p.image_rendering;
            // font-kerning, font-feature-settings, font-variation-settings,
            // font-language-override (CSS Fonts 4) heredan. text-rendering
            // (SVG 1.1) hereda.
            style.font_kerning = p.font_kerning;
            style.font_feature_settings = p.font_feature_settings.clone();
            style.font_variation_settings = p.font_variation_settings.clone();
            style.font_language_override = p.font_language_override.clone();
            style.text_rendering = p.text_rendering;
            // text-orientation hereda. filter, backdrop-filter,
            // overscroll-behavior y scroll-snap-type NO heredan.
            style.text_orientation = p.text_orientation;
            // CSS Tables 3 — border-collapse, border-spacing, caption-side y
            // empty-cells heredan; table-layout NO.
            style.border_collapse = p.border_collapse;
            style.border_spacing_h = p.border_spacing_h;
            style.border_spacing_v = p.border_spacing_v;
            style.caption_side = p.caption_side;
            style.empty_cells = p.empty_cells;
            // CSS Fragmentation — orphans/widows heredan; break-before/after NO.
            // CSS Color Adjustment — color-scheme hereda.
            style.orphans = p.orphans;
            style.widows = p.widows;
            style.color_scheme = p.color_scheme;
            // CSS Lists 3 — list-style-{position,image,type} heredan. CSS
            // Generated Content 3 — quotes hereda. counter-set NO.
            style.list_style_position = p.list_style_position;
            style.list_style_image = p.list_style_image.clone();
            style.quotes = p.quotes.clone();
            // CSS Text Decoration 4 — text-underline-position hereda.
            // CSS Text 3 — text-justify hereda. CSS Color Adjustment 1 —
            // print-color-adjust y forced-color-adjust heredan. line-clamp NO.
            style.text_underline_position = p.text_underline_position;
            style.text_justify = p.text_justify;
            style.print_color_adjust = p.print_color_adjust;
            style.forced_color_adjust = p.forced_color_adjust;
            // CSS Text 4 — hyphenate-character, hyphenate-limit-chars y
            // line-height-step heredan. CSS Text Inline 3 — text-size-adjust
            // hereda. CSS Fonts 4 — font-variant-emoji hereda.
            style.hyphenate_character = p.hyphenate_character.clone();
            style.hyphenate_limit_chars = p.hyphenate_limit_chars;
            style.line_height_step = p.line_height_step;
            style.text_size_adjust = p.text_size_adjust;
            style.font_variant_emoji = p.font_variant_emoji;
            // CSS Fonts 4 — todos los font-variant-* heredan.
            style.font_variant_caps = p.font_variant_caps;
            style.font_variant_numeric = p.font_variant_numeric;
            style.font_variant_ligatures = p.font_variant_ligatures;
            style.font_variant_east_asian = p.font_variant_east_asian;
            style.font_variant_position = p.font_variant_position;
            // CSS Text Decoration 4 — text-emphasis-* heredan. CSS Ruby 1 —
            // ruby-position hereda.
            style.text_emphasis_style = p.text_emphasis_style.clone();
            style.text_emphasis_color = p.text_emphasis_color;
            style.text_emphasis_position = p.text_emphasis_position;
            style.ruby_position = p.ruby_position;
            style.ruby_align = p.ruby_align;
            style.ruby_overhang = p.ruby_overhang;
            // CSS Scrollbars 1 — scrollbar-width y scrollbar-color heredan.
            // scrollbar-gutter (CSS Overflow 3), overflow-anchor (CSS Scroll
            // Anchoring 1) y overflow-clip-margin (CSS Overflow 4) NO heredan.
            style.scrollbar_width = p.scrollbar_width;
            style.scrollbar_color = p.scrollbar_color;
            // CSS Text 3/4 — text-align-last, text-wrap, line-break,
            // hanging-punctuation y text-decoration-skip-ink heredan.
            style.text_align_last = p.text_align_last;
            style.text_wrap = p.text_wrap;
            style.line_break = p.line_break;
            style.hanging_punctuation = p.hanging_punctuation;
            style.text_decoration_skip_ink = p.text_decoration_skip_ink;
            // CSS Fonts 4 — font-optical-sizing y font-synthesis-* heredan.
            style.font_optical_sizing = p.font_optical_sizing;
            style.font_synthesis = p.font_synthesis;
            // CSS Fonts 5 — font-size-adjust hereda. CSS Images 3 —
            // image-orientation hereda. NOTA: place-items/place-content
            // son shorthands de longhands no-heredables, no se enchufan
            // acá (la cascada de cada longhand ya pegó arriba).
            style.font_size_adjust = p.font_size_adjust;
            style.image_orientation = p.image_orientation;
            // CSS Text 4 — white-space-collapse, text-wrap-mode,
            // text-wrap-style, text-spacing-trim, text-box-trim heredan.
            style.white_space_collapse = p.white_space_collapse;
            style.text_wrap_mode = p.text_wrap_mode;
            style.text_wrap_style = p.text_wrap_style;
            style.text_spacing_trim = p.text_spacing_trim;
            style.text_box_trim = p.text_box_trim;
            // CSS MathML 3 Core — math-style, math-depth, math-shift heredan.
            // CSS Inline Layout 3 — text-box-edge hereda. CSS Basic UI 4 —
            // field-sizing NO hereda.
            style.math_style = p.math_style;
            style.math_depth = p.math_depth;
            style.math_shift = p.math_shift;
            style.text_box_edge = p.text_box_edge;
            // CSS Color HDR 1 — dynamic-range-limit hereda. CSS Position 4 —
            // overlay NO hereda. Fase 7.905.
            style.dynamic_range_limit = p.dynamic_range_limit;
            // CSS Anchor Positioning 1 — anchor-scope hereda;
            // anchor-name y position-anchor NO heredan.
            // CSS View Transitions — view-transition-name y
            // view-transition-class NO heredan.
            style.anchor_scope = p.anchor_scope.clone();
            // CSS Fonts 4 — font-palette y font-variant-alternates heredan.
            // CSS UI 4 — caret-shape hereda. CSS Backgrounds 3 —
            // background-attachment NO hereda.
            style.font_palette = p.font_palette.clone();
            style.font_variant_alternates = p.font_variant_alternates.clone();
            style.caret_shape = p.caret_shape;
            // SVG 2 — dominant-baseline, paint-order heredan;
            // alignment-baseline y baseline-source NO heredan.
            // CSS Lists 3 — marker-side hereda.
            style.dominant_baseline = p.dominant_baseline;
            style.paint_order = p.paint_order;
            style.marker_side = p.marker_side;
            // SVG 2 — fill, stroke y sus opacities/width heredan.
            style.fill = p.fill.clone();
            style.stroke = p.stroke.clone();
            style.fill_opacity = p.fill_opacity;
            style.stroke_opacity = p.stroke_opacity;
            style.stroke_width = p.stroke_width;
            // SVG 2 — el resto del set de stroke también hereda.
            style.stroke_linecap = p.stroke_linecap;
            style.stroke_linejoin = p.stroke_linejoin;
            style.stroke_miterlimit = p.stroke_miterlimit;
            style.stroke_dasharray = p.stroke_dasharray.clone();
            style.stroke_dashoffset = p.stroke_dashoffset;
            // SVG 2 — fill-rule, clip-rule, color-interpolation,
            // shape-rendering heredan; vector-effect NO hereda.
            style.fill_rule = p.fill_rule;
            style.clip_rule = p.clip_rule;
            style.color_interpolation = p.color_interpolation;
            style.shape_rendering = p.shape_rendering;
            // SVG 2 — text-anchor, color-rendering,
            // color-interpolation-filters, glyph-orientation-vertical
            // heredan; transform-box NO hereda.
            style.text_anchor = p.text_anchor;
            style.color_rendering = p.color_rendering;
            style.color_interpolation_filters = p.color_interpolation_filters;
            style.glyph_orientation_vertical = p.glyph_orientation_vertical;
            // SVG 2 — marker-{start,mid,end} heredan; mask-type
            // (CSS Masking 1) NO hereda.
            style.marker_start = p.marker_start.clone();
            style.marker_mid = p.marker_mid.clone();
            style.marker_end = p.marker_end.clone();
            // CSS Values 5 — interpolate-size hereda (CSS Animations 2 elige
            // si keywords como `auto` participan de transitions/animations).
            style.interpolate_size = p.interpolate_size;
            // CSS UI 4 — interactivity hereda (inert se propaga al subtree).
            style.interactivity = p.interactivity;
            // Fase 7.975/7.981/7.983/7.984 — text-align-all (CSS Text 4),
            // kerning (SVG), color-profile (SVG) y voice-range (CSS Speech)
            // heredan. El resto del bloque 7.966-7.985 NO hereda.
            style.text_align_all = p.text_align_all.clone();
            style.kerning = p.kerning.clone();
            style.color_profile = p.color_profile.clone();
            style.voice_range = p.voice_range.clone();
            // Fase 7.991-7.994/7.997/7.1004/7.1005 — line-stacking{,-ruby,-shift,
            // -strategy} (CSS Line Layout 3), text-height, block-progression
            // (legacy) y snap-height heredan. El resto del bloque NO hereda.
            style.line_stacking = p.line_stacking.clone();
            style.line_stacking_ruby = p.line_stacking_ruby.clone();
            style.line_stacking_shift = p.line_stacking_shift.clone();
            style.line_stacking_strategy = p.line_stacking_strategy.clone();
            style.text_height = p.text_height.clone();
            style.block_progression = p.block_progression.clone();
            style.snap_height = p.snap_height.clone();
            // Fase 7.1036/7.1037/7.1041 — -moz-user-focus, -moz-user-input
            // y -moz-image-region heredan en Gecko. El resto del bloque
            // -moz- (7.1035/7.1038-7.1040/7.1042) NO hereda.
            style.moz_user_focus = p.moz_user_focus.clone();
            style.moz_user_input = p.moz_user_input.clone();
            style.moz_image_region = p.moz_image_region.clone();
            // Fase 7.1058/7.1060 — -moz-context-properties y -moz-text-blink
            // heredan en Gecko. El resto del bloque misc NO hereda.
            style.moz_context_properties = p.moz_context_properties.clone();
            style.moz_text_blink = p.moz_text_blink.clone();
            // Fase 7.1063-7.1072 — CSS Fill and Stroke 3: las stroke-*
            // heredan (igual que el resto de fill/stroke, tradición SVG).
            style.stroke_align = p.stroke_align.clone();
            style.stroke_break = p.stroke_break.clone();
            style.stroke_color_css = p.stroke_color_css.clone();
            style.stroke_image = p.stroke_image.clone();
            style.stroke_origin = p.stroke_origin.clone();
            style.stroke_position = p.stroke_position.clone();
            style.stroke_repeat = p.stroke_repeat.clone();
            style.stroke_size = p.stroke_size.clone();
            style.stroke_dash_corner = p.stroke_dash_corner.clone();
            style.stroke_dash_justify = p.stroke_dash_justify.clone();
        }
        // Font-size heredado (antes de la cascada): base contra la que se
        // resuelven `em`/`%`/`larger`/`smaller` de este elemento. Ver Fase 7.223.
        let inherited_font_size = style.font_size;
        let Some(local) = dom::element_name(node) else {
            return style;
        };
        // Defaults por tag — `div`/`p`/`h1` son block. `display` no
        // hereda, así que siempre se setea según el tag local.
        style.display = default_display(&local);

        // `font_weight` por tag (h1..h6/b/strong/th = bold) override
        // el heredado — un `<b>` dentro de un `<p>` no-bold sigue
        // siendo bold.
        let weight_default = default_weight(&local);
        if weight_default != 400 {
            style.font_weight = weight_default;
        }
        // `font_style` por tag (em/i/cite/dfn/var/address = italic).
        // Override el heredado por defecto pero NO si el padre ya lo es
        // (`<em><span>foo</span></em>` debe conservar italic en el span).
        if default_italic(&local) {
            style.font_style = FontStyle::Italic;
        }

        // Cascada en dos pasadas:
        //   1. Decls normales, ordenadas por (specificity, source_index).
        //   2. Decls `!important`, ordenadas igual.
        // Cada decl individual lleva su flag — una misma regla puede
        // tener decls normales y `!important` mezcladas.
        let matched: Vec<(u32, usize, &Rule)> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                // Filtramos por pseudo-element: cuando computamos un
                // pseudo, sólo nos interesan reglas con ese mismo
                // pseudo_element en el selector; cuando computamos el
                // elemento real (target_pseudo=None), ignoramos las
                // reglas que generan pseudo-elements (sino sus decls
                // pegarían al padre).
                r.selector.pseudo_element == target_pseudo
                    && r.matches_in_state(node, hover_active, focus_active)
            })
            .map(|(i, r)| (r.selector.specificity(), i, r))
            .collect();
        // Inline `style="..."` no aplica a pseudo-elements (no podés
        // setear `::before` desde el HTML inline). Sólo lo recogemos
        // cuando computamos el elemento real.
        let inline_decls: Vec<Decl> = if target_pseudo.is_some() {
            Vec::new()
        } else {
            dom::attr(node, "style")
                .map(|s| parse_declarations(&s, &self.vars))
                .unwrap_or_default()
        };

        // Default para resolver `initial`/`unset` de keywords CSS-wide.
        let wide_default = ComputedStyle::default();

        // PASADA 1 — normales.
        let mut normal_apps: Vec<(u32, usize, &Decl)> = Vec::new();
        for (spec, src, rule) in &matched {
            for d in &rule.decls {
                if !d.important {
                    normal_apps.push((*spec, *src, d));
                }
            }
        }
        normal_apps.sort_by_key(|(spec, idx, _)| (*spec, *idx));
        for (_, _, d) in normal_apps {
            apply_decl(d, &mut style, parent, &wide_default);
        }
        // Inline normal (especificidad 1000) cierra la pasada normal.
        for d in &inline_decls {
            if !d.important {
                apply_decl(d, &mut style, parent, &wide_default);
            }
        }

        // PASADA 2 — `!important`. Cualquier important de cualquier
        // regla vence cualquier normal — y entre importants, vuelve a
        // mandar especificidad/orden.
        let mut imp_apps: Vec<(u32, usize, &Decl)> = Vec::new();
        for (spec, src, rule) in &matched {
            for d in &rule.decls {
                if d.important {
                    imp_apps.push((*spec, *src, d));
                }
            }
        }
        imp_apps.sort_by_key(|(spec, idx, _)| (*spec, *idx));
        for (_, _, d) in imp_apps {
            apply_decl(d, &mut style, parent, &wide_default);
        }
        // Inline `!important` (efectiva 10_000 en CSS real, pero acá
        // simplemente cierra la pasada — gana todo lo anterior).
        for d in &inline_decls {
            if d.important {
                apply_decl(d, &mut style, parent, &wide_default);
            }
        }
        // `font-size` relativo: resuelto al cierre contra el font-size
        // heredado (no contra un font-size absoluto del mismo elemento —
        // ese ya habría limpiado el flag en `apply`). Se limpia el buffer.
        if let Some(m) = style.font_size_rel.take() {
            style.font_size = inherited_font_size * m;
        }

        // `currentColor`: resuelto al cierre contra el `color` ya computado
        // (used value). Se vacía el buffer para que NO se herede ni viaje
        // al BoxNode.
        if !style.current_color.is_empty() {
            let cc = style.color;
            for target in std::mem::take(&mut style.current_color) {
                match target {
                    ColorTarget::Background => style.background = Some(cc),
                    ColorTarget::BorderAll => style.border_colors = Sides::all(Some(cc)),
                    ColorTarget::BorderSide(edge) => {
                        set_side(&mut style.border_colors, edge, Some(cc))
                    }
                    ColorTarget::Outline => style.outline.color = Some(cc),
                }
            }
        }

        // Props individuales `translate`/`rotate`/`scale` (CSS Transforms 2):
        // se prependean a la lista `transform` en ese orden, así el render
        // (que multiplica la cadena de izquierda a derecha) compone la matriz
        // como translate·rotate·scale·transform-list. Used value.
        if style.translate.is_some() || style.rotate.is_some() || style.scale.is_some() {
            let mut chain = Vec::with_capacity(style.transforms.len() + 3);
            chain.extend(style.translate);
            chain.extend(style.rotate);
            chain.extend(style.scale);
            chain.append(&mut style.transforms);
            style.transforms = chain;
        }
        style
    }
}

/// Aplica una declaración sobre el estilo. Los keywords CSS-wide
/// (`inherit`/`initial`/`unset`) se resuelven acá porque necesitan el
/// estilo del padre y el default; el resto delega en `Decl::apply`.
/// Fase 7.225.
fn apply_decl(
    d: &Decl,
    s: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    default: &ComputedStyle,
) {
    if let DeclKind::Wide { prop, kw } = &d.kind {
        resolve_wide(*prop, *kw, s, parent, default);
    } else {
        d.apply(s);
    }
}

/// Resuelve un keyword CSS-wide copiando el valor de la propiedad desde el
/// padre (`inherit`, o `unset` sobre prop heredable) o el default (`initial`,
/// o `unset` sobre prop no-heredable). Sin padre, `inherit` cae al default.
fn resolve_wide(
    prop: WideProp,
    kw: WideKw,
    s: &mut ComputedStyle,
    parent: Option<&ComputedStyle>,
    default: &ComputedStyle,
) {
    let use_parent = match kw {
        WideKw::Inherit => true,
        WideKw::Initial => false,
        WideKw::Unset => prop.is_inherited(),
    };
    let src = if use_parent { parent.unwrap_or(default) } else { default };
    match prop {
        WideProp::Color => s.color = src.color,
        WideProp::Background => s.background = src.background,
        WideProp::FontSize => {
            s.font_size = src.font_size;
            // Un font-size concreto descarta cualquier relativo pendiente.
            s.font_size_rel = None;
        }
        WideProp::FontWeight => s.font_weight = src.font_weight,
        WideProp::FontStyle => s.font_style = src.font_style,
        WideProp::FontFamily => s.font_family = src.font_family.clone(),
        WideProp::LineHeight => s.line_height = src.line_height,
        WideProp::TextAlign => s.text_align = src.text_align,
        WideProp::TextDecoration => s.text_decoration = src.text_decoration,
        WideProp::Visibility => s.visibility = src.visibility,
        WideProp::Display => s.display = src.display,
        WideProp::BoxSizing => s.box_sizing = src.box_sizing,
        WideProp::BorderColor => s.border_colors = src.border_colors,
    }
}

#[cfg(test)]
mod tests;
