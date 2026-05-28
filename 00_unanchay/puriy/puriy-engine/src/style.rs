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
    pub margin: Sides<f32>,
    pub padding: Sides<f32>,
    /// Ancho explícito. `Auto` = el default block-fills-parent.
    pub width: LengthVal,
    /// Tope superior — útil para containers narrow ("max-width:800px").
    pub max_width: LengthVal,
    /// Alineación horizontal del texto dentro del box.
    pub text_align: TextAlign,
    /// Altura de línea como multiplicador del font-size. `None` =
    /// default razonable (1.4) en el caller.
    pub line_height: Option<f32>,
    /// Ancho del border en px. `0` = sin border.
    pub border_width: f32,
    /// Color del border. `None` = no se dibuja aunque `border_width > 0`.
    pub border_color: Option<Color>,
    /// Radio del corner-radius en px (0 = esquinas vivas).
    pub border_radius: f32,
    /// `box-shadow` simplificado. `None` = sin sombra.
    pub box_shadow: Option<BoxShadow>,
    /// `text-decoration-line` reducido al subset que pintamos.
    /// `None` = sin decoración (default HTML, salvo `<a>`/`<u>`/`<s>`).
    pub text_decoration: TextDecorationLine,
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

/// Sombra rectangular detrás del box. `blur_px` y `spread_px` se
/// combinan en una expansión efectiva del rect — gaussian blur real
/// queda para cuando el render-pipeline soporte multi-pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub spread_px: f32,
    pub color: Color,
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
    /// Distancia del border al outline. Default 0.
    pub offset: f32,
}

impl Default for Outline {
    fn default() -> Self {
        Self { width: 0.0, color: None, style_active: true, offset: 0.0 }
    }
}

/// Un stop de `linear-gradient`. `pos` es la fracción (0..1) del eje;
/// si `None`, se distribuye automáticamente entre stops adyacentes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub pos: Option<f32>,
}

/// `background-image: linear-gradient(...)`. Subset: ángulo en grados
/// (0 = bottom→top, 90 = left→right), 2+ stops. Conic/radial quedan
/// para más adelante.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    /// Ángulo CSS en grados — 0 = up, 90 = right, 180 = down, 270 = left.
    pub angle_deg: f32,
    pub stops: Vec<GradientStop>,
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
            margin: Sides::all(0.0),
            padding: Sides::all(0.0),
            width: LengthVal::Auto,
            max_width: LengthVal::Auto,
            text_align: TextAlign::Left,
            line_height: None,
            border_width: 0.0,
            border_color: None,
            border_radius: 0.0,
            box_shadow: None,
            text_decoration: TextDecorationLine::None,
            list_style_type: ListStyleType::Disc,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Stretch,
            flex_wrap: FlexWrap::NoWrap,
            gap_row: 0.0,
            gap_column: 0.0,
            box_sizing: BoxSizing::ContentBox,
            min_width: LengthVal::Auto,
            min_height: LengthVal::Auto,
            max_height: LengthVal::Auto,
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
        }
    }
}

/// Almacena reglas parseadas + función de "computar para nodo".
pub struct StyleEngine {
    rules: Vec<Rule>,
    /// CSS variables declaradas en `:root`/`html`/`*`. Se substituyen en
    /// los values en parse-time (y en values de `style="..."` inline en
    /// compute-time). Scope cascade real queda para una iteración futura
    /// — :root cubre el 80% de los usos en el wild.
    vars: HashMap<String, String>,
}

impl StyleEngine {
    /// Construye el engine desde el DOM: parsea cada `<style>` inline +
    /// inyecta el UA stylesheet (los defaults HTML que cssparser no
    /// conoce).
    pub fn from_dom(dom: &DomTree) -> Self {
        let mut rules = ua_stylesheet();
        // Primera pasada: recoger `--name: value` de `:root` de todos
        // los stylesheets para que cualquier `var(--x)` se resuelva sin
        // importar en qué archivo se declaró.
        let mut vars: HashMap<String, String> = HashMap::new();
        for sheet in dom.collect_inline_stylesheets() {
            let cleaned = strip_comments(&sheet);
            extract_root_vars(&cleaned, &mut vars);
        }
        for sheet in dom.collect_inline_stylesheets() {
            rules.extend(parse_stylesheet(&sheet, &vars));
        }
        Self { rules, vars }
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
    /// `hover_fill` en el render.
    pub fn compute_with_parent_in_state(
        &self,
        node: &Handle,
        parent: Option<&ComputedStyle>,
        hover_active: bool,
    ) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        if let Some(p) = parent {
            style.color = p.color;
            style.font_size = p.font_size;
            style.font_weight = p.font_weight;
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
        }
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

        // Cascada en dos pasadas:
        //   1. Decls normales, ordenadas por (specificity, source_index).
        //   2. Decls `!important`, ordenadas igual.
        // Cada decl individual lleva su flag — una misma regla puede
        // tener decls normales y `!important` mezcladas.
        let matched: Vec<(u32, usize, &Rule)> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.matches_in_state(node, hover_active))
            .map(|(i, r)| (r.selector.specificity(), i, r))
            .collect();
        let inline_decls: Vec<Decl> = dom::attr(node, "style")
            .map(|s| parse_declarations(&s, &self.vars))
            .unwrap_or_default();

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
            d.apply(&mut style);
        }
        // Inline normal (especificidad 1000) cierra la pasada normal.
        for d in &inline_decls {
            if !d.important {
                d.apply(&mut style);
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
            d.apply(&mut style);
        }
        // Inline `!important` (efectiva 10_000 en CSS real, pero acá
        // simplemente cierra la pasada — gana todo lo anterior).
        for d in &inline_decls {
            if d.important {
                d.apply(&mut style);
            }
        }
        style
    }
}

#[derive(Debug)]
struct Rule {
    selector: Selector,
    decls: Vec<Decl>,
}

/// Selector encadenado — alterna compound + combinador. `compounds[0]`
/// es el ancestro/hermano más lejano; `compounds.last()` es el sujeto.
/// `combinators[i]` es el combinador entre `compounds[i]` y
/// `compounds[i+1]`.
#[derive(Debug, Clone)]
struct Selector {
    compounds: Vec<Compound>,
    combinators: Vec<Combinator>,
}

impl Selector {
    /// Especificidad CSS — número compuesto `a*100 + b*10 + c` donde:
    /// - `a` = cuentas de `#id` en toda la cadena
    /// - `b` = cuentas de `.class`, `[attr]`, `:pseudo-class`
    /// - `c` = cuentas de tags (`p`, `div`, …); `*` y combinadores no
    ///   suman
    ///
    /// Inline `style="..."` no pasa por acá; el caller le otorga 1000
    /// implícito al aplicarlo después de los selectores.
    fn specificity(&self) -> u32 {
        let mut ids = 0u32;
        let mut classes_etc = 0u32;
        let mut types = 0u32;
        for c in &self.compounds {
            ids += c.ids.len() as u32;
            classes_etc += c.classes.len() as u32;
            classes_etc += c.attrs.len() as u32;
            for p in &c.pseudos {
                match p {
                    // CSS spec: :not(X) aporta la especificidad de X, no
                    // la suya propia. Sumamos las partes del compound
                    // interno.
                    Pseudo::Not(inner) => {
                        ids += inner.ids.len() as u32;
                        classes_etc += inner.classes.len() as u32;
                        classes_etc += inner.attrs.len() as u32;
                        classes_etc += inner.pseudos.len() as u32;
                        if matches!(inner.tag, TagPart::Type(_)) {
                            types += 1;
                        }
                    }
                    _ => classes_etc += 1,
                }
            }
            if matches!(c.tag, TagPart::Type(_)) {
                types += 1;
            }
        }
        ids * 100 + classes_etc * 10 + types
    }
}

/// Combinador CSS entre dos compounds consecutivos.
#[derive(Debug, Clone, Copy)]
enum Combinator {
    /// Whitespace — descendiente cualquier nivel.
    Descendant,
    /// `>` — hijo directo.
    Child,
    /// `+` — hermano adyacente inmediato.
    AdjacentSibling,
    /// `~` — hermano general (posterior, mismo padre).
    GeneralSibling,
}

/// Simple compound — un Tag + 0..N ids/clases/atributos/pseudoclases en
/// cadena (sin espacios). Ejemplos válidos: `a.btn`, `p#hero.alert`,
/// `input[type="checkbox"]`, `li:first-child`, `a[href^="https"]:last-of-type`.
#[derive(Debug, Clone)]
struct Compound {
    tag: TagPart,
    ids: Vec<String>,
    classes: Vec<String>,
    attrs: Vec<AttrMatch>,
    pseudos: Vec<Pseudo>,
}

#[derive(Debug, Clone)]
enum TagPart {
    Universal,
    Type(String),
}

#[derive(Debug, Clone)]
struct AttrMatch {
    name: String,
    op: AttrOp,
    value: String,
}

#[derive(Debug, Clone, Copy)]
enum AttrOp {
    /// `[attr]` — sólo presencia
    Present,
    /// `[attr=value]` — igualdad exacta
    Equals,
    /// `[attr^=value]` — empieza con
    Prefix,
    /// `[attr$=value]` — termina con
    Suffix,
    /// `[attr*=value]` — contiene substring
    Contains,
}

/// Pseudoclases soportadas — la mayoría estructurales (puramente
/// posicionales). `Hover` se evalúa según un flag externo que pasa el
/// caller (`hover_active`); el chrome se encarga de mantenerlo
/// correlacionado con la posición del mouse.
#[derive(Debug, Clone)]
enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    Hover,
    /// `:nth-child(an+b)` — match si la posición 1-indexed del nodo en
    /// el padre satisface `pos = a*k + b` para algún `k >= 0`.
    NthChild {
        a: i32,
        b: i32,
    },
    /// `:not(simple)` — negación de un compound simple sin combinadores
    /// ni `:not` anidado. Almacenamos el compound interno completo.
    Not(Box<Compound>),
}

impl Compound {
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false)
    }

    /// Variante con flag `hover_active` — sólo afecta el matching de
    /// `:hover`. El resto del compound se evalúa idéntico.
    fn matches_in_state(
        &self,
        node: &markup5ever_rcdom::Handle,
        hover_active: bool,
    ) -> bool {
        let Some(local) = dom::element_name(node) else {
            return false;
        };
        if let TagPart::Type(t) = &self.tag {
            if !t.eq_ignore_ascii_case(&local) {
                return false;
            }
        }
        for want in &self.ids {
            if dom::attr(node, "id").as_deref() != Some(want.as_str()) {
                return false;
            }
        }
        if !self.classes.is_empty() {
            let attr = dom::attr(node, "class").unwrap_or_default();
            let present: Vec<&str> = attr.split_whitespace().collect();
            for want in &self.classes {
                if !present.iter().any(|c| c == want) {
                    return false;
                }
            }
        }
        for am in &self.attrs {
            if !attr_matches(node, am) {
                return false;
            }
        }
        for p in &self.pseudos {
            if !pseudo_matches(node, p, hover_active) {
                return false;
            }
        }
        true
    }
}

fn attr_matches(node: &markup5ever_rcdom::Handle, am: &AttrMatch) -> bool {
    let actual = dom::attr(node, &am.name);
    match am.op {
        AttrOp::Present => actual.is_some(),
        op => {
            let Some(actual) = actual else { return false };
            match op {
                AttrOp::Equals => actual == am.value,
                AttrOp::Prefix => actual.starts_with(&am.value),
                AttrOp::Suffix => actual.ends_with(&am.value),
                AttrOp::Contains => actual.contains(&am.value),
                AttrOp::Present => unreachable!(),
            }
        }
    }
}

fn pseudo_matches(
    node: &markup5ever_rcdom::Handle,
    p: &Pseudo,
    hover_active: bool,
) -> bool {
    match p {
        Pseudo::Hover => return hover_active,
        Pseudo::Not(c) => return !c.matches_in_state(node, hover_active),
        _ => {}
    }
    let Some(parent) = parent_of(node) else { return false };
    let kids = parent.children.borrow();
    let mut elems: Vec<markup5ever_rcdom::Handle> = Vec::new();
    for c in kids.iter() {
        if dom::element_name(c).is_some() {
            elems.push(c.clone());
        }
    }
    let Some(pos) = elems.iter().position(|c| std::rc::Rc::ptr_eq(c, node)) else {
        return false;
    };
    match p {
        Pseudo::Hover | Pseudo::Not(_) => unreachable!("ya resueltos arriba"),
        Pseudo::FirstChild => pos == 0,
        Pseudo::LastChild => pos + 1 == elems.len(),
        Pseudo::OnlyChild => elems.len() == 1,
        Pseudo::FirstOfType => {
            let my_tag = dom::element_name(node).unwrap_or_default();
            elems[..pos]
                .iter()
                .all(|c| dom::element_name(c).map(|t| t != my_tag).unwrap_or(true))
        }
        Pseudo::LastOfType => {
            let my_tag = dom::element_name(node).unwrap_or_default();
            elems[pos + 1..]
                .iter()
                .all(|c| dom::element_name(c).map(|t| t != my_tag).unwrap_or(true))
        }
        Pseudo::NthChild { a, b } => {
            // `pos` es 0-indexed, CSS usa 1-indexed.
            let p_css = (pos + 1) as i32;
            let diff = p_css - *b;
            if *a == 0 {
                diff == 0
            } else if *a > 0 {
                diff >= 0 && diff % *a == 0
            } else {
                diff <= 0 && diff % *a == 0
            }
        }
    }
}

impl Rule {
    #[allow(dead_code)]
    fn matches(&self, node: &markup5ever_rcdom::Handle) -> bool {
        self.matches_in_state(node, false)
    }

    fn matches_in_state(&self, node: &markup5ever_rcdom::Handle, hover_active: bool) -> bool {
        let compounds = &self.selector.compounds;
        if compounds.is_empty() {
            return false;
        }
        // El sujeto (último) debe matchear el nodo.
        if !compounds.last().unwrap().matches_in_state(node, hover_active) {
            return false;
        }
        if compounds.len() == 1 {
            return true;
        }
        // Avanzamos derecha→izquierda, encadenando combinadores. Cada
        // combinador define cómo viajar al "siguiente" candidato:
        //   Descendant/Child  → ancestro
        //   Adjacent/General  → hermano anterior
        let combs = &self.selector.combinators;
        // El combinador entre compounds[i-1] y compounds[i] vive en
        // combs[i-1]. Recorremos desde compounds[len-2] hacia 0.
        let mut subject = node.clone();
        let mut i = compounds.len() - 1;
        while i > 0 {
            let comb = combs[i - 1];
            let target = &compounds[i - 1];
            match comb {
                Combinator::Child => {
                    let Some(p) = parent_of(&subject) else { return false };
                    if !target.matches(&p) {
                        return false;
                    }
                    subject = p;
                }
                Combinator::Descendant => {
                    let mut cur = parent_of(&subject);
                    loop {
                        let Some(n) = cur else { return false };
                        if target.matches(&n) {
                            subject = n;
                            break;
                        }
                        cur = parent_of(&n);
                    }
                }
                Combinator::AdjacentSibling => {
                    let Some(prev) = prev_element_sibling(&subject) else { return false };
                    if !target.matches(&prev) {
                        return false;
                    }
                    subject = prev;
                }
                Combinator::GeneralSibling => {
                    let mut cur = prev_element_sibling(&subject);
                    loop {
                        let Some(n) = cur else { return false };
                        if target.matches(&n) {
                            subject = n;
                            break;
                        }
                        cur = prev_element_sibling(&n);
                    }
                }
            }
            i -= 1;
        }
        true
    }
}

fn parent_of(node: &markup5ever_rcdom::Handle) -> Option<markup5ever_rcdom::Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Hermano Element anterior (saltea texto/whitespace nodes). Devuelve
/// `None` si no hay padre o si no hay Element previo bajo el mismo padre.
fn prev_element_sibling(
    node: &markup5ever_rcdom::Handle,
) -> Option<markup5ever_rcdom::Handle> {
    let parent = parent_of(node)?;
    let kids = parent.children.borrow();
    let mut last_elem: Option<markup5ever_rcdom::Handle> = None;
    for child in kids.iter() {
        if std::rc::Rc::ptr_eq(child, node) {
            return last_elem;
        }
        if dom::element_name(child).is_some() {
            last_elem = Some(child.clone());
        }
    }
    None
}

/// Una declaración CSS individual + flag `!important`.
#[derive(Debug, Clone)]
struct Decl {
    kind: DeclKind,
    important: bool,
}

#[derive(Debug, Clone)]
enum DeclKind {
    Color(Color),
    Background(Color),
    Display(Display),
    FontSize(f32),
    FontWeight(u16),
    Margin(Sides<f32>),
    MarginTop(f32),
    MarginRight(f32),
    MarginBottom(f32),
    MarginLeft(f32),
    Padding(Sides<f32>),
    PaddingTop(f32),
    PaddingRight(f32),
    PaddingBottom(f32),
    PaddingLeft(f32),
    Width(LengthVal),
    MaxWidth(LengthVal),
    TextAlign(TextAlign),
    LineHeight(f32),
    BorderWidth(f32),
    BorderColor(Color),
    /// `border-style: solid` activa el dibujo del border; `none`/`hidden`
    /// lo desactiva (color → None).
    BorderEnabled(bool),
    BorderRadius(f32),
    /// `None` = `box-shadow: none` (limpia la sombra).
    BoxShadow(Option<BoxShadow>),
    TextDecoration(TextDecorationLine),
    ListStyleType(ListStyleType),
    FlexDirection(FlexDirection),
    JustifyContent(JustifyContent),
    AlignItems(AlignItems),
    FlexWrap(FlexWrap),
    /// `gap: A B` setea ambos (row=A, column=B); `gap: V` los iguala.
    Gap { row: f32, column: f32 },
    RowGap(f32),
    ColumnGap(f32),
    BoxSizing(BoxSizing),
    MinWidth(LengthVal),
    MinHeight(LengthVal),
    MaxHeight(LengthVal),
    Overflow(Overflow),
    WhiteSpace(WhiteSpace),
    TextTransform(TextTransform),
    Opacity(f32),
    AlignSelf(AlignSelf),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(LengthVal),
    OutlineWidth(f32),
    OutlineColor(Color),
    OutlineStyle(bool),
    OutlineOffset(f32),
    BackgroundGradient(LinearGradient),
    /// `background-image: none` limpia el gradient (un autor puede
    /// querer overridear un gradient heredado).
    BackgroundGradientNone,
}

impl Decl {
    fn apply(&self, s: &mut ComputedStyle) {
        match &self.kind {
            DeclKind::Color(c) => s.color = *c,
            DeclKind::Background(c) => s.background = Some(*c),
            DeclKind::Display(d) => s.display = *d,
            DeclKind::FontSize(v) => s.font_size = *v,
            DeclKind::FontWeight(w) => s.font_weight = *w,
            DeclKind::Margin(v) => s.margin = *v,
            DeclKind::MarginTop(v) => s.margin.top = *v,
            DeclKind::MarginRight(v) => s.margin.right = *v,
            DeclKind::MarginBottom(v) => s.margin.bottom = *v,
            DeclKind::MarginLeft(v) => s.margin.left = *v,
            DeclKind::Padding(v) => s.padding = *v,
            DeclKind::PaddingTop(v) => s.padding.top = *v,
            DeclKind::PaddingRight(v) => s.padding.right = *v,
            DeclKind::PaddingBottom(v) => s.padding.bottom = *v,
            DeclKind::PaddingLeft(v) => s.padding.left = *v,
            DeclKind::Width(v) => s.width = *v,
            DeclKind::MaxWidth(v) => s.max_width = *v,
            DeclKind::TextAlign(a) => s.text_align = *a,
            DeclKind::LineHeight(v) => s.line_height = Some(*v),
            DeclKind::BorderWidth(v) => s.border_width = *v,
            DeclKind::BorderColor(c) => s.border_color = Some(*c),
            DeclKind::BorderEnabled(on) => {
                if !*on {
                    s.border_color = None;
                    s.border_width = 0.0;
                }
            }
            DeclKind::BorderRadius(v) => s.border_radius = *v,
            DeclKind::BoxShadow(v) => s.box_shadow = *v,
            DeclKind::TextDecoration(t) => s.text_decoration = *t,
            DeclKind::ListStyleType(t) => s.list_style_type = *t,
            DeclKind::FlexDirection(d) => s.flex_direction = *d,
            DeclKind::JustifyContent(j) => s.justify_content = *j,
            DeclKind::AlignItems(a) => s.align_items = *a,
            DeclKind::FlexWrap(w) => s.flex_wrap = *w,
            DeclKind::Gap { row, column } => {
                s.gap_row = *row;
                s.gap_column = *column;
            }
            DeclKind::RowGap(v) => s.gap_row = *v,
            DeclKind::ColumnGap(v) => s.gap_column = *v,
            DeclKind::BoxSizing(b) => s.box_sizing = *b,
            DeclKind::MinWidth(v) => s.min_width = *v,
            DeclKind::MinHeight(v) => s.min_height = *v,
            DeclKind::MaxHeight(v) => s.max_height = *v,
            DeclKind::Overflow(o) => s.overflow = *o,
            DeclKind::WhiteSpace(w) => s.white_space = *w,
            DeclKind::TextTransform(t) => s.text_transform = *t,
            DeclKind::Opacity(v) => s.opacity = *v,
            DeclKind::AlignSelf(a) => s.align_self = *a,
            DeclKind::FlexGrow(v) => s.flex_grow = *v,
            DeclKind::FlexShrink(v) => s.flex_shrink = *v,
            DeclKind::FlexBasis(v) => s.flex_basis = *v,
            DeclKind::OutlineWidth(v) => s.outline.width = *v,
            DeclKind::OutlineColor(c) => s.outline.color = Some(*c),
            DeclKind::OutlineStyle(active) => {
                s.outline.style_active = *active;
                if !*active {
                    s.outline.color = None;
                }
            }
            DeclKind::OutlineOffset(v) => s.outline.offset = *v,
            DeclKind::BackgroundGradient(g) => s.background_gradient = Some(g.clone()),
            DeclKind::BackgroundGradientNone => s.background_gradient = None,
        }
    }
}

fn default_display(tag: &str) -> Display {
    match tag {
        "html" | "body" | "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
        | "li" | "header" | "footer" | "section" | "article" | "nav" | "main" | "aside"
        | "form" | "pre" | "blockquote" | "hr" => Display::Block,
        "head" | "title" | "style" | "script" | "meta" | "link" => Display::None,
        _ => Display::Inline,
    }
}

fn default_weight(tag: &str) -> u16 {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "b" | "strong" | "th" => 700,
        _ => 400,
    }
}

/// UA stylesheet mínimo — defaults HTML5 que cssparser por sí solo no
/// inyecta. Mantén corto: sólo lo necesario para no devolver páginas
/// "blancas" sin reglas autor.
fn ua_stylesheet() -> Vec<Rule> {
    fn ty(s: &str) -> Selector {
        Selector {
            compounds: vec![Compound {
                tag: TagPart::Type(s.into()),
                ids: vec![],
                classes: vec![],
                attrs: vec![],
                pseudos: vec![],
            }],
            combinators: vec![],
        }
    }
    vec![
        Rule {
            selector: ty("h1"),
            decls: vec![
                Decl { kind: DeclKind::FontSize(32.0), important: false },
                Decl { kind: DeclKind::Margin(Sides::all(20.0)), important: false },
            ],
        },
        Rule {
            selector: ty("h2"),
            decls: vec![
                Decl { kind: DeclKind::FontSize(24.0), important: false },
                Decl { kind: DeclKind::Margin(Sides::all(18.0)), important: false },
            ],
        },
        Rule {
            selector: ty("p"),
            decls: vec![Decl { kind: DeclKind::Margin(Sides::all(12.0)), important: false }],
        },
        Rule {
            selector: ty("body"),
            decls: vec![Decl { kind: DeclKind::Padding(Sides::all(8.0)), important: false }],
        },
        // Defaults de text-decoration. `<a>` y `<u>`/`<ins>` van con
        // underline; `<s>`/`<strike>`/`<del>` tachadas. Cualquier autor
        // puede override con `text-decoration: none` en su stylesheet.
        Rule {
            selector: ty("a"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::Underline),
                important: false,
            }],
        },
        Rule {
            selector: ty("u"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::Underline),
                important: false,
            }],
        },
        Rule {
            selector: ty("ins"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::Underline),
                important: false,
            }],
        },
        Rule {
            selector: ty("s"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::LineThrough),
                important: false,
            }],
        },
        Rule {
            selector: ty("strike"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::LineThrough),
                important: false,
            }],
        },
        Rule {
            selector: ty("del"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::LineThrough),
                important: false,
            }],
        },
        // Listas: `<ol>` numérico, `<ul>` con bullets. La propiedad se
        // hereda, así que poniéndola en el container es suficiente — los
        // `<li>` descendientes la recogen vía inheritance.
        Rule {
            selector: ty("ol"),
            decls: vec![Decl {
                kind: DeclKind::ListStyleType(ListStyleType::Decimal),
                important: false,
            }],
        },
        Rule {
            selector: ty("ul"),
            decls: vec![Decl {
                kind: DeclKind::ListStyleType(ListStyleType::Disc),
                important: false,
            }],
        },
        Rule {
            selector: ty("menu"),
            decls: vec![Decl {
                kind: DeclKind::ListStyleType(ListStyleType::Disc),
                important: false,
            }],
        },
    ]
}

// ----- parser -----
//
// Para Fase 2 no usamos cssparser AtRule/QualifiedRule (su API rotó
// entre 0.33→0.35 y nuestro subset cabe en 30 líneas). Si Fase 3 mete
// nesting / `@media` / `!important`, migrar a `cssparser::StyleSheetParser`
// con un visitor.

fn parse_stylesheet(css: &str, vars: &HashMap<String, String>) -> Vec<Rule> {
    let mut out = Vec::new();
    // Strip comentarios /* ... */
    let css = strip_comments(css);
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Encuentra '{' que abre el body.
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        i += brace + 1;
        let Some(close) = css[i..].find('}') else { break };
        let body = &css[i..i + close];
        i += close + 1;
        if sel_raw.is_empty() {
            continue;
        }
        // Selectores múltiples separados por ',': uno por uno.
        for sel in sel_raw.split(',') {
            let sel = sel.trim();
            let Some(selector) = parse_selector(sel) else {
                // Selectores no soportados (combinadores, pseudoclases,
                // atributos) ignoramos en silencio — la regla queda
                // inerte y el documento sigue parseando.
                continue;
            };
            out.push(Rule { selector, decls: parse_declarations(body, vars) });
        }
    }
    out
}

/// Pasada previa al parseo real: encuentra bloques `:root { ... }`,
/// `html { ... }` o `* { ... }` y recoge cualquier declaración `--name:
/// value` en el mapa global de variables. Los conflictos (mismo nombre
/// en dos bloques) los gana el último — se acerca bastante a la cascada
/// CSS para vars declaradas en root.
fn extract_root_vars(css: &str, vars: &mut HashMap<String, String>) {
    let mut i = 0;
    while i < css.len() {
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        let body_start = i + brace + 1;
        let Some(close) = css[body_start..].find('}') else { break };
        let body = &css[body_start..body_start + close];
        i = body_start + close + 1;
        let mut is_root = false;
        for sel in sel_raw.split(',') {
            let sel = sel.trim();
            if sel == ":root" || sel == "html" || sel == "*" {
                is_root = true;
                break;
            }
        }
        if !is_root {
            continue;
        }
        for chunk in body.split(';') {
            let Some((prop, value)) = chunk.split_once(':') else {
                continue;
            };
            let prop = prop.trim();
            if let Some(name) = prop.strip_prefix("--") {
                vars.insert(name.to_string(), value.trim().to_string());
            }
        }
    }
}

/// Reemplaza `var(--name)` y `var(--name, fallback)` en `value` por el
/// valor recogido en `vars`. Si la variable no existe y hay fallback, lo
/// usa; sino, sustituye por cadena vacía. La sustitución es recursiva
/// (un value de var puede a su vez contener `var(...)`).
fn substitute_vars(value: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("var(") {
        out.push_str(&rest[..start]);
        let inside_start = start + 4;
        // Buscar el `)` que cierra, respetando nesting de paréntesis
        // (para tolerar `var(--x, calc(1px))` aunque no parseemos calc).
        let mut depth = 1usize;
        let bytes = rest[inside_start..].as_bytes();
        let mut close_pos: Option<usize> = None;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close_pos = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close_pos else {
            // Paréntesis colgado — devolvemos lo que quedaba pegado.
            out.push_str(&rest[start..]);
            return out;
        };
        let args = &rest[inside_start..inside_start + close];
        let (name, fallback) = match args.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (args.trim(), None),
        };
        let var_name = name.strip_prefix("--").unwrap_or("");
        let replacement = vars
            .get(var_name)
            .cloned()
            .or_else(|| fallback.map(|s| s.to_string()))
            .unwrap_or_default();
        // Recursión: el value resuelto puede contener más var().
        out.push_str(&substitute_vars(&replacement, vars));
        rest = &rest[inside_start + close + 1..];
    }
    out.push_str(rest);
    out
}

/// Parsea un selector encadenado. Soporta:
/// - simples compound: `*`, `tag`, `.class`, `#id`, `a.btn`, `p#hero.alert`
/// - selectores de atributo: `[href]`, `[type="text"]`, `[href^="https"]`,
///   `[src$=".png"]`, `[class*="foo"]`
/// - pseudoclases estructurales: `:first-child`, `:last-child`,
///   `:only-child`, `:first-of-type`, `:last-of-type`
/// - combinadores: descendiente (whitespace), hijo directo `>`,
///   hermano adyacente `+`, hermano general `~`
///
/// Pseudoclases de estado (`:hover`, `:focus`, `:active`), `:not(...)`,
/// `:nth-child(...)` y pseudo-elementos (`::before`) siguen sin soporte —
/// el selector entero se ignora si los menciona.
fn parse_selector(sel: &str) -> Option<Selector> {
    let sel = sel.trim();
    if sel.is_empty() {
        return Some(Selector {
            compounds: vec![Compound {
                tag: TagPart::Universal,
                ids: vec![],
                classes: vec![],
                attrs: vec![],
                pseudos: vec![],
            }],
            combinators: vec![],
        });
    }
    // Tokenizamos: cada compound es una secuencia sin espacios ni
    // combinadores; los combinadores ('>', '+', '~') están separados por
    // whitespace en CSS canónico o pegados. Normalizamos respetando lo
    // que viva dentro de `[...]` o `(...)`.
    let normalized = normalize_combinators(sel);
    let mut compounds: Vec<Compound> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut pending_combinator: Option<Combinator> = None;
    let mut first = true;
    for tok in normalized.split_whitespace() {
        match tok {
            ">" => pending_combinator = Some(Combinator::Child),
            "+" => pending_combinator = Some(Combinator::AdjacentSibling),
            "~" => pending_combinator = Some(Combinator::GeneralSibling),
            _ => {
                let compound = parse_compound(tok)?;
                if first {
                    first = false;
                } else {
                    combinators.push(pending_combinator.take().unwrap_or(Combinator::Descendant));
                }
                compounds.push(compound);
            }
        }
    }
    if compounds.is_empty() {
        return None;
    }
    // Si el selector terminó con un combinador colgado, es inválido.
    if pending_combinator.is_some() {
        return None;
    }
    Some(Selector { compounds, combinators })
}

/// Inserta espacios alrededor de `>`/`+`/`~` para que `split_whitespace`
/// los aísle como tokens propios. Si caen dentro de `[…]` o `(…)` los
/// dejamos intactos — `[href*="a>b"]` o `:not(a+b)` deben pasar al
/// compound parser sin romperse.
fn normalize_combinators(sel: &str) -> String {
    let mut out = String::with_capacity(sel.len() + 4);
    let mut in_bracket = false;
    let mut paren_depth: u32 = 0;
    for c in sel.chars() {
        match c {
            '[' => {
                in_bracket = true;
                out.push(c);
            }
            ']' => {
                in_bracket = false;
                out.push(c);
            }
            '(' => {
                paren_depth += 1;
                out.push(c);
            }
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                out.push(c);
            }
            '>' | '+' | '~' if !in_bracket && paren_depth == 0 => {
                out.push(' ');
                out.push(c);
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Parsea un compound: opcional tag/`*` seguido de cualquier número de
/// `.class`, `#id`, `[attr...]`, `:pseudo`. Devuelve `None` si encuentra
/// caracteres no esperados, una pseudo no soportada, o `::pseudo-element`.
fn parse_compound(sel: &str) -> Option<Compound> {
    let bytes = sel.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    // Tag opcional (puede ser `*` o un nombre).
    let tag = if bytes[0] == b'*' {
        i = 1;
        TagPart::Universal
    } else if is_ident_byte(bytes[0]) {
        let start = i;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        TagPart::Type(sel[start..i].to_string())
    } else {
        TagPart::Universal
    };
    let mut ids = Vec::new();
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudos = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'.' | b'#' => {
                let marker = bytes[i];
                i += 1;
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let ident = sel[start..i].to_string();
                if marker == b'.' {
                    classes.push(ident);
                } else {
                    ids.push(ident);
                }
            }
            b'[' => {
                let inner_start = i + 1;
                let rel_close = sel[inner_start..].find(']')?;
                let inner = &sel[inner_start..inner_start + rel_close];
                attrs.push(parse_attr_match(inner)?);
                i = inner_start + rel_close + 1;
            }
            b':' => {
                i += 1;
                // `::pseudo-element` (e.g. ::before) — rechazamos.
                if i < bytes.len() && bytes[i] == b':' {
                    return None;
                }
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name = sel[start..i].to_ascii_lowercase();
                // Funcionales: `:nth-child(...)`, `:not(...)`. Detectamos
                // y consumimos los argumentos.
                if i < bytes.len() && bytes[i] == b'(' {
                    let arg_start = i + 1;
                    let rel_close = sel[arg_start..].find(')')?;
                    let arg = &sel[arg_start..arg_start + rel_close];
                    let p = match name.as_str() {
                        "nth-child" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthChild { a, b }
                        }
                        "not" => {
                            let inner = parse_compound(arg)?;
                            // Anti-recursión: `:not(:not(...))` rechazamos.
                            if inner
                                .pseudos
                                .iter()
                                .any(|p| matches!(p, Pseudo::Not(_)))
                            {
                                return None;
                            }
                            Pseudo::Not(Box::new(inner))
                        }
                        _ => return None,
                    };
                    pseudos.push(p);
                    i = arg_start + rel_close + 1;
                    continue;
                }
                let p = match name.as_str() {
                    "first-child" => Pseudo::FirstChild,
                    "last-child" => Pseudo::LastChild,
                    "only-child" => Pseudo::OnlyChild,
                    "first-of-type" => Pseudo::FirstOfType,
                    "last-of-type" => Pseudo::LastOfType,
                    "hover" => Pseudo::Hover,
                    _ => return None,
                };
                pseudos.push(p);
            }
            _ => return None,
        }
    }
    if matches!(tag, TagPart::Universal)
        && ids.is_empty()
        && classes.is_empty()
        && attrs.is_empty()
        && pseudos.is_empty()
        && sel != "*"
    {
        return None;
    }
    Some(Compound { tag, ids, classes, attrs, pseudos })
}

/// Parsea el interior de `[...]`: `name`, `name=val`, `name="val"`,
/// `name^=val`, `name$=val`, `name*=val`. Devuelve `None` si el formato
/// no encaja.
fn parse_attr_match(inner: &str) -> Option<AttrMatch> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    let ops: &[(&str, AttrOp)] = &[
        ("^=", AttrOp::Prefix),
        ("$=", AttrOp::Suffix),
        ("*=", AttrOp::Contains),
        ("=", AttrOp::Equals),
    ];
    for (sym, op) in ops {
        if let Some(pos) = inner.find(sym) {
            let name = inner[..pos].trim().to_string();
            if name.is_empty() {
                return None;
            }
            let raw = inner[pos + sym.len()..].trim();
            let value = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
            return Some(AttrMatch { name, op: *op, value });
        }
    }
    Some(AttrMatch {
        name: inner.to_string(),
        op: AttrOp::Present,
        value: String::new(),
    })
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                i += 2 + end + 2;
                continue;
            }
            break;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_declarations(css: &str, vars: &HashMap<String, String>) -> Vec<Decl> {
    // Cada decl separada por `;`. Detectamos `!important` recortando
    // el sufijo del value antes de pasarlo al parser de tipo. La
    // shorthand `border:` se expande inline a 1..3 decls atómicas.
    let mut out = Vec::new();
    for chunk in css.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        // Las declaraciones de variables (`--name: value`) ya se
        // recogieron en la pasada de `extract_root_vars`. Acá las
        // saltamos para no intentar parsearlas como propiedades reales.
        if prop.starts_with("--") {
            continue;
        }
        let value = value.trim();
        let (value, important) = match strip_important(value) {
            Some(stripped) => (stripped, true),
            None => (value, false),
        };
        // Sustituye `var(--name)` antes de parsear. `substitute_vars` es
        // cheap si el value no contiene `var(` (early-out al primer find).
        let substituted = substitute_vars(value, vars);
        let value = substituted.as_str();
        if prop.eq_ignore_ascii_case("border") {
            out.extend(parse_border_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("flex") {
            out.extend(parse_flex_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("outline") {
            out.extend(parse_outline_shorthand(value, important));
            continue;
        }
        if let Some(kind) = decl_kind_from_pair(prop, value) {
            out.push(Decl { kind, important });
        }
    }
    out
}

/// Si `value` termina en `!important` (con o sin espacios), devuelve la
/// porción antes del bang. Sino, `None`.
fn strip_important(value: &str) -> Option<&str> {
    let v = value.trim_end();
    if v.len() < "!important".len() {
        return None;
    }
    let tail = &v[v.len() - "!important".len()..];
    if tail.eq_ignore_ascii_case("!important") {
        Some(v[..v.len() - "!important".len()].trim_end())
    } else {
        None
    }
}

fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    match prop.to_ascii_lowercase().as_str() {
        "color" => parse_color(value).map(DeclKind::Color),
        "background-color" | "background" => parse_color(value).map(DeclKind::Background),
        "display" => parse_display(value).map(DeclKind::Display),
        "font-size" => parse_length_px(value).map(DeclKind::FontSize),
        "font-weight" => parse_weight(value).map(DeclKind::FontWeight),
        "margin" => parse_sides(value).map(DeclKind::Margin),
        "margin-top" => parse_length_px(value).map(DeclKind::MarginTop),
        "margin-right" => parse_length_px(value).map(DeclKind::MarginRight),
        "margin-bottom" => parse_length_px(value).map(DeclKind::MarginBottom),
        "margin-left" => parse_length_px(value).map(DeclKind::MarginLeft),
        "padding" => parse_sides(value).map(DeclKind::Padding),
        "padding-top" => parse_length_px(value).map(DeclKind::PaddingTop),
        "padding-right" => parse_length_px(value).map(DeclKind::PaddingRight),
        "padding-bottom" => parse_length_px(value).map(DeclKind::PaddingBottom),
        "padding-left" => parse_length_px(value).map(DeclKind::PaddingLeft),
        "width" => parse_length_or_pct(value).map(DeclKind::Width),
        "max-width" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "text-align" => parse_text_align(value).map(DeclKind::TextAlign),
        "line-height" => parse_line_height(value).map(DeclKind::LineHeight),
        "border-width" => parse_length_px(value).map(DeclKind::BorderWidth),
        "border-color" => parse_color(value).map(DeclKind::BorderColor),
        "border-style" => parse_border_style(value).map(DeclKind::BorderEnabled),
        "border-radius" => parse_length_px(value).map(DeclKind::BorderRadius),
        "box-shadow" => Some(DeclKind::BoxShadow(parse_box_shadow(value))),
        "text-decoration" | "text-decoration-line" => {
            parse_text_decoration(value).map(DeclKind::TextDecoration)
        }
        "list-style-type" => parse_list_style_type(value).map(DeclKind::ListStyleType),
        // `list-style` shorthand reducido: sólo capturamos el `-type`.
        // Image y position los ignoramos — `none` desactiva el marker
        // entero (matchea el comportamiento del browser).
        "list-style" => parse_list_style_shorthand(value).map(DeclKind::ListStyleType),
        "flex-direction" => parse_flex_direction(value).map(DeclKind::FlexDirection),
        "flex-wrap" => parse_flex_wrap(value).map(DeclKind::FlexWrap),
        "justify-content" => parse_justify_content(value).map(DeclKind::JustifyContent),
        "align-items" => parse_align_items(value).map(DeclKind::AlignItems),
        "gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        "box-sizing" => parse_box_sizing(value).map(DeclKind::BoxSizing),
        "min-width" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-height" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-height" => parse_length_or_pct(value).map(DeclKind::MaxHeight),
        "overflow" | "overflow-x" | "overflow-y" => {
            parse_overflow(value).map(DeclKind::Overflow)
        }
        "white-space" => parse_white_space(value).map(DeclKind::WhiteSpace),
        "text-transform" => parse_text_transform(value).map(DeclKind::TextTransform),
        "opacity" => parse_opacity(value).map(DeclKind::Opacity),
        "align-self" => parse_align_self(value).map(DeclKind::AlignSelf),
        "flex-grow" => value.trim().parse::<f32>().ok().map(DeclKind::FlexGrow),
        "flex-shrink" => value.trim().parse::<f32>().ok().map(DeclKind::FlexShrink),
        "flex-basis" => parse_length_or_pct(value).map(DeclKind::FlexBasis),
        // `flex` y `outline` son shorthands múltiples — se expanden en
        // `parse_declarations` antes de llegar acá.
        "flex" | "outline" => None,
        "outline-width" => parse_length_px(value).map(DeclKind::OutlineWidth),
        "outline-color" => parse_color(value).map(DeclKind::OutlineColor),
        "outline-style" => parse_border_style(value).map(DeclKind::OutlineStyle),
        "outline-offset" => parse_length_px(value).map(DeclKind::OutlineOffset),
        "background-image" => parse_background_image(value),
        // `border: 1px solid #ccc` — shorthand. Devolvemos un único
        // DeclKind sintético: en realidad ya hay 3 sub-decls que el
        // caller debe emitir, así que delegamos a una ruta especial vía
        // parse_declarations (ver más arriba). Acá no podemos producir
        // varios, así que ignoramos — la entrada se rellena en
        // parse_declarations cuando ve `border`.
        "border" => None,
        _ => None,
    }
}

/// Parsea el argumento de `:nth-child(...)`. Soporta:
/// - palabras clave: `odd` (= `2n+1`), `even` (= `2n`)
/// - número entero: `3` → `(0, 3)` (sólo la 3a)
/// - `n` → `(1, 0)` (todos), `-n` → `(-1, 0)`
/// - `an` → `(a, 0)`; `an+b` y `an-b` → `(a, ±b)`
/// - `-n+b` → `(-1, b)`
///
/// Devuelve `Some((a, b))` o `None` si el formato no encaja.
fn parse_nth_arg(arg: &str) -> Option<(i32, i32)> {
    let s: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.to_ascii_lowercase();
    if s == "odd" {
        return Some((2, 1));
    }
    if s == "even" {
        return Some((2, 0));
    }
    // Caso entero puro: "3" o "-3".
    if let Ok(n) = s.parse::<i32>() {
        return Some((0, n));
    }
    // Buscar la 'n' que separa coeficiente de constante.
    let n_pos = s.find('n')?;
    let coeff_str = &s[..n_pos];
    let rest = &s[n_pos + 1..];
    let a: i32 = match coeff_str {
        "" => 1,
        "-" => -1,
        "+" => 1,
        other => other.parse().ok()?,
    };
    let b: i32 = if rest.is_empty() { 0 } else { rest.parse().ok()? };
    Some((a, b))
}

/// Parsea `box-shadow: <offset-x> <offset-y> [blur] [spread] <color>`
/// o `box-shadow: none`. Devuelve `None` (= no-shadow) si:
/// - value es exactamente `none`, o
/// - falta el offset-x/offset-y, o
/// - no se reconoce el color.
///
/// `inset` y múltiples sombras separadas por coma no soportadas — el
/// resto del declaration se ignora silenciosamente.
fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return None;
    }
    // Toma sólo la primera sombra (si hay coma).
    let first = v.split(',').next().unwrap_or(v).trim();
    let mut lengths: Vec<f32> = Vec::with_capacity(4);
    let mut color: Option<Color> = None;
    for tok in first.split_whitespace() {
        if tok.eq_ignore_ascii_case("inset") {
            // No soportado todavía — abortamos.
            return None;
        }
        if let Some(l) = parse_length_px(tok) {
            lengths.push(l);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        spread_px: lengths.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::rgb(0, 0, 0)),
    })
}

fn parse_border_style(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" => Some(true),
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

/// Parsea el shorthand `border: <width> <style> <color>` (componentes en
/// cualquier orden). Devuelve hasta 3 decls. Si falta el style, se asume
/// `solid`. Cualquier "none" en la posición de style desactiva el border.
fn parse_border_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut style_on: Option<bool> = None;
    for tok in value.split_whitespace() {
        if let Some(w) = parse_length_px(tok) {
            width = Some(w);
            continue;
        }
        if let Some(c) = parse_color(tok) {
            color = Some(c);
            continue;
        }
        if let Some(s) = parse_border_style(tok) {
            style_on = Some(s);
            continue;
        }
    }
    // Defaults razonables: si hay width+color sin style, asumimos solid.
    if style_on.is_none() && (width.is_some() || color.is_some()) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderEnabled(on), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderWidth(w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderColor(c), important });
    }
    out
}

/// Parsea `text-decoration` o `text-decoration-line`. Acepta el shorthand
/// con varios tokens — busca el primer keyword reconocido como line y
/// devuelve eso. Estilos (`dotted`/`wavy`) y color se ignoran (sólo
/// pintamos línea sólida del color del texto).
fn parse_text_decoration(value: &str) -> Option<TextDecorationLine> {
    for tok in value.split_whitespace() {
        match tok.to_ascii_lowercase().as_str() {
            "none" => return Some(TextDecorationLine::None),
            "underline" => return Some(TextDecorationLine::Underline),
            "line-through" => return Some(TextDecorationLine::LineThrough),
            "overline" => return Some(TextDecorationLine::Overline),
            _ => {}
        }
    }
    None
}

/// Parsea `list-style-type: <keyword>`. Acepta los aliases comunes
/// (`lower-latin` = `lower-alpha`, `upper-latin` = `upper-alpha`).
/// Keywords no soportados (`georgian`, `hebrew`, …) caen a `None` y la
/// declaración se ignora — el caller mantiene el valor anterior.
fn parse_list_style_type(s: &str) -> Option<ListStyleType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(ListStyleType::None),
        "disc" => Some(ListStyleType::Disc),
        "circle" => Some(ListStyleType::Circle),
        "square" => Some(ListStyleType::Square),
        "decimal" => Some(ListStyleType::Decimal),
        "lower-alpha" | "lower-latin" => Some(ListStyleType::LowerAlpha),
        "upper-alpha" | "upper-latin" => Some(ListStyleType::UpperAlpha),
        "lower-roman" => Some(ListStyleType::LowerRoman),
        "upper-roman" => Some(ListStyleType::UpperRoman),
        _ => None,
    }
}

/// Shorthand `list-style: [type] [position] [image]` muy reducido. Sólo
/// extraemos el primer token que matchee un `-type` keyword. `list-style:
/// none` desactiva el marker (matchea browsers — `none` ahí setea ambos
/// `-type` e `-image` a none, y como no tenemos `-image`, alcanza con
/// poner `-type` en `None`).
fn parse_list_style_shorthand(s: &str) -> Option<ListStyleType> {
    for tok in s.split_whitespace() {
        if let Some(t) = parse_list_style_type(tok) {
            return Some(t);
        }
    }
    None
}

fn parse_text_align(s: &str) -> Option<TextAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        "justify" => Some(TextAlign::Justify),
        _ => None,
    }
}

/// Acepta `auto`, `Npx`, `Nrem`/`Nem` (→ px), `N%`. Sin unidad y
/// distinto de `0` → falla (a diferencia de `parse_length_px`, que
/// asume px).
fn parse_length_or_pct(s: &str) -> Option<LengthVal> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthVal::Auto);
    }
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    parse_length_px(s).map(LengthVal::Px)
}

/// Acepta multiplicador adimensional (`1.5`, `1.6`), `Npx`, `Nem`/`Nrem`.
/// Devuelve siempre un multiplicador (px se divide por 16; `em`/`rem`
/// salen como ya están). Imperfecto pero alcanza para Fase 4.
fn parse_line_height(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("px") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v / 16.0);
    }
    if let Some(num) = s.strip_suffix("rem") {
        return num.trim().parse().ok();
    }
    if let Some(num) = s.strip_suffix("em") {
        return num.trim().parse().ok();
    }
    s.parse::<f32>().ok()
}

fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    // hex #RRGGBB / #RGB / #RRGGBBAA / #RGBA
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            return Some(Color::rgb(r, g, b));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some(Color { r, g, b, a });
        }
        if hex.len() == 4 {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
            return Some(Color { r, g, b, a });
        }
    }
    // rgb()/rgba() — coma legacy o whitespace moderno, con alpha por
    // 4to arg o sufijo `/ alpha`.
    if let Some(args) = strip_fn(s, "rgba").or_else(|| strip_fn(s, "rgb")) {
        return parse_rgb_func(args);
    }
    if let Some(args) = strip_fn(s, "hsla").or_else(|| strip_fn(s, "hsl")) {
        return parse_hsl_func(args);
    }
    // Nombres comunes.
    NAMED_COLORS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, c)| *c)
}

/// Si `s` es de la forma `name(…)`, devuelve los argumentos crudos
/// (sin paréntesis). Tolera espacios entre el nombre y `(`. Match del
/// nombre case-insensitive.
fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let s = s.trim();
    if !s.get(..name.len())?.eq_ignore_ascii_case(name) {
        return None;
    }
    let rest = s[name.len()..].trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.trim())
}

/// Parsea los argumentos de `rgb(…)` o `rgba(…)`. Acepta sintaxis
/// legacy (separador coma, alpha como 4to arg) y moderna (whitespace
/// + `/ alpha`). Cada canal RGB tolera entero 0-255 o porcentaje. El
/// alpha tolera fracción 0-1 o porcentaje.
fn parse_rgb_func(args: &str) -> Option<Color> {
    let (rgb, alpha) = split_color_args(args)?;
    if rgb.len() != 3 {
        return None;
    }
    let r = parse_color_chan(rgb[0])?;
    let g = parse_color_chan(rgb[1])?;
    let b = parse_color_chan(rgb[2])?;
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Parsea `hsl(…)` / `hsla(…)`. H = grados (0-360, se wrappea), S/L =
/// porcentaje (0-100). Alpha igual que rgba.
fn parse_hsl_func(args: &str) -> Option<Color> {
    let (parts, alpha) = split_color_args(args)?;
    if parts.len() != 3 {
        return None;
    }
    let h = parse_hue(parts[0])?;
    let s = parse_pct(parts[1])?;
    let l = parse_pct(parts[2])?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    let a = match alpha {
        Some(a_str) => parse_alpha(a_str)?,
        None => 255,
    };
    Some(Color { r, g, b, a })
}

/// Tokeniza los args de un color function. Devuelve `(canales, alpha?)`.
/// Resuelve coma vs whitespace y la sintaxis moderna `r g b / a`.
fn split_color_args(args: &str) -> Option<(Vec<&str>, Option<&str>)> {
    let args = args.trim();
    // Sintaxis moderna: `R G B / A`. La barra separa el alpha.
    if let Some(slash) = args.find('/') {
        let main = args[..slash].trim();
        let alpha = args[slash + 1..].trim();
        let parts: Vec<&str> = main.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }
        return Some((parts, Some(alpha)));
    }
    // Legacy: comas separan TODO (incluido el alpha).
    if args.contains(',') {
        let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
        if parts.len() == 4 {
            let (rgb, a) = parts.split_at(3);
            return Some((rgb.to_vec(), Some(a[0])));
        }
        return Some((parts, None));
    }
    // Moderna sin alpha: solo whitespace.
    let parts: Vec<&str> = args.split_whitespace().collect();
    Some((parts, None))
}

/// Canal RGB: entero 0-255 o porcentaje 0%-100%.
fn parse_color_chan(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    s.parse::<i32>().ok().map(|n| n.clamp(0, 255) as u8)
}

/// Alpha: fracción 0.0-1.0 o porcentaje 0%-100%.
fn parse_alpha(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    let f: f32 = s.parse().ok()?;
    Some((f.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Hue: `Ndeg` o número crudo (grados implícitos). `Nrad`/`Nturn` no
/// soportados — caen a `None` y la función devuelve `None`.
fn parse_hue(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("deg").unwrap_or(s);
    s.trim().parse().ok()
}

/// Porcentaje 0%-100% → fracción 0.0-1.0.
fn parse_pct(s: &str) -> Option<f32> {
    let s = s.trim().strip_suffix('%')?;
    let pct: f32 = s.trim().parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

/// HSL→RGB estándar (CSS Color Module L3). h en grados, s/l en 0..1.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r1), to_u8(g1), to_u8(b1))
}

/// Parsea un value tipo `margin: <1..4 longitudes>`. Devuelve `None` si
/// algún token no es longitud válida o si hay menos de 1 / más de 4.
fn parse_sides(value: &str) -> Option<Sides<f32>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Vec<f32> = parts
        .iter()
        .map(|t| parse_length_px(t))
        .collect::<Option<Vec<_>>>()?;
    Some(match parsed.as_slice() {
        [a] => Sides::all(*a),
        [v, h] => Sides { top: *v, right: *h, bottom: *v, left: *h },
        [t, h, b] => Sides { top: *t, right: *h, bottom: *b, left: *h },
        [t, r, b, l] => Sides { top: *t, right: *r, bottom: *b, left: *l },
        _ => return None,
    })
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::BLACK),
    ("white", Color::WHITE),
    ("red", Color::rgb_const(255, 0, 0)),
    ("green", Color::rgb_const(0, 128, 0)),
    ("blue", Color::rgb_const(0, 0, 255)),
    ("gray", Color::rgb_const(128, 128, 128)),
    ("grey", Color::rgb_const(128, 128, 128)),
    ("silver", Color::rgb_const(192, 192, 192)),
    ("maroon", Color::rgb_const(128, 0, 0)),
    ("yellow", Color::rgb_const(255, 255, 0)),
    ("olive", Color::rgb_const(128, 128, 0)),
    ("lime", Color::rgb_const(0, 255, 0)),
    ("aqua", Color::rgb_const(0, 255, 255)),
    ("cyan", Color::rgb_const(0, 255, 255)),
    ("teal", Color::rgb_const(0, 128, 128)),
    ("navy", Color::rgb_const(0, 0, 128)),
    ("fuchsia", Color::rgb_const(255, 0, 255)),
    ("magenta", Color::rgb_const(255, 0, 255)),
    ("purple", Color::rgb_const(128, 0, 128)),
    ("orange", Color::rgb_const(255, 165, 0)),
    ("pink", Color::rgb_const(255, 192, 203)),
    ("brown", Color::rgb_const(165, 42, 42)),
    ("gold", Color::rgb_const(255, 215, 0)),
    ("indigo", Color::rgb_const(75, 0, 130)),
    ("violet", Color::rgb_const(238, 130, 238)),
    ("crimson", Color::rgb_const(220, 20, 60)),
    ("darkblue", Color::rgb_const(0, 0, 139)),
    ("darkgreen", Color::rgb_const(0, 100, 0)),
    ("darkred", Color::rgb_const(139, 0, 0)),
    ("darkgray", Color::rgb_const(169, 169, 169)),
    ("lightgray", Color::rgb_const(211, 211, 211)),
    ("lightblue", Color::rgb_const(173, 216, 230)),
    ("lightgreen", Color::rgb_const(144, 238, 144)),
    ("transparent", Color::TRANSPARENT),
];

fn parse_weight(s: &str) -> Option<u16> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "lighter" => Some(300),
        "bolder" => Some(700),
        num => num.parse().ok(),
    }
}

fn parse_display(s: &str) -> Option<Display> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "none" => Some(Display::None),
        _ => None,
    }
}

fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

fn parse_justify_content(s: &str) -> Option<JustifyContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" | "left" => Some(JustifyContent::Start),
        "center" => Some(JustifyContent::Center),
        "end" | "flex-end" | "right" => Some(JustifyContent::End),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" => Some(AlignItems::Start),
        "center" => Some(AlignItems::Center),
        "end" | "flex-end" => Some(AlignItems::End),
        "stretch" => Some(AlignItems::Stretch),
        "baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

/// `gap: V` ⇒ row=V, column=V. `gap: R C` ⇒ row=R, column=C. Coincide
/// con la semántica CSS shorthand (primer valor = row, segundo = column).
fn parse_gap(value: &str) -> Option<(f32, f32)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.as_slice() {
        [v] => {
            let v = parse_length_px(v)?;
            Some((v, v))
        }
        [r, c] => Some((parse_length_px(r)?, parse_length_px(c)?)),
        _ => None,
    }
}

fn parse_box_sizing(s: &str) -> Option<BoxSizing> {
    match s.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

fn parse_overflow(s: &str) -> Option<Overflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        // hidden/clip/auto/scroll todos los tratamos como Hidden por
        // ahora (no soportamos scroll real; clip y hidden cortan igual).
        "hidden" | "clip" | "auto" | "scroll" => Some(Overflow::Hidden),
        _ => None,
    }
}

fn parse_white_space(s: &str) -> Option<WhiteSpace> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "nowrap" => Some(WhiteSpace::NoWrap),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None,
    }
}

fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None,
    }
}

/// Acepta `0..1` o `0%..100%`. Clampa.
fn parse_opacity(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct / 100.0).clamp(0.0, 1.0));
    }
    s.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
}

fn parse_align_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(AlignSelf::Auto),
        "start" | "flex-start" => Some(AlignSelf::Start),
        "center" => Some(AlignSelf::Center),
        "end" | "flex-end" => Some(AlignSelf::End),
        "stretch" => Some(AlignSelf::Stretch),
        "baseline" => Some(AlignSelf::Baseline),
        _ => None,
    }
}

/// `flex: <grow> [<shrink>] [<basis>]`. Casos especiales:
/// - `flex: none` → `0 0 auto`
/// - `flex: auto` → `1 1 auto`
/// - `flex: <number>` → `N 1 0%` (basis 0%, common preset)
/// Devuelve 3 decls atómicas (grow + shrink + basis).
fn parse_flex_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let v = value.trim().to_ascii_lowercase();
    let (grow, shrink, basis) = if v == "none" {
        (0.0_f32, 0.0_f32, LengthVal::Auto)
    } else if v == "auto" {
        (1.0_f32, 1.0_f32, LengthVal::Auto)
    } else if v == "initial" {
        (0.0_f32, 1.0_f32, LengthVal::Auto)
    } else {
        let parts: Vec<&str> = value.split_whitespace().collect();
        match parts.as_slice() {
            [g] => {
                // `flex: 1` ⇒ `1 1 0%`
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                (g, 1.0, LengthVal::Pct(0.0))
            }
            [g, s_or_b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                // El segundo puede ser shrink (número solo) o basis (longitud).
                if let Some(b) = parse_length_or_pct(s_or_b) {
                    (g, 1.0, b)
                } else if let Some(s) = s_or_b.parse::<f32>().ok() {
                    (g, s, LengthVal::Pct(0.0))
                } else {
                    return Vec::new();
                }
            }
            [g, s, b] => {
                let Some(g) = g.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(s) = s.parse::<f32>().ok() else {
                    return Vec::new();
                };
                let Some(b) = parse_length_or_pct(b) else {
                    return Vec::new();
                };
                (g, s, b)
            }
            _ => return Vec::new(),
        }
    };
    vec![
        Decl { kind: DeclKind::FlexGrow(grow), important },
        Decl { kind: DeclKind::FlexShrink(shrink), important },
        Decl { kind: DeclKind::FlexBasis(basis), important },
    ]
}

/// `outline: <width> <style> <color>`. Tokens en cualquier orden.
fn parse_outline_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut width: Option<f32> = None;
    let mut color: Option<Color> = None;
    let mut style_active: Option<bool> = None;
    for tok in value.split_whitespace() {
        if width.is_none() {
            if let Some(w) = parse_length_px(tok) {
                width = Some(w);
                continue;
            }
        }
        if style_active.is_none() {
            if let Some(active) = parse_border_style(tok) {
                style_active = Some(active);
                continue;
            }
        }
        if color.is_none() {
            if let Some(c) = parse_color(tok) {
                color = Some(c);
                continue;
            }
        }
    }
    let mut out = Vec::new();
    let active = style_active.unwrap_or(true);
    if !active {
        // `outline-style: none` apaga: width=0 + color=None.
        out.push(Decl { kind: DeclKind::OutlineStyle(false), important });
        return out;
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::OutlineWidth(w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::OutlineColor(c), important });
    }
    if style_active.is_some() {
        out.push(Decl { kind: DeclKind::OutlineStyle(true), important });
    }
    out
}

/// `background-image: linear-gradient(...)` o `none`. Devuelve un
/// `DeclKind` listo (Background o BackgroundGradient o None).
fn parse_background_image(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::BackgroundGradientNone);
    }
    if let Some(args) = strip_fn(v, "linear-gradient") {
        return parse_linear_gradient(args).map(DeclKind::BackgroundGradient);
    }
    // url(...) y otros gradientes no soportados — silencio.
    None
}

/// Parsea el contenido de `linear-gradient(...)`. Sintaxis aceptada:
/// - `linear-gradient(<angle>?, <stop>, <stop>, ...)`
/// - `linear-gradient(to <side>?, <stop>, <stop>, ...)`
/// `<angle>` en `Ndeg` o `Nturn` (turn × 360 = grados). Default 180
/// (top→bottom). `to right`=90, `to left`=270, `to top`=0, `to bottom`=180,
/// combinaciones diagonales (`to top right`=45) también. Stops: `<color>
/// <pos>?` donde pos es `N%` o `Npx`.
fn parse_linear_gradient(args: &str) -> Option<LinearGradient> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    if parts.len() < 2 {
        return None;
    }
    let (angle_deg, stops_start) = parse_gradient_direction(parts[0]);
    let stops_start_idx = if angle_deg.is_some() { 1 } else { 0 };
    let angle_deg = angle_deg.unwrap_or(180.0);
    let mut stops: Vec<GradientStop> = Vec::new();
    for raw in &parts[stops_start_idx..] {
        if let Some(s) = parse_gradient_stop(raw) {
            stops.push(s);
        }
    }
    if stops.len() < 2 {
        return None;
    }
    let _ = stops_start;
    Some(LinearGradient { angle_deg, stops })
}

/// Si el token es una dirección/ángulo válido devuelve `(Some(deg),
/// true)`; si no encaja, `(None, false)` para que el caller lo trate
/// como stop.
fn parse_gradient_direction(s: &str) -> (Option<f32>, bool) {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("to ") {
        let deg = match rest.trim() {
            "top" => 0.0,
            "right" => 90.0,
            "bottom" => 180.0,
            "left" => 270.0,
            "top right" | "right top" => 45.0,
            "bottom right" | "right bottom" => 135.0,
            "bottom left" | "left bottom" => 225.0,
            "top left" | "left top" => 315.0,
            _ => return (None, false),
        };
        return (Some(deg), true);
    }
    if let Some(num) = lower.strip_suffix("deg") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v), true);
        }
    }
    if let Some(num) = lower.strip_suffix("turn") {
        if let Ok(v) = num.trim().parse::<f32>() {
            return (Some(v * 360.0), true);
        }
    }
    (None, false)
}

fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.as_slice() {
        [c] => Some(GradientStop { color: parse_color(c)?, pos: None }),
        [c, p] => {
            let color = parse_color(c)?;
            let pos = if let Some(pct) = p.strip_suffix('%') {
                pct.trim().parse::<f32>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0))
            } else if let Some(px) = parse_length_px(p) {
                // Aproximación: tratamos px como 0..1 dividiendo por 100.
                // En el wild la mayoría usa %, así que esta heurística
                // raramente importa.
                Some((px / 100.0).clamp(0.0, 1.0))
            } else {
                None
            };
            Some(GradientStop { color, pos })
        }
        _ => None,
    }
}

/// Acepta `12px`, `1.5rem` (tratada como em*16), `0`. Sin unidad → px.
fn parse_length_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s == "0" {
        return Some(0.0);
    }
    if let Some(num) = s.strip_suffix("px") {
        return num.trim().parse().ok();
    }
    if let Some(num) = s.strip_suffix("rem") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * 16.0);
    }
    if let Some(num) = s.strip_suffix("em") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * 16.0);
    }
    s.parse().ok()
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
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
    fn parsea_length() {
        assert_eq!(parse_length_px("12px"), Some(12.0));
        assert_eq!(parse_length_px("1.5em"), Some(24.0));
        assert_eq!(parse_length_px("0"), Some(0.0));
        assert_eq!(parse_length_px("xyz"), None);
    }

    #[test]
    fn parsea_regla_simple() {
        let rules = parse_stylesheet("p { color: red; font-size: 14px; }", &HashMap::new());
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
        // anchors[0] tiene class="btn"
        assert_eq!(eng.compute(&anchors[0]).color, Color::rgb(255, 0, 0));
        // anchors[1] sin class
        assert_eq!(eng.compute(&anchors[1]).color, Color::BLACK);
        // span.btn no es <a>
        assert_eq!(eng.compute(&spans[0]).color, Color::BLACK);
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
        // a[href] → rojo; a sin href → negro; span → negro
        assert_eq!(eng.compute(&elems[0]).color, Color::rgb(255, 0, 0));
        assert_eq!(eng.compute(&elems[1]).color, Color::BLACK);
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
        assert_eq!(eng.compute(&anchors[1]).color, Color::BLACK);
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
        );
        assert!(matches!(s[0].decls[0].kind, DeclKind::TextAlign(TextAlign::Center)));
        assert!(matches!(s[1].decls[0].kind, DeclKind::TextAlign(TextAlign::Right)));
    }

    #[test]
    fn parsea_line_height() {
        let s = parse_stylesheet(
            "p { line-height: 1.5 } h1 { line-height: 32px }",
            &HashMap::new(),
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
        assert!((a.border_width - 2.0).abs() < 1e-6);
        assert_eq!(a.border_color, Some(Color::rgb(255, 0, 0)));
        let b = eng.compute(&divs[1]);
        assert!((b.border_width - 1.0).abs() < 1e-6);
        assert_eq!(b.border_color, Some(Color::rgb(0, 0, 255)));
        let c = eng.compute(&divs[2]);
        assert_eq!(c.border_color, None); // `none` deshabilita
        assert!((c.border_width - 0.0).abs() < 1e-6);
        let d = eng.compute(&divs[3]);
        assert!((d.border_radius - 8.0).abs() < 1e-6);
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
        assert!((st.border_width - 3.0).abs() < 1e-6);
        assert_eq!(st.border_color, Some(Color::rgb(0, 0xff, 0)));
        assert!((st.border_radius - 5.0).abs() < 1e-6);
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
        let a = eng.compute(&divs[0]).box_shadow.unwrap();
        assert!((a.offset_x - 2.0).abs() < 1e-6);
        assert!((a.offset_y - 4.0).abs() < 1e-6);
        assert!((a.blur_px - 8.0).abs() < 1e-6);
        assert!((a.spread_px - 1.0).abs() < 1e-6);
        assert_eq!(a.color, Color::BLACK);
        let b = eng.compute(&divs[1]).box_shadow.unwrap();
        assert_eq!(b.color, Color::rgb(255, 0, 0));
        assert!((b.blur_px - 0.0).abs() < 1e-6);
        assert!((b.spread_px - 0.0).abs() < 1e-6);
        let c = eng.compute(&divs[2]).box_shadow;
        assert!(c.is_none());
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
    fn parsea_list_style_shorthand() {
        // Cuando aparece un keyword reconocido, se captura.
        assert_eq!(parse_list_style_shorthand("square inside"), Some(ListStyleType::Square));
        assert_eq!(parse_list_style_shorthand("none"), Some(ListStyleType::None));
        // Sin keywords reconocibles, devolvemos None y el caller mantiene
        // el valor anterior.
        assert_eq!(parse_list_style_shorthand("url(foo.png)"), None);
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
        assert!((s.border_width - 3.0).abs() < 1e-6);
        assert_eq!(s.border_color, Some(Color::rgb(255, 165, 0)));
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
        assert!((g.angle_deg - 90.0).abs() < 1e-6);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[0].color, Color::rgb(255, 0, 0));
        assert_eq!(g.stops[1].color, Color::rgb(0, 0, 255));

        let g = parse_linear_gradient("45deg, red 0%, blue 100%").unwrap();
        assert!((g.angle_deg - 45.0).abs() < 1e-6);
        assert_eq!(g.stops[0].pos, Some(0.0));
        assert_eq!(g.stops[1].pos, Some(1.0));

        // Default 180 (top→bottom) cuando no se da dirección.
        let g = parse_linear_gradient("red, blue").unwrap();
        assert!((g.angle_deg - 180.0).abs() < 1e-6);
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
}
