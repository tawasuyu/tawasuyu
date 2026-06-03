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
        for sheet in sheets {
            rules.extend(parse_stylesheet(sheet, &vars, vp));
        }
        Self { rules, vars, keyframes }
    }

    /// Tabla de `@keyframes` parseados (name → definición). Vacía si el
    /// documento no declara animaciones. El runtime de tween (Fase B4+)
    /// la cruzará con `ComputedStyle::animation`; hoy es sólo lectura.
    pub fn keyframes(&self) -> &HashMap<String, Keyframes> {
        &self.keyframes
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
mod tests {
    use super::*;

    #[test]
    fn parsea_hex_color() {
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("red"), Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn parsea_radial_gradient() {
        let grad = |v: &str| match parse_background_image(v) {
            Some(DeclKind::BackgroundGradient(g)) => g,
            other => panic!("esperaba gradiente, {other:?}"),
        };
        // Sin prelude: default farthest-corner at center, 2 stops.
        let g = grad("radial-gradient(red, blue)");
        let spec = g.radial().expect("debe ser radial");
        assert_eq!(spec.size, RadialSize::FarthestCorner);
        assert_eq!(spec.cx, LengthVal::Pct(50.0));
        assert_eq!(spec.cy, LengthVal::Pct(50.0));
        assert_eq!(g.stops.len(), 2);
        // shape + size + posición.
        let g = grad("radial-gradient(circle closest-side at 30% 70%, red 0%, blue 100%)");
        let spec = g.radial().unwrap();
        assert_eq!(spec.size, RadialSize::ClosestSide);
        assert_eq!(spec.cx, LengthVal::Pct(30.0));
        assert_eq!(spec.cy, LengthVal::Pct(70.0));
        // Sólo `at <pos>` con keywords.
        let g = grad("radial-gradient(at top left, red, blue)");
        let spec = g.radial().unwrap();
        assert_eq!(spec.cx, LengthVal::Pct(0.0));
        assert_eq!(spec.cy, LengthVal::Pct(0.0));
        // El lineal sigue sin radial.
        assert!(grad("linear-gradient(to right, red, blue)").radial().is_none());
    }

    #[test]
    fn parsea_conic_gradient() {
        let grad = |v: &str| match parse_background_image(v) {
            Some(DeclKind::BackgroundGradient(g)) => g,
            other => panic!("esperaba gradiente, {other:?}"),
        };
        let conic = |g: &LinearGradient| match g.geometry {
            GradientGeometry::Conic { from_deg, cx, cy } => (from_deg, cx, cy),
            other => panic!("esperaba conic, {other:?}"),
        };
        // Sin prelude: from 0 at center.
        let (from, cx, cy) = conic(&grad("conic-gradient(red, blue)"));
        assert_eq!(from, 0.0);
        assert_eq!(cx, LengthVal::Pct(50.0));
        assert_eq!(cy, LengthVal::Pct(50.0));
        // from <angle> + at <pos>; turn → grados.
        let (from, cx, cy) = conic(&grad("conic-gradient(from 0.25turn at 20% 80%, red, blue)"));
        assert!((from - 90.0).abs() < 1e-3);
        assert_eq!(cx, LengthVal::Pct(20.0));
        assert_eq!(cy, LengthVal::Pct(80.0));
        // Sólo `from <deg>`.
        let (from, _, _) = conic(&grad("conic-gradient(from 45deg, red, blue)"));
        assert!((from - 45.0).abs() < 1e-3);
        assert_eq!(grad("conic-gradient(red, blue)").stops.len(), 2);

        // Posiciones de stop angulares: `90deg`/`0.25turn` → Px(grados); `%`
        // sigue siendo Pct. El render trata el eje cónico como 360°.
        let g = grad("conic-gradient(red 90deg, blue 0.25turn, lime 75%)");
        assert_eq!(g.stops[0].pos, Some(LengthVal::Px(90.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(90.0)));
        assert_eq!(g.stops[2].pos, Some(LengthVal::Pct(75.0)));
        // Doble posición angular `red 0deg 90deg` ⇒ dos stops.
        let g = grad("repeating-conic-gradient(red 0deg 90deg, blue 90deg 180deg)");
        assert_eq!(g.stops.len(), 4);
        assert_eq!(g.stops[0].pos, Some(LengthVal::Px(0.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(90.0)));
    }

    #[test]
    fn parsea_repeating_gradients_y_stops_px() {
        let grad = |v: &str| match parse_background_image(v) {
            Some(DeclKind::BackgroundGradient(g)) => g,
            other => panic!("esperaba gradiente, {other:?}"),
        };

        // `repeating-*` activa el flag; el no-repetido lo deja en false.
        assert!(grad("repeating-linear-gradient(red, blue 20px)").repeating);
        assert!(grad("repeating-radial-gradient(circle, red, blue 30px)").repeating);
        assert!(grad("repeating-conic-gradient(red, blue 25%)").repeating);
        assert!(!grad("linear-gradient(red, blue)").repeating);
        assert!(matches!(
            grad("repeating-linear-gradient(45deg, red, blue 10px)").geometry,
            GradientGeometry::Linear { .. }
        ));

        // Posiciones de stop: % → Pct, px → Px reales (no la vieja heurística
        // /100), `auto`/sin posición → None.
        let g = grad("linear-gradient(red 40%, blue 30px)");
        assert_eq!(g.stops[0].pos, Some(LengthVal::Pct(40.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(30.0)));

        // Doble posición `#ccc 0 10px` ⇒ dos stops del mismo color (franjas).
        let g = grad("repeating-linear-gradient(#ccc 0 10px, #fff 10px 20px)");
        assert_eq!(g.stops.len(), 4);
        assert_eq!(g.stops[0].color, g.stops[1].color);
        assert_eq!(g.stops[0].pos, Some(LengthVal::Px(0.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(10.0)));
        assert_eq!(g.stops[2].color, g.stops[3].color);
        assert_eq!(g.stops[3].pos, Some(LengthVal::Px(20.0)));
    }

    #[test]
    fn parsea_named_colors_extendidos() {
        // Tabla CSS3 completa: colores que antes dropeaban la declaración.
        assert_eq!(parse_color("coral"), Some(Color::rgb(255, 127, 80)));
        assert_eq!(parse_color("tomato"), Some(Color::rgb(255, 99, 71)));
        assert_eq!(parse_color("slateblue"), Some(Color::rgb(106, 90, 205)));
        assert_eq!(parse_color("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
        assert_eq!(parse_color("darkslategray"), Some(Color::rgb(47, 79, 79)));
        // Case-insensitive + variante grey.
        assert_eq!(parse_color("SteelBlue"), Some(Color::rgb(70, 130, 180)));
        assert_eq!(parse_color("dimgrey"), Some(Color::rgb(105, 105, 105)));
        // No-color sigue siendo None.
        assert_eq!(parse_color("notacolor"), None);
    }

    #[test]
    fn parsea_length() {
        assert_eq!(parse_length_px("12px"), Some(12.0));
        assert_eq!(parse_length_px("1.5em"), Some(24.0));
        assert_eq!(parse_length_px("0"), Some(0.0));
        assert_eq!(parse_length_px("xyz"), None);
    }

    #[test]
    fn parse_content_value_acepta_string_quoted() {
        assert_eq!(
            parse_content_value(r#""hola""#),
            Some(vec![ContentItem::Text("hola".into())])
        );
        assert_eq!(
            parse_content_value(r#"'mundo'"#),
            Some(vec![ContentItem::Text("mundo".into())])
        );
        assert_eq!(parse_content_value("none"), None);
        assert_eq!(parse_content_value("normal"), None);
        // Sin comillas y sin counter()/attr() → None.
        assert_eq!(parse_content_value("foo"), None);
    }

    #[test]
    fn parse_content_value_respeta_escapes() {
        assert_eq!(
            parse_content_value(r#""linea1\nlinea2""#),
            Some(vec![ContentItem::Text("linea1nlinea2".into())]) // \n no especial
        );
        assert_eq!(
            parse_content_value(r#""con \"quote\" adentro""#),
            Some(vec![ContentItem::Text(r#"con "quote" adentro"#.into())])
        );
    }

    #[test]
    fn parse_content_value_concat_counter_attr() {
        let items = parse_content_value(r#""Sección " counter(sec) ": " attr(data-title)"#)
            .expect("debería parsear");
        assert_eq!(
            items,
            vec![
                ContentItem::Text("Sección ".into()),
                ContentItem::Counter("sec".into()),
                ContentItem::Text(": ".into()),
                ContentItem::Attr("data-title".into()),
            ]
        );
    }

    #[test]
    fn parse_counter_list_acepta_pares_y_defaults() {
        assert_eq!(
            parse_counter_list("section 0 chapter 5", 0),
            vec![("section".into(), 0), ("chapter".into(), 5)]
        );
        // Default cuando no hay valor explícito.
        assert_eq!(
            parse_counter_list("h2", 1),
            vec![("h2".into(), 1)]
        );
        assert_eq!(parse_counter_list("none", 0), Vec::<(String, i32)>::new());
    }

    #[test]
    fn pseudo_element_extrae_del_selector() {
        let html = r##"<html><head><style>
            p::before { content: "PRE " }
            p::after { content: " POST" }
            p:before { content: "legacy" }
        </style></head><body><p>x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let before = eng.compute_pseudo(&p, PseudoElement::Before, None);
        let after = eng.compute_pseudo(&p, PseudoElement::After, None);
        // `:before` legacy también matchea Before pero llega después; el
        // último gana en empate de especificidad.
        assert_eq!(
            before.and_then(|s| s.content),
            Some(vec![ContentItem::Text("legacy".into())])
        );
        assert_eq!(
            after.and_then(|s| s.content),
            Some(vec![ContentItem::Text(" POST".into())])
        );
    }

    #[test]
    fn pseudo_element_sin_content_no_se_materializa() {
        // Una regla `::before` sin content → compute_pseudo devuelve None.
        let html = r##"<html><head><style>
            p::before { color: red }
        </style></head><body><p>x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert!(eng.compute_pseudo(&p, PseudoElement::Before, None).is_none());
    }

    #[test]
    fn reglas_pseudo_no_pegan_al_elemento_real() {
        // `p::before { color: red }` NO debe afectar el color de `<p>`
        // — sólo de su `::before`.
        let html = r##"<html><head><style>
            p::before { content: "X"; color: red }
            p { color: blue }
        </style></head><body><p>texto</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.color, Color::rgb(0, 0, 255)); // blue, no red
    }

    #[test]
    fn parsea_z_index() {
        let html = r##"<html><head><style>
            .a { z-index: 5 }
            .b { z-index: -2 }
            .c { z-index: auto }
        </style></head><body>
            <div class="a"></div>
            <div class="b"></div>
            <div class="c"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 3);
        assert_eq!(eng.compute(&divs[0]).z_index, 5);
        assert_eq!(eng.compute(&divs[1]).z_index, -2);
        assert_eq!(eng.compute(&divs[2]).z_index, 0); // auto → 0
    }

    #[test]
    fn parsea_object_fit_y_llega_a_computed() {
        // Parser: keywords válidos (case-insensitive) e inválido → None.
        assert_eq!(parse_object_fit("cover"), Some(ObjectFit::Cover));
        assert_eq!(parse_object_fit("scale-down"), Some(ObjectFit::ScaleDown));
        assert_eq!(parse_object_fit("CONTAIN"), Some(ObjectFit::Contain));
        assert_eq!(parse_object_fit("fill"), Some(ObjectFit::Fill));
        assert_eq!(parse_object_fit("stretch"), None);

        let html = r##"<html><head><style>
            img.cov { object-fit: cover }
            img.plain { color: red }
        </style></head><body>
            <img class="cov" src="x.png">
            <img class="plain" src="y.png">
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut imgs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("img") {
                imgs.push(n.clone());
            }
        });
        assert_eq!(imgs.len(), 2);
        assert_eq!(eng.compute(&imgs[0]).object_fit, Some(ObjectFit::Cover));
        // Sin object-fit declarado → None (el chrome mantiene su encaje
        // por defecto, contain responsivo vía el compositor).
        assert_eq!(eng.compute(&imgs[1]).object_fit, None);
    }

    #[test]
    fn parsea_object_position_reusa_background_position() {
        let html = r##"<html><head><style>
            img.tr { object-position: right top }
            img.pct { object-position: 25% 75% }
            img.plain { color: red }
        </style></head><body>
            <img class="tr" src="a.png">
            <img class="pct" src="b.png">
            <img class="plain" src="c.png">
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut imgs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("img") {
                imgs.push(n.clone());
            }
        });
        assert_eq!(imgs.len(), 3);
        // `right top` → x=100% (derecha), y=0% (arriba).
        assert_eq!(
            eng.compute(&imgs[0]).object_position,
            Some(BackgroundPosition { x: LengthVal::Pct(100.0), y: LengthVal::Pct(0.0) })
        );
        assert_eq!(
            eng.compute(&imgs[1]).object_position,
            Some(BackgroundPosition { x: LengthVal::Pct(25.0), y: LengthVal::Pct(75.0) })
        );
        // Sin declarar → None (el chrome centra).
        assert_eq!(eng.compute(&imgs[2]).object_position, None);
    }

    #[test]
    fn caret_color_fase_7_238() {
        // Parser puro.
        assert_eq!(parse_caret_color("auto"), None);
        assert_eq!(parse_caret_color("AUTO"), None);
        assert_eq!(parse_caret_color("currentColor"), None);
        assert_eq!(parse_caret_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_caret_color("zigzag"), None);

        // End-to-end: declarado, sin declarar, y herencia padre→hijo
        // (vía `compute_with_parent` — `compute()` no traversa).
        let html = r##"<html><head><style>
            body { caret-color: #00ff00 }
            input.a { caret-color: red }
            input.auto { caret-color: auto }
            input.plain {}
        </style></head><body>
          <input class="a"><input class="auto"><input class="plain">
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("input") => inputs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.caret_color, Some(Color::rgb(0, 255, 0)));
        assert_eq!(inputs.len(), 3);
        assert_eq!(
            eng.compute_with_parent(&inputs[0], Some(&body_cs)).caret_color,
            Some(Color::rgb(255, 0, 0))
        );
        assert_eq!(eng.compute_with_parent(&inputs[1], Some(&body_cs)).caret_color, None);
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&inputs[2], Some(&body_cs)).caret_color,
            Some(Color::rgb(0, 255, 0))
        );
    }

    #[test]
    fn accent_color_fase_7_239() {
        assert_eq!(parse_auto_or_color("auto"), None);
        assert_eq!(parse_auto_or_color("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
        assert_eq!(parse_auto_or_color("zigzag"), None);

        let html = r##"<html><head><style>
            body { accent-color: #112233 }
            input.a { accent-color: blue }
            input.auto { accent-color: auto }
            input.plain {}
        </style></head><body>
          <input class="a"><input class="auto"><input class="plain">
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("input") => inputs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.accent_color, Some(Color::rgb(0x11, 0x22, 0x33)));
        assert_eq!(
            eng.compute_with_parent(&inputs[0], Some(&body_cs)).accent_color,
            Some(Color::rgb(0, 0, 255))
        );
        assert_eq!(eng.compute_with_parent(&inputs[1], Some(&body_cs)).accent_color, None);
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&inputs[2], Some(&body_cs)).accent_color,
            Some(Color::rgb(0x11, 0x22, 0x33))
        );
    }

    #[test]
    fn cursor_fase_7_240() {
        // Parser puro: keywords reconocidos + fallback `auto` para
        // lo no soportado + tail-of-list (CSS `cursor: url(...), pointer`).
        assert_eq!(parse_cursor("pointer"), Some(Cursor::Pointer));
        assert_eq!(parse_cursor("POINTER"), Some(Cursor::Pointer));
        assert_eq!(parse_cursor("not-allowed"), Some(Cursor::NotAllowed));
        assert_eq!(parse_cursor("zoom-in"), Some(Cursor::ZoomIn));
        assert_eq!(parse_cursor("nesw-resize"), Some(Cursor::NeswResize));
        assert_eq!(parse_cursor("xyz"), Some(Cursor::Auto));
        // Lista CSS — tomamos el último fallback reconocido.
        assert_eq!(parse_cursor("url(a.png), pointer"), Some(Cursor::Pointer));
        assert_eq!(parse_cursor("url(a.png), nope"), Some(Cursor::Auto));

        // End-to-end: declarado y heredado.
        let html = r##"<html><head><style>
            body { cursor: text }
            a.btn { cursor: pointer }
            a.plain {}
        </style></head><body>
          <a class="btn">x</a><a class="plain">y</a>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut anchors = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("a") => anchors.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.cursor, Cursor::Text);
        assert_eq!(eng.compute_with_parent(&anchors[0], Some(&body_cs)).cursor, Cursor::Pointer);
        // Heredado de body.
        assert_eq!(eng.compute_with_parent(&anchors[1], Some(&body_cs)).cursor, Cursor::Text);
    }

    #[test]
    fn text_overflow_fase_7_241() {
        assert_eq!(parse_text_overflow("clip"), Some(TextOverflow::Clip));
        assert_eq!(parse_text_overflow("ELLIPSIS"), Some(TextOverflow::Ellipsis));
        assert_eq!(parse_text_overflow("fade"), None);

        let html = r##"<html><head><style>
            body { text-overflow: ellipsis }
            p.a { text-overflow: clip }
            p.plain {}
        </style></head><body>
          <p class="a"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_overflow, TextOverflow::Ellipsis);
        // text-overflow NO hereda — el hijo sin declarar mantiene el default (Clip),
        // no toma el `ellipsis` del body.
        let p_a = eng.compute_with_parent(&ps[0], Some(&body_cs));
        assert_eq!(p_a.text_overflow, TextOverflow::Clip);
        let p_plain = eng.compute_with_parent(&ps[1], Some(&body_cs));
        assert_eq!(p_plain.text_overflow, TextOverflow::Clip);
    }

    #[test]
    fn scroll_behavior_fase_7_242() {
        assert_eq!(parse_scroll_behavior("auto"), Some(ScrollBehavior::Auto));
        assert_eq!(parse_scroll_behavior("SMOOTH"), Some(ScrollBehavior::Smooth));
        assert_eq!(parse_scroll_behavior("instant"), None);

        let html = r##"<html><head><style>
            body { scroll-behavior: smooth }
            div.a { scroll-behavior: auto }
            div.plain {}
        </style></head><body>
          <div class="a"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.scroll_behavior, ScrollBehavior::Smooth);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_behavior,
            ScrollBehavior::Auto
        );
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).scroll_behavior,
            ScrollBehavior::Smooth
        );
    }

    #[test]
    fn tab_size_fase_7_243() {
        assert_eq!(parse_tab_size("4"), Some(TabSize::Chars(4)));
        assert_eq!(parse_tab_size("0"), Some(TabSize::Chars(0)));
        assert_eq!(parse_tab_size("32px"), Some(TabSize::Px(32.0)));
        assert_eq!(parse_tab_size("-1"), None);
        assert_eq!(parse_tab_size("xx"), None);

        let html = r##"<html><head><style>
            body { tab-size: 4 }
            pre.a { tab-size: 16px }
            pre.plain {}
        </style></head><body>
          <pre class="a"></pre><pre class="plain"></pre>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut pres = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("pre") => pres.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.tab_size, TabSize::Chars(4));
        assert_eq!(
            eng.compute_with_parent(&pres[0], Some(&body_cs)).tab_size,
            TabSize::Px(16.0)
        );
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&pres[1], Some(&body_cs)).tab_size,
            TabSize::Chars(4)
        );
    }

    #[test]
    fn user_select_fase_7_244() {
        assert_eq!(parse_user_select("none"), Some(UserSelect::None));
        assert_eq!(parse_user_select("TEXT"), Some(UserSelect::Text));
        assert_eq!(parse_user_select("all"), Some(UserSelect::All));
        assert_eq!(parse_user_select("contain"), Some(UserSelect::Contain));
        assert_eq!(parse_user_select("auto"), Some(UserSelect::Auto));
        assert_eq!(parse_user_select("nada"), None);

        let html = r##"<html><head><style>
            body { user-select: text }
            div.lock { user-select: none }
            div.plain {}
        </style></head><body>
          <div class="lock"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.user_select, UserSelect::Text);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).user_select,
            UserSelect::None
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).user_select,
            UserSelect::Text
        );
    }

    #[test]
    fn overflow_wrap_fase_7_245() {
        assert_eq!(parse_overflow_wrap("normal"), Some(OverflowWrap::Normal));
        assert_eq!(parse_overflow_wrap("break-word"), Some(OverflowWrap::BreakWord));
        assert_eq!(parse_overflow_wrap("ANYWHERE"), Some(OverflowWrap::Anywhere));
        assert_eq!(parse_overflow_wrap("nope"), None);

        // `word-wrap` alias legacy.
        let html = r##"<html><head><style>
            body { word-wrap: break-word }
            p.b {}
            p.over { overflow-wrap: anywhere }
        </style></head><body>
          <p class="b"></p><p class="over"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.overflow_wrap, OverflowWrap::BreakWord);
        // Heredado del body.
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).overflow_wrap,
            OverflowWrap::BreakWord
        );
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).overflow_wrap,
            OverflowWrap::Anywhere
        );
    }

    #[test]
    fn word_break_fase_7_246() {
        assert_eq!(parse_word_break("normal"), Some(WordBreak::Normal));
        assert_eq!(parse_word_break("break-all"), Some(WordBreak::BreakAll));
        assert_eq!(parse_word_break("keep-all"), Some(WordBreak::KeepAll));
        // `break-word` legacy → Normal por compat.
        assert_eq!(parse_word_break("break-word"), Some(WordBreak::Normal));
        assert_eq!(parse_word_break("nada"), None);

        let html = r##"<html><head><style>
            body { word-break: break-all }
            p.k { word-break: keep-all }
            p.plain {}
        </style></head><body>
          <p class="k"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.word_break, WordBreak::BreakAll);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).word_break,
            WordBreak::KeepAll
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).word_break,
            WordBreak::BreakAll
        );
    }

    #[test]
    fn hyphens_fase_7_247() {
        assert_eq!(parse_hyphens("none"), Some(Hyphens::None));
        assert_eq!(parse_hyphens("MANUAL"), Some(Hyphens::Manual));
        assert_eq!(parse_hyphens("auto"), Some(Hyphens::Auto));
        assert_eq!(parse_hyphens("x"), None);

        let html = r##"<html><head><style>
            body { -webkit-hyphens: auto }
            p.off { hyphens: none }
            p.plain {}
        </style></head><body>
          <p class="off"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.hyphens, Hyphens::Auto);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).hyphens,
            Hyphens::None
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).hyphens,
            Hyphens::Auto
        );
    }

    #[test]
    fn resize_fase_7_248() {
        assert_eq!(parse_resize("none"), Some(Resize::None));
        assert_eq!(parse_resize("both"), Some(Resize::Both));
        assert_eq!(parse_resize("HORIZONTAL"), Some(Resize::Horizontal));
        assert_eq!(parse_resize("vertical"), Some(Resize::Vertical));
        assert_eq!(parse_resize("block"), Some(Resize::Block));
        assert_eq!(parse_resize("inline"), Some(Resize::Inline));
        assert_eq!(parse_resize("auto"), None);

        // `resize` NO se hereda (CSS UI 4).
        let html = r##"<html><head><style>
            body { resize: both }
            div.r { resize: vertical }
            div.plain {}
        </style></head><body>
          <div class="r"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.resize, Resize::Both);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).resize,
            Resize::Vertical
        );
        // NO se hereda → default `None`.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).resize,
            Resize::None
        );
    }

    #[test]
    fn writing_mode_fase_7_249() {
        assert_eq!(parse_writing_mode("horizontal-tb"), Some(WritingMode::HorizontalTb));
        assert_eq!(parse_writing_mode("VERTICAL-RL"), Some(WritingMode::VerticalRl));
        assert_eq!(parse_writing_mode("vertical-lr"), Some(WritingMode::VerticalLr));
        assert_eq!(parse_writing_mode("sideways-rl"), Some(WritingMode::SidewaysRl));
        assert_eq!(parse_writing_mode("sideways-lr"), Some(WritingMode::SidewaysLr));
        // Aliases legacy fuera de scope.
        assert_eq!(parse_writing_mode("lr-tb"), None);
        assert_eq!(parse_writing_mode("nope"), None);

        let html = r##"<html><head><style>
            body { writing-mode: vertical-rl }
            p.over { writing-mode: horizontal-tb }
            p.plain {}
        </style></head><body>
          <p class="over"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.writing_mode, WritingMode::VerticalRl);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).writing_mode,
            WritingMode::HorizontalTb
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).writing_mode,
            WritingMode::VerticalRl
        );
    }

    #[test]
    fn direction_fase_7_250() {
        assert_eq!(parse_direction("ltr"), Some(Direction::Ltr));
        assert_eq!(parse_direction("RTL"), Some(Direction::Rtl));
        assert_eq!(parse_direction("auto"), None);

        let html = r##"<html><head><style>
            body { direction: rtl }
            div.lr { direction: ltr }
            div.plain {}
        </style></head><body>
          <div class="lr"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.direction, Direction::Rtl);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).direction,
            Direction::Ltr
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).direction,
            Direction::Rtl
        );
    }

    #[test]
    fn unicode_bidi_fase_7_251() {
        assert_eq!(parse_unicode_bidi("normal"), Some(UnicodeBidi::Normal));
        assert_eq!(parse_unicode_bidi("embed"), Some(UnicodeBidi::Embed));
        assert_eq!(parse_unicode_bidi("ISOLATE"), Some(UnicodeBidi::Isolate));
        assert_eq!(parse_unicode_bidi("bidi-override"), Some(UnicodeBidi::BidiOverride));
        assert_eq!(parse_unicode_bidi("isolate-override"), Some(UnicodeBidi::IsolateOverride));
        assert_eq!(parse_unicode_bidi("plaintext"), Some(UnicodeBidi::Plaintext));
        assert_eq!(parse_unicode_bidi("xxx"), None);

        // `unicode-bidi` NO se hereda (CSS Writing Modes 3).
        let html = r##"<html><head><style>
            body { unicode-bidi: embed }
            span.b { unicode-bidi: isolate }
            span.plain {}
        </style></head><body>
          <span class="b"></span><span class="plain"></span>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.unicode_bidi, UnicodeBidi::Embed);
        assert_eq!(
            eng.compute_with_parent(&spans[0], Some(&body_cs)).unicode_bidi,
            UnicodeBidi::Isolate
        );
        // NO se hereda → default Normal.
        assert_eq!(
            eng.compute_with_parent(&spans[1], Some(&body_cs)).unicode_bidi,
            UnicodeBidi::Normal
        );
    }

    #[test]
    fn font_stretch_fase_7_252() {
        // Keywords.
        assert!((parse_font_stretch("normal").unwrap() - 1.0).abs() < 1e-3);
        assert!((parse_font_stretch("CONDENSED").unwrap() - 0.75).abs() < 1e-3);
        assert!((parse_font_stretch("ultra-expanded").unwrap() - 2.0).abs() < 1e-3);
        assert!((parse_font_stretch("ultra-condensed").unwrap() - 0.50).abs() < 1e-3);
        // Porcentaje.
        assert!((parse_font_stretch("125%").unwrap() - 1.25).abs() < 1e-3);
        // Clamp: 300% → 200%.
        assert!((parse_font_stretch("300%").unwrap() - 2.0).abs() < 1e-3);
        assert!((parse_font_stretch("10%").unwrap() - 0.5).abs() < 1e-3);
        assert_eq!(parse_font_stretch("nope"), None);

        let html = r##"<html><head><style>
            body { font-stretch: expanded }
            p.c { font-stretch: 75% }
            p.plain {}
        </style></head><body>
          <p class="c"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!((body_cs.font_stretch - 1.25).abs() < 1e-3);
        assert!(
            (eng.compute_with_parent(&ps[0], Some(&body_cs)).font_stretch - 0.75).abs() < 1e-3
        );
        // Heredado.
        assert!(
            (eng.compute_with_parent(&ps[1], Some(&body_cs)).font_stretch - 1.25).abs() < 1e-3
        );
    }

    #[test]
    fn image_rendering_fase_7_253() {
        assert_eq!(parse_image_rendering("auto"), Some(ImageRendering::Auto));
        assert_eq!(parse_image_rendering("SMOOTH"), Some(ImageRendering::Smooth));
        assert_eq!(parse_image_rendering("crisp-edges"), Some(ImageRendering::CrispEdges));
        assert_eq!(parse_image_rendering("pixelated"), Some(ImageRendering::Pixelated));
        // Legacy CSS2 → Auto.
        assert_eq!(parse_image_rendering("optimizeSpeed"), Some(ImageRendering::Auto));
        assert_eq!(parse_image_rendering("nope"), None);

        let html = r##"<html><head><style>
            body { image-rendering: pixelated }
            img.over { image-rendering: smooth }
            img.plain {}
        </style></head><body>
          <img class="over"/><img class="plain"/>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut imgs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("img") => imgs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.image_rendering, ImageRendering::Pixelated);
        assert_eq!(
            eng.compute_with_parent(&imgs[0], Some(&body_cs)).image_rendering,
            ImageRendering::Smooth
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&imgs[1], Some(&body_cs)).image_rendering,
            ImageRendering::Pixelated
        );
    }

    #[test]
    fn mix_blend_mode_fase_7_254() {
        assert_eq!(parse_blend_mode("normal"), Some(BlendMode::Normal));
        assert_eq!(parse_blend_mode("MULTIPLY"), Some(BlendMode::Multiply));
        assert_eq!(parse_blend_mode("color-dodge"), Some(BlendMode::ColorDodge));
        assert_eq!(parse_blend_mode("plus-lighter"), Some(BlendMode::PlusLighter));
        assert_eq!(parse_blend_mode("nope"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { mix-blend-mode: multiply }
            div.s { mix-blend-mode: screen }
            div.plain {}
        </style></head><body>
          <div class="s"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.mix_blend_mode, BlendMode::Multiply);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mix_blend_mode,
            BlendMode::Screen
        );
        // NO se hereda → default `Normal`.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).mix_blend_mode,
            BlendMode::Normal
        );
    }

    #[test]
    fn background_blend_mode_fase_7_255() {
        // Lista de varios modos.
        let list = parse_blend_mode_list("multiply, screen, OVERLAY");
        assert_eq!(
            list,
            vec![BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay]
        );
        // Inválidos individuales caen a Normal (no rompen la lista).
        let list2 = parse_blend_mode_list("multiply, BANANA, color");
        assert_eq!(
            list2,
            vec![BlendMode::Multiply, BlendMode::Normal, BlendMode::Color]
        );

        let html = r##"<html><head><style>
            div.bg { background-blend-mode: multiply, screen }
        </style></head><body><div class="bg"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let cs = eng.compute(&divs[0]);
        assert_eq!(
            cs.background_blend_mode,
            vec![BlendMode::Multiply, BlendMode::Screen]
        );
    }

    #[test]
    fn isolation_fase_7_256() {
        assert_eq!(parse_isolation("auto"), Some(Isolation::Auto));
        assert_eq!(parse_isolation("ISOLATE"), Some(Isolation::Isolate));
        assert_eq!(parse_isolation("nope"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { isolation: isolate }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.isolation, Isolation::Isolate);
        // Default Auto en el hijo.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).isolation,
            Isolation::Auto
        );
    }

    #[test]
    fn will_change_fase_7_257() {
        // `auto` y `auto, x` se aplanan: `auto` se descarta.
        assert!(parse_will_change("auto").is_empty());
        assert_eq!(
            parse_will_change("scroll-position, contents"),
            vec![WillChangeHint::ScrollPosition, WillChangeHint::Contents]
        );
        // Property arbitraria conservada lowercase.
        assert_eq!(
            parse_will_change("Transform, OPACITY"),
            vec![
                WillChangeHint::Property("transform".to_string()),
                WillChangeHint::Property("opacity".to_string()),
            ]
        );

        // NO se hereda.
        let html = r##"<html><head><style>
            body { will-change: transform }
            div.over { will-change: scroll-position }
            div.plain {}
        </style></head><body>
          <div class="over"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.will_change,
            vec![WillChangeHint::Property("transform".to_string())]
        );
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).will_change,
            vec![WillChangeHint::ScrollPosition]
        );
        // NO se hereda → vacío.
        assert!(
            eng.compute_with_parent(&divs[1], Some(&body_cs))
                .will_change
                .is_empty()
        );
    }

    #[test]
    fn appearance_fase_7_258() {
        assert_eq!(parse_appearance("none"), Some(Appearance::None));
        assert_eq!(parse_appearance("AUTO"), Some(Appearance::Auto));
        assert_eq!(parse_appearance("textfield"), Some(Appearance::Textfield));
        assert_eq!(
            parse_appearance("menulist-button"),
            Some(Appearance::MenulistButton)
        );
        // Compat legacy → Auto.
        assert_eq!(parse_appearance("searchfield"), Some(Appearance::Auto));
        assert_eq!(parse_appearance("nope"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { appearance: none }
            input.btn { -webkit-appearance: button }
            input.plain {}
        </style></head><body>
          <input class="btn"/><input class="plain"/>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("input") => inputs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.appearance, Appearance::None);
        assert_eq!(
            eng.compute_with_parent(&inputs[0], Some(&body_cs)).appearance,
            Appearance::Button
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&inputs[1], Some(&body_cs)).appearance,
            Appearance::Auto
        );
    }

    #[test]
    fn font_kerning_fase_7_259() {
        assert_eq!(parse_font_kerning("auto"), Some(FontKerning::Auto));
        assert_eq!(parse_font_kerning("NORMAL"), Some(FontKerning::Normal));
        assert_eq!(parse_font_kerning("none"), Some(FontKerning::None));
        assert_eq!(parse_font_kerning("xx"), None);

        let html = r##"<html><head><style>
            body { font-kerning: normal }
            p.off { font-kerning: none }
            p.plain {}
        </style></head><body>
          <p class="off"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_kerning, FontKerning::Normal);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).font_kerning,
            FontKerning::None
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).font_kerning,
            FontKerning::Normal
        );
    }

    #[test]
    fn font_feature_settings_fase_7_260() {
        // `normal` → vacío.
        assert!(parse_font_feature_settings("normal").is_empty());
        // Default value = 1.
        let r = parse_font_feature_settings("\"liga\"");
        assert_eq!(r, vec![FontFeatureSetting { tag: *b"liga", value: 1 }]);
        // on/off + número.
        let r2 = parse_font_feature_settings("\"liga\" on, \"smcp\" off, \"ss01\" 2");
        assert_eq!(
            r2,
            vec![
                FontFeatureSetting { tag: *b"liga", value: 1 },
                FontFeatureSetting { tag: *b"smcp", value: 0 },
                FontFeatureSetting { tag: *b"ss01", value: 2 },
            ]
        );
        // Tags inválidas (longitud) se descartan.
        let r3 = parse_font_feature_settings("\"abc\", \"lig\"");
        assert!(r3.is_empty());

        let html = r##"<html><head><style>
            body { font-feature-settings: "liga" on }
            p.over { font-feature-settings: "smcp" }
        </style></head><body>
          <p class="over"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.font_feature_settings,
            vec![FontFeatureSetting { tag: *b"liga", value: 1 }]
        );
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).font_feature_settings,
            vec![FontFeatureSetting { tag: *b"smcp", value: 1 }]
        );
    }

    #[test]
    fn font_variation_settings_fase_7_261() {
        assert!(parse_font_variation_settings("normal").is_empty());
        let r = parse_font_variation_settings("\"wght\" 700, \"wdth\" 80, \"slnt\" -15.5");
        assert_eq!(r.len(), 3);
        assert_eq!(&r[0].tag, b"wght");
        assert!((r[0].value - 700.0).abs() < 1e-3);
        assert_eq!(&r[2].tag, b"slnt");
        assert!((r[2].value + 15.5).abs() < 1e-3);

        let html = r##"<html><head><style>
            body { font-variation-settings: "wght" 700 }
            p.plain {}
        </style></head><body><p class="plain"></p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_variation_settings.len(), 1);
        assert_eq!(&body_cs.font_variation_settings[0].tag, b"wght");
        // Heredado.
        let p_cs = eng.compute_with_parent(&ps[0], Some(&body_cs));
        assert_eq!(p_cs.font_variation_settings.len(), 1);
    }

    #[test]
    fn font_language_override_fase_7_262() {
        assert_eq!(parse_font_language_override("normal"), None);
        assert_eq!(
            parse_font_language_override("\"DEU\""),
            Some("DEU".to_string())
        );
        // Single-quote también.
        assert_eq!(
            parse_font_language_override("'TRK'"),
            Some("TRK".to_string())
        );
        // Sin comillas o vacío.
        assert_eq!(parse_font_language_override("DEU"), None);
        assert_eq!(parse_font_language_override("\"\""), None);

        let html = r##"<html><head><style>
            body { font-language-override: "DEU" }
            p.over { font-language-override: "ROM" }
            p.plain {}
        </style></head><body>
          <p class="over"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_language_override.as_deref(), Some("DEU"));
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs))
                .font_language_override
                .as_deref(),
            Some("ROM")
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs))
                .font_language_override
                .as_deref(),
            Some("DEU")
        );
    }

    #[test]
    fn text_rendering_fase_7_263() {
        assert_eq!(parse_text_rendering("auto"), Some(TextRendering::Auto));
        assert_eq!(
            parse_text_rendering("optimizeSpeed"),
            Some(TextRendering::OptimizeSpeed)
        );
        assert_eq!(
            parse_text_rendering("OptimizeLegibility"),
            Some(TextRendering::OptimizeLegibility)
        );
        assert_eq!(
            parse_text_rendering("geometricprecision"),
            Some(TextRendering::GeometricPrecision)
        );
        assert_eq!(parse_text_rendering("nope"), None);

        let html = r##"<html><head><style>
            body { text-rendering: optimizeLegibility }
            p.plain {}
        </style></head><body><p class="plain"></p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_rendering, TextRendering::OptimizeLegibility);
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).text_rendering,
            TextRendering::OptimizeLegibility
        );
    }

    #[test]
    fn filter_fase_7_264() {
        // `none` y vacío → vacío.
        assert!(parse_filter_list("none").is_empty());
        assert!(parse_filter_list("").is_empty());

        // Funciones simples.
        let r = parse_filter_list("blur(4px) brightness(120%) hue-rotate(45deg)");
        assert_eq!(r.len(), 3);
        assert!(matches!(r[0], FilterFn::Blur(v) if (v - 4.0).abs() < 1e-3));
        assert!(matches!(r[1], FilterFn::Brightness(v) if (v - 1.2).abs() < 1e-3));
        assert!(matches!(r[2], FilterFn::HueRotate(v) if (v - 45.0).abs() < 1e-3));

        // Número unitless + porcentaje.
        let r2 = parse_filter_list("opacity(0.5) saturate(50%) grayscale(1)");
        assert!(matches!(r2[0], FilterFn::Opacity(v) if (v - 0.5).abs() < 1e-3));
        assert!(matches!(r2[1], FilterFn::Saturate(v) if (v - 0.5).abs() < 1e-3));
        assert!(matches!(r2[2], FilterFn::Grayscale(v) if (v - 1.0).abs() < 1e-3));

        // hue-rotate con rad/turn.
        let r3 = parse_filter_list("hue-rotate(0.5turn) hue-rotate(3.14159rad)");
        assert!(matches!(r3[0], FilterFn::HueRotate(v) if (v - 180.0).abs() < 1e-1));
        assert!(matches!(r3[1], FilterFn::HueRotate(v) if (v - 180.0).abs() < 1.0));

        // drop-shadow reusa box-shadow.
        let r4 = parse_filter_list("drop-shadow(2px 3px red)");
        assert!(matches!(&r4[0], FilterFn::DropShadow(s) if (s.offset_x - 2.0).abs() < 1e-3));

        // Función desconocida descartada (sólo se queda la conocida).
        let r5 = parse_filter_list("nope(1) blur(2px) bogus(x)");
        assert_eq!(r5.len(), 1);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { filter: blur(2px) }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.filter.len(), 1);
        // NO se hereda → vacío.
        assert!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .filter
                .is_empty()
        );
    }

    #[test]
    fn backdrop_filter_fase_7_265() {
        let r = parse_filter_list("blur(8px) saturate(180%)");
        assert_eq!(r.len(), 2);

        let html = r##"<html><head><style>
            div.glass { -webkit-backdrop-filter: blur(10px) }
        </style></head><body><div class="glass"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let cs = eng.compute(&divs[0]);
        assert_eq!(cs.backdrop_filter.len(), 1);
        assert!(matches!(cs.backdrop_filter[0], FilterFn::Blur(v) if (v - 10.0).abs() < 1e-3));
    }

    #[test]
    fn text_orientation_fase_7_266() {
        assert_eq!(parse_text_orientation("mixed"), Some(TextOrientation::Mixed));
        assert_eq!(parse_text_orientation("UPRIGHT"), Some(TextOrientation::Upright));
        assert_eq!(parse_text_orientation("sideways"), Some(TextOrientation::Sideways));
        assert_eq!(
            parse_text_orientation("sideways-right"),
            Some(TextOrientation::SidewaysRight)
        );
        assert_eq!(parse_text_orientation("nope"), None);

        let html = r##"<html><head><style>
            body { text-orientation: upright }
            p.over { text-orientation: sideways }
            p.plain {}
        </style></head><body>
          <p class="over"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_orientation, TextOrientation::Upright);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).text_orientation,
            TextOrientation::Sideways
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).text_orientation,
            TextOrientation::Upright
        );
    }

    #[test]
    fn overscroll_behavior_fase_7_267() {
        assert_eq!(parse_overscroll_behavior("auto"), Some(OverscrollBehavior::Auto));
        assert_eq!(parse_overscroll_behavior("CONTAIN"), Some(OverscrollBehavior::Contain));
        assert_eq!(parse_overscroll_behavior("none"), Some(OverscrollBehavior::None));
        assert_eq!(parse_overscroll_behavior("nope"), None);

        // Shorthand: `contain none` → x=contain, y=none. `auto` solo → x=y=auto.
        let html = r##"<html><head><style>
            body { overscroll-behavior: contain none }
            div.solo { overscroll-behavior: contain }
            div.split { overscroll-behavior-x: none; overscroll-behavior-y: auto }
            div.plain {}
        </style></head><body>
          <div class="solo"></div><div class="split"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        // 2-valor: x=contain, y=none.
        assert_eq!(body_cs.overscroll_behavior_x, OverscrollBehavior::Contain);
        assert_eq!(body_cs.overscroll_behavior_y, OverscrollBehavior::None);
        // 1-valor: x=y=contain.
        let solo_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(solo_cs.overscroll_behavior_x, OverscrollBehavior::Contain);
        assert_eq!(solo_cs.overscroll_behavior_y, OverscrollBehavior::Contain);
        // Longhands separadas.
        let split_cs = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(split_cs.overscroll_behavior_x, OverscrollBehavior::None);
        assert_eq!(split_cs.overscroll_behavior_y, OverscrollBehavior::Auto);
        // NO se hereda → default Auto.
        let plain_cs = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert_eq!(plain_cs.overscroll_behavior_x, OverscrollBehavior::Auto);
        assert_eq!(plain_cs.overscroll_behavior_y, OverscrollBehavior::Auto);
    }

    #[test]
    fn scroll_snap_type_fase_7_268() {
        assert_eq!(parse_scroll_snap_type("none"), Some(ScrollSnapType(None)));
        assert_eq!(
            parse_scroll_snap_type("x"),
            Some(ScrollSnapType(Some((ScrollSnapAxis::X, ScrollSnapStrictness::Proximity))))
        );
        assert_eq!(
            parse_scroll_snap_type("y mandatory"),
            Some(ScrollSnapType(Some((ScrollSnapAxis::Y, ScrollSnapStrictness::Mandatory))))
        );
        assert_eq!(
            parse_scroll_snap_type("BOTH proximity"),
            Some(ScrollSnapType(Some((ScrollSnapAxis::Both, ScrollSnapStrictness::Proximity))))
        );
        assert_eq!(parse_scroll_snap_type("xy"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { scroll-snap-type: y mandatory }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.scroll_snap_type,
            ScrollSnapType(Some((ScrollSnapAxis::Y, ScrollSnapStrictness::Mandatory)))
        );
        // NO se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_snap_type,
            ScrollSnapType(None)
        );
    }

    #[test]
    fn scroll_snap_align_fase_7_269() {
        assert_eq!(parse_scroll_snap_align("none"), Some(ScrollSnapAlign::None));
        assert_eq!(parse_scroll_snap_align("START"), Some(ScrollSnapAlign::Start));
        assert_eq!(parse_scroll_snap_align("end"), Some(ScrollSnapAlign::End));
        assert_eq!(parse_scroll_snap_align("center"), Some(ScrollSnapAlign::Center));
        assert_eq!(parse_scroll_snap_align("middle"), None);

        // Shorthand: 1 valor → ambos ejes; 2 valores → block + inline.
        let html = r##"<html><head><style>
            body { scroll-snap-align: start end }
            div.solo { scroll-snap-align: center }
            div.split { scroll-snap-align-block: end; scroll-snap-align-inline: start }
            div.plain {}
        </style></head><body>
          <div class="solo"></div><div class="split"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.scroll_snap_align_block, ScrollSnapAlign::Start);
        assert_eq!(body_cs.scroll_snap_align_inline, ScrollSnapAlign::End);
        let solo = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(solo.scroll_snap_align_block, ScrollSnapAlign::Center);
        assert_eq!(solo.scroll_snap_align_inline, ScrollSnapAlign::Center);
        let split = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(split.scroll_snap_align_block, ScrollSnapAlign::End);
        assert_eq!(split.scroll_snap_align_inline, ScrollSnapAlign::Start);
        // NO se hereda → default None.
        let plain = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert_eq!(plain.scroll_snap_align_block, ScrollSnapAlign::None);
        assert_eq!(plain.scroll_snap_align_inline, ScrollSnapAlign::None);
    }

    #[test]
    fn scroll_snap_stop_fase_7_270() {
        assert_eq!(parse_scroll_snap_stop("normal"), Some(ScrollSnapStop::Normal));
        assert_eq!(parse_scroll_snap_stop("ALWAYS"), Some(ScrollSnapStop::Always));
        assert_eq!(parse_scroll_snap_stop("never"), None);

        let html = r##"<html><head><style>
            body { scroll-snap-stop: always }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.scroll_snap_stop, ScrollSnapStop::Always);
        // NO se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_snap_stop,
            ScrollSnapStop::Normal
        );
    }

    #[test]
    fn scroll_padding_fase_7_271() {
        // Sides 1–4 valores con `LengthVal`.
        assert_eq!(
            parse_sides_lp("10px"),
            Some(Sides {
                top: LengthVal::Px(10.0),
                right: LengthVal::Px(10.0),
                bottom: LengthVal::Px(10.0),
                left: LengthVal::Px(10.0),
            })
        );
        assert_eq!(
            parse_sides_lp("auto 5%"),
            Some(Sides {
                top: LengthVal::Auto,
                right: LengthVal::Pct(5.0),
                bottom: LengthVal::Auto,
                left: LengthVal::Pct(5.0),
            })
        );
        assert!(parse_sides_lp("nope").is_none());

        let html = r##"<html><head><style>
            body { scroll-padding: 10px 20px 30px 40px }
            div.lh { scroll-padding-top: 5px; scroll-padding-left: 15% }
            div.plain {}
        </style></head><body>
          <div class="lh"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.scroll_padding.top, LengthVal::Px(10.0));
        assert_eq!(body_cs.scroll_padding.right, LengthVal::Px(20.0));
        assert_eq!(body_cs.scroll_padding.bottom, LengthVal::Px(30.0));
        assert_eq!(body_cs.scroll_padding.left, LengthVal::Px(40.0));
        // Longhands sobre default (NO se hereda del body — empieza en auto).
        let lh = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(lh.scroll_padding.top, LengthVal::Px(5.0));
        assert_eq!(lh.scroll_padding.right, LengthVal::Auto);
        assert_eq!(lh.scroll_padding.bottom, LengthVal::Auto);
        assert_eq!(lh.scroll_padding.left, LengthVal::Pct(15.0));
        // NO se hereda → todos auto.
        let plain = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(plain.scroll_padding.top, LengthVal::Auto);
        assert_eq!(plain.scroll_padding.left, LengthVal::Auto);
    }

    #[test]
    fn scroll_margin_fase_7_272() {
        let html = r##"<html><head><style>
            body { scroll-margin: 8px 16px }
            div.full { scroll-margin: 1px 2px 3px 4px }
            div.lh { scroll-margin-top: 7px; scroll-margin-right: 9px }
            div.plain {}
        </style></head><body>
          <div class="full"></div><div class="lh"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        // 2-valor: top/bottom=8, left/right=16.
        assert_eq!(body_cs.scroll_margin.top, 8.0);
        assert_eq!(body_cs.scroll_margin.right, 16.0);
        assert_eq!(body_cs.scroll_margin.bottom, 8.0);
        assert_eq!(body_cs.scroll_margin.left, 16.0);
        // 4-valor explícito.
        let full = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(full.scroll_margin.top, 1.0);
        assert_eq!(full.scroll_margin.right, 2.0);
        assert_eq!(full.scroll_margin.bottom, 3.0);
        assert_eq!(full.scroll_margin.left, 4.0);
        // Longhands sobre default (NO hereda).
        let lh = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(lh.scroll_margin.top, 7.0);
        assert_eq!(lh.scroll_margin.right, 9.0);
        assert_eq!(lh.scroll_margin.bottom, 0.0);
        assert_eq!(lh.scroll_margin.left, 0.0);
        // NO se hereda → todo 0.
        let plain = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert_eq!(plain.scroll_margin.top, 0.0);
        assert_eq!(plain.scroll_margin.right, 0.0);
    }

    #[test]
    fn touch_action_fase_7_273() {
        assert_eq!(parse_touch_action("auto"), Some(TouchAction::Auto));
        assert_eq!(parse_touch_action("NONE"), Some(TouchAction::None));
        assert_eq!(parse_touch_action("manipulation"), Some(TouchAction::Manipulation));
        assert_eq!(
            parse_touch_action("pan-x"),
            Some(TouchAction::Pan { pan_x: true, pan_y: false, pinch_zoom: false })
        );
        // `pan-left` se aplasta a pan-x; `pan-up` a pan-y; combinable con pinch-zoom.
        assert_eq!(
            parse_touch_action("pan-left pan-up pinch-zoom"),
            Some(TouchAction::Pan { pan_x: true, pan_y: true, pinch_zoom: true })
        );
        // Token inválido descarta la regla entera.
        assert_eq!(parse_touch_action("pan-x bogus"), None);
        // Sin pan ni pinch-zoom no es válido (no debería pasar por el path
        // de palabras sueltas, pero por las dudas).
        assert_eq!(parse_touch_action(""), None);

        let html = r##"<html><head><style>
            body { touch-action: pan-y pinch-zoom }
            div.none { touch-action: none }
            div.plain {}
        </style></head><body>
          <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.touch_action,
            TouchAction::Pan { pan_x: false, pan_y: true, pinch_zoom: true }
        );
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).touch_action,
            TouchAction::None
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).touch_action,
            TouchAction::Auto
        );
    }

    #[test]
    fn clip_path_fase_7_274() {
        assert!(parse_clip_path("none").is_none());
        assert!(parse_clip_path("").is_none());
        // inset 1-valor.
        let r = parse_clip_path("inset(10px)").unwrap();
        assert_eq!(
            r,
            ClipPath::Inset { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0, radius: 0.0 }
        );
        // inset 4-valor con `round`.
        let r = parse_clip_path("inset(1px 2px 3px 4px round 5px)").unwrap();
        assert_eq!(
            r,
            ClipPath::Inset { top: 1.0, right: 2.0, bottom: 3.0, left: 4.0, radius: 5.0 }
        );
        // circle con radio + centro.
        let r = parse_clip_path("circle(30px at 50% 50%)").unwrap();
        assert!(matches!(
            r,
            ClipPath::Circle { radius, cx: LengthVal::Pct(50.0), cy: LengthVal::Pct(50.0) }
                if (radius - 30.0).abs() < 1e-3
        ));
        // ellipse default centro.
        let r = parse_clip_path("ellipse(20px 10px)").unwrap();
        assert!(matches!(
            r,
            ClipPath::Ellipse { rx, ry, cx: LengthVal::Pct(50.0), cy: LengthVal::Pct(50.0) }
                if (rx - 20.0).abs() < 1e-3 && (ry - 10.0).abs() < 1e-3
        ));
        // Función desconocida → None.
        assert!(parse_clip_path("polygon(0 0, 100% 0, 50% 100%)").is_none());

        // e2e: body con clip-path, div sin → NO se hereda.
        let html = r##"<html><head><style>
            body { clip-path: circle(50px) }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(matches!(body_cs.clip_path, Some(ClipPath::Circle { .. })));
        assert!(eng.compute_with_parent(&divs[0], Some(&body_cs)).clip_path.is_none());
    }

    #[test]
    fn mask_image_fase_7_275() {
        assert!(parse_mask_image("none").is_none());
        assert!(parse_mask_image("").is_none());
        assert_eq!(
            parse_mask_image("url(mask.png)"),
            Some(MaskImage::Url("mask.png".to_string()))
        );
        assert_eq!(
            parse_mask_image("url(\"m.svg\")"),
            Some(MaskImage::Url("m.svg".to_string()))
        );
        // Lo que no es `url(...)` cae a None (subset).
        assert!(parse_mask_image("linear-gradient(red, blue)").is_none());

        // Alias `-webkit-mask` y shorthand `mask` redirigen al subset url-only.
        let html = r##"<html><head><style>
            body { mask: url(body.png) }
            div.legacy { -webkit-mask-image: url('legacy.png') }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.mask_image, Some(MaskImage::Url("body.png".to_string())));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mask_image,
            Some(MaskImage::Url("legacy.png".to_string()))
        );
        // NO se hereda.
        assert!(eng.compute_with_parent(&divs[1], Some(&body_cs)).mask_image.is_none());
    }

    #[test]
    fn content_visibility_fase_7_276() {
        assert_eq!(parse_content_visibility("visible"), Some(ContentVisibility::Visible));
        assert_eq!(parse_content_visibility("AUTO"), Some(ContentVisibility::Auto));
        assert_eq!(parse_content_visibility("hidden"), Some(ContentVisibility::Hidden));
        assert_eq!(parse_content_visibility("nope"), None);

        let html = r##"<html><head><style>
            body { content-visibility: auto }
            div.h { content-visibility: hidden }
            div.plain {}
        </style></head><body>
          <div class="h"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.content_visibility, ContentVisibility::Auto);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).content_visibility,
            ContentVisibility::Hidden
        );
        // NO se hereda → default Visible.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).content_visibility,
            ContentVisibility::Visible
        );
    }

    #[test]
    fn contain_fase_7_277() {
        // Keywords compuestos.
        assert_eq!(parse_contain("none"), Some(ContainFlags::default()));
        assert_eq!(parse_contain("STRICT"), Some(ContainFlags::STRICT));
        assert_eq!(parse_contain("content"), Some(ContainFlags::CONTENT));
        // Bitset libre.
        let mixed = parse_contain("layout paint").unwrap();
        assert!(mixed.layout && mixed.paint);
        assert!(!mixed.size && !mixed.style);
        // `inline-size` también.
        assert!(parse_contain("inline-size").unwrap().inline_size);
        // Token inválido descarta.
        assert!(parse_contain("bogus").is_none());

        let html = r##"<html><head><style>
            body { contain: strict }
            div.lp { contain: layout paint }
            div.plain {}
        </style></head><body>
          <div class="lp"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.contain, ContainFlags::STRICT);
        let lp = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(lp.contain.layout && lp.contain.paint && !lp.contain.size);
        // NO se hereda → all-false.
        assert!(eng.compute_with_parent(&divs[1], Some(&body_cs)).contain.is_none());
    }

    #[test]
    fn column_count_fase_7_278() {
        assert_eq!(parse_column_count("auto"), None);
        assert_eq!(parse_column_count("3"), Some(3));
        assert_eq!(parse_column_count("0"), None);
        assert_eq!(parse_column_count("-2"), None);
        assert_eq!(parse_column_count("nope"), None);

        let html = r##"<html><head><style>
            body { column-count: 4 }
            div.auto { column-count: auto }
            div.plain {}
        </style></head><body>
          <div class="auto"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.column_count, Some(4));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_count,
            None
        );
        // NO se hereda → default None.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).column_count,
            None
        );
    }

    #[test]
    fn column_width_fase_7_279() {
        let html = r##"<html><head><style>
            body { column-width: 200px }
            div.auto { column-width: auto }
            div.pct { column-width: 30% }
            div.plain {}
        </style></head><body>
          <div class="auto"></div><div class="pct"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.column_width, LengthVal::Px(200.0));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_width,
            LengthVal::Auto
        );
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).column_width,
            LengthVal::Pct(30.0)
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[2], Some(&body_cs)).column_width,
            LengthVal::Auto
        );
    }

    #[test]
    fn column_rule_fase_7_280() {
        // Longhands sueltos + shorthand. `currentColor` → None (defer al render).
        let html = r##"<html><head><style>
            body { column-rule: 2px dashed red }
            div.lh { column-rule-color: blue; column-rule-width: 3px; column-rule-style: dotted }
            div.cc { column-rule: 1px solid currentColor }
            div.none { column-rule: 4px solid black; column-rule-style: none }
            div.plain {}
        </style></head><body>
          <div class="lh"></div><div class="cc"></div><div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.column_rule_width, 2.0);
        assert_eq!(body_cs.column_rule_style, BorderLineStyle::Dashed);
        assert!(body_cs.column_rule_style_active);
        assert_eq!(body_cs.column_rule_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Longhands.
        let lh = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(lh.column_rule_width, 3.0);
        assert_eq!(lh.column_rule_style, BorderLineStyle::Dotted);
        assert_eq!(lh.column_rule_color.map(|c| (c.r, c.g, c.b)), Some((0, 0, 255)));
        // currentColor → None.
        let cc = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(cc.column_rule_color, None);
        // `column-rule-style: none` apaga.
        let none = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert!(!none.column_rule_style_active);
        // NO se hereda → defaults (width 0, color None, style_active false).
        let plain = eng.compute_with_parent(&divs[3], Some(&body_cs));
        assert_eq!(plain.column_rule_width, 0.0);
        assert!(!plain.column_rule_style_active);
        assert_eq!(plain.column_rule_color, None);
    }

    #[test]
    fn column_fill_fase_7_281() {
        assert_eq!(parse_column_fill("auto"), Some(ColumnFill::Auto));
        assert_eq!(parse_column_fill("BALANCE"), Some(ColumnFill::Balance));
        assert_eq!(parse_column_fill("balance-all"), Some(ColumnFill::BalanceAll));
        assert_eq!(parse_column_fill("nope"), None);

        let html = r##"<html><head><style>
            body { column-fill: auto }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.column_fill, ColumnFill::Auto);
        // NO se hereda → default Balance.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_fill,
            ColumnFill::Balance
        );
    }

    #[test]
    fn column_span_fase_7_282() {
        assert_eq!(parse_column_span("none"), Some(ColumnSpan::None));
        assert_eq!(parse_column_span("ALL"), Some(ColumnSpan::All));
        assert_eq!(parse_column_span("partial"), None);

        let html = r##"<html><head><style>
            body { column-span: all }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.column_span, ColumnSpan::All);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_span,
            ColumnSpan::None
        );
    }

    #[test]
    fn break_inside_fase_7_283() {
        assert_eq!(parse_break_inside("auto"), Some(BreakInside::Auto));
        assert_eq!(parse_break_inside("avoid"), Some(BreakInside::Avoid));
        assert_eq!(parse_break_inside("AVOID-PAGE"), Some(BreakInside::AvoidPage));
        assert_eq!(parse_break_inside("avoid-column"), Some(BreakInside::AvoidColumn));
        assert_eq!(parse_break_inside("avoid-region"), Some(BreakInside::AvoidRegion));
        assert_eq!(parse_break_inside("nope"), None);

        // Alias legacy `page-break-inside`.
        let html = r##"<html><head><style>
            body { break-inside: avoid }
            div.legacy { page-break-inside: avoid }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.break_inside, BreakInside::Avoid);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).break_inside,
            BreakInside::Avoid
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).break_inside,
            BreakInside::Auto
        );
    }

    #[test]
    fn table_layout_fase_7_284() {
        assert_eq!(parse_table_layout("auto"), Some(TableLayout::Auto));
        assert_eq!(parse_table_layout("FIXED"), Some(TableLayout::Fixed));
        assert_eq!(parse_table_layout("nope"), None);

        let html = r##"<html><head><style>
            body { table-layout: fixed }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.table_layout, TableLayout::Fixed);
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).table_layout,
            TableLayout::Auto
        );
    }

    #[test]
    fn border_collapse_fase_7_285() {
        assert_eq!(parse_border_collapse("separate"), Some(BorderCollapse::Separate));
        assert_eq!(parse_border_collapse("COLLAPSE"), Some(BorderCollapse::Collapse));
        assert_eq!(parse_border_collapse("merge"), None);

        let html = r##"<html><head><style>
            body { border-collapse: collapse }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.border_collapse, BorderCollapse::Collapse);
        // SÍ se hereda → div sin propio gana el del padre.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).border_collapse,
            BorderCollapse::Collapse
        );
    }

    #[test]
    fn border_spacing_fase_7_286() {
        assert_eq!(parse_border_spacing("5px"), Some((5.0, 5.0)));
        assert_eq!(parse_border_spacing("5px 10px"), Some((5.0, 10.0)));
        assert!(parse_border_spacing("5px 10px 15px").is_none());

        let html = r##"<html><head><style>
            body { border-spacing: 3px 7px }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.border_spacing_h, 3.0);
        assert_eq!(body_cs.border_spacing_v, 7.0);
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(plain.border_spacing_h, 3.0);
        assert_eq!(plain.border_spacing_v, 7.0);
    }

    #[test]
    fn caption_side_fase_7_287() {
        assert_eq!(parse_caption_side("top"), Some(CaptionSide::Top));
        assert_eq!(parse_caption_side("BOTTOM"), Some(CaptionSide::Bottom));
        // Logicals se aplastan.
        assert_eq!(parse_caption_side("block-start"), Some(CaptionSide::Top));
        assert_eq!(parse_caption_side("inline-end"), Some(CaptionSide::Bottom));
        assert_eq!(parse_caption_side("middle"), None);

        let html = r##"<html><head><style>
            body { caption-side: bottom }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.caption_side, CaptionSide::Bottom);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).caption_side,
            CaptionSide::Bottom
        );
    }

    #[test]
    fn empty_cells_fase_7_288() {
        assert_eq!(parse_empty_cells("show"), Some(EmptyCells::Show));
        assert_eq!(parse_empty_cells("HIDE"), Some(EmptyCells::Hide));
        assert_eq!(parse_empty_cells("nope"), None);

        let html = r##"<html><head><style>
            body { empty-cells: hide }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.empty_cells, EmptyCells::Hide);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).empty_cells,
            EmptyCells::Hide
        );
    }

    #[test]
    fn break_before_fase_7_289() {
        assert_eq!(parse_break_between("auto"), Some(BreakBetween::Auto));
        assert_eq!(parse_break_between("AVOID-PAGE"), Some(BreakBetween::AvoidPage));
        assert_eq!(parse_break_between("page"), Some(BreakBetween::Page));
        assert_eq!(parse_break_between("recto"), Some(BreakBetween::Recto));
        assert_eq!(parse_break_between("column"), Some(BreakBetween::Column));
        assert_eq!(parse_break_between("avoid-region"), Some(BreakBetween::AvoidRegion));
        assert_eq!(parse_break_between("nope"), None);

        // Alias legacy `page-break-before`.
        let html = r##"<html><head><style>
            body { break-before: page }
            div.legacy { page-break-before: always }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.break_before, BreakBetween::Page);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).break_before,
            BreakBetween::Always
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).break_before,
            BreakBetween::Auto
        );
    }

    #[test]
    fn break_after_fase_7_290() {
        // Mismo parser, comparte el dominio con break-before.
        let html = r##"<html><head><style>
            body { break-after: avoid-column }
            div.legacy { page-break-after: left }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.break_after, BreakBetween::AvoidColumn);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).break_after,
            BreakBetween::Left
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).break_after,
            BreakBetween::Auto
        );
    }

    #[test]
    fn orphans_fase_7_291() {
        assert_eq!(parse_positive_int("3"), Some(3));
        assert_eq!(parse_positive_int("0"), None);
        assert_eq!(parse_positive_int("-1"), None);
        assert_eq!(parse_positive_int("nope"), None);

        let html = r##"<html><head><style>
            body { orphans: 4 }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.orphans, 4);
        // SÍ se hereda.
        assert_eq!(eng.compute_with_parent(&divs[0], Some(&body_cs)).orphans, 4);
    }

    #[test]
    fn widows_fase_7_292() {
        let html = r##"<html><head><style>
            body { widows: 5 }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.widows, 5);
        // SÍ se hereda.
        assert_eq!(eng.compute_with_parent(&divs[0], Some(&body_cs)).widows, 5);
    }

    #[test]
    fn color_scheme_fase_7_293() {
        assert_eq!(parse_color_scheme("normal"), Some(ColorScheme::NORMAL));
        assert_eq!(
            parse_color_scheme("light dark"),
            Some(ColorScheme { light: true, dark: true, only: false })
        );
        assert_eq!(
            parse_color_scheme("only LIGHT"),
            Some(ColorScheme { light: true, dark: false, only: true })
        );
        // Duplicado descarta.
        assert!(parse_color_scheme("light light").is_none());
        // Token desconocido descarta.
        assert!(parse_color_scheme("light sepia").is_none());
        // `only` solo no es válido.
        assert!(parse_color_scheme("only").is_none());

        let html = r##"<html><head><style>
            body { color-scheme: light dark }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(body_cs.color_scheme.light && body_cs.color_scheme.dark);
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(plain.color_scheme.light && plain.color_scheme.dark);
    }

    #[test]
    fn list_style_position_fase_7_294() {
        assert_eq!(parse_list_style_position("outside"), Some(ListStylePosition::Outside));
        assert_eq!(parse_list_style_position("INSIDE"), Some(ListStylePosition::Inside));
        assert_eq!(parse_list_style_position("middle"), None);

        let html = r##"<html><head><style>
            body { list-style-position: inside }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.list_style_position, ListStylePosition::Inside);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).list_style_position,
            ListStylePosition::Inside
        );
    }

    #[test]
    fn list_style_image_fase_7_295() {
        assert_eq!(parse_list_style_image("none"), None);
        assert_eq!(parse_list_style_image(""), None);
        assert_eq!(
            parse_list_style_image("url(bullet.png)"),
            Some("bullet.png".to_string())
        );
        assert_eq!(
            parse_list_style_image("url(\"b.svg\")"),
            Some("b.svg".to_string())
        );
        // Subset: no aceptamos gradients ni image() todavía.
        assert_eq!(parse_list_style_image("linear-gradient(red, blue)"), None);

        let html = r##"<html><head><style>
            body { list-style-image: url(bullet.png) }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.list_style_image, Some("bullet.png".to_string()));
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).list_style_image,
            Some("bullet.png".to_string())
        );
    }

    #[test]
    fn list_style_shorthand_fase_7_296() {
        // Shorthand cubre los 3 longhands en orden libre.
        let html = r##"<html><head><style>
            body { list-style: square inside url(b.png) }
            div.none { list-style: none }
            div.plain {}
        </style></head><body>
          <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.list_style_type, ListStyleType::Square);
        assert_eq!(body_cs.list_style_position, ListStylePosition::Inside);
        assert_eq!(body_cs.list_style_image, Some("b.png".to_string()));
        // `list-style: none` apaga type + image.
        let none = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(none.list_style_type, ListStyleType::None);
        assert_eq!(none.list_style_image, None);
        // position no se toca con `none` — sigue heredando del padre.
        assert_eq!(none.list_style_position, ListStylePosition::Inside);
    }

    #[test]
    fn counter_set_fase_7_297() {
        // Default = 0 (a diferencia de counter-increment cuyo default es 1).
        let html = r##"<html><head><style>
            body { counter-set: page 1 chapter }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.counter_set.len(), 2);
        assert_eq!(body_cs.counter_set[0], ("page".to_string(), 1));
        assert_eq!(body_cs.counter_set[1], ("chapter".to_string(), 0));
        // NO se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(plain.counter_set.is_empty());
    }

    #[test]
    fn quotes_fase_7_298() {
        assert_eq!(parse_quotes("auto"), Quotes::Auto);
        assert_eq!(parse_quotes("none"), Quotes::None);
        let q = parse_quotes(r#""«" "»" "‹" "›""#);
        assert_eq!(
            q,
            Quotes::Pairs(vec![
                ("«".to_string(), "»".to_string()),
                ("‹".to_string(), "›".to_string()),
            ])
        );
        // Impares (sin cierre) → cae a Auto.
        assert_eq!(parse_quotes(r#""«" "»" "‹""#), Quotes::Auto);
        // Sin comillas → cae a Auto.
        assert_eq!(parse_quotes("foo bar"), Quotes::Auto);

        let html = r##"<html><head><style>
            body { quotes: "“" "”" "‘" "’" }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        if let Quotes::Pairs(pairs) = &body_cs.quotes {
            assert_eq!(pairs.len(), 2);
            assert_eq!(pairs[0], ("“".to_string(), "”".to_string()));
            assert_eq!(pairs[1], ("‘".to_string(), "’".to_string()));
        } else {
            panic!("esperaba Pairs, vino {:?}", body_cs.quotes);
        }
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(matches!(plain.quotes, Quotes::Pairs(_)));
    }

    #[test]
    fn text_underline_position_fase_7_299() {
        assert_eq!(parse_text_underline_position("auto"), Some(TextUnderlinePosition::Auto));
        assert_eq!(
            parse_text_underline_position("FROM-FONT"),
            Some(TextUnderlinePosition::FromFont)
        );
        assert_eq!(parse_text_underline_position("under"), Some(TextUnderlinePosition::Under));
        assert_eq!(parse_text_underline_position("left"), Some(TextUnderlinePosition::Left));
        assert_eq!(parse_text_underline_position("right"), Some(TextUnderlinePosition::Right));
        assert_eq!(parse_text_underline_position("middle"), None);

        let html = r##"<html><head><style>
            body { text-underline-position: under }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_underline_position, TextUnderlinePosition::Under);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_underline_position,
            TextUnderlinePosition::Under
        );
    }

    #[test]
    fn text_justify_fase_7_300() {
        assert_eq!(parse_text_justify("auto"), Some(TextJustify::Auto));
        assert_eq!(parse_text_justify("INTER-WORD"), Some(TextJustify::InterWord));
        assert_eq!(
            parse_text_justify("inter-character"),
            Some(TextJustify::InterCharacter)
        );
        assert_eq!(parse_text_justify("distribute"), Some(TextJustify::Distribute));
        assert_eq!(parse_text_justify("nope"), None);

        let html = r##"<html><head><style>
            body { text-justify: inter-word }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_justify, TextJustify::InterWord);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_justify,
            TextJustify::InterWord
        );
    }

    #[test]
    fn print_color_adjust_fase_7_301() {
        assert_eq!(
            parse_print_color_adjust("economy"),
            Some(PrintColorAdjust::Economy)
        );
        assert_eq!(parse_print_color_adjust("EXACT"), Some(PrintColorAdjust::Exact));
        assert_eq!(parse_print_color_adjust("nope"), None);

        // Alias legacy `color-adjust` debería rutear igual.
        let html = r##"<html><head><style>
            body { print-color-adjust: exact }
            div.legacy { color-adjust: economy }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.print_color_adjust, PrintColorAdjust::Exact);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).print_color_adjust,
            PrintColorAdjust::Economy
        );
        // SÍ se hereda → div.plain hereda Exact del body.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).print_color_adjust,
            PrintColorAdjust::Exact
        );
    }

    #[test]
    fn forced_color_adjust_fase_7_302() {
        assert_eq!(parse_forced_color_adjust("auto"), Some(ForcedColorAdjust::Auto));
        assert_eq!(parse_forced_color_adjust("NONE"), Some(ForcedColorAdjust::None));
        assert_eq!(
            parse_forced_color_adjust("preserve"),
            Some(ForcedColorAdjust::Preserve)
        );
        assert_eq!(parse_forced_color_adjust("nope"), None);

        let html = r##"<html><head><style>
            body { forced-color-adjust: none }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.forced_color_adjust, ForcedColorAdjust::None);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).forced_color_adjust,
            ForcedColorAdjust::None
        );
    }

    #[test]
    fn line_clamp_fase_7_303() {
        assert_eq!(parse_line_clamp("none"), None);
        assert_eq!(parse_line_clamp("3"), Some(3));
        assert_eq!(parse_line_clamp("0"), None);
        assert_eq!(parse_line_clamp("nope"), None);

        let html = r##"<html><head><style>
            body { -webkit-line-clamp: 2 }
            div.std { line-clamp: 5 }
            div.plain {}
        </style></head><body>
          <div class="std"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.line_clamp, Some(2));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).line_clamp,
            Some(5)
        );
        // NO se hereda → default None.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).line_clamp,
            None
        );
    }

    #[test]
    fn text_decoration_color_y_style() {
        // Parser de longhands sueltos.
        assert_eq!(
            parse_text_decoration_style("dotted"),
            Some(TextDecorationStyle::Dotted)
        );
        assert_eq!(parse_text_decoration_style("WAVY"), Some(TextDecorationStyle::Wavy));
        assert_eq!(parse_text_decoration_style("zigzag"), None);

        let html = r##"<html><head><style>
            p.full { text-decoration: underline dotted red }
            p.color { text-decoration-color: rgb(0,128,0) }
            p.style { text-decoration-style: dashed }
            p.cc { color: blue; text-decoration: line-through currentColor }
            p.plain { color: red }
        </style></head><body>
            <p class="full">a</p>
            <p class="color">b</p>
            <p class="style">c</p>
            <p class="cc">d</p>
            <p class="plain">e</p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 5);
        // Shorthand: line + style + color de un mismo `text-decoration`.
        let full = eng.compute(&ps[0]);
        assert_eq!(full.text_decoration, TextDecorationLine::Underline);
        assert_eq!(full.text_decoration_style, TextDecorationStyle::Dotted);
        assert_eq!(full.text_decoration_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Longhand de color suelto (no toca line/style).
        let color = eng.compute(&ps[1]);
        assert_eq!(color.text_decoration_color.map(|c| (c.r, c.g, c.b)), Some((0, 128, 0)));
        assert_eq!(color.text_decoration_style, TextDecorationStyle::Solid);
        // Longhand de style suelto.
        assert_eq!(eng.compute(&ps[2]).text_decoration_style, TextDecorationStyle::Dashed);
        // `currentColor` explícito → None (el render sigue al `color`).
        let cc = eng.compute(&ps[3]);
        assert_eq!(cc.text_decoration, TextDecorationLine::LineThrough);
        assert_eq!(cc.text_decoration_color, None);
        // Sin declarar → defaults (color None = currentColor, style Solid).
        let plain = eng.compute(&ps[4]);
        assert_eq!(plain.text_decoration_color, None);
        assert_eq!(plain.text_decoration_style, TextDecorationStyle::Solid);
    }

    #[test]
    fn outline_style_dashed_dotted() {
        let html = r##"<html><head><style>
            div.sh { outline: 2px dashed red }
            div.ls { outline-color: blue; outline-width: 3px; outline-style: dotted }
            div.db { outline: 4px double green }
            div.none { outline: 1px solid black; outline-style: none }
            div.plain { outline: 1px solid black }
        </style></head><body>
            <div class="sh"></div><div class="ls"></div><div class="db"></div>
            <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 5);
        let sh = eng.compute(&divs[0]).outline;
        assert_eq!(sh.style, BorderLineStyle::Dashed);
        assert!(sh.style_active);
        assert_eq!(sh.width, 2.0);
        assert_eq!(eng.compute(&divs[1]).outline.style, BorderLineStyle::Dotted);
        assert_eq!(eng.compute(&divs[2]).outline.style, BorderLineStyle::Double);
        // `outline-style: none` apaga (style_active=false).
        assert!(!eng.compute(&divs[3]).outline.style_active);
        // Default → Solid.
        assert_eq!(eng.compute(&divs[4]).outline.style, BorderLineStyle::Solid);
    }

    #[test]
    fn border_style_dashed_dotted_double() {
        // Parser del keyword → patrón visual.
        assert_eq!(parse_border_line_style("dashed"), Some(BorderLineStyle::Dashed));
        assert_eq!(parse_border_line_style("DOTTED"), Some(BorderLineStyle::Dotted));
        assert_eq!(parse_border_line_style("double"), Some(BorderLineStyle::Double));
        // Estilos 3D (desde Fase 7.237) — mapean a sus variantes.
        assert_eq!(parse_border_line_style("groove"), Some(BorderLineStyle::Groove));
        assert_eq!(parse_border_line_style("RIDGE"), Some(BorderLineStyle::Ridge));
        assert_eq!(parse_border_line_style("inset"), Some(BorderLineStyle::Inset));
        assert_eq!(parse_border_line_style("outset"), Some(BorderLineStyle::Outset));
        assert_eq!(parse_border_line_style("zigzag"), None);

        let html = r##"<html><head><style>
            div.sh { border: 2px dashed red }
            div.ls { border-width: 3px; border-color: blue; border-style: dotted }
            div.db { border: 4px double green }
            div.none { border: 1px solid black; border-style: none }
            div.plain { border: 1px solid black }
        </style></head><body>
            <div class="sh"></div><div class="ls"></div><div class="db"></div>
            <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 5);
        // Shorthand `border: 2px dashed red`.
        let sh = eng.compute(&divs[0]);
        assert_eq!(sh.border_style, BorderLineStyle::Dashed);
        assert_eq!(sh.border_widths.top, 2.0);
        // Longhand `border-style: dotted` (sobre width/color sueltos).
        assert_eq!(eng.compute(&divs[1]).border_style, BorderLineStyle::Dotted);
        // `double`.
        assert_eq!(eng.compute(&divs[2]).border_style, BorderLineStyle::Double);
        // `border-style: none` desactiva el border (width→0) — el patrón
        // queda como estaba (Solid) pero no se pinta.
        let nb = eng.compute(&divs[3]);
        assert_eq!(nb.border_widths.top, 0.0);
        // Sin estilo explícito → Solid default.
        assert_eq!(eng.compute(&divs[4]).border_style, BorderLineStyle::Solid);
    }

    #[test]
    fn border_style_3d_fase_7_237() {
        // Los 4 estilos 3D llegan a `ComputedStyle.border_style` por
        // shorthand y longhand. El render por par de lados se prueba
        // visualmente — acá sólo el mapeo.
        let html = r##"<html><head><style>
            div.gr { border: 4px groove #888 }
            div.rg { border: 4px ridge #888 }
            div.ins { border: 4px inset #888 }
            div.out { border: 4px outset #888 }
            div.lh { border: 4px solid #888; border-style: groove }
        </style></head><body>
            <div class="gr"></div><div class="rg"></div>
            <div class="ins"></div><div class="out"></div>
            <div class="lh"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 5);
        assert_eq!(eng.compute(&divs[0]).border_style, BorderLineStyle::Groove);
        assert_eq!(eng.compute(&divs[1]).border_style, BorderLineStyle::Ridge);
        assert_eq!(eng.compute(&divs[2]).border_style, BorderLineStyle::Inset);
        assert_eq!(eng.compute(&divs[3]).border_style, BorderLineStyle::Outset);
        // El longhand `border-style: groove` pisa el `solid` del
        // shorthand previo.
        assert_eq!(eng.compute(&divs[4]).border_style, BorderLineStyle::Groove);
        // Y el width sobrevive (border-style: groove no apaga el border).
        assert_eq!(eng.compute(&divs[4]).border_widths.top, 4.0);
    }

    #[test]
    fn text_decoration_thickness_y_underline_offset() {
        let html = r##"<html><head><style>
            p.t { text-decoration: underline; text-decoration-thickness: 3px }
            p.o { text-decoration: underline; text-underline-offset: 2px }
            p.auto { text-decoration: underline; text-decoration-thickness: auto;
                     text-underline-offset: auto }
            p.ff { text-decoration-thickness: from-font }
            p.plain { text-decoration: underline }
        </style></head><body>
            <p class="t">a</p><p class="o">b</p><p class="auto">c</p>
            <p class="ff">d</p><p class="plain">e</p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 5);
        assert_eq!(eng.compute(&ps[0]).text_decoration_thickness, Some(3.0));
        assert_eq!(eng.compute(&ps[1]).text_underline_offset, Some(2.0));
        // `auto` explícito → None (default derivado).
        let a = eng.compute(&ps[2]);
        assert_eq!(a.text_decoration_thickness, None);
        assert_eq!(a.text_underline_offset, None);
        // `from-font` → None (igual que auto en nuestro modelo).
        assert_eq!(eng.compute(&ps[3]).text_decoration_thickness, None);
        // Sin declarar → None ambos.
        let plain = eng.compute(&ps[4]);
        assert_eq!(plain.text_decoration_thickness, None);
        assert_eq!(plain.text_underline_offset, None);
    }

    #[test]
    fn font_size_acepta_calc_y_clamp() {
        // Tipografía fluida: font-size con funciones matemáticas de
        // unidades absolutas resuelve en parse-time.
        let html = r#"<html><head><style>
            .a{font-size:calc(10px + 6px)}
            .b{font-size:clamp(1rem, 2rem, 3rem)}
            .c{font-size:min(30px, 20px)}
        </style></head><body>
            <p class="a">a</p><p class="b">b</p><p class="c">c</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).font_size, 16.0); // 10+6
        assert_eq!(eng.compute(&ps[1]).font_size, 32.0); // 2rem = 32px
        assert_eq!(eng.compute(&ps[2]).font_size, 20.0); // min
    }

    #[test]
    fn font_shorthand_expande_longhands() {
        // `font:` shorthand reparte style/weight/size/line-height/family.
        let html = r#"<html><head><style>
            .a{font:italic bold 20px/1.5 "Helvetica", sans-serif}
            .b{font:16px serif}
            .c{font:300 2rem monospace}
            .d{font:caption}
        </style></head><body>
            <p class="a">a</p><p class="b">b</p>
            <p class="c">c</p><p class="d">d</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        // .a — todos los ejes presentes.
        let a = eng.compute(&ps[0]);
        assert_eq!(a.font_style, FontStyle::Italic);
        assert_eq!(a.font_weight, 700);
        assert_eq!(a.font_size, 20.0);
        assert!((a.line_height.unwrap() - 1.5).abs() < 1e-6);
        assert_eq!(a.font_family.as_deref(), Some(r#""Helvetica", sans-serif"#));
        // .b — sólo size + family; el resto queda en defaults heredados.
        let b = eng.compute(&ps[1]);
        assert_eq!(b.font_size, 16.0);
        assert_eq!(b.font_style, FontStyle::Normal);
        assert_eq!(b.font_family.as_deref(), Some("serif"));
        // .c — weight numérico + rem.
        let c = eng.compute(&ps[2]);
        assert_eq!(c.font_weight, 300);
        assert_eq!(c.font_size, 32.0);
        assert_eq!(c.font_family.as_deref(), Some("monospace"));
        // .d — fuente de sistema: shorthand ignorado, size queda en default UA.
        assert_eq!(eng.compute(&ps[3]).font_size, 16.0);
    }

    #[test]
    fn css_wide_keywords_inherit_initial_unset() {
        let html = r#"<html><head><style>
            .bg{background-color:inherit}
            .initc{color:initial}
            .unsbg{background-color:unset}
            .unsc{color:unset}
            .dispinh{display:inherit}
        </style></head><body>
            <div style="color:red; background-color:blue; display:block">
                <span class="bg">a</span><span class="initc">b</span>
                <span class="unsbg">c</span><span class="unsc">d</span>
                <span class="dispinh">e</span>
            </div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut div = None;
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("div") => div = Some(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        let parent = eng.compute(div.as_ref().unwrap());
        assert_eq!(parent.color, Color::rgb(255, 0, 0));
        assert_eq!(parent.background, Some(Color::rgb(0, 0, 255)));
        let c = |i: usize| eng.compute_with_parent(&spans[i], Some(&parent));
        // background-color: inherit fuerza herencia de una prop NO heredable.
        assert_eq!(c(0).background, Some(Color::rgb(0, 0, 255)));
        // color: initial resetea al default (negro), ignorando la herencia.
        assert_eq!(c(1).color, Color::BLACK);
        // background-color: unset = initial (no heredable) → None.
        assert_eq!(c(2).background, None);
        // color: unset = inherit (heredable) → rojo del padre.
        assert_eq!(c(3).color, Color::rgb(255, 0, 0));
        // display: inherit toma el block del padre (un span sería inline).
        assert_eq!(c(4).display, Display::Block);
    }

    #[test]
    fn font_size_relativo_em_pct_keywords() {
        // `em`/`%`/`larger` resuelven contra el font-size HEREDADO (20px);
        // `rem` y los keywords absolutos quedan fijos.
        let html = r#"<html><head><style>
            .em{font-size:1.5em}
            .pct{font-size:150%}
            .larger{font-size:larger}
            .large{font-size:large}
            .rem{font-size:2rem}
        </style></head><body>
            <div style="font-size:20px">
                <p class="em">a</p><p class="pct">b</p>
                <p class="larger">c</p><p class="large">d</p>
                <p class="rem">e</p>
            </div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut div = None;
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("div") => div = Some(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        // El `<div style="font-size:20px">` es el padre heredado.
        let parent = eng.compute(div.as_ref().unwrap());
        assert_eq!(parent.font_size, 20.0);
        let fs = |i: usize| eng.compute_with_parent(&ps[i], Some(&parent)).font_size;
        assert_eq!(fs(0), 30.0); // 1.5em × 20
        assert_eq!(fs(1), 30.0); // 150% × 20
        assert!((fs(2) - 24.0).abs() < 1e-3); // larger = ×1.2 × 20
        assert_eq!(fs(3), 18.0); // large = absoluto
        assert_eq!(fs(4), 32.0); // 2rem = root 16
    }

    #[test]
    fn margin_auto_centra_horizontal() {
        // `margin: 0 auto` y longhands con `auto` marcan el flag de centrado
        // sin perder los px verticales.
        let html = r#"<html><head><style>
            .a{margin:0 auto}
            .b{margin:10px 20px 30px auto}
            .c{margin-left:auto; margin-right:auto}
            .d{margin:8px}
            .e{margin-left:auto}
            .e{margin-left:12px}
        </style></head><body>
            <div class="a">a</div><div class="b">b</div>
            <div class="c">c</div><div class="d">d</div>
            <div class="e">e</div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ds = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                ds.push(n.clone());
            }
        });
        // .a — `0 auto`: top/bottom 0, left/right auto.
        let a = eng.compute(&ds[0]);
        assert!(a.margin_left_auto && a.margin_right_auto);
        assert_eq!(a.margin.top, 0.0);
        // .b — `10 20 30 auto`: sólo left es auto; right=20px no.
        let b = eng.compute(&ds[1]);
        assert!(b.margin_left_auto && !b.margin_right_auto);
        assert_eq!(b.margin.top, 10.0);
        assert_eq!(b.margin.right, 20.0);
        assert_eq!(b.margin.bottom, 30.0);
        // .c — longhands auto en ambos lados.
        let c = eng.compute(&ds[2]);
        assert!(c.margin_left_auto && c.margin_right_auto);
        // .d — sin auto.
        let d = eng.compute(&ds[3]);
        assert!(!d.margin_left_auto && !d.margin_right_auto);
        assert_eq!(d.margin.left, 8.0);
        // .e — un px posterior pisa el auto previo (mismo selector/orden).
        let e = eng.compute(&ds[4]);
        assert!(!e.margin_left_auto);
        assert_eq!(e.margin.left, 12.0);
    }

    #[test]
    fn parsea_calc_solo_px() {
        // calc(10px + 5px) resuelve a Px(15) en parse time.
        assert_eq!(parse_length_or_pct("calc(10px + 5px)"), Some(LengthVal::Px(15.0)));
        assert_eq!(parse_length_or_pct("calc(20px - 5px)"), Some(LengthVal::Px(15.0)));
    }

    #[test]
    fn parsea_calc_solo_pct() {
        assert_eq!(parse_length_or_pct("calc(80% - 10%)"), Some(LengthVal::Pct(70.0)));
        assert_eq!(parse_length_or_pct("calc(50% + 20%)"), Some(LengthVal::Pct(70.0)));
    }

    #[test]
    fn parsea_calc_mixto_pierde_offset_px() {
        // Mezcla pct + px: conservamos el Pct e ignoramos el px (no
        // tenemos container width acá; taffy no soporta calc nativo).
        // Esto es una limitación documentada del soporte de calc.
        assert_eq!(parse_length_or_pct("calc(100% - 20px)"), Some(LengthVal::Pct(100.0)));
        assert_eq!(parse_length_or_pct("calc(50% + 10px)"), Some(LengthVal::Pct(50.0)));
    }

    #[test]
    fn parsea_calc_invalido_devuelve_none() {
        // Tokens incompletos / mismatched parens / op desconocido.
        assert!(parse_length_or_pct("calc(10px +)").is_none());
        assert!(parse_length_or_pct("calc(10px").is_none());
        // Sumar número y longitud es inválido (CSS).
        assert!(parse_length_or_pct("calc(10px + 2)").is_none());
        // longitud * longitud inválido.
        assert!(parse_length_or_pct("calc(10px * 5px)").is_none());
        // división por cero.
        assert!(parse_length_or_pct("calc(10px / 0)").is_none());
    }

    #[test]
    fn parsea_calc_mul_div_y_precedencia() {
        // `*` y `/` por escalar.
        assert_eq!(parse_length_or_pct("calc(10px * 2)"), Some(LengthVal::Px(20.0)));
        assert_eq!(parse_length_or_pct("calc(2 * 10px)"), Some(LengthVal::Px(20.0)));
        assert_eq!(parse_length_or_pct("calc(100px / 4)"), Some(LengthVal::Px(25.0)));
        // Precedencia: `*` antes que `+`.
        assert_eq!(parse_length_or_pct("calc(10px + 2 * 5px)"), Some(LengthVal::Px(20.0)));
        // Paréntesis fuerzan el orden.
        assert_eq!(parse_length_or_pct("calc((10px + 2px) * 3)"), Some(LengthVal::Px(36.0)));
        // % puro con `/`.
        assert_eq!(parse_length_or_pct("calc(90% / 3)"), Some(LengthVal::Pct(30.0)));
        // Unidades absolutas: rem→px (×16).
        assert_eq!(parse_length_or_pct("calc(1rem + 4px)"), Some(LengthVal::Px(20.0)));
    }

    #[test]
    fn parsea_min_max_clamp() {
        // min/max con px puro → exacto.
        assert_eq!(parse_length_or_pct("min(10px, 20px)"), Some(LengthVal::Px(10.0)));
        assert_eq!(parse_length_or_pct("max(10px, 20px, 5px)"), Some(LengthVal::Px(20.0)));
        // clamp(lo, val, hi) acota.
        assert_eq!(parse_length_or_pct("clamp(10px, 15px, 20px)"), Some(LengthVal::Px(15.0)));
        assert_eq!(parse_length_or_pct("clamp(10px, 5px, 20px)"), Some(LengthVal::Px(10.0)));
        assert_eq!(parse_length_or_pct("clamp(10px, 25px, 20px)"), Some(LengthVal::Px(20.0)));
        // Unidades mezcladas pero todas absolutas (rem→px) → exacto.
        assert_eq!(parse_length_or_pct("clamp(1rem, 2rem, 3rem)"), Some(LengthVal::Px(32.0)));
        // % puro.
        assert_eq!(parse_length_or_pct("max(50%, 80%)"), Some(LengthVal::Pct(80.0)));
        // Mezcla px/% incomparable → degrada al primer arg.
        assert_eq!(parse_length_or_pct("min(100%, 600px)"), Some(LengthVal::Pct(100.0)));
        // clamp incomparable → degrada al valor central.
        assert_eq!(parse_length_or_pct("clamp(1rem, 50%, 3rem)"), Some(LengthVal::Pct(50.0)));
        // calc anidado dentro de min.
        assert_eq!(parse_length_or_pct("min(calc(10px + 5px), 20px)"), Some(LengthVal::Px(15.0)));
    }

    #[test]
    fn parsea_regla_simple() {
        let rules = parse_stylesheet("p { color: red; font-size: 14px; }", &HashMap::new(), DEFAULT_VIEWPORT);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.compounds.len(), 1);
        assert!(matches!(
            &rules[0].selector.compounds[0].tag,
            TagPart::Type(t) if t == "p"
        ));
        assert_eq!(rules[0].decls.len(), 2);
    }

    #[test]
    fn selector_compound_matchea() {
        // `a.btn` matchea sólo `<a class="btn">`.
        let html = r##"<html><head><style>a.btn{color:red}</style></head><body>
                <a class="btn" href="#">click</a>
                <a href="#">otro</a>
                <span class="btn">no soy a</span>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("a") => anchors.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        assert_eq!(anchors.len(), 2);
        assert_eq!(spans.len(), 1);
        // anchors[0] tiene class="btn" — `.btn { color: red }` pisa
        // el azul-de-link del UA stylesheet.
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(255, 0, 0));
        // anchors[1] sin class — sólo aplica el UA, que pinta `<a>`
        // con el azul clásico de browser (0, 0, 238).
        assert_eq!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 238));
        // span.btn no es <a> — no aplica el UA de link.
        assert_eq!(eng.compute(&spans[0]).color, Color::BLACK);
    }

    #[test]
    fn current_color_se_resuelve_al_color() {
        let html = r#"<html><head><style>
            .a { color: red; border-color: currentColor; }
            .b { border: 2px solid currentColor; color: rgb(0,128,0); }
            .c { background-color: currentColor; color: blue; }
            .d { outline: 2px solid currentColor; color: #ff8800; }
        </style></head><body>
            <div class="a"></div>
            <div class="b"></div>
            <div class="c"></div>
            <div class="d"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 4);
        // .a — border-color: currentColor = rojo en los 4 lados.
        let a = eng.compute(&divs[0]);
        assert_eq!(a.border_colors.top, Some(Color::rgb(255, 0, 0)));
        assert_eq!(a.border_colors.left, Some(Color::rgb(255, 0, 0)));
        // El buffer transitorio queda vacío (no se hereda ni viaja al box).
        assert!(a.current_color.is_empty());
        // .b — el `color` se declara DESPUÉS del border en la regla; la
        // resolución post-pass igual lo toma (verde), no el negro previo.
        let b = eng.compute(&divs[1]);
        assert_eq!(b.border_colors.top, Some(Color::rgb(0, 128, 0)));
        assert_eq!(b.border_widths.top, 2.0);
        // .c — background = el color del elemento (azul).
        let c = eng.compute(&divs[2]);
        assert_eq!(c.background, Some(Color::rgb(0, 0, 255)));
        // .d — outline color = el color (#ff8800).
        let d = eng.compute(&divs[3]);
        assert_eq!(d.outline.color, Some(Color::rgb(255, 136, 0)));
        assert_eq!(d.outline.width, 2.0);
    }

    #[test]
    fn current_color_hereda_el_color_del_ancestro() {
        let html = r#"<html><head><style>
            .parent { color: rgb(10,20,30); }
            .child { border-color: currentColor; }
        </style></head><body>
            <div class="parent"><span class="child"></span></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let (mut parent, mut child) = (None, None);
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("div") => parent = Some(n.clone()),
                Some("span") => child = Some(n.clone()),
                _ => {}
            }
        });
        let parent = parent.unwrap();
        let child = child.unwrap();
        let ps = eng.compute(&parent);
        // El hijo no declara `color`: `currentColor` toma el heredado.
        let cs = eng.compute_with_parent(&child, Some(&ps));
        assert_eq!(cs.color, Color::rgb(10, 20, 30)); // heredado
        assert_eq!(cs.border_colors.top, Some(Color::rgb(10, 20, 30)));
    }

    #[test]
    fn pseudo_empty_matchea() {
        let html = r#"<html><head><style>div:empty{color:red}</style></head><body>
            <div class="vacio"></div>
            <div class="ws">   </div>
            <div class="texto">hola</div>
            <div class="hijo"><span></span></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let red = Color::rgb(255, 0, 0);
        assert_eq!(eng.compute(&divs[0]).color, red); // vacío
        assert_eq!(eng.compute(&divs[1]).color, red); // sólo whitespace → :empty
        assert_eq!(eng.compute(&divs[2]).color, Color::BLACK); // tiene texto
        assert_eq!(eng.compute(&divs[3]).color, Color::BLACK); // tiene hijo elemento
    }

    #[test]
    fn pseudo_root_matchea_html() {
        let html = r#"<html><head><style>:root{color:#008000}</style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut html_el = None;
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("html") {
                html_el = Some(n.clone());
            }
        });
        assert_eq!(eng.compute(&html_el.unwrap()).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn pseudo_any_link_matchea() {
        let html = r#"<html><head><style>:any-link{color:#0000ff}</style></head><body>
            <a href="/x">con</a><a>sin</a>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("a") {
                anchors.push(n.clone());
            }
        });
        assert_eq!(anchors.len(), 2);
        // <a href> matchea :any-link (especificidad 10 > UA `a`).
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(0, 0, 255));
        // <a> sin href NO matchea :any-link.
        assert_ne!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn pseudo_has_relacional() {
        let html = r#"<html><head><style>
            .has-span:has(span){color:red}
            .has-child:has(> .active){color:rgb(0,128,0)}
            .has-adj:has(+ p){color:rgb(0,0,255)}
        </style></head><body>
            <div id="d1" class="has-span"><span>x</span></div>
            <div id="d2" class="has-span"><b>y</b></div>
            <div id="d3" class="has-child"><em class="active"></em></div>
            <div id="d4" class="has-child"><p><em class="active"></em></p></div>
            <div id="d5" class="has-adj">t</div><p>z</p>
            <div id="d6" class="has-adj">t</div><span>z</span>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // Descendiente: matchea con span, no sin él.
        assert_eq!(eng.compute(&by_id("d1")).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&by_id("d2")).color, Color::BLACK);
        // Hijo directo (`> .active`): matchea sólo si es hijo DIRECTO.
        assert_eq!(eng.compute(&by_id("d3")).color, Color::rgb(0, 128, 0));
        assert_eq!(eng.compute(&by_id("d4")).color, Color::BLACK); // .active es nieto
        // Hermano adyacente (`+ p`): matchea sólo si el siguiente es <p>.
        assert_eq!(eng.compute(&by_id("d5")).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&by_id("d6")).color, Color::BLACK); // siguiente es <span>
    }

    #[test]
    fn pseudo_lang_matchea() {
        let html = r#"<html lang="en-US"><head><style>
            :lang(en){color:rgb(0,0,255)}
            .fr:lang(fr){color:rgb(0,128,0)}
        </style></head><body>
            <p id="hereda">x</p>
            <p id="propio" lang="fr" class="fr">y</p>
            <p id="otro" lang="de">z</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // Hereda `lang="en-US"` del <html> → :lang(en) matchea (subtag).
        assert_eq!(eng.compute(&by_id("hereda")).color, Color::rgb(0, 0, 255));
        // lang propio "fr" → .fr:lang(fr) matchea (verde), no :lang(en).
        assert_eq!(eng.compute(&by_id("propio")).color, Color::rgb(0, 128, 0));
        // lang "de" → ni :lang(en) ni :lang(fr).
        assert_eq!(eng.compute(&by_id("otro")).color, Color::BLACK);
    }

    #[test]
    fn selector_hijo_directo_matchea() {
        // `ul > li` matchea `<li>` que es hijo *directo* de `<ul>`. Un
        // `<li>` dentro de `<ol>` adentro de `<ul>` no debe matchear.
        let html = r#"<html><head><style>ul > li{color:#0a0}</style></head>
            <body>
              <ul><li>directo</li></ul>
              <ol><li>indirecto</li></ol>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 2);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_adyacente_matchea() {
        // `h2 + p` matchea sólo el primer `<p>` inmediatamente después
        // de un `<h2>`.
        let html = r#"<html><head><style>h2+p{color:#00f}</style></head>
            <body>
              <h2>t</h2><p>uno</p><p>dos</p>
              <p>aislado</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_hermano_general_matchea() {
        // `h2 ~ p` matchea TODOS los `<p>` hermanos posteriores a un `<h2>`.
        let html = r#"<html><head><style>h2~p{color:#00f}</style></head>
            <body>
              <p>antes</p><h2>t</h2><p>uno</p><span>x</span><p>dos</p>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // El primero está antes del h2 → no aplica.
        assert_eq!(eng.compute(&ps[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[1]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn selector_descendiente_matchea() {
        // `.menu li` matchea sólo los `<li>` dentro de `.menu`.
        let html = r#"<html><head><style>.menu li{color:#00aa00}</style></head>
            <body>
              <ul class="menu"><li>uno</li><li>dos</li></ul>
              <ul><li>tres</li></ul>
            </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 3);
        // Los dos primeros viven en .menu → verde
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0xaa, 0));
        // El tercero no
        assert_eq!(eng.compute(&lis[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_class_matchea() {
        let html = r#"<html><head><style>.alert{color:red}</style></head><body><p class="alert">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ps: Vec<_> = {
            let mut acc = Vec::new();
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::element_name(n).as_deref() == Some("p") {
                    acc.push(n.clone());
                }
            });
            acc
        };
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_id_matchea() {
        let html = r#"<html><head><style>#hero{color:#0000ff}</style></head><body><p id="hero">x</p><p>y</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_presente() {
        // `[href]` matchea cualquier elemento con atributo `href`.
        let html = r#"<html><head><style>[href]{color:red}</style></head>
            <body><a href="x">link</a><a>sin</a><span>no a</span></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut elems = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if matches!(
                crate::dom::element_name(n).as_deref(),
                Some("a") | Some("span")
            ) {
                elems.push(n.clone());
            }
        });
        // a[href] → rojo (la regla `[href]{color:red}` con
        // especificidad 10 pisa el UA `a{color:#00ee}`); a sin href no
        // matchea pero recibe el UA = azul-link; span → BLACK default.
        assert_eq!(eng.compute(&elems[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&elems[1]).color, Color::rgb(0, 0, 238));
        assert_eq!(eng.compute(&elems[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_equals() {
        // `input[type="checkbox"]` matchea sólo el checkbox.
        let html = r##"<html><head><style>input[type="checkbox"]{color:#00aa00}</style></head>
            <body>
              <input type="checkbox">
              <input type="text">
              <input>
            </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("input") {
                inputs.push(n.clone());
            }
        });
        assert_eq!(inputs.len(), 3);
        assert_eq!(eng.compute(&inputs[0]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&inputs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&inputs[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_attr_prefix_suffix_contains() {
        let html = r##"<html><head><style>
            a[href^="https"]{color:#00f}
            img[src$=".png"]{color:#0f0}
            div[class*="warn"]{color:#f00}
        </style></head>
        <body>
            <a href="https://x">seguro</a>
            <a href="http://x">inseguro</a>
            <img src="logo.png">
            <img src="logo.jpg">
            <div class="banner warn-strong">!!</div>
            <div class="banner">--</div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut anchors = Vec::new();
        let mut imgs = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("a") => anchors.push(n.clone()),
            Some("img") => imgs.push(n.clone()),
            Some("div") => divs.push(n.clone()),
            _ => {}
        });
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(0, 0, 255));
        // anchors[1] no matchea `[href^="https"]` pero recibe el UA
        // de `<a>` (azul 0,0,238).
        assert_eq!(eng.compute(&anchors[1]).color, Color::rgb(0, 0, 238));
        assert_eq!(eng.compute(&imgs[0]).color, Color::rgb(0, 255, 0));
        assert_eq!(eng.compute(&imgs[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&divs[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&divs[1]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_only_child() {
        let html = r#"<html><head><style>
            li:first-child{color:#00f}
            li:last-child{background:#0f0}
            p:only-child{color:#f0f}
        </style></head>
        <body>
          <ul><li>a</li><li>b</li><li>c</li></ul>
          <section><p>solo</p></section>
          <section><p>uno</p><p>dos</p></section>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("li") => lis.push(n.clone()),
            Some("p") => ps.push(n.clone()),
            _ => {}
        });
        // li:first-child sólo el primero
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0, 0, 255));
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        // li:last-child sólo el tercero (background)
        assert!(eng.compute(&lis[0]).background.is_none());
        assert_eq!(eng.compute(&lis[2]).background, Some(Color::rgb(0, 255, 0)));
        // p:only-child el primero (único en su section), no los otros dos
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(255, 0, 255));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::BLACK);
    }

    #[test]
    fn selector_first_last_of_type() {
        let html = r#"<html><head><style>
            p:first-of-type{color:#00f}
            p:last-of-type{color:#0a0}
        </style></head>
        <body>
          <div>x</div>
          <p>uno</p>
          <span>y</span>
          <p>dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 3);
        // primer <p> → azul (es :first-of-type aunque haya <div> y <span> antes)
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0, 0, 255));
        // del medio → ninguno (last gana cascada al último pero a este ninguno)
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        // último <p> → verde
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0, 0xaa, 0));
    }

    #[test]
    fn parsea_width_max_width() {
        let s = parse_stylesheet(
            "p { width: 80%; max-width: 800px } div { width: auto }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        assert_eq!(s.len(), 2);
        assert!(matches!(s[0].decls[0].kind, DeclKind::Width(LengthVal::Pct(80.0))));
        assert!(matches!(s[0].decls[1].kind, DeclKind::MaxWidth(LengthVal::Px(800.0))));
        assert!(matches!(s[1].decls[0].kind, DeclKind::Width(LengthVal::Auto)));
    }

    #[test]
    fn parsea_text_align() {
        let s = parse_stylesheet(
            "h1 { text-align: center } p { text-align: right }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        assert!(matches!(s[0].decls[0].kind, DeclKind::TextAlign(TextAlign::Center)));
        assert!(matches!(s[1].decls[0].kind, DeclKind::TextAlign(TextAlign::Right)));
    }

    #[test]
    fn parsea_line_height() {
        let s = parse_stylesheet(
            "p { line-height: 1.5 } h1 { line-height: 32px }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        // 1.5 → 1.5
        assert!(matches!(s[0].decls[0].kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6));
        // 32px sobre font-size 16px estimado → 2.0
        assert!(matches!(s[1].decls[0].kind, DeclKind::LineHeight(v) if (v - 2.0).abs() < 1e-6));
    }

    #[test]
    fn computa_width_y_text_align() {
        let html = r#"<html><head><style>
            .narrow{max-width:600px;text-align:center;line-height:1.6}
        </style></head><body><div class="narrow">x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert_eq!(st.max_width, LengthVal::Px(600.0));
        assert_eq!(st.text_align, TextAlign::Center);
        assert!((st.line_height.unwrap() - 1.6).abs() < 1e-6);
    }

    #[test]
    fn hereda_color_y_font_size_del_padre() {
        // `<p style="color:red; font-size:20px">foo <em>bar</em></p>` —
        // el `<em>` no tiene regla propia pero hereda color y tamaño.
        let html = r#"<html><body><p style="color:red; font-size:20px">foo<em>bar</em></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
        let em = dom.find("em").unwrap();
        let em_style = eng.compute_with_parent(&em, Some(&p_style));
        assert_eq!(em_style.color, Color::rgb(255, 0, 0));
        assert!((em_style.font_size - 20.0).abs() < 1e-6);
    }

    #[test]
    fn no_hereda_propiedades_no_heredables() {
        // background y margin/padding NO heredan.
        let html = r#"<html><body><div style="background:red; margin:30px"><p>x</p></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let div_style = eng.compute_with_parent(&div, None);
        assert_eq!(div_style.background, Some(Color::rgb(255, 0, 0)));
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, Some(&div_style));
        assert_eq!(p_style.background, None);
        // margin del <p> es 12px (UA default), no 30px del padre.
        assert!((p_style.margin.top - 12.0).abs() < 1e-6);
        assert!((p_style.margin.bottom - 12.0).abs() < 1e-6);
    }

    #[test]
    fn font_weight_bold_local_no_propaga_a_padre_no_bold() {
        // Un `<b>` dentro de `<p>` no-bold sigue siendo bold.
        let html = "<html><body><p>foo<b>bar</b></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.font_weight, 400);
        let b = dom.find("b").unwrap();
        let b_style = eng.compute_with_parent(&b, Some(&p_style));
        assert_eq!(b_style.font_weight, 700);
    }

    #[test]
    fn box_tree_propaga_color_a_hoja_de_texto() {
        // Verifica el bug original: el text leaf debe heredar el color
        // del `<p>` padre.
        let html = r#"<html><body><p style="color: #00ff00">verde</p></body></html>"#;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut leaf_colors = Vec::new();
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("verde") {
                leaf_colors.push(b.color);
            }
        });
        assert_eq!(leaf_colors.len(), 1);
        assert_eq!(leaf_colors[0], Color::rgb(0, 0xff, 0));
    }

    #[test]
    fn specificity_calculada_correctamente() {
        // `body p` = 0,0,2 → 2
        let s1 = parse_selector("body p").unwrap();
        assert_eq!(s1.specificity(), 2);
        // `.menu li` = 0,1,1 → 11
        let s2 = parse_selector(".menu li").unwrap();
        assert_eq!(s2.specificity(), 11);
        // `#hero` = 1,0,0 → 100
        let s3 = parse_selector("#hero").unwrap();
        assert_eq!(s3.specificity(), 100);
        // `a.btn[href^="https"]:first-child` = 0,3,1 → 31
        let s4 = parse_selector(r#"a.btn[href^="https"]:first-child"#).unwrap();
        assert_eq!(s4.specificity(), 31);
        // `nav > a#x.y` = 1,1,2 → 112
        let s5 = parse_selector("nav > a#x.y").unwrap();
        assert_eq!(s5.specificity(), 112);
    }

    #[test]
    fn id_vence_a_tag_aunque_llegue_antes() {
        // `#hero { color: blue }` está ANTES que `body p { color: red }`
        // en el stylesheet — sin especificidad, el último (rojo) ganaba.
        // Con especificidad, el #id (100 > 2) gana azul.
        let html = r#"<html><head><style>
            #hero { color: blue }
            body p { color: red }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn clase_vence_a_tag() {
        // `.alert` (10) > `p` (1) aunque ambos matcheen.
        let html = r#"<html><head><style>
            .alert { color: red }
            p { color: blue }
        </style></head><body><p class="alert">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn inline_style_vence_a_id() {
        // Inline tiene especificidad implícita 1000 — gana sobre `#hero`.
        let html = r##"<html><head><style>
            #hero { color: blue }
        </style></head><body><p id="hero" style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn empate_de_especificidad_gana_el_ultimo() {
        // Dos selectores con misma especificidad: gana el que llega después.
        let html = r#"<html><head><style>
            p { color: red }
            p { color: blue }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn important_vence_normal_de_mayor_especificidad() {
        // `body p { color: red !important }` (spec=2) debe vencer a
        // `#hero { color: blue }` (spec=100) — important rompe la
        // jerarquía de especificidad dentro del mismo origen.
        let html = r#"<html><head><style>
            body p { color: red !important }
            #hero { color: blue }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn important_inline_vence_important_de_id() {
        // Inline !important vence cualquier !important de selector.
        let html = r##"<html><head><style>
            #hero { color: red !important }
        </style></head><body><p id="hero" style="color: green !important">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn normal_inline_pierde_contra_important_de_regla() {
        // Inline normal (1000) pierde contra !important de cualquier selector.
        let html = r##"<html><head><style>
            p { color: red !important }
        </style></head><body><p style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn parsea_border_shorthand() {
        let html = r#"<html><head><style>
            .a { border: 2px solid #ff0000 }
            .b { border: 1px dashed blue !important }
            .c { border: none }
            .d { border-radius: 8px }
        </style></head><body>
          <div class="a"></div><div class="b"></div>
          <div class="c"></div><div class="d"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 4);
        let a = eng.compute(&divs[0]);
        assert!((a.border_widths.top - 2.0).abs() < 1e-6);
        assert_eq!(a.border_colors.top, Some(Color::rgb(255, 0, 0)));
        let b = eng.compute(&divs[1]);
        assert!((b.border_widths.top - 1.0).abs() < 1e-6);
        assert_eq!(b.border_colors.top, Some(Color::rgb(0, 0, 255)));
        let c = eng.compute(&divs[2]);
        assert_eq!(c.border_colors.top, None); // `none` deshabilita
        assert!((c.border_widths.top - 0.0).abs() < 1e-6);
        let d = eng.compute(&divs[3]);
        assert!((d.border_radii.top_left - 8.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_per_side() {
        // `border-top: 2px solid red` setea sólo el top; `border-bottom-color`
        // sólo el color del bottom; `border-right-width` sólo el ancho derecho.
        let html = r#"<html><head><style>
            div {
                border-top: 2px solid #ff0000;
                border-bottom-color: #0000ff;
                border-bottom-width: 4px;
                border-bottom-style: solid;
                border-right-width: 1px;
                border-right-color: #00ff00;
                border-right-style: solid;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let s = eng.compute(&div);
        // Top: del shorthand
        assert!((s.border_widths.top - 2.0).abs() < 1e-6);
        assert_eq!(s.border_colors.top, Some(Color::rgb(255, 0, 0)));
        // Bottom: 3 longhand
        assert!((s.border_widths.bottom - 4.0).abs() < 1e-6);
        assert_eq!(s.border_colors.bottom, Some(Color::rgb(0, 0, 255)));
        // Right: 3 longhand
        assert!((s.border_widths.right - 1.0).abs() < 1e-6);
        assert_eq!(s.border_colors.right, Some(Color::rgb(0, 0xff, 0)));
        // Left: no se tocó
        assert_eq!(s.border_widths.left, 0.0);
        assert_eq!(s.border_colors.left, None);
    }

    #[test]
    fn parsea_border_radius_per_corner() {
        let html = r#"<html><head><style>
            div {
                border-top-left-radius: 4px;
                border-top-right-radius: 8px;
                border-bottom-right-radius: 12px;
                border-bottom-left-radius: 16px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let s = eng.compute(&div);
        assert!((s.border_radii.top_left - 4.0).abs() < 1e-6);
        assert!((s.border_radii.top_right - 8.0).abs() < 1e-6);
        assert!((s.border_radii.bottom_right - 12.0).abs() < 1e-6);
        assert!((s.border_radii.bottom_left - 16.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_propiedades_individuales() {
        let html = r#"<html><head><style>
            div { border-width: 3px; border-color: #00ff00; border-style: solid; border-radius: 5px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert!((st.border_widths.top - 3.0).abs() < 1e-6);
        assert_eq!(st.border_colors.top, Some(Color::rgb(0, 0xff, 0)));
        assert!((st.border_radii.top_left - 5.0).abs() < 1e-6);
    }

    #[test]
    fn hover_state_activa_regla_solo_cuando_corresponde() {
        // `.btn:hover { background: red }`: matchea con hover_active=true,
        // no matchea sin él.
        let html = r##"<html><head><style>
            .btn:hover { background: #ff0000 }
            .btn { background: #ffffff }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let base = eng.compute_with_parent_in_state(&a, None, false);
        let hover = eng.compute_with_parent_in_state(&a, None, true);
        assert_eq!(base.background, Some(Color::rgb(255, 255, 255)));
        assert_eq!(hover.background, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn hover_pseudo_aporta_a_specificity() {
        // `.btn:hover` debe tener specificity 0,2,0 → 20 (clase 10 + pseudo 10)
        let s = parse_selector(".btn:hover").unwrap();
        assert_eq!(s.specificity(), 20);
    }

    #[test]
    fn box_tree_expone_hover_background() {
        let html = r##"<html><head><style>
            .btn { background: white }
            .btn:hover { background: #ffaa00 }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut hover_bgs = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                hover_bgs.push(b.hover_background);
            }
        });
        assert_eq!(hover_bgs.len(), 1);
        assert_eq!(hover_bgs[0], Some(Color::rgb(0xff, 0xaa, 0)));
    }

    #[test]
    fn parsea_box_shadow_completo() {
        let html = r#"<html><head><style>
            .a { box-shadow: 2px 4px 8px 1px #000000 }
            .b { box-shadow: 1px 2px red }
            .c { box-shadow: none }
        </style></head><body>
          <div class="a"></div><div class="b"></div><div class="c"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let a_list = eng.compute(&divs[0]).box_shadows.clone();
        assert_eq!(a_list.len(), 1);
        let a = a_list[0];
        assert!((a.offset_x - 2.0).abs() < 1e-6);
        assert!((a.offset_y - 4.0).abs() < 1e-6);
        assert!((a.blur_px - 8.0).abs() < 1e-6);
        assert!((a.spread_px - 1.0).abs() < 1e-6);
        assert_eq!(a.color, Color::BLACK);
        assert!(!a.inset);
        let b = eng.compute(&divs[1]).box_shadows[0];
        assert_eq!(b.color, Color::rgb(255, 0, 0));
        assert!((b.blur_px - 0.0).abs() < 1e-6);
        assert!((b.spread_px - 0.0).abs() < 1e-6);
        assert!(eng.compute(&divs[2]).box_shadows.is_empty());
    }

    #[test]
    fn box_shadow_multi_e_inset_fase_7_236() {
        let html = r#"<html><head><style>
            .multi { box-shadow: 2px 2px #000, 4px 4px red, inset 1px 1px blue }
            .ins   { box-shadow: inset 3px 4px 5px 6px #00ff00 }
            .noop  { box-shadow: garbage }
        </style></head><body>
          <div class="multi"></div><div class="ins"></div><div class="noop"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let list = eng.compute(&divs[0]).box_shadows.clone();
        assert_eq!(list.len(), 3, "tres sombras en la lista");
        assert!(!list[0].inset && list[0].color == Color::BLACK);
        assert!(!list[1].inset && list[1].color == Color::rgb(255, 0, 0));
        assert!(list[2].inset && list[2].color == Color::rgb(0, 0, 255));
        let ins = eng.compute(&divs[1]).box_shadows[0];
        assert!(ins.inset);
        assert!((ins.offset_x - 3.0).abs() < 1e-6);
        assert!((ins.offset_y - 4.0).abs() < 1e-6);
        assert!((ins.blur_px - 5.0).abs() < 1e-6);
        assert!((ins.spread_px - 6.0).abs() < 1e-6);
        assert_eq!(ins.color, Color::rgb(0, 255, 0));
        assert!(eng.compute(&divs[2]).box_shadows.is_empty());
    }

    #[test]
    fn parse_nth_arg_acepta_formatos_comunes() {
        assert_eq!(parse_nth_arg("odd"), Some((2, 1)));
        assert_eq!(parse_nth_arg("even"), Some((2, 0)));
        assert_eq!(parse_nth_arg("3"), Some((0, 3)));
        assert_eq!(parse_nth_arg("n"), Some((1, 0)));
        assert_eq!(parse_nth_arg("2n"), Some((2, 0)));
        assert_eq!(parse_nth_arg("2n+1"), Some((2, 1)));
        assert_eq!(parse_nth_arg("3n -2"), Some((3, -2)));
        assert_eq!(parse_nth_arg("-n+3"), Some((-1, 3)));
        assert_eq!(parse_nth_arg("xyz"), None);
    }

    #[test]
    fn selector_nth_child_aplica() {
        // `li:nth-child(odd)` matchea li 1, 3 (1-indexed).
        let html = r#"<html><head><style>
            li:nth-child(odd) { color: #f00 }
            li:nth-child(2n) { color: #00f }
        </style></head><body><ul>
          <li>a</li><li>b</li><li>c</li><li>d</li>
        </ul></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 4);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0xff, 0, 0)); // odd
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0, 0xff)); // even (2n)
        assert_eq!(eng.compute(&lis[2]).color, Color::rgb(0xff, 0, 0)); // odd
        assert_eq!(eng.compute(&lis[3]).color, Color::rgb(0, 0, 0xff)); // even
    }

    #[test]
    fn selector_nth_child_n_fija() {
        // `:nth-child(3)` matchea SÓLO la tercera.
        let html = r#"<html><head><style>
            li:nth-child(3) { color: #0a0 }
        </style></head><body><ul>
          <li>1</li><li>2</li><li>3</li><li>4</li>
        </ul></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&lis[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&lis[2]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[3]).color, Color::BLACK);
    }

    #[test]
    fn selector_not_excluye() {
        // `p:not(.skip)` matchea todos los <p> excepto los con class skip.
        let html = r#"<html><head><style>
            p:not(.skip) { color: #f00 }
        </style></head><body>
          <p>uno</p>
          <p class="skip">dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0xff, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0xff, 0, 0));
    }

    #[test]
    fn specificity_not_aporta_la_del_argumento() {
        // `:not(#x)` aporta 100 (la del #id interno).
        let s = parse_selector(":not(#x)").unwrap();
        assert_eq!(s.specificity(), 100);
        // `a:not(.b)` aporta 1 (tag) + 10 (.b interno) = 11.
        let s = parse_selector("a:not(.b)").unwrap();
        assert_eq!(s.specificity(), 11);
    }

    #[test]
    fn not_anidado_se_rechaza() {
        // `:not(:not(p))` debe ignorarse, no soportamos recursión.
        assert!(parse_selector(":not(:not(p))").is_none());
    }

    #[test]
    fn cascada_inline_sobrescribe() {
        let html = "<html><head><style>p { color: red }</style></head><body><p style='color:blue'>x</p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let style = eng.compute(&p);
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn parsea_text_decoration() {
        assert_eq!(parse_text_decoration("underline"), Some(TextDecorationLine::Underline));
        assert_eq!(parse_text_decoration("line-through"), Some(TextDecorationLine::LineThrough));
        assert_eq!(parse_text_decoration("overline"), Some(TextDecorationLine::Overline));
        assert_eq!(parse_text_decoration("none"), Some(TextDecorationLine::None));
        // Shorthand con varios tokens: capturamos el line, ignoramos color/estilo.
        assert_eq!(
            parse_text_decoration("underline dotted red"),
            Some(TextDecorationLine::Underline)
        );
        assert_eq!(parse_text_decoration("solid red"), None);
    }

    #[test]
    fn ua_aplica_underline_a_link() {
        let html = "<html><body><a href='/x'>click</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let style = eng.compute(&a);
        assert_eq!(style.text_decoration, TextDecorationLine::Underline);
    }

    #[test]
    fn ua_aplica_line_through_a_del() {
        let html = "<html><body><del>removed</del></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("del").unwrap();
        let style = eng.compute(&d);
        assert_eq!(style.text_decoration, TextDecorationLine::LineThrough);
    }

    #[test]
    fn text_decoration_se_hereda_a_descendiente_inline() {
        // <a>foo <b>bar</b></a>: el `<b>` debe heredar underline desde `<a>`.
        let html =
            "<html><body><a href='/x'>foo <b>bar</b></a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let a_style = eng.compute(&a);
        let b = dom.find("b").unwrap();
        let b_style = eng.compute_with_parent(&b, Some(&a_style));
        assert_eq!(b_style.text_decoration, TextDecorationLine::Underline);
    }

    #[test]
    fn parsea_list_style_type() {
        assert_eq!(parse_list_style_type("disc"), Some(ListStyleType::Disc));
        assert_eq!(parse_list_style_type("circle"), Some(ListStyleType::Circle));
        assert_eq!(parse_list_style_type("square"), Some(ListStyleType::Square));
        assert_eq!(parse_list_style_type("decimal"), Some(ListStyleType::Decimal));
        assert_eq!(parse_list_style_type("lower-alpha"), Some(ListStyleType::LowerAlpha));
        assert_eq!(parse_list_style_type("lower-latin"), Some(ListStyleType::LowerAlpha));
        assert_eq!(parse_list_style_type("UPPER-ROMAN"), Some(ListStyleType::UpperRoman));
        assert_eq!(parse_list_style_type("none"), Some(ListStyleType::None));
        assert_eq!(parse_list_style_type("georgian"), None);
    }

#[test]
    fn ua_aplica_decimal_a_ol_y_disc_a_ul() {
        let html = "<html><body><ol><li>x</li></ol><ul><li>y</li></ul></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ol = dom.find("ol").unwrap();
        let ul = dom.find("ul").unwrap();
        assert_eq!(eng.compute(&ol).list_style_type, ListStyleType::Decimal);
        assert_eq!(eng.compute(&ul).list_style_type, ListStyleType::Disc);
    }

    #[test]
    fn list_style_type_hereda_de_padre_a_li() {
        // El `<ol>` recibe `decimal` por UA; el `<li>` no tiene regla
        // propia pero hereda el valor.
        let html = "<html><body><ol><li>x</li></ol></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ol = dom.find("ol").unwrap();
        let ol_style = eng.compute_with_parent(&ol, None);
        let li = dom.find("li").unwrap();
        let li_style = eng.compute_with_parent(&li, Some(&ol_style));
        assert_eq!(li_style.list_style_type, ListStyleType::Decimal);
    }

    #[test]
    fn text_decoration_none_override_padre() {
        let html = "<html><head><style>a { text-decoration: none }</style></head><body><a href='/x'>plain</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let style = eng.compute(&a);
        assert_eq!(style.text_decoration, TextDecorationLine::None);
    }

    #[test]
    fn parsea_rgb_legacy_y_moderno() {
        // Legacy con comas.
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(Color::rgb(255, 0, 0)));
        // Moderno con whitespace.
        assert_eq!(parse_color("rgb(0 128 255)"), Some(Color::rgb(0, 128, 255)));
        // Porcentajes.
        assert_eq!(parse_color("rgb(100%, 0%, 50%)"), Some(Color::rgb(255, 0, 128)));
        // Sobre/sub-rango → clamp.
        assert_eq!(parse_color("rgb(300, -10, 128)"), Some(Color::rgb(255, 0, 128)));
    }

    #[test]
    fn parsea_rgba_y_slash_alpha() {
        // Alpha como 4to arg (legacy).
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some(Color { r: 255, g: 0, b: 0, a: 128 }));
        // Alpha como porcentaje.
        assert_eq!(parse_color("rgba(0, 0, 0, 50%)"), Some(Color { r: 0, g: 0, b: 0, a: 128 }));
        // Sintaxis moderna `R G B / A`.
        assert_eq!(parse_color("rgb(255 0 0 / 0.5)"), Some(Color { r: 255, g: 0, b: 0, a: 128 }));
        // `rgba` también acepta moderno.
        assert_eq!(parse_color("rgba(0 255 0 / 100%)"), Some(Color::rgb(0, 255, 0)));
    }

    #[test]
    fn parsea_hsl_basico() {
        // hsl(0, 100%, 50%) = rojo puro.
        let red = parse_color("hsl(0, 100%, 50%)").unwrap();
        assert_eq!(red, Color::rgb(255, 0, 0));
        // hsl(120, 100%, 50%) = verde puro.
        let green = parse_color("hsl(120, 100%, 50%)").unwrap();
        assert_eq!(green, Color::rgb(0, 255, 0));
        // hsl(240, 100%, 50%) = azul puro.
        let blue = parse_color("hsl(240, 100%, 50%)").unwrap();
        assert_eq!(blue, Color::rgb(0, 0, 255));
        // hsl(0, 0%, 50%) = gris medio.
        let gray = parse_color("hsl(0, 0%, 50%)").unwrap();
        assert_eq!(gray, Color::rgb(128, 128, 128));
    }

    #[test]
    fn parsea_hsla_con_alpha() {
        let c = parse_color("hsla(0, 100%, 50%, 0.5)").unwrap();
        assert_eq!(c, Color { r: 255, g: 0, b: 0, a: 128 });
        // Moderno con slash.
        let c2 = parse_color("hsl(120 100% 50% / 0.25)").unwrap();
        assert_eq!(c2, Color { r: 0, g: 255, b: 0, a: 64 });
    }

    #[test]
    fn parsea_hue_unidades_y_none() {
        // 0.5turn = 180deg = cyan; 200grad = 180deg; π rad = 180deg.
        let cyan = Color::rgb(0, 255, 255);
        assert_eq!(parse_color("hsl(0.5turn 100% 50%)").unwrap(), cyan);
        assert_eq!(parse_color("hsl(200grad 100% 50%)").unwrap(), cyan);
        assert_eq!(parse_color("hsl(3.14159265rad 100% 50%)").unwrap(), cyan);
        // `none` en hue ⇒ 0deg = rojo.
        assert_eq!(parse_color("hwb(none 0% 0%)").unwrap(), Color::rgb(255, 0, 0));
    }

    #[test]
    fn parsea_hwb() {
        // hwb sin blancura ni negrura = hue puro.
        assert_eq!(parse_color("hwb(0 0% 0%)").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(parse_color("hwb(120 0% 0%)").unwrap(), Color::rgb(0, 255, 0));
        // 50% blancura clarea el rojo.
        assert_eq!(parse_color("hwb(0 50% 0%)").unwrap(), Color::rgb(255, 128, 128));
        // 50% negrura lo oscurece.
        assert_eq!(parse_color("hwb(0 0% 50%)").unwrap(), Color::rgb(128, 0, 0));
        // W+B ≥ 100% ⇒ gris W/(W+B).
        assert_eq!(parse_color("hwb(0 100% 100%)").unwrap(), Color::rgb(128, 128, 128));
        // Alpha por slash.
        assert_eq!(parse_color("hwb(0 0% 0% / 0.5)").unwrap(), Color { r: 255, g: 0, b: 0, a: 128 });
    }

    #[test]
    fn parsea_oklab_y_oklch() {
        // Blanco y negro son deterministas.
        assert_eq!(parse_color("oklab(1 0 0)").unwrap(), Color::rgb(255, 255, 255));
        assert_eq!(parse_color("oklab(0 0 0)").unwrap(), Color::rgb(0, 0, 0));
        assert_eq!(parse_color("oklch(1 0 0)").unwrap(), Color::rgb(255, 255, 255));
        // Alpha + `none` en lightness.
        assert_eq!(parse_color("oklch(none 0 0 / 0.5)").unwrap(), Color { r: 0, g: 0, b: 0, a: 128 });
        // Rojo sRGB ≈ oklch(0.628 0.2577 29.23) — tolerancia.
        let red = parse_color("oklch(0.628 0.2577 29.23)").unwrap();
        assert!(red.r > 245 && red.g < 25 && red.b < 25, "oklch rojo: {red:?}");
        // Porcentajes: L 100% = 1.0.
        assert_eq!(parse_color("oklch(100% 0 0)").unwrap(), Color::rgb(255, 255, 255));
    }

    #[test]
    fn parsea_lab_y_lch() {
        // Blanco D50 y negro.
        let white = parse_color("lab(100 0 0)").unwrap();
        assert!(white.r >= 253 && white.g >= 253 && white.b >= 253, "lab blanco: {white:?}");
        assert_eq!(parse_color("lab(0 0 0)").unwrap(), Color::rgb(0, 0, 0));
        let white_lch = parse_color("lch(100 0 0)").unwrap();
        assert!(white_lch.r >= 253 && white_lch.g >= 253 && white_lch.b >= 253);
        // Rojo sRGB ≈ lab(54.29 80.81 69.89) — tolerancia.
        let red = parse_color("lab(54.29 80.81 69.89)").unwrap();
        assert!(red.r > 245 && red.g < 25 && red.b < 25, "lab rojo: {red:?}");
    }

    #[test]
    fn parsea_color_func() {
        // srgb directo.
        assert_eq!(parse_color("color(srgb 1 0 0)").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(parse_color("color(srgb 0 1 0)").unwrap(), Color::rgb(0, 255, 0));
        // srgb-linear pasa por la gamma sRGB al codificar.
        assert_eq!(parse_color("color(srgb-linear 1 1 1)").unwrap(), Color::rgb(255, 255, 255));
        let mid = parse_color("color(srgb-linear 0.5 0.5 0.5)").unwrap();
        assert!((mid.r as i32 - 188).abs() <= 1, "srgb-linear 0.5: {mid:?}");
        // display-p3: blanco = blanco; verde P3 puro recorta al gamut sRGB.
        assert_eq!(parse_color("color(display-p3 1 1 1)").unwrap(), Color::rgb(255, 255, 255));
        assert_eq!(parse_color("color(display-p3 0 1 0)").unwrap(), Color::rgb(0, 255, 0));
        // Alpha.
        assert_eq!(parse_color("color(srgb 1 0 0 / 0.5)").unwrap(), Color { r: 255, g: 0, b: 0, a: 128 });
        // Espacio no soportado ⇒ None (degrada, no rompe el parseo).
        assert!(parse_color("color(rec2020 1 0 0)").is_none());
    }

    #[test]
    fn parsea_color_mix() {
        // 50/50 en sRGB.
        assert_eq!(parse_color("color-mix(in srgb, red, blue)").unwrap(), Color::rgb(128, 0, 128));
        assert_eq!(parse_color("color-mix(in srgb, white, black)").unwrap(), Color::rgb(128, 128, 128));
        // Porcentaje en el primer color.
        assert_eq!(parse_color("color-mix(in srgb, red 25%, blue)").unwrap(), Color::rgb(64, 0, 191));
        // Porcentaje en el segundo color (equivalente).
        assert_eq!(parse_color("color-mix(in srgb, red, blue 75%)").unwrap(), Color::rgb(64, 0, 191));
        // Ambos porcentajes se normalizan (20+20 → 50/50).
        assert_eq!(parse_color("color-mix(in srgb, red 20%, blue 20%)").unwrap(), Color::rgb(128, 0, 128));
        // Alpha se interpola.
        let alpha = parse_color("color-mix(in srgb, #ff000000, #ff0000ff)").unwrap();
        assert_eq!(alpha, Color { r: 255, g: 0, b: 0, a: 128 });
        // Espacio no soportado degrada a sRGB (no rompe el parseo).
        assert_eq!(parse_color("color-mix(in jzazbz, red, blue)").unwrap(), Color::rgb(128, 0, 128));
    }

    #[test]
    fn parsea_color_mix_perceptual() {
        // En oklab/oklch el mix de rojo y azul da un púrpura perceptual
        // (ambos canales presentes, verde bajo). Tolerancia.
        let ok = parse_color("color-mix(in oklab, red, blue)").unwrap();
        assert!(ok.r > 40 && ok.b > 40 && ok.g < 90, "oklab mix: {ok:?}");
        // oklch parsea y produce un color válido distinto del negro.
        let oklch = parse_color("color-mix(in oklch, red, blue)").unwrap();
        assert!(oklch.r as u32 + oklch.g as u32 + oklch.b as u32 > 0, "oklch mix: {oklch:?}");
        // Mezclar un color consigo mismo lo deja igual (sanity).
        assert_eq!(parse_color("color-mix(in oklab, red, red)").unwrap().r, 255);
    }

    #[test]
    fn parsea_hex_8_y_4_chars() {
        // #RRGGBBAA.
        assert_eq!(parse_color("#ff000080"), Some(Color { r: 255, g: 0, b: 0, a: 128 }));
        // #RGBA expande cada nibble * 17.
        assert_eq!(parse_color("#f00f"), Some(Color { r: 255, g: 0, b: 0, a: 255 }));
        assert_eq!(parse_color("#0008"), Some(Color { r: 0, g: 0, b: 0, a: 136 }));
    }

    #[test]
    fn named_colors_extendidos() {
        assert_eq!(parse_color("orange"), Some(Color::rgb(255, 165, 0)));
        assert_eq!(parse_color("navy"), Some(Color::rgb(0, 0, 128)));
        assert_eq!(parse_color("teal"), Some(Color::rgb(0, 128, 128)));
        assert_eq!(parse_color("CRIMSON"), Some(Color::rgb(220, 20, 60))); // case-insensitive
        assert_eq!(parse_color("lightblue"), Some(Color::rgb(173, 216, 230)));
        // Alias.
        assert_eq!(parse_color("grey"), parse_color("gray"));
        assert_eq!(parse_color("cyan"), parse_color("aqua"));
        assert_eq!(parse_color("magenta"), parse_color("fuchsia"));
    }

    #[test]
    fn parsea_sides_shorthand_1_2_3_4() {
        assert_eq!(parse_sides("10px"), Some(Sides::all(10.0)));
        assert_eq!(
            parse_sides("10px 20px"),
            Some(Sides { top: 10.0, right: 20.0, bottom: 10.0, left: 20.0 }),
        );
        assert_eq!(
            parse_sides("10px 20px 30px"),
            Some(Sides { top: 10.0, right: 20.0, bottom: 30.0, left: 20.0 }),
        );
        assert_eq!(
            parse_sides("10px 20px 30px 40px"),
            Some(Sides { top: 10.0, right: 20.0, bottom: 30.0, left: 40.0 }),
        );
        // 5 valores → inválido.
        assert_eq!(parse_sides("1px 2px 3px 4px 5px"), None);
        // Token no-longitud → inválido.
        assert_eq!(parse_sides("10px bad 20px"), None);
    }

    #[test]
    fn margin_shorthand_aplica_4_lados() {
        let html = r#"<html><head><style>
            div { margin: 5px 10px 15px 20px }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.margin.top, 5.0);
        assert_eq!(s.margin.right, 10.0);
        assert_eq!(s.margin.bottom, 15.0);
        assert_eq!(s.margin.left, 20.0);
    }

    #[test]
    fn padding_shorthand_2_valores_eje_vertical_horizontal() {
        let html = r#"<html><head><style>
            div { padding: 8px 16px }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.padding.top, 8.0);
        assert_eq!(s.padding.bottom, 8.0);
        assert_eq!(s.padding.left, 16.0);
        assert_eq!(s.padding.right, 16.0);
    }

    #[test]
    fn margin_individual_pisa_shorthand_por_cascada() {
        // El shorthand setea todo a 10px, después `margin-top: 50px` lo pisa.
        let html = r#"<html><head><style>
            div { margin: 10px; margin-top: 50px }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.margin.top, 50.0);
        assert_eq!(s.margin.right, 10.0);
        assert_eq!(s.margin.bottom, 10.0);
        assert_eq!(s.margin.left, 10.0);
    }

    #[test]
    fn parsea_display_flex_y_inline_flex() {
        assert_eq!(parse_display("flex"), Some(Display::Flex));
        assert_eq!(parse_display("inline-flex"), Some(Display::InlineFlex));
        assert_eq!(parse_display("FLEX"), Some(Display::Flex));
    }

    #[test]
    fn parsea_flex_direction() {
        assert_eq!(parse_flex_direction("row"), Some(FlexDirection::Row));
        assert_eq!(parse_flex_direction("column"), Some(FlexDirection::Column));
        assert_eq!(parse_flex_direction("row-reverse"), Some(FlexDirection::RowReverse));
        assert_eq!(parse_flex_direction("column-reverse"), Some(FlexDirection::ColumnReverse));
        assert_eq!(parse_flex_direction("diagonal"), None);
    }

    #[test]
    fn parsea_justify_y_align() {
        // Aceptamos los alias `flex-start`/`flex-end` ↔ `start`/`end`.
        assert_eq!(parse_justify_content("flex-start"), Some(JustifyContent::Start));
        assert_eq!(parse_justify_content("space-between"), Some(JustifyContent::SpaceBetween));
        assert_eq!(parse_justify_content("space-around"), Some(JustifyContent::SpaceAround));
        assert_eq!(parse_align_items("flex-end"), Some(AlignItems::End));
        assert_eq!(parse_align_items("stretch"), Some(AlignItems::Stretch));
        assert_eq!(parse_align_items("baseline"), Some(AlignItems::Baseline));
    }

    #[test]
    fn parsea_flex_wrap() {
        assert_eq!(parse_flex_wrap("nowrap"), Some(FlexWrap::NoWrap));
        assert_eq!(parse_flex_wrap("wrap"), Some(FlexWrap::Wrap));
        assert_eq!(parse_flex_wrap("wrap-reverse"), Some(FlexWrap::WrapReverse));
    }

    #[test]
    fn parsea_gap_1_y_2_valores() {
        assert_eq!(parse_gap("12px"), Some((12.0, 12.0)));
        assert_eq!(parse_gap("4px 8px"), Some((4.0, 8.0)));
        assert_eq!(parse_gap("0"), Some((0.0, 0.0)));
        assert_eq!(parse_gap("a b c"), None);
    }

    #[test]
    fn computa_flex_container_completo() {
        let html = r#"<html><head><style>
            .row {
                display: flex;
                flex-direction: row;
                justify-content: space-between;
                align-items: center;
                gap: 16px 24px;
                flex-wrap: wrap;
            }
        </style></head><body><div class="row"><span>a</span><span>b</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.display, Display::Flex);
        assert_eq!(s.flex_direction, FlexDirection::Row);
        assert_eq!(s.justify_content, JustifyContent::SpaceBetween);
        assert_eq!(s.align_items, AlignItems::Center);
        assert_eq!(s.flex_wrap, FlexWrap::Wrap);
        assert_eq!(s.gap_row, 16.0);
        assert_eq!(s.gap_column, 24.0);
    }

    #[test]
    fn parsea_align_content_valores_y_alias() {
        assert_eq!(parse_align_content("space-between"), Some(AlignContent::SpaceBetween));
        assert_eq!(parse_align_content("flex-start"), Some(AlignContent::Start));
        assert_eq!(parse_align_content("flex-end"), Some(AlignContent::End));
        assert_eq!(parse_align_content("center"), Some(AlignContent::Center));
        assert_eq!(parse_align_content("stretch"), Some(AlignContent::Stretch));
        // `normal` y `baseline` colapsan al default.
        assert_eq!(parse_align_content("normal"), Some(AlignContent::Normal));
        assert_eq!(parse_align_content("baseline"), Some(AlignContent::Normal));
        assert_eq!(parse_align_content("garbage"), None);
    }

    #[test]
    fn align_content_computa_en_flex_y_default_normal() {
        let html = r#"<html><head><style>
            .multi { display: flex; flex-wrap: wrap; align-content: space-around; }
        </style></head><body>
            <div class="multi"><span>a</span></div>
            <section style="display:flex"><span>b</span></section>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let multi = dom.find("div").unwrap();
        assert_eq!(eng.compute(&multi).align_content, AlignContent::SpaceAround);
        // Sin declaración, el default es Normal (no hereda del flujo).
        let plain = dom.find("section").unwrap();
        assert_eq!(eng.compute(&plain).align_content, AlignContent::Normal);
    }

    #[test]
    fn place_shorthands_expanden_ambos_ejes() {
        let html = r#"<html><head><style>
            .a { display: grid; place-content: center space-between; }
            .b { display: grid; place-items: stretch; }
            .c { place-self: end center; }
        </style></head><body>
            <div class="a"></div><div class="b"></div>
            <span class="c">x</span>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        // place-content: align-content + justify-content.
        let pc = parse_declarations("place-content: center space-between", &HashMap::new());
        assert!(pc.iter().any(|d| matches!(d.kind, DeclKind::AlignContent(AlignContent::Center))));
        assert!(pc
            .iter()
            .any(|d| matches!(d.kind, DeclKind::JustifyContent(JustifyContent::SpaceBetween))));
        // place-items con un solo valor → align-items + justify-items iguales.
        let pi = parse_declarations("place-items: stretch", &HashMap::new());
        assert!(pi.iter().any(|d| matches!(d.kind, DeclKind::AlignItems(AlignItems::Stretch))));
        assert!(pi.iter().any(|d| matches!(d.kind, DeclKind::JustifyItems(AlignItems::Stretch))));
        // place-self: align-self + justify-self.
        let ps = parse_declarations("place-self: end center", &HashMap::new());
        assert!(ps.iter().any(|d| matches!(d.kind, DeclKind::AlignSelf(AlignSelf::End))));
        assert!(ps.iter().any(|d| matches!(d.kind, DeclKind::JustifySelf(AlignSelf::Center))));
        // Y que computa end-to-end sobre el árbol.
        let a = eng.compute(&dom.find("div").unwrap());
        assert_eq!(a.align_content, AlignContent::Center);
        assert_eq!(a.justify_content, JustifyContent::SpaceBetween);
        let c = eng.compute(&dom.find("span").unwrap());
        assert_eq!(c.align_self, AlignSelf::End);
        assert_eq!(c.justify_self, AlignSelf::Center);
    }

    #[test]
    fn justify_items_y_self_grid_parse_y_computa() {
        // Parsers (incluye alias left/right y descarte de `normal`).
        assert_eq!(parse_justify_items("center"), Some(AlignItems::Center));
        assert_eq!(parse_justify_items("left"), Some(AlignItems::Start));
        assert_eq!(parse_justify_items("right"), Some(AlignItems::End));
        assert_eq!(parse_justify_items("stretch"), Some(AlignItems::Stretch));
        assert_eq!(parse_justify_items("normal"), None);
        assert_eq!(parse_justify_self("auto"), Some(AlignSelf::Auto));
        assert_eq!(parse_justify_self("right"), Some(AlignSelf::End));
        assert_eq!(parse_justify_self("flex-start"), Some(AlignSelf::Start));

        let html = r#"<html><head><style>
            .g { display: grid; justify-items: center; }
            .cell { justify-self: end; }
        </style></head><body>
            <div class="g"><span class="cell">x</span></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let g = eng.compute(&dom.find("div").unwrap());
        assert_eq!(g.justify_items, Some(AlignItems::Center));
        let cell = eng.compute(&dom.find("span").unwrap());
        assert_eq!(cell.justify_self, AlignSelf::End);
        // Default sin declaración.
        assert_eq!(g.justify_self, AlignSelf::Auto);
    }

    #[test]
    fn aspect_ratio_propiedad_ratio_numero_y_auto() {
        let html = r#"<html><head><style>
            .wide { aspect-ratio: 16 / 9; }
            .num  { aspect-ratio: 1.5; }
            .both { aspect-ratio: auto 4/3; }
            .reset{ aspect-ratio: auto; }
        </style></head><body>
            <div class="wide"></div><div class="num"></div>
            <div class="both"></div><div class="reset"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        // Verificamos el parse vía decl_kind_from_pair (más preciso que
        // depender del orden de los div en el árbol).
        let r = |css: &str| match decl_kind_from_pair("aspect-ratio", css) {
            Some(DeclKind::AspectRatio(v)) => v,
            other => panic!("inesperado: {other:?}"),
        };
        assert!((r("16 / 9").unwrap() - 16.0 / 9.0).abs() < 1e-6);
        assert!((r("1.5").unwrap() - 1.5).abs() < 1e-6);
        assert!((r("auto 4/3").unwrap() - 4.0 / 3.0).abs() < 1e-6);
        assert_eq!(r("auto"), None);
        assert!(decl_kind_from_pair("aspect-ratio", "garbage").is_none());
        // Y que computa en el árbol (default None sin declaración).
        let plain = eng.compute(&dom.find("body").unwrap());
        assert_eq!(plain.aspect_ratio, None);
    }

    #[test]
    fn row_gap_y_column_gap_individuales_pisan_shorthand() {
        let html = r#"<html><head><style>
            div {
                display: flex;
                gap: 10px;
                row-gap: 30px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        // row-gap pisa la mitad del shorthand; column-gap del shorthand sigue (10).
        assert_eq!(s.gap_row, 30.0);
        assert_eq!(s.gap_column, 10.0);
    }

    #[test]
    fn css_var_basico_sobre_root() {
        let html = r#"<html><head><style>
            :root { --primary: #ff0000 }
            p { color: var(--primary) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn css_var_con_fallback() {
        // `--missing` no existe → usa el fallback `blue`.
        let html = r#"<html><head><style>
            p { color: var(--missing, blue) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn css_var_se_declara_en_html_y_asterisco() {
        // Variables declaradas en `html` y `*` también valen (no solo `:root`).
        let html = r#"<html><head><style>
            html { --a: #aa0000 }
            * { --b: 5px }
            p { color: var(--a); margin: var(--b) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.color, Color::rgb(0xaa, 0, 0));
        assert_eq!(s.margin.top, 5.0);
    }

    #[test]
    fn css_var_recursiva() {
        // `--secondary` se define como `var(--primary)` — la sustitución
        // debe resolver hasta el valor base.
        let html = r#"<html><head><style>
            :root {
                --primary: rgb(0, 200, 100);
                --secondary: var(--primary);
            }
            p { color: var(--secondary) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 200, 100));
    }

    #[test]
    fn css_var_en_inline_style() {
        // `style="..."` también debe resolver var().
        let html = r#"<html><head><style>
            :root { --hi: hsl(120, 100%, 50%) }
        </style></head><body>
          <p style="background: var(--hi)">x</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).background, Some(Color::rgb(0, 255, 0)));
    }

    #[test]
    fn css_var_inexistente_sin_fallback_borra_declaracion() {
        // `var(--nope)` sin fallback resuelve a "" — el parser de color
        // rechaza el value y la decl se ignora silenciosamente.
        // El color debe quedar en el default BLACK heredado.
        let html = r#"<html><head><style>
            p { color: var(--nope) }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::BLACK);
    }

    #[test]
    fn css_var_multiple_en_un_value() {
        // Shorthand `border: var(--w) solid var(--c)`.
        let html = r#"<html><head><style>
            :root { --w: 3px; --c: orange }
            div { border: var(--w) solid var(--c) }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!((s.border_widths.top - 3.0).abs() < 1e-6);
        assert_eq!(s.border_colors.top, Some(Color::rgb(255, 165, 0)));
    }

    #[test]
    fn parsea_box_sizing() {
        assert_eq!(parse_box_sizing("content-box"), Some(BoxSizing::ContentBox));
        assert_eq!(parse_box_sizing("border-box"), Some(BoxSizing::BorderBox));
        assert_eq!(parse_box_sizing("WeIrD"), None);
    }

    #[test]
    fn computa_min_max_sizes() {
        let html = r#"<html><head><style>
            div {
                min-width: 100px;
                min-height: 50px;
                max-height: 200px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!(matches!(s.min_width, LengthVal::Px(100.0)));
        assert!(matches!(s.min_height, LengthVal::Px(50.0)));
        assert!(matches!(s.max_height, LengthVal::Px(200.0)));
    }

    #[test]
    fn parsea_overflow_alias() {
        assert_eq!(parse_overflow("visible"), Some(Overflow::Visible));
        assert_eq!(parse_overflow("hidden"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("auto"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("scroll"), Some(Overflow::Hidden));
        assert_eq!(parse_overflow("clip"), Some(Overflow::Hidden));
    }

    #[test]
    fn parsea_white_space_y_se_hereda() {
        let html = r#"<html><head><style>
            pre { white-space: pre }
        </style></head><body><pre>line1
line2</pre></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let pre = dom.find("pre").unwrap();
        let s = eng.compute(&pre);
        assert_eq!(s.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn parsea_text_transform_y_se_hereda() {
        let html = r#"<html><head><style>
            p { text-transform: uppercase }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn parsea_opacity_clampa() {
        assert_eq!(parse_opacity("0.5"), Some(0.5));
        assert_eq!(parse_opacity("100%"), Some(1.0));
        assert_eq!(parse_opacity("0"), Some(0.0));
        assert_eq!(parse_opacity("2"), Some(1.0)); // clamp arriba
        assert_eq!(parse_opacity("-0.5"), Some(0.0)); // clamp abajo
    }

    #[test]
    fn parsea_align_self() {
        assert_eq!(parse_align_self("auto"), Some(AlignSelf::Auto));
        assert_eq!(parse_align_self("flex-end"), Some(AlignSelf::End));
        assert_eq!(parse_align_self("stretch"), Some(AlignSelf::Stretch));
    }

    #[test]
    fn parsea_flex_shorthand_presets() {
        let decls = parse_flex_shorthand("none", false);
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 0.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 0.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Auto)));

        let decls = parse_flex_shorthand("auto", false);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 1.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 1.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Auto)));

        let decls = parse_flex_shorthand("1", false);
        // `flex: 1` ⇒ `1 1 0%`
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 1.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 1.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Pct(0.0))));
    }

    #[test]
    fn parsea_flex_shorthand_3_valores() {
        let decls = parse_flex_shorthand("2 0 200px", false);
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].kind, DeclKind::FlexGrow(g) if g == 2.0));
        assert!(matches!(decls[1].kind, DeclKind::FlexShrink(s) if s == 0.0));
        assert!(matches!(decls[2].kind, DeclKind::FlexBasis(LengthVal::Px(200.0))));
    }

    #[test]
    fn parsea_outline_shorthand() {
        let decls = parse_outline_shorthand("2px solid orange", false);
        let mut has_w = false; let mut has_s = false; let mut has_c = false;
        for d in &decls {
            match &d.kind {
                DeclKind::OutlineWidth(w) => { has_w = (*w - 2.0).abs() < 1e-6; }
                DeclKind::OutlineStyle(active) => { has_s = *active; }
                DeclKind::OutlineColor(c) => { has_c = *c == Color::rgb(255, 165, 0); }
                _ => {}
            }
        }
        assert!(has_w && has_s && has_c);

        let decls = parse_outline_shorthand("none", false);
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0].kind, DeclKind::OutlineStyle(false)));
    }

    #[test]
    fn parsea_linear_gradient_basico() {
        let g = parse_linear_gradient("to right, #f00, #00f").unwrap();
        assert!((g.angle_deg() - 90.0).abs() < 1e-6);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[0].color, Color::rgb(255, 0, 0));
        assert_eq!(g.stops[1].color, Color::rgb(0, 0, 255));

        let g = parse_linear_gradient("45deg, red 0%, blue 100%").unwrap();
        assert!((g.angle_deg() - 45.0).abs() < 1e-6);
        assert_eq!(g.stops[0].pos, Some(LengthVal::Pct(0.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Pct(100.0)));

        // Default 180 (top→bottom) cuando no se da dirección.
        let g = parse_linear_gradient("red, blue").unwrap();
        assert!((g.angle_deg() - 180.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_background_image_gradient_y_none() {
        // `background-image: linear-gradient(...)` produce un Gradient.
        let html = r#"<html><head><style>
            div { background-image: linear-gradient(to right, red, blue) }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert!(s.background_gradient.is_some());

        // `background-image: none` deshabilita.
        let html2 = r#"<html><head><style>
            div { background-image: linear-gradient(red, blue); background-image: none }
        </style></head><body><div></div></body></html>"#;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let d2 = dom2.find("div").unwrap();
        assert!(eng2.compute(&d2).background_gradient.is_none());
    }

    #[test]
    fn parsea_background_size_position_repeat() {
        // Fase 7.204 — keywords y valores de las tres props de background.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // background-size
        assert_eq!(compute("background-size: cover").background_size, BackgroundSize::Cover);
        assert_eq!(compute("background-size: contain").background_size, BackgroundSize::Contain);
        assert_eq!(
            compute("background-size: 50% auto").background_size,
            BackgroundSize::Explicit { x: LengthVal::Pct(50.0), y: LengthVal::Auto }
        );
        assert_eq!(
            compute("background-size: 100px 40px").background_size,
            BackgroundSize::Explicit { x: LengthVal::Px(100.0), y: LengthVal::Px(40.0) }
        );

        // background-repeat (incluye sintaxis de dos valores)
        assert_eq!(
            compute("background-repeat: no-repeat").background_repeat,
            BackgroundRepeat::NoRepeat
        );
        assert_eq!(
            compute("background-repeat: repeat-x").background_repeat,
            BackgroundRepeat::RepeatX
        );
        assert_eq!(
            compute("background-repeat: repeat no-repeat").background_repeat,
            BackgroundRepeat::RepeatX
        );
        assert_eq!(
            compute("background-repeat: no-repeat repeat").background_repeat,
            BackgroundRepeat::RepeatY
        );

        // background-position: keyword posicional, orden invertido y %.
        let p = compute("background-position: right bottom").background_position;
        assert_eq!((p.x, p.y), (LengthVal::Pct(100.0), LengthVal::Pct(100.0)));
        let p = compute("background-position: top left").background_position; // invertido
        assert_eq!((p.x, p.y), (LengthVal::Pct(0.0), LengthVal::Pct(0.0)));
        let p = compute("background-position: 10px 20px").background_position;
        assert_eq!((p.x, p.y), (LengthVal::Px(10.0), LengthVal::Px(20.0)));
        let p = compute("background-position: center").background_position; // un solo valor
        assert_eq!((p.x, p.y), (LengthVal::Pct(50.0), LengthVal::Pct(50.0)));
    }

    #[test]
    fn shorthand_background_expande_color_imagen_posicion_size_repeat() {
        // Fase 7.205 — el shorthand `background:` reparte sus piezas en los
        // longhands. Reusa los value-parsers de cada sub-propiedad.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // Color suelto.
        let s = compute("background: #ff0000");
        assert_eq!(s.background, Some(Color::rgb(255, 0, 0)));

        // Imagen + repeat + position / size (con `/` pegado o suelto).
        let s = compute("background: url(bg.png) no-repeat center / cover");
        assert_eq!(s.background_image_url.as_deref(), Some("bg.png"));
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(
            (s.background_position.x, s.background_position.y),
            (LengthVal::Pct(50.0), LengthVal::Pct(50.0))
        );
        assert_eq!(s.background_size, BackgroundSize::Cover);

        // `/` pegado a los tokens (`center/contain`) y orden invertido de
        // keywords de position, color al final.
        let s = compute("background: url(p.png) repeat-x top left, url(otra.png)");
        assert_eq!(s.background_image_url.as_deref(), Some("p.png")); // sólo la 1ª capa
        assert_eq!(s.background_repeat, BackgroundRepeat::RepeatX);
        assert_eq!(
            (s.background_position.x, s.background_position.y),
            (LengthVal::Pct(0.0), LengthVal::Pct(0.0)) // top left → x=0%, y=0%
        );

        // attachment/box se aceptan y descartan; el color sigue tomándose.
        let s = compute("background: green url(g.png) fixed border-box no-repeat 10px 20px / 50px");
        assert_eq!(s.background, Some(Color::rgb(0, 128, 0)));
        assert_eq!(s.background_image_url.as_deref(), Some("g.png"));
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(
            (s.background_position.x, s.background_position.y),
            (LengthVal::Px(10.0), LengthVal::Px(20.0))
        );
        assert_eq!(
            s.background_size,
            BackgroundSize::Explicit { x: LengthVal::Px(50.0), y: LengthVal::Auto }
        );
    }

    #[test]
    fn background_props_default_y_se_propagan_al_box() {
        // Defaults CSS: auto / 0% 0% / repeat. Y un override viaja al BoxNode.
        let eng = crate::Engine::new();
        let html = r#"<html><body>
            <div id="plain" style="background-image: url(x.png)"></div>
            <div id="cov" style="background-image: url(x.png); background-size: cover;
                 background-position: 50% 50%; background-repeat: no-repeat"></div>
        </body></html>"#;
        let doc = eng.load_html("about:test", html);
        let mut plain = None;
        let mut cov = None;
        doc.box_tree.walk(|b| match b.element_id.as_deref() {
            Some("plain") => plain = Some((b.background_size, b.background_repeat, b.background_position)),
            Some("cov") => cov = Some((b.background_size, b.background_repeat, b.background_position)),
            _ => {}
        });
        let (psize, prep, ppos) = plain.expect("plain box");
        assert_eq!(psize, BackgroundSize::Auto);
        assert_eq!(prep, BackgroundRepeat::Repeat);
        assert_eq!((ppos.x, ppos.y), (LengthVal::Pct(0.0), LengthVal::Pct(0.0)));
        let (csize, crep, cpos) = cov.expect("cov box");
        assert_eq!(csize, BackgroundSize::Cover);
        assert_eq!(crep, BackgroundRepeat::NoRepeat);
        assert_eq!((cpos.x, cpos.y), (LengthVal::Pct(50.0), LengthVal::Pct(50.0)));
    }

    #[test]
    fn background_origin_clip_longhand_shorthand_y_box() {
        // Fase 7.207 — `background-origin` / `background-clip`.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // Defaults CSS: origin = padding-box, clip = border-box.
        let s = compute("color: red");
        assert_eq!(s.background_origin, BackgroundOrigin::PaddingBox);
        assert_eq!(s.background_clip, BackgroundClip::BorderBox);

        // Longhands.
        let s = compute("background-origin: content-box; background-clip: padding-box");
        assert_eq!(s.background_origin, BackgroundOrigin::ContentBox);
        assert_eq!(s.background_clip, BackgroundClip::PaddingBox);

        // `text` ahora es un valor real (Fase 7.208).
        let s = compute("background-clip: text");
        assert_eq!(s.background_clip, BackgroundClip::Text);

        // Shorthand con UNA caja → fija origin Y clip.
        let s = compute("background: url(b.png) content-box");
        assert_eq!(s.background_origin, BackgroundOrigin::ContentBox);
        assert_eq!(s.background_clip, BackgroundClip::ContentBox);

        // Shorthand con DOS cajas → 1ª = origin, 2ª = clip.
        let s = compute("background: url(b.png) padding-box content-box");
        assert_eq!(s.background_origin, BackgroundOrigin::PaddingBox);
        assert_eq!(s.background_clip, BackgroundClip::ContentBox);

        // Propagación al BoxNode (vía build).
        let eng = crate::Engine::new();
        let doc = eng.load_html(
            "about:test",
            r#"<html><body><div id="d" style="background-image: url(x.png);
               background-origin: content-box; background-clip: padding-box"></div></body></html>"#,
        );
        let mut got = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                got = Some((b.background_origin, b.background_clip));
            }
        });
        let (o, c) = got.expect("box d");
        assert_eq!(o, BackgroundOrigin::ContentBox);
        assert_eq!(c, BackgroundClip::PaddingBox);
    }

    #[test]
    fn background_clip_text_parsea_y_propaga_a_la_hoja() {
        // Fase 7.208 — `background-clip: text` (+ `-webkit-` prefix) y la
        // propagación del gradiente del elemento estilado a su hoja de texto.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };
        assert_eq!(compute("background-clip: text").background_clip, BackgroundClip::Text);
        assert_eq!(
            compute("-webkit-background-clip: text").background_clip,
            BackgroundClip::Text
        );

        // El gradiente vive en el <h1>; su hoja de texto hija lo hereda junto
        // con el clip:text para rellenar los glifos.
        let eng = crate::Engine::new();
        let doc = eng.load_html(
            "about:test",
            r#"<html><body><h1 style="background-image: linear-gradient(90deg, red, blue);
               -webkit-background-clip: text; color: transparent">Hola</h1></body></html>"#,
        );
        let mut leaf = None;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("Hola") {
                leaf = Some((b.background_clip, b.background_gradient.is_some()));
            }
        });
        let (clip, has_grad) = leaf.expect("hoja de texto Hola");
        assert_eq!(clip, BackgroundClip::Text);
        assert!(has_grad, "la hoja debería heredar el gradiente del <h1>");
    }

    #[test]
    fn background_capas_multiples_shorthand_y_longhand() {
        // Fase 7.206 — la lista `background: a, b` reparte la capa 0 en los
        // campos sueltos y las capas 2..N en background_extra_layers.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // Shorthand: capa 0 (arriba) = url(top) no-repeat center/cover; capa
        // extra = url(bottom) repeat-x con defaults de size/position.
        let s = compute("background: url(top.png) no-repeat center / cover, url(bottom.png) repeat-x");
        assert_eq!(s.background_image_url.as_deref(), Some("top.png"));
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(s.background_size, BackgroundSize::Cover);
        assert_eq!(s.background_extra_layers.len(), 1);
        let ex = &s.background_extra_layers[0];
        assert_eq!(ex.image, BackgroundImage::Url("bottom.png".into()));
        assert_eq!(ex.repeat, BackgroundRepeat::RepeatX);
        assert_eq!(ex.size, BackgroundSize::Auto); // default
        assert_eq!((ex.position.x, ex.position.y), (LengthVal::Pct(0.0), LengthVal::Pct(0.0)));

        // Gradiente arriba de una imagen, y color sólo en la última capa.
        let s = compute("background: linear-gradient(red, blue), url(img.png) green");
        assert!(s.background_gradient.is_some()); // capa 0 = gradiente
        assert_eq!(s.background, Some(Color::rgb(0, 128, 0))); // color de la última capa
        assert_eq!(s.background_extra_layers.len(), 1);
        assert_eq!(s.background_extra_layers[0].image, BackgroundImage::Url("img.png".into()));

        // Una sola capa resetea las extra (la shorthand siempre emite la lista).
        let s = compute("background-image: url(a.png), url(b.png); background: blue");
        assert!(s.background_extra_layers.is_empty());
        assert_eq!(s.background, Some(Color::rgb(0, 0, 255)));

        // Longhand `background-image` con varias capas.
        let s = compute("background-image: url(a.png), url(b.png), url(c.png)");
        assert_eq!(s.background_image_url.as_deref(), Some("a.png"));
        assert_eq!(s.background_extra_layers.len(), 2);
        assert_eq!(s.background_extra_layers[0].image, BackgroundImage::Url("b.png".into()));
        assert_eq!(s.background_extra_layers[1].image, BackgroundImage::Url("c.png".into()));
    }

    #[test]
    fn background_capas_extra_resueltas_viajan_al_box() {
        // La capa extra de gradiente se resuelve y viaja al BoxNode (las url()
        // que no resuelven se descartan; el gradiente siempre pinta).
        let eng = crate::Engine::new();
        let html = r#"<html><body>
            <div id="d" style="background: url(x.png) no-repeat, linear-gradient(red, blue)"></div>
        </body></html>"#;
        let doc = eng.load_html("about:test", html);
        let mut layers = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                layers = Some(b.background_extra_layers.len());
                // El gradiente de la capa extra está presente.
                assert!(b.background_extra_layers.iter().any(|l| l.gradient.is_some()));
            }
        });
        assert_eq!(layers, Some(1));
    }

    #[test]
    fn parsea_padding_individual_4_lados() {
        let html = r#"<html><head><style>
            div {
                padding-top: 1px;
                padding-right: 2px;
                padding-bottom: 3px;
                padding-left: 4px;
            }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.padding.top, 1.0);
        assert_eq!(s.padding.right, 2.0);
        assert_eq!(s.padding.bottom, 3.0);
        assert_eq!(s.padding.left, 4.0);
    }

    #[test]
    fn parsea_position_y_insets() {
        let html = r#"<html><head><style>
            div { position: absolute; top: 10px; left: 50%; bottom: auto; right: 20px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.position, Position::Absolute);
        assert!(matches!(s.inset_top, LengthVal::Px(10.0)));
        assert!(matches!(s.inset_left, LengthVal::Pct(50.0)));
        assert!(matches!(s.inset_bottom, LengthVal::Auto));
        assert!(matches!(s.inset_right, LengthVal::Px(20.0)));

        let dom2 = DomTree::parse(r#"<html><body><nav style="position:sticky"></nav></body></html>"#);
        let eng2 = StyleEngine::from_dom(&dom2);
        let n = dom2.find("nav").unwrap();
        assert_eq!(eng2.compute(&n).position, Position::Sticky);
    }

    #[test]
    fn parsea_transforms_cadena() {
        let t = parse_transforms("translate(10px, 20px) scale(2) rotate(45deg)").unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], Transform::Translate(10.0, 20.0));
        assert_eq!(t[1], Transform::Scale(2.0, 2.0));
        assert_eq!(t[2], Transform::Rotate(45.0));

        let t = parse_transforms("translateX(5px) scaleY(0.5) rotate(0.5turn)").unwrap();
        assert_eq!(t[0], Transform::Translate(5.0, 0.0));
        assert_eq!(t[1], Transform::Scale(1.0, 0.5));
        assert_eq!(t[2], Transform::Rotate(180.0));

        assert!(parse_transforms("none").unwrap().is_empty());
    }

    #[test]
    fn parsea_transforms_skew_y_matrix() {
        // skew(x), skew(x, y), skewX, skewY (ángulos con unidad).
        let t = parse_transforms("skew(10deg) skew(10deg, 20deg) skewX(0.25turn) skewY(15deg)").unwrap();
        assert_eq!(t[0], Transform::Skew(10.0, 0.0));
        assert_eq!(t[1], Transform::Skew(10.0, 20.0));
        assert_eq!(t[2], Transform::Skew(90.0, 0.0)); // 0.25turn = 90deg
        assert_eq!(t[3], Transform::Skew(0.0, 15.0));
        // matrix(a,b,c,d,e,f) — afín 2D completa.
        let t = parse_transforms("matrix(1, 0, 0, 1, 30, 40)").unwrap();
        assert_eq!(t[0], Transform::Matrix(1.0, 0.0, 0.0, 1.0, 30.0, 40.0));
        // matrix con escala/rotación.
        let t = parse_transforms("matrix(2, 0, 0, 0.5, 0, 0)").unwrap();
        assert_eq!(t[0], Transform::Matrix(2.0, 0.0, 0.0, 0.5, 0.0, 0.0));
        // matrix con aridad incorrecta → None.
        assert!(parse_transforms("matrix(1, 0, 0)").is_none());
    }

    #[test]
    fn parsea_text_shadow_simple_y_multiple() {
        let sh = parse_text_shadows("2px 3px 4px red").unwrap();
        assert_eq!(sh.len(), 1);
        assert_eq!(sh[0].offset_x, 2.0);
        assert_eq!(sh[0].offset_y, 3.0);
        assert_eq!(sh[0].blur_px, 4.0);
        assert_eq!(sh[0].color, Color::rgb(255, 0, 0));

        let sh = parse_text_shadows("1px 1px black, -1px -1px white").unwrap();
        assert_eq!(sh.len(), 2);
        assert_eq!(sh[0].color, Color::BLACK);
        assert_eq!(sh[1].color, Color::WHITE);
        assert_eq!(sh[1].offset_x, -1.0);

        let sh = parse_text_shadows("none").unwrap();
        assert!(sh.is_empty());
    }

    #[test]
    fn parsea_vertical_align() {
        assert_eq!(parse_vertical_align("baseline"), Some(VerticalAlign::Baseline));
        assert_eq!(parse_vertical_align("middle"), Some(VerticalAlign::Middle));
        assert_eq!(parse_vertical_align("text-top"), Some(VerticalAlign::Top));
        assert_eq!(parse_vertical_align("super"), Some(VerticalAlign::Super));
    }

    #[test]
    fn parsea_visibility_y_pointer_events_heredan() {
        let html = r#"<html><head><style>
            .h { visibility: hidden; pointer-events: none }
        </style></head><body>
          <div class="h"><p>oculto</p></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let p = dom.find("p").unwrap();
        let d_style = eng.compute_with_parent(&d, None);
        let p_style = eng.compute_with_parent(&p, Some(&d_style));
        assert_eq!(p_style.visibility, Visibility::Hidden);
        assert_eq!(p_style.pointer_events, PointerEvents::None);
    }

    #[test]
    fn parsea_text_indent_y_word_spacing_heredan() {
        let html = r#"<html><head><style>
            p { text-indent: 30px; word-spacing: 5px }
        </style></head><body><p>x <span>y</span></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let span = dom.find("span").unwrap();
        let p_style = eng.compute(&p);
        let span_style = eng.compute_with_parent(&span, Some(&p_style));
        assert_eq!(p_style.text_indent, 30.0);
        assert_eq!(p_style.word_spacing, 5.0);
        assert_eq!(span_style.word_spacing, 5.0);
        assert_eq!(span_style.text_indent, 30.0);
    }

    #[test]
    fn parsea_letter_spacing_hereda_y_normal_es_cero() {
        let html = r#"<html><head><style>
            p { letter-spacing: 2px }
            .tight { letter-spacing: normal }
        </style></head><body>
            <p>x <span>y</span></p>
            <div class="tight">z</div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p_style = eng.compute(&dom.find("p").unwrap());
        let span_style = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&p_style));
        assert_eq!(p_style.letter_spacing, 2.0);
        // Hereda al inline hijo.
        assert_eq!(span_style.letter_spacing, 2.0);
        // `normal` ⇒ 0px.
        let tight = eng.compute(&dom.find("div").unwrap());
        assert_eq!(tight.letter_spacing, 0.0);
    }

    #[test]
    fn parsea_display_grid_y_template() {
        let html = r#"<html><head><style>
            .grid {
                display: grid;
                grid-template-columns: 100px 1fr 2fr;
                grid-template-rows: repeat(3, auto);
                grid-gap: 8px 16px;
            }
        </style></head><body><div class="grid"></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.display, Display::Grid);
        assert_eq!(s.grid_template_columns.len(), 3);
        assert!(matches!(s.grid_template_columns[0], GridTrackSize::Px(100.0)));
        assert!(matches!(s.grid_template_columns[1], GridTrackSize::Fr(1.0)));
        assert!(matches!(s.grid_template_columns[2], GridTrackSize::Fr(2.0)));
        assert_eq!(s.grid_template_rows.len(), 3);
        assert!(matches!(s.grid_template_rows[0], GridTrackSize::Auto));
        assert_eq!(s.gap_row, 8.0);
        assert_eq!(s.gap_column, 16.0);
    }

    #[test]
    fn unidades_viewport_resuelven() {
        assert_eq!(parse_length_px("50vw"), Some(640.0));
        assert_eq!(parse_length_px("25vh"), Some(200.0));
        assert_eq!(parse_length_px("10vmin"), Some(80.0));
        assert_eq!(parse_length_px("10vmax"), Some(128.0));
    }

    #[test]
    fn viewport_scope_cambia_y_restaura_la_resolucion() {
        // Fuera de scope: DEFAULT_VIEWPORT (1280×800).
        assert_eq!(parse_length_px("50vw"), Some(640.0));
        {
            let _g = ViewportScope::new(Viewport { width: 800.0, height: 600.0, dpr: 1.0 });
            assert_eq!(parse_length_px("50vw"), Some(400.0));
            assert_eq!(parse_length_px("50vh"), Some(300.0));
            assert_eq!(parse_length_px("50vmin"), Some(300.0));
            assert_eq!(parse_length_px("50vmax"), Some(400.0));
            // Anida: el scope interno gana y el externo se recupera al salir.
            {
                let _g2 = ViewportScope::new(Viewport { width: 200.0, height: 200.0, dpr: 1.0 });
                assert_eq!(parse_length_px("50vw"), Some(100.0));
            }
            assert_eq!(parse_length_px("50vw"), Some(400.0));
        }
        // Al dropear el guard, vuelve a DEFAULT.
        assert_eq!(parse_length_px("50vw"), Some(640.0));
    }

    #[test]
    fn media_query_filtra_segun_viewport() {
        assert!(!evaluate_media_query("(max-width: 600px)", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query("(min-width: 1024px)", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query(
            "(min-width: 800px) and (max-width: 1920px)",
            DEFAULT_VIEWPORT,
        ));
        assert!(!evaluate_media_query("print", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query("screen", DEFAULT_VIEWPORT));

        let html = r#"<html><head><style>
            @media (max-width: 600px) { p { color: red } }
            @media (min-width: 1024px) { p { color: blue } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn media_query_orientation_resolution_y_combinadores() {
        let portrait = Viewport { width: 400.0, height: 900.0, dpr: 1.0 };
        let landscape = Viewport { width: 900.0, height: 400.0, dpr: 1.0 };
        let retina = Viewport { width: 900.0, height: 400.0, dpr: 2.0 };

        // orientation.
        assert!(evaluate_media_query("(orientation: portrait)", portrait));
        assert!(!evaluate_media_query("(orientation: portrait)", landscape));
        assert!(evaluate_media_query("(orientation: landscape)", landscape));

        // resolution (dppx / x / dpi).
        assert!(evaluate_media_query("(min-resolution: 2dppx)", retina));
        assert!(!evaluate_media_query("(min-resolution: 2dppx)", landscape));
        assert!(evaluate_media_query("(min-resolution: 2x)", retina));
        assert!(evaluate_media_query("(min-resolution: 192dpi)", retina));
        assert!(evaluate_media_query("(max-resolution: 1dppx)", landscape));

        // Lista OR (`,`): matchea si cualquiera lo hace.
        assert!(evaluate_media_query("(max-width: 100px), (orientation: landscape)", landscape));
        assert!(!evaluate_media_query("(max-width: 100px), (max-height: 100px)", landscape));

        // `not` invierte la query completa.
        assert!(evaluate_media_query("not (max-width: 100px)", landscape));
        assert!(!evaluate_media_query("not (orientation: landscape)", landscape));

        // Preferencias: reportamos tema claro y sin reducción de movimiento.
        assert!(evaluate_media_query("(prefers-color-scheme: light)", landscape));
        assert!(!evaluate_media_query("(prefers-color-scheme: dark)", landscape));
        assert!(evaluate_media_query("(prefers-reduced-motion: no-preference)", landscape));

        // `and` mezclando dimensión + orientación + resolución.
        assert!(evaluate_media_query(
            "screen and (min-width: 800px) and (orientation: landscape) and (min-resolution: 2dppx)",
            retina,
        ));
        assert!(!evaluate_media_query(
            "screen and (min-width: 800px) and (min-resolution: 2dppx)",
            landscape, // dpr 1.0 → falla la última
        ));

        // aspect-ratio (W/H y número). landscape = 900/400 = 2.25.
        assert!(evaluate_media_query("(min-aspect-ratio: 16/9)", landscape)); // 2.25 >= 1.77
        assert!(!evaluate_media_query("(min-aspect-ratio: 16/9)", portrait)); // 0.44 < 1.77
        assert!(evaluate_media_query("(max-aspect-ratio: 1/1)", portrait)); // 0.44 <= 1.0
        assert!(!evaluate_media_query("(max-aspect-ratio: 1/1)", landscape)); // 2.25 > 1.0
        assert!(evaluate_media_query("(min-aspect-ratio: 2)", landscape)); // 2.25 >= 2

        // Feature desconocida no descalifica (lenient, igual que antes).
        assert!(evaluate_media_query("(quantum-foam: 3)", landscape));
    }

    #[test]
    fn from_dom_with_viewport_selecciona_media_por_ancho_real() {
        let html = r#"<html><head><style>
            p { color: green }
            @media (max-width: 600px) { p { color: red } }
            @media (min-width: 601px) { p { color: blue } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);

        // Viewport angosto → gana la regla red.
        let eng = StyleEngine::from_dom_with_viewport(&dom, Viewport { width: 500.0, height: 800.0, dpr: 1.0 });
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0), "ancho 500 → red");

        // Viewport ancho → gana la regla blue.
        let eng = StyleEngine::from_dom_with_viewport(&dom, Viewport { width: 1200.0, height: 800.0, dpr: 1.0 });
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255), "ancho 1200 → blue");

        // `from_dom` sin viewport cae en DEFAULT_VIEWPORT (1280) → blue.
        let eng = StyleEngine::from_dom(&dom);
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255), "default 1280 → blue");
    }

    #[test]
    fn attr_selector_flag_case_insensitive() {
        let html = r#"<html><head><style>
            [data-x="hello" i] { color: rgb(0,0,255) }
            [type="EMAIL"] { color: rgb(255,0,0) }
            [href^="HTTP" i] { color: rgb(0,128,0) }
        </style></head><body>
            <p id="a" data-x="HELLO">a</p>
            <input id="c" type="email">
            <input id="d" type="EMAIL">
            <a id="e" href="https://x">e</a>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // `[data-x="hello" i]` matchea "HELLO" (insensible).
        assert_eq!(eng.compute(&by_id("a")).color, Color::rgb(0, 0, 255));
        // `[type="EMAIL"]` SIN flag es case-sensitive: "email" no matchea.
        assert_ne!(eng.compute(&by_id("c")).color, Color::rgb(255, 0, 0));
        // "EMAIL" exacto sí matchea.
        assert_eq!(eng.compute(&by_id("d")).color, Color::rgb(255, 0, 0));
        // Prefijo con flag i: `[href^="HTTP" i]` matchea "https://x".
        assert_eq!(eng.compute(&by_id("e")).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn css_nesting_expande_y_aplica() {
        let html = r#"<html><head><style>
            .card {
                color: rgb(1,1,1);
                .title { color: rgb(0,0,255) }
                &.active { color: rgb(0,128,0) }
            }
            .menu { & > li { color: rgb(255,0,0) } }
        </style></head><body>
            <div id="c1" class="card"><span id="t" class="title">t</span></div>
            <div id="c2" class="card active">a</div>
            <ul class="menu"><li id="li1">x</li></ul>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // Declaración propia del padre.
        assert_eq!(eng.compute(&by_id("c1")).color, Color::rgb(1, 1, 1));
        // Anidada descendiente implícita: `.card .title`.
        assert_eq!(eng.compute(&by_id("t")).color, Color::rgb(0, 0, 255));
        // `&.active` → `.card.active` (mayor especificidad gana al padre).
        assert_eq!(eng.compute(&by_id("c2")).color, Color::rgb(0, 128, 0));
        // `& > li` → `.menu > li`.
        assert_eq!(eng.compute(&by_id("li1")).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn media_query_sintaxis_de_rango() {
        // DEFAULT_VIEWPORT = 1280 × 800, dpr 1.
        let vp = DEFAULT_VIEWPORT;
        // `feature op value`.
        assert!(evaluate_media_query("(width >= 600px)", vp));
        assert!(!evaluate_media_query("(width <= 600px)", vp));
        assert!(evaluate_media_query("(width >= 1280px)", vp));
        assert!(!evaluate_media_query("(width > 1280px)", vp));
        assert!(evaluate_media_query("(width < 2000px)", vp));
        // `value op feature` (orden invertido).
        assert!(evaluate_media_query("(600px < width)", vp));
        assert!(!evaluate_media_query("(2000px < width)", vp));
        // Rango de dos lados.
        assert!(evaluate_media_query("(400px <= width <= 1500px)", vp));
        assert!(!evaluate_media_query("(400px <= width <= 800px)", vp));
        // Sin espacios.
        assert!(evaluate_media_query("(width>=600px)", vp));
        // height + combinación con `and`.
        assert!(evaluate_media_query("(height < 1000px) and (width > 1000px)", vp));
        // El path `feature: value` clásico sigue funcionando (regresión).
        assert!(evaluate_media_query("(min-width: 600px)", vp));
        assert!(!evaluate_media_query("(max-width: 600px)", vp));
    }

    #[test]
    fn ua_body_lleva_margin_8() {
        // Cualquier página sin CSS de autor debe arrancar con el body
        // margin: 8px (default del browser real).
        let html = "<html><body>x</body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let body = dom.find("body").unwrap();
        let s = eng.compute(&body);
        assert_eq!(s.margin, Sides::all(8.0));
    }

    #[test]
    fn ua_h3_h4_h5_h6_tienen_tamanos_propios() {
        // Antes h3+ caían al default 16 (igual que `<p>`). Ahora cada
        // nivel tiene tamaño y margin propios.
        for (tag, expected) in
            [("h3", 19.0), ("h4", 16.0), ("h5", 13.0), ("h6", 11.0)]
        {
            let html = format!("<html><body><{tag}>x</{tag}></body></html>");
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            let node = dom.find(tag).unwrap();
            let s = eng.compute(&node);
            assert_eq!(s.font_size, expected, "{tag} font-size");
        }
    }

    #[test]
    fn ua_ul_y_ol_padding_left_para_bullets() {
        let html = "<html><body><ul><li>x</li></ul></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ul = dom.find("ul").unwrap();
        let s = eng.compute(&ul);
        assert_eq!(s.padding.left, 40.0);
    }

    #[test]
    fn ua_a_color_azul_default() {
        let html = "<html><body><a href=#>link</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let s = eng.compute(&a);
        assert_eq!(s.color, Color::rgb(0, 0, 238));
    }

    #[test]
    fn ua_svg_y_canvas_inline_block_video_none() {
        // SVG y `<canvas>` se renderizan (primitivas vía vello / comandos 2D
        // del runtime), así que quedan como inline-block (Fase 7.196 cableó
        // canvas). math/video/audio/etc. siguen ocultos hasta tener renderer.
        let html = "<html><body><svg></svg><canvas></canvas><video></video></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let svg = dom.find("svg").unwrap();
        let canvas = dom.find("canvas").unwrap();
        let video = dom.find("video").unwrap();
        assert_eq!(eng.compute(&svg).display, Display::InlineBlock);
        assert_eq!(eng.compute(&canvas).display, Display::InlineBlock);
        assert_eq!(eng.compute(&video).display, Display::None);
    }

    #[test]
    fn ua_table_layout_minimo() {
        let html = "<html><body><table><tr><td>a</td><td>b</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let table = dom.find("table").unwrap();
        let tr = dom.find("tr").unwrap();
        let td = dom.find("td").unwrap();
        assert_eq!(eng.compute(&table).display, Display::Block);
        // tr es Flex row para que td/td queden lado a lado.
        assert_eq!(eng.compute(&tr).display, Display::Flex);
        // td es InlineBlock para que el row de flex no le dé 100% width.
        assert_eq!(eng.compute(&td).display, Display::InlineBlock);
    }

    #[test]
    fn ua_table_cells_tienen_border_y_padding() {
        // Tablas sin CSS de autor deben mostrar bordes para que la grilla
        // se vea — sino tablas sin estilo (Wikipedia raw, RFC docs, etc.)
        // colapsan visualmente.
        let html = "<html><body><table><tr><th>h</th><td>d</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let th = dom.find("th").unwrap();
        let td = dom.find("td").unwrap();
        let s_th = eng.compute(&th);
        let s_td = eng.compute(&td);
        assert_eq!(s_th.border_widths.top, 1.0);
        assert!(s_th.border_colors.top.is_some());
        assert_eq!(s_td.border_widths.top, 1.0);
        assert_eq!(s_th.padding, Sides::all(4.0));
        assert_eq!(s_td.padding, Sides::all(4.0));
        // `<th>` lleva un bg gris claro para destacarlo como header.
        assert_eq!(s_th.background, Some(Color::rgb(242, 242, 242)));
    }

    #[test]
    fn ua_colgroup_y_col_ocultos() {
        // `<colgroup><col>` son metadatos de columna — no se renderean.
        let html = "<html><body><table><colgroup><col><col></colgroup><tr><td>x</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let colgroup = dom.find("colgroup").unwrap();
        let col = dom.find("col").unwrap();
        assert_eq!(eng.compute(&colgroup).display, Display::None);
        assert_eq!(eng.compute(&col).display, Display::None);
    }

    #[test]
    fn ua_caption_centrado() {
        let html = "<html><body><table><caption>Tabla X</caption><tr><td>a</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let cap = dom.find("caption").unwrap();
        let s = eng.compute(&cap);
        assert_eq!(s.display, Display::Block);
        assert_eq!(s.text_align, TextAlign::Center);
    }

    #[test]
    fn ua_sub_y_sup_aplican_vertical_align() {
        let html = "<html><body><p>H<sub>2</sub>O y E=mc<sup>2</sup></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let sub = dom.find("sub").unwrap();
        let sup = dom.find("sup").unwrap();
        assert_eq!(eng.compute(&sub).vertical_align, VerticalAlign::Sub);
        assert_eq!(eng.compute(&sup).vertical_align, VerticalAlign::Super);
    }

    #[test]
    fn supports_query_filtra_por_parser() {
        assert!(evaluate_supports_query("(display: flex)"));
        assert!(evaluate_supports_query("(color: red)"));
        assert!(!evaluate_supports_query("(display: garbage)"));

        let html = r#"<html><head><style>
            @supports (display: flex) { p { color: green } }
            @supports (display: garbage) { p { color: red } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn supports_query_and_or_not_selector() {
        // and: ambas soportadas.
        assert!(evaluate_supports_query("(display: grid) and (color: red)"));
        assert!(!evaluate_supports_query("(display: grid) and (frobnicate: 1)"));
        // or: alguna soportada.
        assert!(evaluate_supports_query("(display: grid) or (frobnicate: 1)"));
        assert!(!evaluate_supports_query("(frob: 1) or (nicate: 2)"));
        // not.
        assert!(evaluate_supports_query("not (frobnicate: 1)"));
        assert!(!evaluate_supports_query("not (display: grid)"));
        // selector(): soportado si el selector parsea.
        assert!(evaluate_supports_query("selector(.a > .b)"));
        // agrupación anidada.
        assert!(evaluate_supports_query("((display: grid))"));
        assert!(evaluate_supports_query("(display: grid) and ((color: red) or (frob: 1))"));
        // @supports con `and` aplica el bloque end-to-end.
        let html = r#"<html><head><style>
            @supports (display: grid) and (color: red) { p { color: rgb(0,0,255) } }
            @supports (display: grid) and (frob: 1) { p { color: rgb(255,0,0) } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    // === Fase B1: @keyframes ===

    #[test]
    fn keyframes_from_to_se_parsean() {
        let html = r#"<html><head><style>
            @keyframes fade {
                from { opacity: 0; }
                to { opacity: 1; }
            }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("fade").expect("keyframes fade ausente");
        assert_eq!(kf.steps.len(), 2);
        assert_eq!(kf.steps[0].offset, 0.0);
        assert_eq!(kf.steps[0].declarations, vec![("opacity".into(), "0".into())]);
        assert_eq!(kf.steps[1].offset, 1.0);
        assert_eq!(kf.steps[1].declarations, vec![("opacity".into(), "1".into())]);
    }

    #[test]
    fn keyframes_porcentajes_y_orden() {
        // Pasos declarados fuera de orden deben quedar ordenados por offset.
        let html = r#"<html><head><style>
            @keyframes slide {
                100% { left: 100px; }
                0% { left: 0px; }
                50% { left: 40px; top: 10px; }
            }
        </style></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("slide").unwrap();
        let offsets: Vec<f32> = kf.steps.iter().map(|s| s.offset).collect();
        assert_eq!(offsets, vec![0.0, 0.5, 1.0]);
        // El paso del 50% conserva las dos declaraciones en orden.
        assert_eq!(
            kf.steps[1].declarations,
            vec![("left".into(), "40px".into()), ("top".into(), "10px".into())]
        );
    }

    #[test]
    fn keyframes_selector_multiple_comparte_decls() {
        // `0%, 100% { ... }` genera dos pasos con las mismas decls.
        let html = r#"<html><head><style>
            @keyframes pulse {
                0%, 100% { transform: scale(1); }
                50% { transform: scale(1.2); }
            }
        </style></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("pulse").unwrap();
        assert_eq!(kf.steps.len(), 3);
        assert_eq!(kf.steps[0].offset, 0.0);
        assert_eq!(kf.steps[2].offset, 1.0);
        assert_eq!(kf.steps[0].declarations, kf.steps[2].declarations);
    }

    #[test]
    fn keyframes_prefijo_vendor_y_no_rompe_reglas_normales() {
        // `@-webkit-keyframes` se captura igual; y las reglas normales
        // alrededor del at-rule siguen aplicándose.
        let html = r#"<html><head><style>
            p { color: red; }
            @-webkit-keyframes spin { from { opacity: 0 } to { opacity: 1 } }
            p { color: green; }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        assert!(eng.keyframes().contains_key("spin"));
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    // === Fase B2: animation shorthand ===

    fn anim_de(decl: &str) -> AnimationBinding {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        eng.compute(&p).animation.expect("animation ausente")
    }

    #[test]
    fn animation_shorthand_completo() {
        let a = anim_de("animation: spin 2s ease-in-out 0.5s infinite alternate forwards");
        assert_eq!(a.name, "spin");
        assert_eq!(a.duration_s, 2.0);
        assert_eq!(a.timing, EasingFunction::EaseInOut);
        assert_eq!(a.delay_s, 0.5);
        assert_eq!(a.iterations, AnimationIterations::Infinite);
        assert_eq!(a.direction, AnimationDirection::Alternate);
        assert_eq!(a.fill_mode, AnimationFillMode::Forwards);
    }

    #[test]
    fn animation_orden_laxo_y_defaults() {
        // Tokens en orden no canónico + count numérico + ms.
        let a = anim_de("animation: 200ms linear 3 fade");
        assert_eq!(a.name, "fade");
        assert!((a.duration_s - 0.2).abs() < 1e-6);
        assert_eq!(a.timing, EasingFunction::Linear);
        assert_eq!(a.iterations, AnimationIterations::Count(3.0));
        assert_eq!(a.delay_s, 0.0);
        assert_eq!(a.direction, AnimationDirection::Normal);
        assert_eq!(a.fill_mode, AnimationFillMode::None);
    }

    #[test]
    fn animation_cubic_bezier_no_se_parte_por_comas() {
        let a = anim_de("animation: bounce 1s cubic-bezier(0.1, 0.7, 1.0, 0.1)");
        assert_eq!(a.name, "bounce");
        assert_eq!(a.duration_s, 1.0);
        assert_eq!(a.timing, EasingFunction::CubicBezier(0.1, 0.7, 1.0, 0.1));
    }

    #[test]
    fn animation_none_limpia() {
        let html = r#"<html><body><p style="animation: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).animation, None);
    }

    // === Fase B3: transition shorthand ===

    fn trans_de(decl: &str) -> Vec<TransitionBinding> {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        eng.compute(&p).transitions
    }

    #[test]
    fn transition_simple() {
        let t = trans_de("transition: opacity 200ms ease");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "opacity");
        assert!((t[0].duration_s - 0.2).abs() < 1e-6);
        assert_eq!(t[0].timing, EasingFunction::Ease);
        assert_eq!(t[0].delay_s, 0.0);
    }

    #[test]
    fn transition_lista_multiple() {
        let t = trans_de("transition: opacity 200ms ease, transform 0.3s ease-in 0.1s");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].property, "opacity");
        assert_eq!(t[1].property, "transform");
        assert!((t[1].duration_s - 0.3).abs() < 1e-6);
        assert_eq!(t[1].timing, EasingFunction::EaseIn);
        assert!((t[1].delay_s - 0.1).abs() < 1e-6);
    }

    #[test]
    fn transition_default_property_es_all() {
        // Sin nombre de propiedad, default `all` (CSS spec).
        let t = trans_de("transition: 1s");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "all");
        assert_eq!(t[0].duration_s, 1.0);
        assert_eq!(t[0].timing, EasingFunction::Ease);
    }

    #[test]
    fn transition_steps_y_none() {
        let t = trans_de("transition: width 2s steps(4, end)");
        assert_eq!(t[0].timing, EasingFunction::Steps(4, false));

        let html = r#"<html><body><p style="transition: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert!(eng.compute(&p).transitions.is_empty());
    }
}
