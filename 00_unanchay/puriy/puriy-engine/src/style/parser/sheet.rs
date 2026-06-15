//! Parsing a nivel hoja: `parse_stylesheet`/`parse_rules_block`, at-rules
//! (`@media`/`@keyframes`/`@import`/`@supports`), UA stylesheet y defaults por
//! tag, `var()` substitution, `@keyframes`, animation/transition, y helpers de
//! split top-level. Sub-módulo de `parser` (regla #1). `use super::*`.
use super::*;

pub(crate) fn default_display(tag: &str) -> Display {
    match tag {
        "html" | "body" | "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol"
        | "li" | "header" | "footer" | "section" | "article" | "nav" | "main" | "aside"
        | "form" | "pre" | "blockquote" | "hr" | "figure" | "figcaption" | "details"
        | "summary" | "dialog" | "menu" | "address" | "fieldset" | "legend" | "dl" | "dd"
        | "dt" | "caption" => Display::Block,
        // Tables — semánticamente correctos serían display-table-*, pero
        // tratamos tr como flex-row, td/th como inline-block para que
        // la grilla se rinda razonablemente sin un layout engine de
        // tables completo.
        "table" | "thead" | "tbody" | "tfoot" => Display::Block,
        // `<colgroup>` y `<col>` son metadatos de columna en la spec
        // CSS table layout, NO se renderean como cajas propias — su rol
        // es definir width de columnas (que acá no soportamos). Ocultar
        // evita que tablas con esos elementos muestren espacios fantasma.
        "colgroup" | "col" => Display::None,
        "tr" => Display::Flex,
        "td" | "th" => Display::InlineBlock,
        // Form widgets: inline-block para que respeten width/height
        // pero no rompan el row del padre.
        "button" | "select" | "textarea" | "label" => Display::InlineBlock,
        "head" | "title" | "style" | "script" | "meta" | "link" => Display::None,
        // `<option>` / `<optgroup>`: el chrome los recolecta en
        // `SelectInfo` cuando ve un `<select>` padre y los renderea
        // como popup. Como hijos directos del DOM serían texto suelto.
        "option" | "optgroup" => Display::None,
        // `<svg>`: lo tratamos como inline-block — el engine recolecta
        // las primitivas (rect/circle/line) en `BoxNode.svg` y el chrome
        // las pinta. Sus descendientes (los `<rect>`/`<path>`/etc.) NO
        // entran al box tree.
        "svg" => Display::InlineBlock,
        // `<canvas>`: inline-block dimensionado por sus atributos
        // `width`/`height` (default 300×150 por spec). El engine marca el
        // `BoxNode.canvas` con el tamaño intrínseco y el chrome drena los
        // comandos 2D del runtime JS para pintarlos con vello (Fase 7.196).
        // Sus hijos (contenido de fallback) NO entran al box tree porque
        // soportamos canvas.
        "canvas" => Display::InlineBlock,
        // `<iframe>` no tiene engine de sub-página todavía, pero
        // mostrarlo como block placeholder (border + label con la URL)
        // es mejor que ocultarlo — el lector ve QUE hay contenido
        // embebido y dónde apunta. El placeholder lo arma boxes.
        "iframe" => Display::Block,
        // math/video/audio/object/embed: sin renderer todavía.
        // Ocultos para no derramar texto basura en la página.
        "math" | "video" | "audio" | "object" | "embed" => Display::None,
        _ => Display::Inline,
    }
}

/// `true` si el tag se oculta por defecto en la hoja UA (`script`/`style`/
/// `head`/`option`/`colgroup`/`canvas`/...). Lo usa `boxes::build_node` para
/// distinguir el `display:none` "de ruido UA" (que se descarta del box tree)
/// del puesto por el autor (que se retiene como box oculto, para poder
/// mostrarlo con un toggle de clase vía restyle). Fase 7.185.
pub(crate) fn tag_defaults_to_none(tag: &str) -> bool {
    default_display(tag) == Display::None
}

pub(crate) fn default_weight(tag: &str) -> u16 {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "b" | "strong" | "th" => 700,
        _ => 400,
    }
}

/// Tags que el UA stylesheet pone en italic por default (CSS spec).
pub(crate) fn default_italic(tag: &str) -> bool {
    matches!(
        tag,
        "em" | "i" | "cite" | "dfn" | "var" | "address" | "blockquote"
    )
}

/// UA stylesheet mínimo — defaults HTML5 que cssparser por sí solo no
/// inyecta. Mantén corto: sólo lo necesario para no devolver páginas
/// "blancas" sin reglas autor.
pub(crate) fn ua_stylesheet() -> Vec<Rule> {
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
            pseudo_element: None,
        }
    }
    fn decl(kind: DeclKind) -> Decl {
        Decl { kind, important: false }
    }
    fn sides_lrtb(t: f32, r: f32, b: f32, l: f32) -> Sides<f32> {
        Sides { top: t, right: r, bottom: b, left: l }
    }
    // Tamaños y márgenes de heading siguen el patrón de Firefox / Chrome
    // (em-based, redondeado a px sobre font-size 16). h1 sólo dentro del
    // primer `<section>`/`<article>` sería 1.5em según spec, pero ese
    // matching contextual queda para más adelante — usamos 2em fijo.
    vec![
        Rule {
            layer: None,
            selector: ty("body"),
            decls: vec![
                // Browser real default es `margin: 8px` (no padding). Lo
                // dejamos así para que páginas sin CSS no queden pegadas
                // al borde de la ventana.
                decl(DeclKind::Margin(Sides::all(8.0))),
                // CSS spec default es `font-family: serif`. Browsers
                // mapean "serif" a Times New Roman, Georgia, etc. según
                // el sistema. `parley::FontStack::Source("serif")` ya
                // delega esa resolución a la system font config.
                decl(DeclKind::FontFamily("serif".to_string())),
            ],
        },
        Rule {
            layer: None,
            selector: ty("h1"),
            decls: vec![
                decl(DeclKind::FontSize(32.0)),
                decl(DeclKind::Margin(sides_lrtb(21.0, 0.0, 21.0, 0.0))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("h2"),
            decls: vec![
                decl(DeclKind::FontSize(24.0)),
                decl(DeclKind::Margin(sides_lrtb(19.0, 0.0, 19.0, 0.0))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("h3"),
            decls: vec![
                decl(DeclKind::FontSize(19.0)),
                decl(DeclKind::Margin(sides_lrtb(19.0, 0.0, 19.0, 0.0))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("h4"),
            decls: vec![
                decl(DeclKind::FontSize(16.0)),
                decl(DeclKind::Margin(sides_lrtb(21.0, 0.0, 21.0, 0.0))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("h5"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::Margin(sides_lrtb(22.0, 0.0, 22.0, 0.0))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("h6"),
            decls: vec![
                decl(DeclKind::FontSize(11.0)),
                decl(DeclKind::Margin(sides_lrtb(25.0, 0.0, 25.0, 0.0))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("p"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0)))],
        },
        // Listas: padding-left para los bullets/numerales (el marker se
        // pinta antes del contenido, necesita espacio para no chocar
        // con el borde izquierdo del block).
        Rule {
            layer: None,
            selector: ty("ul"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::Padding(sides_lrtb(0.0, 0.0, 0.0, 40.0))),
                decl(DeclKind::ListStyleType(ListStyleType::Disc)),
            ],
        },
        Rule {
            layer: None,
            selector: ty("ol"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::Padding(sides_lrtb(0.0, 0.0, 0.0, 40.0))),
                decl(DeclKind::ListStyleType(ListStyleType::Decimal)),
            ],
        },
        Rule {
            layer: None,
            selector: ty("blockquote"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(10.0, 40.0, 10.0, 40.0)))],
        },
        Rule {
            layer: None,
            selector: ty("dl"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0)))],
        },
        Rule {
            layer: None,
            selector: ty("dd"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(0.0, 0.0, 0.0, 40.0)))],
        },
        Rule {
            layer: None,
            selector: ty("pre"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::WhiteSpace(WhiteSpace::Pre)),
            ],
        },
        Rule {
            layer: None,
            selector: ty("hr"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(192, 192, 192))),
                decl(DeclKind::BorderEnabled(true)),
            ],
        },
        // Color por defecto de los links — azul clásico de navegadores.
        // Esto se cascadea bajo el override del chrome que pinta links
        // con un blue ligeramente más oscuro (30,90,200).
        Rule {
            layer: None,
            selector: ty("a"),
            decls: vec![
                decl(DeclKind::Color(Color::rgb(0, 0, 238))),
                decl(DeclKind::TextDecoration(TextDecorationLine::Underline)),
            ],
        },
        // Defaults de text-decoration. `<a>` y `<u>`/`<ins>` van con
        // underline; `<s>`/`<strike>`/`<del>` tachadas. Cualquier autor
        // puede override con `text-decoration: none` en su stylesheet.
        Rule {
            layer: None,
            selector: ty("a"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::Underline),
                important: false,
            }],
        },
        Rule {
            layer: None,
            selector: ty("u"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::Underline))],
        },
        Rule {
            layer: None,
            selector: ty("ins"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::Underline))],
        },
        Rule {
            layer: None,
            selector: ty("s"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            layer: None,
            selector: ty("strike"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            layer: None,
            selector: ty("del"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            layer: None,
            selector: ty("menu"),
            decls: vec![decl(DeclKind::ListStyleType(ListStyleType::Disc))],
        },
        // Tables: bordes celulares mínimos para que la grilla se vea sin
        // CSS de autor. Browsers reales no dibujan bordes hasta que un
        // stylesheet lo pida, pero acá preferimos mostrarlos por default
        // — la mayoría de páginas con `<table>` sin estilo asumen un
        // "look spreadsheet" y tablas sin bordes salen invisibles.
        Rule {
            layer: None,
            selector: ty("table"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0)))],
        },
        Rule {
            layer: None,
            selector: ty("td"),
            decls: vec![
                decl(DeclKind::Padding(Sides::all(4.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(204, 204, 204))),
                decl(DeclKind::BorderEnabled(true)),
            ],
        },
        Rule {
            layer: None,
            selector: ty("th"),
            decls: vec![
                decl(DeclKind::Padding(Sides::all(4.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(204, 204, 204))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::rgb(242, 242, 242))),
            ],
        },
        // `<caption>` es el título de la tabla — centrado encima de las
        // filas. Sin esto el caption queda alineado a la izquierda
        // como cualquier block.
        Rule {
            layer: None,
            selector: ty("caption"),
            decls: vec![
                decl(DeclKind::TextAlign(TextAlign::Center)),
                decl(DeclKind::Padding(Sides::all(4.0))),
            ],
        },
        // `<iframe>` placeholder: border gris discreto + padding +
        // margin vertical para que se distinga del flujo. El label
        // con la URL lo inyecta `boxes::build_node`.
        Rule {
            layer: None,
            selector: ty("iframe"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0))),
                decl(DeclKind::Padding(Sides::all(8.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(180, 180, 180))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::rgb(248, 248, 248))),
                decl(DeclKind::Color(Color::rgb(100, 100, 100))),
            ],
        },
        // <small>/<sub>/<sup>: tamaño relativo. CSS spec usa `smaller`
        // (~83% del padre). Acá usamos 13px como aproximación.
        Rule {
            layer: None,
            selector: ty("small"),
            decls: vec![decl(DeclKind::FontSize(13.0))],
        },
        Rule {
            layer: None,
            selector: ty("sub"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::VerticalAlign(VerticalAlign::Sub)),
            ],
        },
        Rule {
            layer: None,
            selector: ty("sup"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::VerticalAlign(VerticalAlign::Super)),
            ],
        },
        Rule {
            layer: None,
            selector: ty("button"),
            decls: vec![
                decl(DeclKind::Padding(Sides { top: 1.0, right: 6.0, bottom: 1.0, left: 6.0 })),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(118, 118, 118))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::rgb(239, 239, 239))),
            ],
        },
        Rule {
            layer: None,
            selector: ty("input"),
            decls: vec![
                decl(DeclKind::Padding(Sides { top: 1.0, right: 2.0, bottom: 1.0, left: 2.0 })),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(118, 118, 118))),
                decl(DeclKind::BorderEnabled(true)),
                decl(DeclKind::Background(Color::WHITE)),
            ],
        },
    ]
}

// ----- parser -----
//
// Para Fase 2 no usamos cssparser AtRule/QualifiedRule (su API rotó
// entre 0.33→0.35 y nuestro subset cabe en 30 líneas). Si Fase 3 mete
// nesting / `@media` / `!important`, migrar a `cssparser::StyleSheetParser`
// con un visitor.

/// Registro de orden de capas de cascada (CSS Cascade Layers). Asigna a cada
/// nombre de capa cualificado (`a`, `a.b`) un índice incremental por orden de
/// PRIMERA aparición — sea por `@layer a, b;` (statement de predeclaración) o
/// por `@layer a { ... }` (bloque). Capas anónimas reciben un índice fresco.
/// Fase 7.1214.
#[derive(Default)]
pub(crate) struct LayerReg {
    order: HashMap<String, u32>,
    next: u32,
    anon: u32,
}
impl LayerReg {
    fn resolve(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.order.get(name) {
            return i;
        }
        let i = self.next;
        self.next += 1;
        self.order.insert(name.to_string(), i);
        i
    }
    fn anon(&mut self) -> u32 {
        self.anon += 1;
        let key = format!("\u{0}anon{}", self.anon);
        self.resolve(&key)
    }
}

/// Compone el nombre cualificado de una capa anidada: `prefix` + "." + `name`.
fn qualify_layer(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(crate) fn parse_stylesheet(css: &str, vars: &HashMap<String, String>, vp: Viewport) -> Vec<Rule> {
    let css = strip_comments(css);
    let mut reg = LayerReg::default();
    parse_rules_block(&css, vars, vp, None, "", &mut reg)
}

/// Parsea un bloque de reglas — el cuerpo de un stylesheet completo o
/// el contenido de un `@media` / `@supports`. Soporta:
/// - reglas normales `selector { decls }`
/// - `@media (condition) { ... }` recursivo — eval contra `viewport`
/// - `@supports (prop: value) { ... }` recursivo — eval por parser
/// - `@-rules` desconocidos (`@font-face`, `@keyframes`, etc.) los
///   saltea silenciosamente
pub(crate) fn parse_rules_block(
    css: &str,
    vars: &HashMap<String, String>,
    viewport: Viewport,
    layer: Option<u32>,
    layer_prefix: &str,
    reg: &mut LayerReg,
) -> Vec<Rule> {
    let mut out = Vec::new();
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Salta whitespace inicial.
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Detecta @-rule.
        if bytes[i] == b'@' {
            let rest = &css[i..];
            let Some(rule_end) = at_rule_end(rest) else {
                break;
            };
            let chunk = &rest[..rule_end];
            i += rule_end;
            // Distinguimos at-rules con bloque `{...}` vs at-rules statement
            // que terminan en `;` (ej: @import, @charset).
            let lower = chunk.trim_start().to_ascii_lowercase();
            if let Some(rest_after) = lower.strip_prefix("@media") {
                let cond = parse_at_rule_condition(chunk, "@media");
                let body = parse_at_rule_body(chunk);
                if evaluate_media_query(cond, viewport) {
                    out.extend(parse_rules_block(body, vars, viewport, layer, layer_prefix, reg));
                }
                let _ = rest_after;
                continue;
            }
            if lower.starts_with("@supports") {
                let cond = parse_at_rule_condition(chunk, "@supports");
                let body = parse_at_rule_body(chunk);
                if evaluate_supports_query(cond) {
                    out.extend(parse_rules_block(body, vars, viewport, layer, layer_prefix, reg));
                }
                continue;
            }
            // `@layer` (CSS Cascade Layers) — Fase 7.1214: ahora SÍ modelamos el
            // orden de capa. Dos formas:
            //  - Statement `@layer a, b, c;` (sin `{`): sólo predeclara el orden
            //    de las capas (resuelve cada nombre, asignando índice incremental).
            //  - Bloque `@layer [name] { ... }`: las reglas internas se tagean con
            //    el índice de esa capa; un `@layer` anónimo recibe un índice fresco.
            // El orden de capa se respeta en la cascada (ver `compute_internal`):
            // unlayered gana a layered para normales, y pierde para `!important`.
            if lower.starts_with("@layer") {
                // Nombre(s) entre `@layer` y `{` o `;`.
                let head_end = chunk.find('{').unwrap_or(chunk.len());
                let names_str = chunk[..head_end]
                    .trim_start()
                    .strip_prefix("@layer")
                    .or_else(|| chunk[..head_end].trim_start().get(6..))
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if chunk.contains('{') {
                    // Bloque: una sola capa (la primera de la lista, si hubiera).
                    let name = names_str.split(',').next().unwrap_or("").trim();
                    let (idx, prefix) = if name.is_empty() {
                        (reg.anon(), layer_prefix.to_string())
                    } else {
                        let q = qualify_layer(layer_prefix, name);
                        (reg.resolve(&q), q)
                    };
                    let body = parse_at_rule_body(chunk);
                    out.extend(parse_rules_block(body, vars, viewport, Some(idx), &prefix, reg));
                } else {
                    // Statement: predeclara orden de cada nombre.
                    for name in names_str.split(',') {
                        let name = name.trim();
                        if !name.is_empty() {
                            reg.resolve(&qualify_layer(layer_prefix, name));
                        }
                    }
                }
                continue;
            }
            // Fase 7.936 — `@container [name] (cond) { ... }` (Container
            // Queries) y `@scope (root) [to (limit)] { ... }` (CSS Scoping):
            // NO evaluamos la condición de contenedor ni la proximidad del
            // scope (no hay layout de contenedor ni árbol de scoping acá),
            // pero APLANAMOS sus reglas igual que `@layer`/`@media` verdadero
            // — estrictamente mejor que dropearlas (sino se pierde todo el CSS
            // dentro de un container query). Las reglas internas pueden usar
            // `:scope` (inerte=true) y `&` (nesting), ya soportados. Heredan la
            // capa del contexto.
            if lower.starts_with("@container") || lower.starts_with("@scope") {
                if chunk.contains('{') {
                    let body = parse_at_rule_body(chunk);
                    out.extend(parse_rules_block(body, vars, viewport, layer, layer_prefix, reg));
                }
                continue;
            }
            // `@starting-style` y `@position-try` NO se aplanan: sus reglas sólo
            // valen en transiciones de aparición / fallback de anchor — aplicarlas
            // en estado estable sería incorrecto. Se saltan (drop correcto).
            // @-rule desconocido: lo saltamos sin parsear.
            continue;
        }
        // Regla normal: `selector { body }`. El body puede contener reglas
        // anidadas (CSS Nesting) además de declaraciones.
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        i += brace + 1;
        let Some(close) = matching_close_brace(&css[i..]) else { break };
        let body = &css[i..i + close];
        i += close + 1;
        if sel_raw.is_empty() {
            continue;
        }
        emit_style_rules(sel_raw, body, &mut out, vars, viewport, layer);
    }
    out
}

/// Emite las reglas de un bloque `prelude { body }`, soportando **CSS
/// Nesting**: el `body` puede mezclar declaraciones y reglas anidadas. Una
/// regla anidada se expande contra cada selector del padre — `&` se
/// sustituye por el selector padre, y una regla sin `&` recibe un
/// combinador descendiente implícito (`.a { .b {} }` ≡ `.a .b`). Recursivo
/// (soporta anidamiento de cualquier profundidad). Ver Fase 7.218.
pub(crate) fn emit_style_rules(
    prelude: &str,
    body: &str,
    out: &mut Vec<Rule>,
    vars: &HashMap<String, String>,
    viewport: Viewport,
    layer: Option<u32>,
) {
    let (decl_str, nested) = split_rule_body(body);
    let parents: Vec<&str> = split_top_level_commas(prelude);
    // Declaraciones de este nivel.
    let decls = parse_declarations(&decl_str, vars);
    for p in &parents {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(selector) = parse_selector(p) {
            out.push(Rule { selector, decls: decls.clone(), layer });
        }
    }
    // Reglas anidadas: expandir `&` contra cada selector padre.
    for (nprelude, nbody) in &nested {
        let nprelude = nprelude.trim();
        // At-rules anidados (`@media`, etc.) no soportados en nesting todavía.
        if nprelude.is_empty() || nprelude.starts_with('@') {
            continue;
        }
        let mut expanded: Vec<String> = Vec::new();
        for p in &parents {
            for ns in split_top_level_commas(nprelude) {
                expanded.push(expand_amp(p.trim(), ns.trim()));
            }
        }
        emit_style_rules(&expanded.join(", "), nbody, out, vars, viewport, layer);
    }
}

/// Separa el body de una regla en (declaraciones, reglas anidadas). Una
/// regla anidada es un `prelude { ... }` a profundidad 0; el resto son
/// declaraciones (terminadas en `;` o el remanente final).
pub(crate) fn split_rule_body(body: &str) -> (String, Vec<(String, String)>) {
    let mut decls = String::new();
    let mut nested: Vec<(String, String)> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut seg_start = 0;
    while i < body.len() {
        match bytes[i] {
            b';' => {
                decls.push_str(&body[seg_start..=i]);
                i += 1;
                seg_start = i;
            }
            b'{' => {
                let nprelude = body[seg_start..i].trim().to_string();
                let after = &body[i + 1..];
                let close = matching_close_brace(after).unwrap_or(after.len());
                let nbody = after[..close].to_string();
                nested.push((nprelude, nbody));
                i = i + 1 + close + 1;
                seg_start = i;
            }
            _ => i += 1,
        }
    }
    let tail = body[seg_start..].trim();
    if !tail.is_empty() {
        decls.push_str(tail);
    }
    (decls, nested)
}

/// Expande un selector anidado contra su padre (CSS Nesting). Si contiene
/// `&`, lo sustituye textualmente por el selector padre; si no, le antepone
/// un combinador descendiente (`& <nested>`).
pub(crate) fn expand_amp(parent: &str, nested: &str) -> String {
    if nested.contains('&') {
        nested.replace('&', parent)
    } else {
        format!("{parent} {nested}")
    }
}

/// Encuentra el final del @-rule actual. Para at-rules con bloque,
/// devuelve la posición del `}` cerrando (inclusive). Para at-rules
/// statement (ej: `@import url;`), devuelve la posición del `;`
/// (inclusive). Si nada cuadra, None.
pub(crate) fn at_rule_end(s: &str) -> Option<usize> {
    let semi = s.find(';');
    let brace = s.find('{');
    match (semi, brace) {
        (Some(se), Some(br)) if se < br => Some(se + 1),
        (Some(se), None) => Some(se + 1),
        (_, Some(br)) => {
            // Encuentra el `}` que cierra balanceado.
            let body = &s[br + 1..];
            let close = matching_close_brace(body)?;
            Some(br + 1 + close + 1)
        }
        (None, None) => None,
    }
}

/// Dado el chunk completo del at-rule (`@media (cond) { body }`),
/// extrae la condición entre el nombre y el `{`.
pub(crate) fn parse_at_rule_condition<'a>(chunk: &'a str, name: &str) -> &'a str {
    let after_name = chunk.trim_start().get(name.len()..).unwrap_or("");
    let end = after_name.find('{').unwrap_or(after_name.len());
    after_name[..end].trim()
}

/// Extrae el body entre `{` y el `}` cerrando.
pub(crate) fn parse_at_rule_body(chunk: &str) -> &str {
    let Some(open) = chunk.find('{') else {
        return "";
    };
    let after = &chunk[open + 1..];
    let close = matching_close_brace(after).unwrap_or(after.len());
    &after[..close]
}

/// Busca el `}` que cierra balanceadamente — respeta nesting (`{ ... }`
/// dentro del body cuentan).
pub(crate) fn matching_close_brace(s: &str) -> Option<usize> {
    let mut depth: usize = 1;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Pasada previa al parseo real: encuentra bloques `:root { ... }`,
/// `html { ... }` o `* { ... }` y recoge cualquier declaración `--name:
/// value` en el mapa global de variables. Los conflictos (mismo nombre
/// en dos bloques) los gana el último — se acerca bastante a la cascada
/// CSS para vars declaradas en root.
pub(crate) fn extract_root_vars(css: &str, vars: &mut HashMap<String, String>) {
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

/// Pasada análoga a [`extract_root_vars`] pero para `@keyframes`. Escanea
/// el CSS crudo buscando `@keyframes name { ... }` (también los prefijos
/// vendor `@-webkit-keyframes` / `@-moz-keyframes`) y los acumula en el
/// mapa. Conflictos (mismo `name` en dos sitios) los gana el último.
pub(crate) fn extract_keyframes(css: &str, out: &mut HashMap<String, Keyframes>) {
    // `to_ascii_lowercase` preserva el largo en bytes (ASCII case sólo),
    // así que los índices del lowercase indexan el `css` original sin
    // desfase — necesario para conservar el case del `name` y los values.
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find('@') {
        let at = from + rel;
        let lrest = &lower[at..];
        let prefix = if lrest.starts_with("@keyframes") {
            "@keyframes"
        } else if lrest.starts_with("@-webkit-keyframes") {
            "@-webkit-keyframes"
        } else if lrest.starts_with("@-moz-keyframes") {
            "@-moz-keyframes"
        } else {
            from = at + 1;
            continue;
        };
        let after = &css[at + prefix.len()..];
        let Some(brace_rel) = after.find('{') else { break };
        let name = after[..brace_rel]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        let body_start = at + prefix.len() + brace_rel + 1;
        let Some(close) = matching_close_brace(&css[body_start..]) else {
            break;
        };
        let body = &css[body_start..body_start + close];
        from = body_start + close + 1;
        if name.is_empty() {
            continue;
        }
        let kf = parse_keyframes_body(body);
        if !kf.steps.is_empty() {
            out.insert(name, kf);
        }
    }
}

/// Pasada análoga a [`extract_keyframes`] pero para `@font-face` (CSS Fonts 4).
/// Escanea el CSS crudo buscando `@font-face { ... }`, parsea sus descriptores
/// y acumula una [`FontFaceRule`] por bloque (con `font-family` válido). A
/// diferencia de `@keyframes`, NO hay clave única: dos `@font-face` con el
/// mismo `family` son válidos (cubren rangos/pesos distintos) → lista, no mapa.
pub(crate) fn extract_font_faces(css: &str, out: &mut Vec<FontFaceRule>) {
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find('@') {
        let at = from + rel;
        if !lower[at..].starts_with("@font-face") {
            from = at + 1;
            continue;
        }
        let after = &css[at + "@font-face".len()..];
        let Some(brace_rel) = after.find('{') else { break };
        let body_start = at + "@font-face".len() + brace_rel + 1;
        let Some(close) = matching_close_brace(&css[body_start..]) else {
            break;
        };
        let body = &css[body_start..body_start + close];
        from = body_start + close + 1;
        if let Some(rule) = parse_font_face_body(body) {
            out.push(rule);
        }
    }
}

/// Parsea el cuerpo de un `@font-face`: pares `descriptor: value` (reusa el
/// splitter de keyframes). Devuelve `None` si falta `font-family` (descriptor
/// obligatorio) o si no hay ningún `src` válido.
pub(crate) fn parse_font_face_body(body: &str) -> Option<FontFaceRule> {
    let mut rule = FontFaceRule::default();
    let mut has_family = false;
    for (desc, value) in parse_keyframe_declarations(body) {
        let v = value.trim().to_string();
        match desc.as_str() {
            "font-family" => {
                rule.family = v.trim_matches('"').trim_matches('\'').to_string();
                has_family = !rule.family.is_empty();
            }
            "src" => rule.sources = parse_font_src_list(&v),
            "font-weight" => rule.weight = Some(v),
            "font-style" => rule.style = Some(v),
            "font-stretch" | "font-width" => rule.stretch = Some(v),
            "font-display" => rule.display = Some(v.to_ascii_lowercase()),
            "unicode-range" => rule.unicode_range = Some(v),
            "font-feature-settings" => rule.feature_settings = Some(v),
            "font-variation-settings" => rule.variation_settings = Some(v),
            "ascent-override" => rule.ascent_override = Some(v),
            "descent-override" => rule.descent_override = Some(v),
            "line-gap-override" => rule.line_gap_override = Some(v),
            "size-adjust" => rule.size_adjust = Some(v),
            _ => {}
        }
    }
    if has_family && !rule.sources.is_empty() {
        Some(rule)
    } else {
        None
    }
}

/// Parsea la lista `src:` de un `@font-face` — entradas separadas por coma de
/// nivel superior, cada una `url(...) | local(...)` con `format(...)`/`tech(...)`
/// opcionales. Las entradas sin url ni local se descartan.
pub(crate) fn parse_font_src_list(value: &str) -> Vec<FontSrc> {
    let mut out = Vec::new();
    for entry in split_top_level_comma(value) {
        let mut src = FontSrc::default();
        for tok in split_top_level_ws(&entry) {
            let low = tok.to_ascii_lowercase();
            if let Some(inner) = strip_fn(&tok, "url") {
                src.url = Some(unquote(inner).to_string());
            } else if let Some(inner) = strip_fn(&tok, "local") {
                src.local = Some(unquote(inner).to_string());
            } else if let Some(inner) = strip_fn(&tok, "format") {
                src.format = Some(unquote(inner).to_ascii_lowercase());
            } else if let Some(inner) = strip_fn(&tok, "tech") {
                src.tech = Some(unquote(inner).to_ascii_lowercase());
            } else {
                let _ = low;
            }
        }
        if src.url.is_some() || src.local.is_some() {
            out.push(src);
        }
    }
    out
}

/// Pasada análoga a [`extract_font_faces`] pero para `@property --name { ... }`
/// (CSS Properties & Values API). Acumula una [`PropertyRule`] por bloque con
/// `name` (`--…`) y `syntax` válidos. Un `@property` sin `syntax`, o cuyo
/// `syntax` no sea universal (`*`) y le falte `initial-value`, es inválido por
/// spec → se descarta.
pub(crate) fn extract_at_properties(css: &str, out: &mut Vec<PropertyRule>) {
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find('@') {
        let at = from + rel;
        if !lower[at..].starts_with("@property") {
            from = at + 1;
            continue;
        }
        let after = &css[at + "@property".len()..];
        let Some(brace_rel) = after.find('{') else { break };
        let name = after[..brace_rel].trim().to_string();
        let body_start = at + "@property".len() + brace_rel + 1;
        let Some(close) = matching_close_brace(&css[body_start..]) else {
            break;
        };
        let body = &css[body_start..body_start + close];
        from = body_start + close + 1;
        if !name.starts_with("--") {
            continue;
        }
        if let Some(rule) = parse_at_property_body(&name, body) {
            out.push(rule);
        }
    }
}

/// Parsea el cuerpo de un `@property`. `None` si falta `syntax`, o si el syntax
/// no es universal (`*`) y falta `initial-value`.
pub(crate) fn parse_at_property_body(name: &str, body: &str) -> Option<PropertyRule> {
    let mut syntax: Option<String> = None;
    let mut inherits: Option<bool> = None;
    let mut initial_value: Option<String> = None;
    for (desc, value) in parse_keyframe_declarations(body) {
        let v = value.trim();
        match desc.as_str() {
            "syntax" => syntax = Some(unquote(v).to_string()),
            "inherits" => {
                inherits = match v.to_ascii_lowercase().as_str() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                }
            }
            "initial-value" => initial_value = Some(v.to_string()),
            _ => {}
        }
    }
    let syntax = syntax?;
    if syntax != "*" && initial_value.is_none() {
        return None;
    }
    Some(PropertyRule {
        name: name.to_string(),
        syntax,
        inherits: inherits.unwrap_or(false),
        initial_value,
    })
}

/// Pasada análoga a [`extract_at_properties`] pero para `@counter-style name`
/// (CSS Counter Styles 3). Acumula una [`CounterStyleRule`] por bloque con
/// `name` válido y al menos un `system` o `symbols`/`additive-symbols`.
pub(crate) fn extract_counter_styles(css: &str, out: &mut Vec<CounterStyleRule>) {
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find('@') {
        let at = from + rel;
        if !lower[at..].starts_with("@counter-style") {
            from = at + 1;
            continue;
        }
        let after = &css[at + "@counter-style".len()..];
        let Some(brace_rel) = after.find('{') else { break };
        let name = after[..brace_rel].trim().to_string();
        let body_start = at + "@counter-style".len() + brace_rel + 1;
        let Some(close) = matching_close_brace(&css[body_start..]) else {
            break;
        };
        let body = &css[body_start..body_start + close];
        from = body_start + close + 1;
        if name.is_empty() || name.starts_with("--") {
            continue;
        }
        if let Some(rule) = parse_counter_style_body(&name, body) {
            out.push(rule);
        }
    }
}

/// Parsea el cuerpo de un `@counter-style`. `None` si no trae ningún
/// descriptor definitorio (`system`, `symbols` o `additive-symbols`).
pub(crate) fn parse_counter_style_body(name: &str, body: &str) -> Option<CounterStyleRule> {
    let mut rule = CounterStyleRule { name: name.to_string(), ..Default::default() };
    for (desc, value) in parse_keyframe_declarations(body) {
        let v = value.trim().to_string();
        match desc.as_str() {
            "system" => rule.system = Some(v),
            "symbols" => rule.symbols = Some(v),
            "additive-symbols" => rule.additive_symbols = Some(v),
            "negative" => rule.negative = Some(v),
            "prefix" => rule.prefix = Some(v),
            "suffix" => rule.suffix = Some(v),
            "range" => rule.range = Some(v),
            "pad" => rule.pad = Some(v),
            "speak-as" => rule.speak_as = Some(v),
            "fallback" => rule.fallback = Some(v),
            _ => {}
        }
    }
    if rule.system.is_some() || rule.symbols.is_some() || rule.additive_symbols.is_some() {
        Some(rule)
    } else {
        None
    }
}

/// Pasada análoga para `@page [<sel>] { ... }` (CSS Paged Media 3). Acumula
/// una [`PageRule`] por bloque. El selector (`:left`/nombre/vacío) va entre
/// `@page` y `{`.
pub(crate) fn extract_page_rules(css: &str, out: &mut Vec<PageRule>) {
    let lower = css.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find('@') {
        let at = from + rel;
        if !lower[at..].starts_with("@page") {
            from = at + 1;
            continue;
        }
        // Evita falsos positivos como `@page-foo` (no existe, pero defensivo):
        // el char tras `@page` debe ser whitespace o `{`.
        let next = css[at + "@page".len()..].chars().next();
        if !matches!(next, Some(c) if c.is_whitespace() || c == '{') {
            from = at + 1;
            continue;
        }
        let after = &css[at + "@page".len()..];
        let Some(brace_rel) = after.find('{') else { break };
        let selector = after[..brace_rel].trim().to_string();
        let body_start = at + "@page".len() + brace_rel + 1;
        let Some(close) = matching_close_brace(&css[body_start..]) else {
            break;
        };
        let body = &css[body_start..body_start + close];
        from = body_start + close + 1;
        out.push(parse_page_body(&selector, body));
    }
}

/// Parsea el cuerpo de un `@page`: descriptores de página propios a campos +
/// el resto de declaraciones crudas. Las margin-at-rules anidadas
/// (`@top-center { … }`) se ignoran (pares con `{`/`@` se descartan).
pub(crate) fn parse_page_body(selector: &str, body: &str) -> PageRule {
    let mut rule = PageRule { selector: selector.to_string(), ..Default::default() };
    for (desc, value) in parse_keyframe_declarations(body) {
        if desc.contains('{') || desc.contains('@') || value.contains('{') || value.contains('}') {
            continue;
        }
        let v = value.trim().to_string();
        match desc.as_str() {
            "size" => rule.size = Some(v),
            "marks" => rule.marks = Some(v),
            "bleed" => rule.bleed = Some(v),
            "page-orientation" => rule.page_orientation = Some(v),
            _ => rule.declarations.push((desc, v)),
        }
    }
    rule
}

/// `name( ... )` → contenido interno (case-insensitive del nombre). `None` si
/// `s` no es exactamente esa función.
fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let s = s.trim();
    if s.len() <= name.len() + 1 || !s[..name.len()].eq_ignore_ascii_case(name) {
        return None;
    }
    s[name.len()..].trim_start().strip_prefix('(')?.strip_suffix(')')
}

/// Quita comillas envolventes (`"x"` / `'x'`) y espacios.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('"').and_then(|x| x.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
        .unwrap_or(s)
        .trim()
}

/// Parsea el cuerpo de un `@keyframes`: una secuencia de bloques
/// `selector { decls }` donde `selector` es una lista de offsets
/// (`from`/`to`/`N%`) separados por coma. Los pasos quedan ordenados por
/// offset ascendente.
pub(crate) fn parse_keyframes_body(body: &str) -> Keyframes {
    let mut steps: Vec<KeyframeStep> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < body.len() {
        while i < body.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= body.len() {
            break;
        }
        let Some(brace) = body[i..].find('{') else { break };
        let selector_raw = body[i..i + brace].trim();
        let inner_start = i + brace + 1;
        let Some(close) = matching_close_brace(&body[inner_start..]) else {
            break;
        };
        let inner = &body[inner_start..inner_start + close];
        i = inner_start + close + 1;
        let decls = parse_keyframe_declarations(inner);
        if decls.is_empty() {
            continue;
        }
        for tok in selector_raw.split(',') {
            if let Some(offset) = parse_keyframe_offset(tok.trim()) {
                steps.push(KeyframeStep { offset, declarations: decls.clone() });
            }
        }
    }
    steps.sort_by(|a, b| {
        a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal)
    });
    Keyframes { steps }
}

/// `from` → 0.0, `to` → 1.0, `N%` → N/100. Cualquier otra cosa → None.
pub(crate) fn parse_keyframe_offset(tok: &str) -> Option<f32> {
    let t = tok.trim().to_ascii_lowercase();
    match t.as_str() {
        "from" => Some(0.0),
        "to" => Some(1.0),
        _ => t.strip_suffix('%').and_then(|n| n.trim().parse::<f32>().ok()).map(|p| p / 100.0),
    }
}

/// Pares `prop: value` crudos del cuerpo de un keyframe. No sustituye
/// `var(...)` ni valida la propiedad — eso lo hará el runtime de tween
/// cuando exista (Fase B4+).
pub(crate) fn parse_keyframe_declarations(inner: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in inner.split(';') {
        let Some((prop, value)) = chunk.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        let value = value.trim();
        if prop.is_empty() || value.is_empty() {
            continue;
        }
        out.push((prop.to_ascii_lowercase(), value.to_string()));
    }
    out
}

/// Parsea una duración CSS (`2s`, `200ms`, `0.3s`) a segundos. `0` sin
/// unidad → 0.0. Sin unidad reconocida → None (así un token numérico puro
/// no se confunde con una duración al clasificar el shorthand).
pub(crate) fn parse_time(s: &str) -> Option<f32> {
    let s = s.trim();
    // Fase 7.877 — `calc()` sobre tiempos (el evaluador da segundos).
    if is_math_fn(s) {
        return match eval_calc(s)? {
            CalcVal::Number(n) if n.is_finite() => Some(n),
            _ => None,
        };
    }
    if let Some(num) = s.strip_suffix("ms") {
        return num.trim().parse::<f32>().ok().map(|v| v / 1000.0);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.trim().parse::<f32>().ok();
    }
    if s == "0" {
        return Some(0.0);
    }
    None
}

/// Parsea una `<timing-function>`: keywords (`ease`/`linear`/`ease-in`/
/// `ease-out`/`ease-in-out`/`step-start`/`step-end`), `cubic-bezier(...)`
/// y `steps(n, term)`. None si no encaja.
pub(crate) fn parse_easing(s: &str) -> Option<EasingFunction> {
    let t = s.trim().to_ascii_lowercase();
    match t.as_str() {
        "linear" => return Some(EasingFunction::Linear),
        "ease" => return Some(EasingFunction::Ease),
        "ease-in" => return Some(EasingFunction::EaseIn),
        "ease-out" => return Some(EasingFunction::EaseOut),
        "ease-in-out" => return Some(EasingFunction::EaseInOut),
        "step-start" => return Some(EasingFunction::StepStart),
        "step-end" => return Some(EasingFunction::StepEnd),
        _ => {}
    }
    if let Some(args) = t.strip_prefix("cubic-bezier(").and_then(|r| r.strip_suffix(')')) {
        let nums: Vec<f32> = args.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        if nums.len() == 4 {
            return Some(EasingFunction::CubicBezier(nums[0], nums[1], nums[2], nums[3]));
        }
        return None;
    }
    if let Some(args) = t.strip_prefix("steps(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(',').map(|p| p.trim()).collect();
        let n: u32 = parts.first()?.parse().ok()?;
        let jump_start = parts
            .get(1)
            .map(|p| *p == "start" || *p == "jump-start")
            .unwrap_or(false);
        return Some(EasingFunction::Steps(n, jump_start));
    }
    // `linear(<stop>#)` (CSS Easing 2): cada stop es `<number> <percentage>{0,2}`.
    // Lo validamos pero, como el runtime de tween todavía no anima, colapsamos
    // al keyword `linear` (plumb lossy: descartamos la curva por tramos).
    if let Some(args) = t.strip_prefix("linear(").and_then(|r| r.strip_suffix(')')) {
        let mut any = false;
        for stop in args.split(',') {
            let mut toks = stop.split_whitespace();
            // Primer token: el output (número). Obligatorio.
            let out: f32 = toks.next()?.parse().ok()?;
            let _ = out;
            // Tokens restantes: hasta 2 posiciones en porcentaje.
            let mut pcts = 0;
            for tok in toks {
                let p = tok.strip_suffix('%')?;
                p.trim().parse::<f32>().ok()?;
                pcts += 1;
                if pcts > 2 {
                    return None;
                }
            }
            any = true;
        }
        return if any { Some(EasingFunction::Linear) } else { None };
    }
    None
}

/// Tokeniza un value por whitespace de nivel superior, respetando
/// paréntesis: `cubic-bezier(.1, .2, .3, .4)` queda como un único token.
pub(crate) fn split_top_level_ws(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Separa por comas de nivel superior, respetando paréntesis. Usado para
/// las listas de `transition`/`animation` múltiples.
pub(crate) fn split_top_level_comma(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// `animation: <name> <duration> <timing> <delay> <iteration> <direction>
/// <fill>`. Clasifica cada token por forma, no por posición. `none` →
/// `Animation(None)`. Lista separada por coma → nos quedamos con la
/// primera animación parseable (no hay runtime multi-animación todavía).
pub(crate) fn parse_animation(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::Animation(None));
    }
    let seg = split_top_level_comma(v).into_iter().next()?;
    Some(DeclKind::Animation(parse_one_animation(&seg)))
}

pub(crate) fn parse_one_animation(seg: &str) -> Option<AnimationBinding> {
    let tokens = split_top_level_ws(seg.trim());
    if tokens.is_empty() {
        return None;
    }
    let mut name: Option<String> = None;
    let mut duration: Option<f32> = None;
    let mut delay: Option<f32> = None;
    let mut timing: Option<EasingFunction> = None;
    let mut iterations: Option<AnimationIterations> = None;
    let mut direction: Option<AnimationDirection> = None;
    let mut fill: Option<AnimationFillMode> = None;
    let mut play_state: Option<AnimationPlayState> = None;
    for tok in &tokens {
        let lt = tok.to_ascii_lowercase();
        // Duración primero, delay después (orden posicional de los dos
        // valores de tiempo — único caso donde la posición importa).
        if let Some(t) = parse_time(tok) {
            if duration.is_none() {
                duration = Some(t);
            } else if delay.is_none() {
                delay = Some(t);
            }
            continue;
        }
        if lt == "infinite" {
            iterations = Some(AnimationIterations::Infinite);
            continue;
        }
        // Número puro sin unidad → iteration-count (`parse_time` ya
        // descartó los que llevan `s`/`ms`).
        if let Ok(n) = lt.parse::<f32>() {
            iterations = Some(AnimationIterations::Count(n));
            continue;
        }
        if timing.is_none() {
            if let Some(e) = parse_easing(&lt) {
                timing = Some(e);
                continue;
            }
        }
        match lt.as_str() {
            "normal" => {
                direction = Some(AnimationDirection::Normal);
                continue;
            }
            "reverse" => {
                direction = Some(AnimationDirection::Reverse);
                continue;
            }
            "alternate" => {
                direction = Some(AnimationDirection::Alternate);
                continue;
            }
            "alternate-reverse" => {
                direction = Some(AnimationDirection::AlternateReverse);
                continue;
            }
            "forwards" => {
                fill = Some(AnimationFillMode::Forwards);
                continue;
            }
            "backwards" => {
                fill = Some(AnimationFillMode::Backwards);
                continue;
            }
            "both" => {
                fill = Some(AnimationFillMode::Both);
                continue;
            }
            "running" => {
                play_state = Some(AnimationPlayState::Running);
                continue;
            }
            "paused" => {
                play_state = Some(AnimationPlayState::Paused);
                continue;
            }
            // `none` acá sería `animation-name: none` o `fill-mode: none` —
            // ambiguo y raro en shorthand; lo tratamos como "sin nombre".
            "none" => continue,
            _ => {}
        }
        if name.is_none() {
            name = Some(tok.clone());
        }
    }
    let name = name?;
    Some(AnimationBinding {
        name,
        duration_s: duration.unwrap_or(0.0),
        timing: timing.unwrap_or_default(),
        delay_s: delay.unwrap_or(0.0),
        iterations: iterations.unwrap_or(AnimationIterations::Count(1.0)),
        direction: direction.unwrap_or(AnimationDirection::Normal),
        fill_mode: fill.unwrap_or(AnimationFillMode::None),
        play_state: play_state.unwrap_or(AnimationPlayState::Running),
    })
}

/// `transition: <property> <duration> <timing> <delay>`. Lista separada
/// por coma → varios bindings. `none` → lista vacía.
pub(crate) fn parse_transition(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::Transitions(Vec::new()));
    }
    let mut out = Vec::new();
    for seg in split_top_level_comma(v) {
        if let Some(b) = parse_one_transition(&seg) {
            out.push(b);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(DeclKind::Transitions(out))
    }
}

pub(crate) fn parse_one_transition(seg: &str) -> Option<TransitionBinding> {
    let tokens = split_top_level_ws(seg.trim());
    if tokens.is_empty() {
        return None;
    }
    let mut property: Option<String> = None;
    let mut duration: Option<f32> = None;
    let mut delay: Option<f32> = None;
    let mut timing: Option<EasingFunction> = None;
    for tok in &tokens {
        let lt = tok.to_ascii_lowercase();
        if let Some(t) = parse_time(tok) {
            if duration.is_none() {
                duration = Some(t);
            } else if delay.is_none() {
                delay = Some(t);
            }
            continue;
        }
        if timing.is_none() {
            if let Some(e) = parse_easing(&lt) {
                timing = Some(e);
                continue;
            }
        }
        // El primer token que no es tiempo ni easing es la propiedad
        // (`opacity`, `transform`, `all`, `background-color`...).
        if property.is_none() {
            property = Some(lt);
        }
    }
    Some(TransitionBinding {
        property: property.unwrap_or_else(|| "all".to_string()),
        duration_s: duration.unwrap_or(0.0),
        timing: timing.unwrap_or_default(),
        delay_s: delay.unwrap_or(0.0),
    })
}

/// Reemplaza `var(--name)` y `var(--name, fallback)` en `value` por el
/// valor recogido en `vars`. Si la variable no existe y hay fallback, lo
/// usa; sino, sustituye por cadena vacía. La sustitución es recursiva
/// (un value de var puede a su vez contener `var(...)`).
pub(crate) fn substitute_vars(value: &str, vars: &HashMap<String, String>) -> String {
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

/// Sustituye `env(<name>[, <fallback>])` (CSS Env 1) por su valor. Sin un
/// dispositivo real con áreas seguras / barras de título, los `env()`
/// conocidos (`safe-area-inset-*`, `titlebar-area-*`, `keyboard-inset-*`)
/// valen `0px`; cualquier otro usa el fallback si existe, o `0px`. Mismo
/// motor textual que [`substitute_vars`] (respeta nesting). Fase 7.869.
pub(crate) fn substitute_env(value: &str) -> String {
    if !value.contains("env(") {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("env(") {
        out.push_str(&rest[..start]);
        let inside_start = start + 4;
        let bytes = rest[inside_start..].as_bytes();
        let mut depth = 1usize;
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
            out.push_str(&rest[start..]);
            return out;
        };
        let args = &rest[inside_start..inside_start + close];
        // El nombre puede traer índices (`env(viewport-segment-width 0 0)`);
        // sólo nos importa separar el fallback tras la 1ª coma top-level.
        let fallback = args.split_once(',').map(|(_, f)| f.trim().to_string());
        let replacement = fallback.unwrap_or_else(|| "0px".to_string());
        // Recursión: el fallback puede contener más env().
        out.push_str(&substitute_env(&replacement));
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
/// Divide una lista de selectores (`a, b, :is(c, d)`) por las comas de NIVEL
/// SUPERIOR, respetando las que viven dentro de `(...)` o `[...]` (p.ej. la
/// coma de `:is(h1, h2)` o de `[x="a,b"]` no separa selectores). Fase 7.188.
pub(crate) fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth_paren = 0i32;
    let mut depth_bracket = 0i32;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_bracket += 1,
            ']' => depth_bracket -= 1,
            ',' if depth_paren <= 0 && depth_bracket <= 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Como `str::split_whitespace` pero sin partir dentro de `(...)` o `[...]`
/// — así `:is(h1, h2)` o `[x="a b"]` quedan en un solo token mientras los
/// combinadores descendientes (espacios de nivel 0) sí separan. Fase 7.188.
pub(crate) fn split_ws_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth_p = 0i32;
    let mut depth_b = 0i32;
    let mut start: Option<usize> = None;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth_p += 1,
            ')' => depth_p -= 1,
            '[' => depth_b += 1,
            ']' => depth_b -= 1,
            _ => {}
        }
        if ch.is_whitespace() && depth_p <= 0 && depth_b <= 0 {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}
