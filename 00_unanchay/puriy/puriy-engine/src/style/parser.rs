//! Parsing CSS: hoja de estilos completa (`parse_stylesheet`, at-rules, `@media`,
//! `@keyframes`, `@import`), UA stylesheet y defaults por tag, substitución de
//! `var()`, parsing de selectores (`parse_selector`/`parse_compound`) y de
//! declaraciones (`parse_declarations`/`decl_kind_from_pair` + shorthands de
//! border/box-shadow/animation/transition), y los helpers públicos `parse_color`
//! y `evaluate_media_query`. Extraído de `style/mod.rs` (regla #1). Comparte los
//! tipos del módulo `style` y del crate vía `use super::*`.
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
            selector: ty("h1"),
            decls: vec![
                decl(DeclKind::FontSize(32.0)),
                decl(DeclKind::Margin(sides_lrtb(21.0, 0.0, 21.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h2"),
            decls: vec![
                decl(DeclKind::FontSize(24.0)),
                decl(DeclKind::Margin(sides_lrtb(19.0, 0.0, 19.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h3"),
            decls: vec![
                decl(DeclKind::FontSize(19.0)),
                decl(DeclKind::Margin(sides_lrtb(19.0, 0.0, 19.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h4"),
            decls: vec![
                decl(DeclKind::FontSize(16.0)),
                decl(DeclKind::Margin(sides_lrtb(21.0, 0.0, 21.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h5"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::Margin(sides_lrtb(22.0, 0.0, 22.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("h6"),
            decls: vec![
                decl(DeclKind::FontSize(11.0)),
                decl(DeclKind::Margin(sides_lrtb(25.0, 0.0, 25.0, 0.0))),
            ],
        },
        Rule {
            selector: ty("p"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0)))],
        },
        // Listas: padding-left para los bullets/numerales (el marker se
        // pinta antes del contenido, necesita espacio para no chocar
        // con el borde izquierdo del block).
        Rule {
            selector: ty("ul"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::Padding(sides_lrtb(0.0, 0.0, 0.0, 40.0))),
                decl(DeclKind::ListStyleType(ListStyleType::Disc)),
            ],
        },
        Rule {
            selector: ty("ol"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::Padding(sides_lrtb(0.0, 0.0, 0.0, 40.0))),
                decl(DeclKind::ListStyleType(ListStyleType::Decimal)),
            ],
        },
        Rule {
            selector: ty("blockquote"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(10.0, 40.0, 10.0, 40.0)))],
        },
        Rule {
            selector: ty("dl"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0)))],
        },
        Rule {
            selector: ty("dd"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(0.0, 0.0, 0.0, 40.0)))],
        },
        Rule {
            selector: ty("pre"),
            decls: vec![
                decl(DeclKind::Margin(sides_lrtb(12.0, 0.0, 12.0, 0.0))),
                decl(DeclKind::WhiteSpace(WhiteSpace::Pre)),
            ],
        },
        Rule {
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
            selector: ty("a"),
            decls: vec![Decl {
                kind: DeclKind::TextDecoration(TextDecorationLine::Underline),
                important: false,
            }],
        },
        Rule {
            selector: ty("u"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::Underline))],
        },
        Rule {
            selector: ty("ins"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::Underline))],
        },
        Rule {
            selector: ty("s"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            selector: ty("strike"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            selector: ty("del"),
            decls: vec![decl(DeclKind::TextDecoration(TextDecorationLine::LineThrough))],
        },
        Rule {
            selector: ty("menu"),
            decls: vec![decl(DeclKind::ListStyleType(ListStyleType::Disc))],
        },
        // Tables: bordes celulares mínimos para que la grilla se vea sin
        // CSS de autor. Browsers reales no dibujan bordes hasta que un
        // stylesheet lo pida, pero acá preferimos mostrarlos por default
        // — la mayoría de páginas con `<table>` sin estilo asumen un
        // "look spreadsheet" y tablas sin bordes salen invisibles.
        Rule {
            selector: ty("table"),
            decls: vec![decl(DeclKind::Margin(sides_lrtb(8.0, 0.0, 8.0, 0.0)))],
        },
        Rule {
            selector: ty("td"),
            decls: vec![
                decl(DeclKind::Padding(Sides::all(4.0))),
                decl(DeclKind::BorderWidth(1.0)),
                decl(DeclKind::BorderColor(Color::rgb(204, 204, 204))),
                decl(DeclKind::BorderEnabled(true)),
            ],
        },
        Rule {
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
            selector: ty("small"),
            decls: vec![decl(DeclKind::FontSize(13.0))],
        },
        Rule {
            selector: ty("sub"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::VerticalAlign(VerticalAlign::Sub)),
            ],
        },
        Rule {
            selector: ty("sup"),
            decls: vec![
                decl(DeclKind::FontSize(13.0)),
                decl(DeclKind::VerticalAlign(VerticalAlign::Super)),
            ],
        },
        Rule {
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

pub(crate) fn parse_stylesheet(css: &str, vars: &HashMap<String, String>, vp: Viewport) -> Vec<Rule> {
    let css = strip_comments(css);
    parse_rules_block(&css, vars, vp)
}

/// Parsea un bloque de reglas — el cuerpo de un stylesheet completo o
/// el contenido de un `@media` / `@supports`. Soporta:
/// - reglas normales `selector { decls }`
/// - `@media (condition) { ... }` recursivo — eval contra `viewport`
/// - `@supports (prop: value) { ... }` recursivo — eval por parser
/// - `@-rules` desconocidos (`@font-face`, `@keyframes`, etc.) los
///   saltea silenciosamente
pub(crate) fn parse_rules_block(css: &str, vars: &HashMap<String, String>, viewport: Viewport) -> Vec<Rule> {
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
                    out.extend(parse_rules_block(body, vars, viewport));
                }
                let _ = rest_after;
                continue;
            }
            if lower.starts_with("@supports") {
                let cond = parse_at_rule_condition(chunk, "@supports");
                let body = parse_at_rule_body(chunk);
                if evaluate_supports_query(cond) {
                    out.extend(parse_rules_block(body, vars, viewport));
                }
                continue;
            }
            // @-rule desconocido: lo saltamos sin parsear.
            continue;
        }
        // Regla normal: `selector { decls }`.
        let Some(brace) = css[i..].find('{') else { break };
        let sel_raw = css[i..i + brace].trim();
        i += brace + 1;
        let Some(close) = matching_close_brace(&css[i..]) else { break };
        let body = &css[i..i + close];
        i += close + 1;
        if sel_raw.is_empty() {
            continue;
        }
        for sel in split_top_level_commas(sel_raw) {
            let sel = sel.trim();
            let Some(selector) = parse_selector(sel) else {
                continue;
            };
            out.push(Rule { selector, decls: parse_declarations(body, vars) });
        }
    }
    out
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

pub(crate) fn parse_selector(sel: &str) -> Option<Selector> {
    let sel = sel.trim();
    // Strip pseudo-element del final (`::before`/`::after`). CSS también
    // acepta la sintaxis legacy `:before`/`:after` con un sólo `:` —
    // las aceptamos por compatibilidad. Pueden venir adheridas al
    // último compound (`p::before`) o solas (`::before` matchea
    // implícitamente al universal).
    let (sel, pseudo_element) = strip_pseudo_element(sel);
    if sel.is_empty() {
        let compound = Compound {
            tag: TagPart::Universal,
            ids: vec![],
            classes: vec![],
            attrs: vec![],
            pseudos: vec![],
        };
        return Some(Selector {
            compounds: vec![compound],
            combinators: vec![],
            pseudo_element,
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
    for tok in split_ws_top_level(&normalized) {
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
    if pending_combinator.is_some() {
        return None;
    }
    Some(Selector { compounds, combinators, pseudo_element })
}

/// Si `sel` termina con `::before`/`::after` (o legacy `:before`/`:after`),
/// devuelve `(prefix, Some(PseudoElement))`. Sino devuelve `(sel, None)`.
pub(crate) fn strip_pseudo_element(sel: &str) -> (&str, Option<PseudoElement>) {
    let lower = sel.to_ascii_lowercase();
    for (suffix, pe) in [
        ("::before", PseudoElement::Before),
        ("::after", PseudoElement::After),
        (":before", PseudoElement::Before),
        (":after", PseudoElement::After),
    ] {
        if let Some(prefix) = lower.strip_suffix(suffix) {
            // Cuidado: `:before` no debe matchear cuando es parte de
            // `:before-leaf` (no es un pseudo válido en CSS). Pero al
            // ser sufijo exacto del string, esto no aplica acá. Sí
            // garantizamos que el prefijo no termine en alfanumérico
            // (caso `p:beforex` — el parseo falla al no encontrar
            // pseudoclase válida y lo rechazamos abajo). Acá basta.
            return (&sel[..prefix.len()], Some(pe));
        }
    }
    (sel, None)
}

/// Inserta espacios alrededor de `>`/`+`/`~` para que `split_whitespace`
/// los aísle como tokens propios. Si caen dentro de `[…]` o `(…)` los
/// dejamos intactos — `[href*="a>b"]` o `:not(a+b)` deben pasar al
/// compound parser sin romperse.
pub(crate) fn normalize_combinators(sel: &str) -> String {
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
pub(crate) fn parse_compound(sel: &str) -> Option<Compound> {
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
                        "nth-of-type" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthOfType { a, b }
                        }
                        "nth-last-child" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthLastChild { a, b }
                        }
                        "nth-last-of-type" => {
                            let (a, b) = parse_nth_arg(arg)?;
                            Pseudo::NthLastOfType { a, b }
                        }
                        "not" => {
                            // CSS4: lista de compounds (`:not(.a, .b)`).
                            let mut inner = Vec::new();
                            for part in arg.split(',') {
                                let c = parse_compound(part.trim())?;
                                // Anti-recursión: `:not(:not(...))` rechazamos.
                                if c.pseudos.iter().any(|p| matches!(p, Pseudo::Not(_))) {
                                    return None;
                                }
                                inner.push(c);
                            }
                            if inner.is_empty() {
                                return None;
                            }
                            Pseudo::Not(inner)
                        }
                        "is" | "where" => {
                            // Lista de compounds separados por coma (sin
                            // combinadores adentro — `parse_compound` parsea
                            // uno solo). Split naive por `,` (no contempla
                            // comas dentro de `[attr="a,b"]`, caso raro).
                            let mut inner = Vec::new();
                            for part in arg.split(',') {
                                inner.push(parse_compound(part.trim())?);
                            }
                            if inner.is_empty() {
                                return None;
                            }
                            if name == "is" {
                                Pseudo::Is(inner)
                            } else {
                                Pseudo::Where(inner)
                            }
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
                    "only-of-type" => Pseudo::OnlyOfType,
                    "hover" => Pseudo::Hover,
                    "focus" | "focus-visible" | "focus-within" => Pseudo::Focus,
                    "checked" => Pseudo::Checked,
                    "disabled" => Pseudo::Disabled,
                    "enabled" => Pseudo::Enabled,
                    "required" => Pseudo::Required,
                    "optional" => Pseudo::Optional,
                    "read-only" => Pseudo::ReadOnly,
                    "read-write" => Pseudo::ReadWrite,
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
pub(crate) fn parse_attr_match(inner: &str) -> Option<AttrMatch> {
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

pub(crate) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

pub(crate) fn strip_comments(css: &str) -> String {
    // Operamos a nivel byte para detectar `/*…*/`, pero copiamos slices
    // de la `&str` original para preservar UTF-8 multi-byte (un push de
    // bytes individuales `as char` rompe runs no-ASCII como "▸").
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    let mut chunk_start = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Volcamos el chunk pendiente antes del comentario.
            out.push_str(&css[chunk_start..i]);
            if let Some(end) = css[i + 2..].find("*/") {
                i += 2 + end + 2;
                chunk_start = i;
                continue;
            }
            return out;
        }
        i += 1;
    }
    out.push_str(&css[chunk_start..]);
    out
}

pub(crate) fn parse_declarations(css: &str, vars: &HashMap<String, String>) -> Vec<Decl> {
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
        if let Some(decls) = parse_logical_border(prop, value, important) {
            out.extend(decls);
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "") {
            out.extend(parse_border_side_shorthand(edge, value, important));
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-width") {
            if let Some(w) = parse_length_px(value) {
                out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
            }
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-color") {
            if let Some(c) = parse_color(value) {
                out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
            }
            continue;
        }
        if let Some(edge) = match_border_side_prop(prop, "-style") {
            if let Some(s) = parse_border_style(value) {
                out.push(Decl { kind: DeclKind::BorderSideStyle(edge, s), important });
            }
            continue;
        }
        if let Some(corner) = match_border_corner_prop(prop) {
            if let Some(r) = parse_length_px(value) {
                out.push(Decl { kind: DeclKind::BorderCornerRadius(corner, r), important });
            }
            continue;
        }
        if prop.eq_ignore_ascii_case("flex") {
            out.extend(parse_flex_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("inset") {
            out.extend(parse_inset_shorthand(value, important));
            continue;
        }
        if let Some(decls) = parse_logical_box(prop, value, important) {
            out.extend(decls);
            continue;
        }
        if prop.eq_ignore_ascii_case("flex-flow") {
            out.extend(parse_flex_flow_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("place-content") {
            out.extend(parse_place_content_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("place-items") {
            out.extend(parse_place_items_shorthand(value, important));
            continue;
        }
        if prop.eq_ignore_ascii_case("place-self") {
            out.extend(parse_place_self_shorthand(value, important));
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
pub(crate) fn strip_important(value: &str) -> Option<&str> {
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

pub(crate) fn decl_kind_from_pair(prop: &str, value: &str) -> Option<DeclKind> {
    match prop.to_ascii_lowercase().as_str() {
        "color" => parse_color(value).map(DeclKind::Color),
        "background-color" | "background" => parse_color(value).map(DeclKind::Background),
        "display" => parse_display(value).map(DeclKind::Display),
        "font-size" => parse_length_px(value).map(DeclKind::FontSize),
        "font-weight" => parse_weight(value).map(DeclKind::FontWeight),
        "font-style" => parse_font_style(value).map(DeclKind::FontStyle),
        "font-family" => Some(DeclKind::FontFamily(value.trim().to_string())),
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
        "height" => parse_length_or_pct(value).map(DeclKind::Height),
        "max-width" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "text-align" => parse_text_align(value).map(DeclKind::TextAlign),
        "line-height" => parse_line_height(value).map(DeclKind::LineHeight),
        "border-width" => parse_length_px(value).map(DeclKind::BorderWidth),
        "border-color" => parse_color(value).map(DeclKind::BorderColor),
        "border-style" => parse_border_style(value).map(DeclKind::BorderEnabled),
        "border-radius" => parse_length_px(value).map(DeclKind::BorderRadius),
        "z-index" => {
            // `auto` → 0; sino int. Negativos OK.
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ZIndex(0))
            } else {
                v.parse::<i32>().ok().map(DeclKind::ZIndex)
            }
        }
        "content" => Some(DeclKind::Content(parse_content_value(value))),
        "counter-reset" => Some(DeclKind::CounterReset(parse_counter_list(value, 0))),
        "counter-increment" => Some(DeclKind::CounterIncrement(parse_counter_list(value, 1))),
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
        "align-content" => parse_align_content(value).map(DeclKind::AlignContent),
        "justify-items" => parse_justify_items(value).map(DeclKind::JustifyItems),
        "justify-self" => parse_justify_self(value).map(DeclKind::JustifySelf),
        "gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        "box-sizing" => parse_box_sizing(value).map(DeclKind::BoxSizing),
        "min-width" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-height" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-height" => parse_length_or_pct(value).map(DeclKind::MaxHeight),
        // `aspect-ratio: auto` resetea; `W / H` o un número crudo fijan la
        // relación. La forma `auto W/H` (auto + ratio) toma sólo el ratio.
        "aspect-ratio" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::AspectRatio(None))
            } else {
                // Descarta un prefijo `auto` opcional (`auto 16/9`).
                let v = v.strip_prefix("auto").map(str::trim).unwrap_or(v);
                parse_aspect_ratio(v).map(|r| DeclKind::AspectRatio(Some(r)))
            }
        }
        // Tamaños lógicos → físicos (LTR + escritura horizontal): inline ↔
        // width, block ↔ height. Fase 7.194.
        "inline-size" => parse_length_or_pct(value).map(DeclKind::Width),
        "block-size" => parse_length_or_pct(value).map(DeclKind::Height),
        "min-inline-size" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-block-size" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-inline-size" => parse_length_or_pct(value).map(DeclKind::MaxWidth),
        "max-block-size" => parse_length_or_pct(value).map(DeclKind::MaxHeight),
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
        "position" => parse_position(value).map(DeclKind::Position),
        "top" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetTop),
        "right" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetRight),
        "bottom" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetBottom),
        "left" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetLeft),
        "vertical-align" => parse_vertical_align(value).map(DeclKind::VerticalAlign),
        "visibility" => parse_visibility(value).map(DeclKind::Visibility),
        "pointer-events" => parse_pointer_events(value).map(DeclKind::PointerEvents),
        "text-indent" => parse_length_px(value).map(DeclKind::TextIndent),
        "word-spacing" => parse_length_px(value).map(DeclKind::WordSpacing),
        "letter-spacing" => {
            // `normal` = sin tracking extra (0px).
            if value.trim().eq_ignore_ascii_case("normal") {
                Some(DeclKind::LetterSpacing(0.0))
            } else {
                parse_length_px(value).map(DeclKind::LetterSpacing)
            }
        }
        "text-shadow" => parse_text_shadows(value).map(DeclKind::TextShadows),
        "transform" => parse_transforms(value).map(DeclKind::Transforms),
        "grid-template-columns" => {
            parse_grid_template(value).map(DeclKind::GridTemplateColumns)
        }
        "grid-template-rows" => parse_grid_template(value).map(DeclKind::GridTemplateRows),
        "animation" => parse_animation(value),
        "transition" => parse_transition(value),
        // `grid-gap` (legacy) = `gap`.
        "grid-gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "grid-row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "grid-column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
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
pub(crate) fn parse_nth_arg(arg: &str) -> Option<(i32, i32)> {
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
pub(crate) fn parse_box_shadow(value: &str) -> Option<BoxShadow> {
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

pub(crate) fn parse_border_style(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "solid" | "dashed" | "dotted" | "double" => Some(true),
        "none" | "hidden" => Some(false),
        _ => None,
    }
}

/// Parsea el shorthand `border: <width> <style> <color>` (componentes en
/// cualquier orden). Devuelve hasta 3 decls. Si falta el style, se asume
/// `solid`. Cualquier "none" en la posición de style desactiva el border.
pub(crate) fn parse_border_shorthand(value: &str, important: bool) -> Vec<Decl> {
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

/// Match propiedades `border-{top|right|bottom|left}{suffix}`. `suffix`
/// puede ser "" (shorthand), "-width", "-color", o "-style". Devuelve
/// el `BorderEdge` matcheado, o `None` si no aplica.
pub(crate) fn match_border_side_prop(prop: &str, suffix: &str) -> Option<BorderEdge> {
    let lc = prop.to_ascii_lowercase();
    for (name, edge) in [
        ("border-top", BorderEdge::Top),
        ("border-right", BorderEdge::Right),
        ("border-bottom", BorderEdge::Bottom),
        ("border-left", BorderEdge::Left),
    ] {
        if lc.len() == name.len() + suffix.len()
            && lc.starts_with(name)
            && lc[name.len()..].eq_ignore_ascii_case(suffix)
        {
            return Some(edge);
        }
    }
    None
}

/// Match propiedades `border-{top|bottom}-{left|right}-radius`.
pub(crate) fn match_border_corner_prop(prop: &str) -> Option<BorderCorner> {
    match prop.to_ascii_lowercase().as_str() {
        "border-top-left-radius" => Some(BorderCorner::TopLeft),
        "border-top-right-radius" => Some(BorderCorner::TopRight),
        "border-bottom-right-radius" => Some(BorderCorner::BottomRight),
        "border-bottom-left-radius" => Some(BorderCorner::BottomLeft),
        _ => None,
    }
}

/// Shorthand `border-top: <width> <style> <color>` (componentes en
/// cualquier orden, sólo afecta a un lado). Mismo formato que `border:`
/// pero las decls resultantes son las variantes per-side.
pub(crate) fn parse_border_side_shorthand(edge: BorderEdge, value: &str, important: bool) -> Vec<Decl> {
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
    if style_on.is_none() && (width.is_some() || color.is_some()) {
        style_on = Some(true);
    }
    let mut out = Vec::new();
    if let Some(on) = style_on {
        out.push(Decl { kind: DeclKind::BorderSideStyle(edge, on), important });
    }
    if let Some(w) = width {
        out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
    }
    if let Some(c) = color {
        out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
    }
    out
}

/// Propiedades lógicas de borde → físicas (LTR + escritura horizontal):
/// `border-inline*` ↔ left/right, `border-block*` ↔ top/bottom; `-start` =
/// left/top, `-end` = right/bottom. Cubre el shorthand (`border-inline:`),
/// los de ambos lados por propiedad (`border-inline-width/-color/-style`),
/// los de un lado (`border-inline-start:`) y los longhands de un lado
/// (`border-inline-start-width`, etc.). Fase 7.193.
pub(crate) fn parse_logical_border(prop: &str, value: &str, important: bool) -> Option<Vec<Decl>> {
    let lc = prop.to_ascii_lowercase();
    let rest = lc.strip_prefix("border-")?;
    // (start, end) según el eje.
    let (axis, after) = if let Some(a) = rest.strip_prefix("inline") {
        ((BorderEdge::Left, BorderEdge::Right), a)
    } else if let Some(a) = rest.strip_prefix("block") {
        ((BorderEdge::Top, BorderEdge::Bottom), a)
    } else {
        return None;
    };
    // `after` aísla lado (`-start`/`-end`/ambos) y sub-propiedad.
    let (edges, sub): (Vec<BorderEdge>, &str) = if let Some(s) = after.strip_prefix("-start") {
        (vec![axis.0], s)
    } else if let Some(s) = after.strip_prefix("-end") {
        (vec![axis.1], s)
    } else {
        (vec![axis.0, axis.1], after)
    };
    let mut out = Vec::new();
    for edge in edges {
        match sub {
            "" => out.extend(parse_border_side_shorthand(edge, value, important)),
            "-width" => {
                if let Some(w) = parse_length_px(value) {
                    out.push(Decl { kind: DeclKind::BorderSideWidth(edge, w), important });
                }
            }
            "-color" => {
                if let Some(c) = parse_color(value) {
                    out.push(Decl { kind: DeclKind::BorderSideColor(edge, c), important });
                }
            }
            "-style" => {
                if let Some(s) = parse_border_style(value) {
                    out.push(Decl { kind: DeclKind::BorderSideStyle(edge, s), important });
                }
            }
            _ => return None, // sufijo desconocido → no es una lógica de borde
        }
    }
    Some(out)
}

/// Parsea `text-decoration` o `text-decoration-line`. Acepta el shorthand
/// con varios tokens — busca el primer keyword reconocido como line y
/// devuelve eso. Estilos (`dotted`/`wavy`) y color se ignoran (sólo
/// pintamos línea sólida del color del texto).
pub(crate) fn parse_text_decoration(value: &str) -> Option<TextDecorationLine> {
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
pub(crate) fn parse_list_style_type(s: &str) -> Option<ListStyleType> {
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
pub(crate) fn parse_list_style_shorthand(s: &str) -> Option<ListStyleType> {
    for tok in s.split_whitespace() {
        if let Some(t) = parse_list_style_type(tok) {
            return Some(t);
        }
    }
    None
}

pub(crate) fn parse_text_align(s: &str) -> Option<TextAlign> {
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
pub(crate) fn parse_length_or_pct(s: &str) -> Option<LengthVal> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(LengthVal::Auto);
    }
    if let Some(num) = s.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    if let Some(inner) = strip_calc(s) {
        return parse_calc_expr(inner);
    }
    parse_length_px(s).map(LengthVal::Px)
}

/// Parsea el value de `content:` para pseudo-elements. Soporta una
/// secuencia de items separados por whitespace: strings quoted,
/// `counter(name)` y `attr(name)`. Devuelve `None` para `none`/`normal`
/// (que suprime el pseudo-element) o si encuentra algo no reconocible.
pub(crate) fn parse_content_value(value: &str) -> Option<Vec<ContentItem>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("normal") {
        return None;
    }
    let mut items = Vec::new();
    let mut chars = v.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' || c == '\'' {
            let item = parse_string_literal(&mut chars)?;
            items.push(ContentItem::Text(item));
            continue;
        }
        // Identificador: `counter(...)` o `attr(...)` (case-insensitive).
        let ident = read_ident(&mut chars);
        if ident.is_empty() {
            return None;
        }
        let lower = ident.to_ascii_lowercase();
        // Comer paréntesis de apertura.
        if chars.next() != Some('(') {
            return None;
        }
        let arg = read_until(&mut chars, ')')?;
        let arg = arg.trim();
        // `counter(name[, list-style])`: nos quedamos con el name; el
        // list-style queda para más adelante.
        let name = arg.split(',').next().unwrap_or("").trim();
        if name.is_empty() {
            return None;
        }
        match lower.as_str() {
            "counter" => items.push(ContentItem::Counter(name.to_string())),
            "attr" => items.push(ContentItem::Attr(name.to_string())),
            "url" => {
                // El arg de url() puede venir entre comillas o sin.
                // arg ya fue trimmeado del paréntesis exterior; acá
                // strippeamos comillas si las hay y devolvemos el resto
                // sin trim adicional (las URLs pueden tener espacios
                // encodeados pero no whitespace literal interno).
                let raw = arg.trim();
                let clean = raw
                    .trim_start_matches(['"', '\''].as_ref())
                    .trim_end_matches(['"', '\''].as_ref())
                    .trim()
                    .to_string();
                if clean.is_empty() {
                    return None;
                }
                items.push(ContentItem::Url(clean));
            }
            _ => return None, // `counters(...)` no soportado aún.
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// Lee una string literal (incluyendo las comillas) de `chars` —
/// consume hasta encontrar la comilla de cierre matching. Soporta
/// escape `\X` que vuelca X tal cual. Devuelve None si la string queda
/// sin cerrar.
pub(crate) fn parse_string_literal(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
    let quote = chars.next()?;
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(esc) = chars.next() {
                out.push(esc);
                continue;
            }
            return None;
        }
        if c == quote {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// Lee chars mientras sean alfanuméricos, `-` o `_`. Devuelve el ident
/// como String (vacío si el siguiente char no era válido).
pub(crate) fn read_ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
            chars.next();
        } else {
            break;
        }
    }
    out
}

/// Lee chars hasta el delimitador `end` (exclusivo) — lo consume. Devuelve
/// el contenido. None si no encuentra el delim.
pub(crate) fn read_until(chars: &mut std::iter::Peekable<std::str::Chars>, end: char) -> Option<String> {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == end {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// Parsea `counter-reset` o `counter-increment`. Devuelve pares
/// `(name, value)` — para reset el default es `0`, para increment es
/// `1`. Si el value es `none`, devuelve vec vacío.
pub(crate) fn parse_counter_list(value: &str, default: i32) -> Vec<(String, i32)> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let mut out: Vec<(String, i32)> = Vec::new();
    let toks: Vec<&str> = v.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        let name = toks[i];
        if !is_valid_counter_name(name) {
            // Token no nombre — skip (parser tolerante).
            i += 1;
            continue;
        }
        let value = toks
            .get(i + 1)
            .and_then(|t| t.parse::<i32>().ok());
        if let Some(v) = value {
            out.push((name.to_string(), v));
            i += 2;
        } else {
            out.push((name.to_string(), default));
            i += 1;
        }
    }
    out
}

pub(crate) fn is_valid_counter_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Si `s` matchea `calc(...)` (case-insensitive), devuelve el contenido
/// entre paréntesis. Sino `None`.
pub(crate) fn strip_calc(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();
    let stripped = lower.strip_prefix("calc(")?.strip_suffix(')')?;
    // Recortamos del original (mantiene casing del inner por si tiene
    // hex colors en el futuro — hoy sólo números/units, no importa).
    let start = "calc(".len();
    Some(&s[start..s.len() - 1])
        .filter(|_| !stripped.is_empty())
}

/// Parsea un expression `calc()` mínimo: `<term> <+|-> <term>` o un
/// único `<term>`. Resuelve en parse time, conservando `Pct` cuando hay
/// mezcla (caso `calc(100% - 20px)` queda como `Pct(100)` y se pierde
/// el offset — taffy no soporta calc nativo y aproximarlo a más
/// precisión requeriría conocer el container, que no tenemos acá).
pub(crate) fn parse_calc_expr(inner: &str) -> Option<LengthVal> {
    let toks = tokenize_calc(inner);
    if toks.is_empty() || toks.len() % 2 == 0 {
        // Sin tokens, o longitud par (1+op+term tiene que ser impar).
        return None;
    }
    let mut acc = parse_calc_term(toks[0])?;
    let mut i = 1;
    while i + 1 < toks.len() {
        let op = toks[i];
        let rhs = parse_calc_term(toks[i + 1])?;
        acc = combine_calc(acc, op, rhs)?;
        i += 2;
    }
    Some(acc)
}

/// Tokens del calc: separamos números+unidad de operadores `+`/`-`/`*`/
/// `/`. CSS spec requiere whitespace alrededor de `+`/`-` (no de `*`/`/`).
/// Por simplicidad sólo soportamos `+` y `-` con whitespace.
pub(crate) fn tokenize_calc(s: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b' ' || c == b'\t' || c == b'\n' {
            if i > start {
                out.push(&s[start..i]);
            }
            // Detectar operador como token único si está rodeado de spaces.
            if i + 1 < bytes.len() && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-') {
                // Skip leading spaces hasta el operador.
                let op_start = i + 1;
                if op_start + 1 < bytes.len()
                    && (bytes[op_start + 1] == b' ' || bytes[op_start + 1] == b'\t')
                {
                    out.push(&s[op_start..op_start + 1]);
                    i = op_start + 1;
                    start = i;
                    continue;
                }
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(&s[start..]);
    }
    out
}

pub(crate) fn parse_calc_term(tok: &str) -> Option<LengthVal> {
    let tok = tok.trim();
    if let Some(num) = tok.strip_suffix('%') {
        return num.trim().parse::<f32>().ok().map(LengthVal::Pct);
    }
    parse_length_px(tok).map(LengthVal::Px)
}

pub(crate) fn combine_calc(a: LengthVal, op: &str, b: LengthVal) -> Option<LengthVal> {
    let sign = match op {
        "+" => 1.0,
        "-" => -1.0,
        _ => return None,
    };
    match (a, b) {
        (LengthVal::Px(x), LengthVal::Px(y)) => Some(LengthVal::Px(x + sign * y)),
        (LengthVal::Pct(x), LengthVal::Pct(y)) => Some(LengthVal::Pct(x + sign * y)),
        // Mezcla pct/px: conservamos el pct ignorando el offset px.
        // Aproximación pragmática — taffy no soporta calc nativo y un
        // valor mixto requeriría el container width, no disponible acá.
        (LengthVal::Pct(p), LengthVal::Px(_)) | (LengthVal::Px(_), LengthVal::Pct(p)) => {
            Some(LengthVal::Pct(p))
        }
        _ => None,
    }
}

/// Acepta multiplicador adimensional (`1.5`, `1.6`), `Npx`, `Nem`/`Nrem`.
/// Devuelve siempre un multiplicador (px se divide por 16; `em`/`rem`
/// salen como ya están). Imperfecto pero alcanza para Fase 4.
pub(crate) fn parse_line_height(s: &str) -> Option<f32> {
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

/// Versión pública para que `boxes` parsee colors de attrs SVG.
pub(crate) fn parse_color_named_or_hex(s: &str) -> Option<Color> {
    parse_color(s)
}

/// Parsea un color CSS (`#rgb`/`#rrggbb`/`#rrggbbaa`, `rgb()`/`rgba()`,
/// `hsl()`/`hsla()`, named colors) a [`Color`]. Público para que el chrome
/// pinte `fillStyle`/`strokeStyle` de canvas (Fase 7.196). `None` si no
/// parsea.
pub fn parse_color(s: &str) -> Option<Color> {
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
pub(crate) fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
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
pub(crate) fn parse_rgb_func(args: &str) -> Option<Color> {
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
pub(crate) fn parse_hsl_func(args: &str) -> Option<Color> {
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
pub(crate) fn split_color_args(args: &str) -> Option<(Vec<&str>, Option<&str>)> {
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
pub(crate) fn parse_color_chan(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    s.parse::<i32>().ok().map(|n| n.clamp(0, 255) as u8)
}

/// Alpha: fracción 0.0-1.0 o porcentaje 0%-100%.
pub(crate) fn parse_alpha(s: &str) -> Option<u8> {
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
pub(crate) fn parse_hue(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("deg").unwrap_or(s);
    s.trim().parse().ok()
}

/// Porcentaje 0%-100% → fracción 0.0-1.0.
pub(crate) fn parse_pct(s: &str) -> Option<f32> {
    let s = s.trim().strip_suffix('%')?;
    let pct: f32 = s.trim().parse().ok()?;
    Some((pct / 100.0).clamp(0.0, 1.0))
}

/// HSL→RGB estándar (CSS Color Module L3). h en grados, s/l en 0..1.
pub(crate) fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
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
pub(crate) fn parse_sides(value: &str) -> Option<Sides<f32>> {
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

pub(crate) fn parse_weight(s: &str) -> Option<u16> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "lighter" => Some(300),
        "bolder" => Some(700),
        num => num.parse().ok(),
    }
}

pub(crate) fn parse_font_style(s: &str) -> Option<FontStyle> {
    // CSS spec: normal | italic | oblique [<angle>?]. Tratamos oblique
    // como italic — parley/fontique sintetizan si la fuente no tiene
    // oblique nativo.
    let v = s.trim().to_ascii_lowercase();
    if v == "normal" {
        Some(FontStyle::Normal)
    } else if v == "italic" || v.starts_with("oblique") {
        Some(FontStyle::Italic)
    } else {
        None
    }
}

pub(crate) fn parse_display(s: &str) -> Option<Display> {
    match s.trim().to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "none" => Some(Display::None),
        _ => None,
    }
}

pub(crate) fn parse_flex_direction(s: &str) -> Option<FlexDirection> {
    match s.trim().to_ascii_lowercase().as_str() {
        "row" => Some(FlexDirection::Row),
        "row-reverse" => Some(FlexDirection::RowReverse),
        "column" => Some(FlexDirection::Column),
        "column-reverse" => Some(FlexDirection::ColumnReverse),
        _ => None,
    }
}

pub(crate) fn parse_flex_wrap(s: &str) -> Option<FlexWrap> {
    match s.trim().to_ascii_lowercase().as_str() {
        "nowrap" => Some(FlexWrap::NoWrap),
        "wrap" => Some(FlexWrap::Wrap),
        "wrap-reverse" => Some(FlexWrap::WrapReverse),
        _ => None,
    }
}

pub(crate) fn parse_justify_content(s: &str) -> Option<JustifyContent> {
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

pub(crate) fn parse_align_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "start" | "flex-start" => Some(AlignItems::Start),
        "center" => Some(AlignItems::Center),
        "end" | "flex-end" => Some(AlignItems::End),
        "stretch" => Some(AlignItems::Stretch),
        "baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

/// `align-content`. `normal` y `baseline` colapsan a `Normal` (default de
/// taffy ≈ stretch); el resto mapea directo. `start`/`end` aceptan también
/// la variante `flex-*`.
pub(crate) fn parse_align_content(s: &str) -> Option<AlignContent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" | "baseline" => Some(AlignContent::Normal),
        "start" | "flex-start" => Some(AlignContent::Start),
        "center" => Some(AlignContent::Center),
        "end" | "flex-end" => Some(AlignContent::End),
        "stretch" => Some(AlignContent::Stretch),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        _ => None,
    }
}

/// `justify-items` (grid). Reusa el subset de `align-items` y agrega
/// `left`/`right` (que en escritura LTR equivalen a start/end). `normal`
/// se descarta → queda el default None. `auto`/`legacy` también.
pub(crate) fn parse_justify_items(s: &str) -> Option<AlignItems> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" => Some(AlignItems::Start),
        "right" => Some(AlignItems::End),
        other => parse_align_items(other),
    }
}

/// `justify-self` (grid item). Reusa `align-self` + `left`/`right`.
pub(crate) fn parse_justify_self(s: &str) -> Option<AlignSelf> {
    match s.trim().to_ascii_lowercase().as_str() {
        "left" => Some(AlignSelf::Start),
        "right" => Some(AlignSelf::End),
        other => parse_align_self(other),
    }
}

/// `place-content: <align-content> [<justify-content>]`. Un solo valor
/// setea ambos ejes. Cada mitad se valida con su parser propio; las que no
/// parsean se descartan (el otro eje igual se aplica).
pub(crate) fn parse_place_content_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(ac) = parse_align_content(a) {
        out.push(Decl { kind: DeclKind::AlignContent(ac), important });
    }
    if let Some(jc) = parse_justify_content(b) {
        out.push(Decl { kind: DeclKind::JustifyContent(jc), important });
    }
    out
}

/// `place-items: <align-items> [<justify-items>]`. Un solo valor = ambos.
pub(crate) fn parse_place_items_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(ai) = parse_align_items(a) {
        out.push(Decl { kind: DeclKind::AlignItems(ai), important });
    }
    if let Some(ji) = parse_justify_items(b) {
        out.push(Decl { kind: DeclKind::JustifyItems(ji), important });
    }
    out
}

/// `place-self: <align-self> [<justify-self>]`. Un solo valor = ambos.
pub(crate) fn parse_place_self_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut it = value.split_whitespace();
    let Some(a) = it.next() else { return out };
    let b = it.next().unwrap_or(a);
    if let Some(asf) = parse_align_self(a) {
        out.push(Decl { kind: DeclKind::AlignSelf(asf), important });
    }
    if let Some(jsf) = parse_justify_self(b) {
        out.push(Decl { kind: DeclKind::JustifySelf(jsf), important });
    }
    out
}

/// `gap: V` ⇒ row=V, column=V. `gap: R C` ⇒ row=R, column=C. Coincide
/// con la semántica CSS shorthand (primer valor = row, segundo = column).
pub(crate) fn parse_gap(value: &str) -> Option<(f32, f32)> {
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

pub(crate) fn parse_box_sizing(s: &str) -> Option<BoxSizing> {
    match s.trim().to_ascii_lowercase().as_str() {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

pub(crate) fn parse_overflow(s: &str) -> Option<Overflow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Overflow::Visible),
        // hidden/clip/auto/scroll todos los tratamos como Hidden por
        // ahora (no soportamos scroll real; clip y hidden cortan igual).
        "hidden" | "clip" | "auto" | "scroll" => Some(Overflow::Hidden),
        _ => None,
    }
}

pub(crate) fn parse_white_space(s: &str) -> Option<WhiteSpace> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "nowrap" => Some(WhiteSpace::NoWrap),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None,
    }
}

pub(crate) fn parse_text_transform(s: &str) -> Option<TextTransform> {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None,
    }
}

/// Acepta `0..1` o `0%..100%`. Clampa.
pub(crate) fn parse_opacity(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        let pct: f32 = num.trim().parse().ok()?;
        return Some((pct / 100.0).clamp(0.0, 1.0));
    }
    s.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
}

pub(crate) fn parse_align_self(s: &str) -> Option<AlignSelf> {
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
/// Propiedades lógicas de caja (`margin-inline`/`margin-block`/`padding-*` y
/// sus `-start`/`-end`), mapeadas a las físicas asumiendo LTR + escritura
/// horizontal (el caso por defecto). `inline` ↔ left/right, `block` ↔
/// top/bottom; `start`=left/top, `end`=right/bottom. Las dos-lados aceptan
/// 1–2 valores (`margin-inline: 10px` o `10px 20px`). Devuelve `None` si el
/// nombre no es una propiedad lógica conocida. Fase 7.191.
pub(crate) fn parse_logical_box(prop: &str, value: &str, important: bool) -> Option<Vec<Decl>> {
    use DeclKind::{
        MarginBottom, MarginLeft, MarginRight, MarginTop, PaddingBottom, PaddingLeft,
        PaddingRight, PaddingTop,
    };
    use DeclKind::{InsetBottom, InsetLeft, InsetRight, InsetTop};
    let lower = prop.to_ascii_lowercase();
    // `inset-inline`/`inset-block` y sus `-start`/`-end`: usan `LengthVal`
    // (length/%/auto), no `f32` como margin/padding — firma aparte.
    let inset_two: Option<(fn(LengthVal) -> DeclKind, fn(LengthVal) -> DeclKind)> =
        match lower.as_str() {
            "inset-inline" => Some((InsetLeft, InsetRight)),
            "inset-block" => Some((InsetTop, InsetBottom)),
            _ => None,
        };
    if let Some((a, b)) = inset_two {
        let parts: Vec<&str> = value.split_whitespace().collect();
        let vals: Vec<LengthVal> =
            parts.iter().filter_map(|p| parse_length_or_pct_or_auto(p)).collect();
        if vals.is_empty() || vals.len() != parts.len() {
            return Some(Vec::new());
        }
        let (s, e) = if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
        return Some(vec![
            Decl { kind: a(s), important },
            Decl { kind: b(e), important },
        ]);
    }
    let inset_single: Option<fn(LengthVal) -> DeclKind> = match lower.as_str() {
        "inset-inline-start" => Some(InsetLeft),
        "inset-inline-end" => Some(InsetRight),
        "inset-block-start" => Some(InsetTop),
        "inset-block-end" => Some(InsetBottom),
        _ => None,
    };
    if let Some(ctor) = inset_single {
        return Some(
            parse_length_or_pct_or_auto(value)
                .map(|v| vec![Decl { kind: ctor(v), important }])
                .unwrap_or_default(),
        );
    }
    // Lados emparejados (1–2 valores): (start_ctor, end_ctor).
    let two: Option<(fn(f32) -> DeclKind, fn(f32) -> DeclKind)> = match lower.as_str() {
        "margin-inline" => Some((MarginLeft, MarginRight)),
        "margin-block" => Some((MarginTop, MarginBottom)),
        "padding-inline" => Some((PaddingLeft, PaddingRight)),
        "padding-block" => Some((PaddingTop, PaddingBottom)),
        _ => None,
    };
    if let Some((a, b)) = two {
        let parts: Vec<&str> = value.split_whitespace().collect();
        let vals: Vec<f32> = parts.iter().filter_map(|p| parse_length_px(p)).collect();
        if vals.is_empty() || vals.len() != parts.len() {
            return Some(Vec::new());
        }
        let (s, e) = if vals.len() == 1 { (vals[0], vals[0]) } else { (vals[0], vals[1]) };
        return Some(vec![
            Decl { kind: a(s), important },
            Decl { kind: b(e), important },
        ]);
    }
    // Un solo lado (`-start`/`-end`).
    let single: Option<fn(f32) -> DeclKind> = match lower.as_str() {
        "margin-inline-start" => Some(MarginLeft),
        "margin-inline-end" => Some(MarginRight),
        "margin-block-start" => Some(MarginTop),
        "margin-block-end" => Some(MarginBottom),
        "padding-inline-start" => Some(PaddingLeft),
        "padding-inline-end" => Some(PaddingRight),
        "padding-block-start" => Some(PaddingTop),
        "padding-block-end" => Some(PaddingBottom),
        _ => None,
    };
    let ctor = single?;
    Some(
        parse_length_px(value)
            .map(|v| vec![Decl { kind: ctor(v), important }])
            .unwrap_or_default(),
    )
}

/// `inset: <t> [r] [b] [l]` — 1..4 valores con la distribución de `margin`
/// (1→todos, 2→TB/LR, 3→T/LR/B, 4→TRBL). Cada valor acepta length/%/auto.
/// Expande a los cuatro longhands `top`/`right`/`bottom`/`left`. Fase 7.189.
pub(crate) fn parse_inset_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let vals: Vec<LengthVal> =
        parts.iter().filter_map(|p| parse_length_or_pct_or_auto(p)).collect();
    // Si algún token no parsea, descartamos el shorthand entero (CSS spec).
    if vals.is_empty() || vals.len() != parts.len() {
        return Vec::new();
    }
    let (t, r, b, l) = match vals.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b2] => (*a, *b2, *a, *b2),
        [a, b2, c] => (*a, *b2, *c, *b2),
        [a, b2, c, d, ..] => (*a, *b2, *c, *d),
        [] => return Vec::new(),
    };
    vec![
        Decl { kind: DeclKind::InsetTop(t), important },
        Decl { kind: DeclKind::InsetRight(r), important },
        Decl { kind: DeclKind::InsetBottom(b), important },
        Decl { kind: DeclKind::InsetLeft(l), important },
    ]
}

/// `flex-flow: <direction> || <wrap>` (en cualquier orden) → `flex-direction`
/// + `flex-wrap`. Fase 7.189.
pub(crate) fn parse_flex_flow_shorthand(value: &str, important: bool) -> Vec<Decl> {
    let mut out = Vec::new();
    for tok in value.split_whitespace() {
        if let Some(d) = parse_flex_direction(tok) {
            out.push(Decl { kind: DeclKind::FlexDirection(d), important });
        } else if let Some(w) = parse_flex_wrap(tok) {
            out.push(Decl { kind: DeclKind::FlexWrap(w), important });
        }
    }
    out
}

pub(crate) fn parse_flex_shorthand(value: &str, important: bool) -> Vec<Decl> {
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
pub(crate) fn parse_outline_shorthand(value: &str, important: bool) -> Vec<Decl> {
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
pub(crate) fn parse_background_image(value: &str) -> Option<DeclKind> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(DeclKind::BackgroundGradientNone);
    }
    if let Some(args) = strip_fn(v, "linear-gradient") {
        return parse_linear_gradient(args).map(DeclKind::BackgroundGradient);
    }
    if let Some(args) = strip_fn(v, "url") {
        // url('foo') / url("foo") / url(foo) — trimea comillas.
        let raw = args.trim();
        let unquoted = raw
            .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(raw);
        let url = unquoted.trim();
        if url.is_empty() {
            return None;
        }
        return Some(DeclKind::BackgroundImageUrl(url.to_string()));
    }
    // Otros gradientes (`radial-gradient`, `conic-gradient`) o `cross-fade`
    // no soportados — silencio.
    None
}

/// Parsea el contenido de `linear-gradient(...)`. Sintaxis aceptada:
/// - `linear-gradient(<angle>?, <stop>, <stop>, ...)`
/// - `linear-gradient(to <side>?, <stop>, <stop>, ...)`
/// `<angle>` en `Ndeg` o `Nturn` (turn × 360 = grados). Default 180
/// (top→bottom). `to right`=90, `to left`=270, `to top`=0, `to bottom`=180,
/// combinaciones diagonales (`to top right`=45) también. Stops: `<color>
/// <pos>?` donde pos es `N%` o `Npx`.
pub(crate) fn parse_linear_gradient(args: &str) -> Option<LinearGradient> {
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
pub(crate) fn parse_gradient_direction(s: &str) -> (Option<f32>, bool) {
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

pub(crate) fn parse_gradient_stop(s: &str) -> Option<GradientStop> {
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
/// `Nvw`/`Nvh`/`Nvmin`/`Nvmax` resuelven contra el viewport activo
/// ([`resolve_viewport`]): el real bajo un `ViewportScope` (carga normal),
/// `DEFAULT_VIEWPORT` fuera de él (parsers sueltos en tests).
pub(crate) fn parse_length_px(s: &str) -> Option<f32> {
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
    if let Some(num) = s.strip_suffix("vmin") {
        let v: f32 = num.trim().parse().ok()?;
        let vp = resolve_viewport();
        return Some(v * vp.width.min(vp.height) / 100.0);
    }
    if let Some(num) = s.strip_suffix("vmax") {
        let v: f32 = num.trim().parse().ok()?;
        let vp = resolve_viewport();
        return Some(v * vp.width.max(vp.height) / 100.0);
    }
    if let Some(num) = s.strip_suffix("vw") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * resolve_viewport().width / 100.0);
    }
    if let Some(num) = s.strip_suffix("vh") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(v * resolve_viewport().height / 100.0);
    }
    s.parse().ok()
}

/// `length`, `%` o `auto`. Variante para insets que sí admiten `auto`.
pub(crate) fn parse_length_or_pct_or_auto(s: &str) -> Option<LengthVal> {
    parse_length_or_pct(s.trim())
}

pub(crate) fn parse_position(s: &str) -> Option<Position> {
    match s.trim().to_ascii_lowercase().as_str() {
        "static" => Some(Position::Static),
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        "sticky" => Some(Position::Sticky),
        _ => None,
    }
}

pub(crate) fn parse_vertical_align(s: &str) -> Option<VerticalAlign> {
    match s.trim().to_ascii_lowercase().as_str() {
        "baseline" => Some(VerticalAlign::Baseline),
        "top" | "text-top" => Some(VerticalAlign::Top),
        "middle" => Some(VerticalAlign::Middle),
        "bottom" | "text-bottom" => Some(VerticalAlign::Bottom),
        "super" => Some(VerticalAlign::Super),
        "sub" => Some(VerticalAlign::Sub),
        _ => None,
    }
}

pub(crate) fn parse_visibility(s: &str) -> Option<Visibility> {
    match s.trim().to_ascii_lowercase().as_str() {
        "visible" => Some(Visibility::Visible),
        // `collapse` lo tratamos igual que hidden (sólo aplica a
        // tablas/flex en CSS spec, aproximación segura).
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

pub(crate) fn parse_pointer_events(s: &str) -> Option<PointerEvents> {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(PointerEvents::Auto),
        "none" => Some(PointerEvents::None),
        _ => None,
    }
}

/// `text-shadow: <x> <y> [blur] <color>[, <x> <y> [blur] <color>]*`.
/// `none` → vector vacío. Devuelve None si ningún shadow es válido.
pub(crate) fn parse_text_shadows(value: &str) -> Option<Vec<TextShadow>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for sh in v.split(',') {
        if let Some(s) = parse_one_text_shadow(sh) {
            out.push(s);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_one_text_shadow(s: &str) -> Option<TextShadow> {
    let mut lengths: Vec<f32> = Vec::with_capacity(3);
    let mut color: Option<Color> = None;
    for tok in s.split_whitespace() {
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
    Some(TextShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_px: lengths.get(2).copied().unwrap_or(0.0),
        color: color.unwrap_or(Color::BLACK),
    })
}

/// `transform: none` o cadena de funciones (`rotate(45deg) scale(2)
/// translate(10px, 20px)`). Acepta `translate(x)`, `translate(x, y)`,
/// `translateX(x)`, `translateY(y)`, `scale(s)`, `scale(sx, sy)`,
/// `scaleX(sx)`, `scaleY(sy)`, `rotate(Ndeg|Nrad|Nturn)`.
pub(crate) fn parse_transforms(value: &str) -> Option<Vec<Transform>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut rest = v;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let open = rest.find('(')?;
        let name = rest[..open].trim().to_ascii_lowercase();
        let mut depth = 1usize;
        let bytes = rest[open + 1..].as_bytes();
        let mut close = None;
        for (i, &c) in bytes.iter().enumerate() {
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let args = &rest[open + 1..open + 1 + close];
        let tr = parse_transform_fn(&name, args)?;
        out.push(tr);
        rest = &rest[open + 1 + close + 1..];
    }
    Some(out)
}

pub(crate) fn parse_transform_fn(name: &str, args: &str) -> Option<Transform> {
    let parts: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
    match name {
        "translate" => match parts.as_slice() {
            [x] => Some(Transform::Translate(parse_length_px(x)?, 0.0)),
            [x, y] => Some(Transform::Translate(parse_length_px(x)?, parse_length_px(y)?)),
            _ => None,
        },
        "translatex" => Some(Transform::Translate(parse_length_px(parts[0])?, 0.0)),
        "translatey" => Some(Transform::Translate(0.0, parse_length_px(parts[0])?)),
        "scale" => match parts.as_slice() {
            [s] => {
                let v = s.parse::<f32>().ok()?;
                Some(Transform::Scale(v, v))
            }
            [sx, sy] => {
                Some(Transform::Scale(sx.parse().ok()?, sy.parse().ok()?))
            }
            _ => None,
        },
        "scalex" => Some(Transform::Scale(parts[0].parse().ok()?, 1.0)),
        "scaley" => Some(Transform::Scale(1.0, parts[0].parse().ok()?)),
        "rotate" => {
            let arg = parts[0];
            let deg = if let Some(n) = arg.strip_suffix("deg") {
                n.trim().parse::<f32>().ok()?
            } else if let Some(n) = arg.strip_suffix("rad") {
                let v: f32 = n.trim().parse().ok()?;
                v.to_degrees()
            } else if let Some(n) = arg.strip_suffix("turn") {
                let v: f32 = n.trim().parse().ok()?;
                v * 360.0
            } else {
                // Sin unidad: asumir deg.
                arg.parse::<f32>().ok()?
            };
            Some(Transform::Rotate(deg))
        }
        _ => None,
    }
}

/// `grid-template-columns: <track-list>`. Subset soportado:
/// - `auto`
/// - `Npx` / `N%`
/// - `Nfr`
/// - `repeat(N, <track>)` con repeat de un solo track
/// Tokens separados por whitespace.
pub(crate) fn parse_grid_template(value: &str) -> Option<Vec<GridTrackSize>> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out: Vec<GridTrackSize> = Vec::new();
    // Tokenize: respeta nesting de paréntesis para repeat(N, X).
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    for tok in tokens {
        if let Some(inner) = strip_fn(&tok, "repeat") {
            let parts: Vec<&str> = inner.splitn(2, ',').collect();
            if parts.len() != 2 {
                continue;
            }
            let count: i32 = parts[0].trim().parse().ok()?;
            let track = parse_one_grid_track(parts[1].trim())?;
            for _ in 0..count.max(0) {
                out.push(track);
            }
        } else if let Some(t) = parse_one_grid_track(&tok) {
            out.push(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn parse_one_grid_track(s: &str) -> Option<GridTrackSize> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("auto") {
        return Some(GridTrackSize::Auto);
    }
    if let Some(num) = s.strip_suffix("fr") {
        let v: f32 = num.trim().parse().ok()?;
        return Some(GridTrackSize::Fr(v));
    }
    if let Some(lv) = parse_length_or_pct(s) {
        return Some(match lv {
            LengthVal::Px(v) => GridTrackSize::Px(v),
            LengthVal::Pct(v) => GridTrackSize::Pct(v),
            LengthVal::Auto => GridTrackSize::Auto,
        });
    }
    None
}

/// Evalúa una condición de `@media` contra el viewport por defecto. Subset:
/// `(max-width: Npx)`, `(min-width: Npx)`, encadenados por ` and `.
/// `screen`/`all` se ignoran (siempre true).
/// Evalúa una media query (`@media` en CSS y `window.matchMedia()` en JS) contra
/// el viewport actual. Soporta listas separadas por `,` (OR), `not`/`only`,
/// el combinador ` and `, tipos de media (`screen`/`all`/`print`/`speech`) y
/// las features: `min/max/exact-width`, `min/max/exact-height`, `orientation`
/// (portrait/landscape), `min/max/exact-resolution` (`Ndppx`/`Ndpi`/`Nx` vs
/// `vp.dpr`) y `prefers-color-scheme`/`prefers-reduced-motion` (reportamos
/// light / no-reduce). Features desconocidas se ignoran (no descalifican), igual
/// que el comportamiento previo, para no romper CSS que las use de forma
/// progresiva. Pública porque el chrome (`puriy-llimphi`) la reusa para resolver
/// `matchMedia` contra el viewport real de la ventana.
pub fn evaluate_media_query(condition: &str, vp: Viewport) -> bool {
    let cond = condition.trim().to_ascii_lowercase();
    if cond.is_empty() {
        return true;
    }
    // Media query LIST: separada por comas, matchea si CUALQUIER componente lo hace.
    if cond.contains(',') {
        return cond.split(',').any(|q| evaluate_media_query(q, vp));
    }
    // `not` a nivel de query invierte el resultado completo.
    if let Some(rest) = cond.strip_prefix("not ") {
        return !evaluate_media_query_terms(rest.trim(), vp);
    }
    evaluate_media_query_terms(&cond, vp)
}

/// Evalúa los términos unidos por ` and ` de una query ya sin `,`/`not` de tope.
pub(crate) fn evaluate_media_query_terms(cond: &str, vp: Viewport) -> bool {
    for part in cond.split(" and ").map(|s| s.trim()) {
        if part.is_empty() {
            continue;
        }
        // Tipos de media.
        if part == "all" || part == "screen" {
            continue;
        }
        if part == "print" || part == "speech" || part == "tty" {
            return false;
        }
        let part = part.strip_prefix("only ").unwrap_or(part).trim();
        // Esperamos `(feature)` o `(feature: value)`.
        let Some(inner) = part.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            // Token no reconocido (tipo de media raro): no matchea.
            return false;
        };
        if !evaluate_media_feature(inner.trim(), vp) {
            return false;
        }
    }
    true
}

/// Evalúa UNA feature `(feature)` o `(feature: value)` contra el viewport.
pub(crate) fn evaluate_media_feature(inner: &str, vp: Viewport) -> bool {
    let Some((feature, val)) = inner.split_once(':').map(|(a, b)| (a.trim(), b.trim())) else {
        // Feature booleana (sin valor): matchea si la capacidad "existe".
        return matches!(inner, "color" | "grid" | "hover" | "pointer");
    };
    match feature {
        "max-width" => parse_length_px(val).is_some_and(|l| vp.width <= l),
        "min-width" => parse_length_px(val).is_some_and(|l| vp.width >= l),
        "width" => parse_length_px(val).is_some_and(|l| (vp.width - l).abs() < 0.5),
        "max-height" => parse_length_px(val).is_some_and(|l| vp.height <= l),
        "min-height" => parse_length_px(val).is_some_and(|l| vp.height >= l),
        "height" => parse_length_px(val).is_some_and(|l| (vp.height - l).abs() < 0.5),
        "orientation" => match val {
            "portrait" => vp.height >= vp.width,
            "landscape" => vp.width > vp.height,
            _ => false,
        },
        "min-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr >= r),
        "max-resolution" => parse_resolution_dppx(val).is_some_and(|r| vp.dpr <= r),
        "resolution" => parse_resolution_dppx(val).is_some_and(|r| (vp.dpr - r).abs() < 0.01),
        "min-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height >= r)
        }
        "max-aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| vp.width / vp.height <= r)
        }
        "aspect-ratio" => {
            parse_aspect_ratio(val).is_some_and(|r| (vp.width / vp.height - r).abs() < 0.01)
        }
        // Preferencias del usuario: reportamos tema claro y sin reducción.
        "prefers-color-scheme" => val == "light" || val == "no-preference",
        "prefers-reduced-motion" => val == "no-preference",
        "prefers-contrast" => val == "no-preference",
        "hover" => val == "hover",
        "any-hover" => val == "hover",
        "pointer" => val == "fine",
        "any-pointer" => val == "fine",
        // Feature desconocida: no descalifica (comportamiento previo lenient).
        _ => true,
    }
}

/// Parsea un aspect-ratio de media query a un float `ancho/alto`. Acepta la
/// forma `W/H` (`16/9`) y el número suelto (`1.5`). `None` si no parsea o el
/// alto es cero.
pub(crate) fn parse_aspect_ratio(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some((w, h)) = v.split_once('/') {
        let w: f32 = w.trim().parse().ok()?;
        let h: f32 = h.trim().parse().ok()?;
        if h == 0.0 {
            return None;
        }
        Some(w / h)
    } else {
        v.parse::<f32>().ok()
    }
}

/// Parsea una resolución de media query a `dppx` (dots per px). Acepta
/// `Ndppx`, `Nx` (alias de dppx) y `Ndpi` (96dpi = 1dppx). `None` si no parsea.
pub(crate) fn parse_resolution_dppx(val: &str) -> Option<f32> {
    let v = val.trim();
    if let Some(n) = v.strip_suffix("dppx").or_else(|| v.strip_suffix('x')) {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = v.strip_suffix("dpi") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0)
    } else if let Some(n) = v.strip_suffix("dpcm") {
        n.trim().parse::<f32>().ok().map(|d| d / 96.0 * 2.54)
    } else {
        None
    }
}

/// Evalúa una condición `@supports (prop: value)` ⇒ true si nuestro
/// parser puede convertirla a algún DeclKind. Subset minimal: no
/// soporta `and`/`or`/`not` por ahora.
pub(crate) fn evaluate_supports_query(condition: &str) -> bool {
    let cond = condition.trim();
    let Some(inner) = cond.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let Some((prop, val)) = inner.split_once(':') else {
        return false;
    };
    decl_kind_from_pair(prop.trim(), val.trim()).is_some()
}

/// Indica que `cssparser` está enlazado aunque el subset actual no use
/// la API completa — la presencia del crate evita que `cargo` lo
/// pruebe y deja el camino abierto para Fase 3.
#[doc(hidden)]
pub fn _cssparser_anchor() {
    let _ = cssparser::ParserInput::new("");
}
