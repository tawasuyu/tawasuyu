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
        assert!(parse_length_or_pct("calc(10px * 2)").is_none());
        assert!(parse_length_or_pct("calc(10px").is_none());
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
