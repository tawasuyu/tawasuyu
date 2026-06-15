//! Fase 7.922+ — descriptores de at-rules (`@font-face`, …). Frente nuevo:
//! parse de reglas globales que antes se salteaban en bloque. Verificado vía
//! `StyleEngine::from_sheets_with_viewport` + accessor `font_faces()`.
use super::super::*;
use crate::dom::DomTree;

fn engine(css: &str) -> StyleEngine {
    StyleEngine::from_sheets_with_viewport(&[css.to_string()], DEFAULT_VIEWPORT)
}

/// Computa el color del primer `<p>` de un documento cuyo `<style>` contiene
/// `css`. Verifica que reglas dentro de `@layer` SÍ entran a la cascada.
fn p_color(css: &str) -> (u8, u8, u8) {
    let html = format!("<html><head><style>{css}</style></head><body><p></p></body></html>");
    let dom = DomTree::parse(&html);
    let eng = StyleEngine::from_dom(&dom);
    let mut p = None;
    crate::dom::walk(&dom.document(), &mut |n| {
        if crate::dom::element_name(n).as_deref() == Some("p") {
            p = Some(n.clone());
        }
    });
    let cs = eng.compute(p.as_ref().expect("hay <p>"));
    (cs.color.r, cs.color.g, cs.color.b)
}

// ── Fase 7.922 — @font-face (descriptores) ─────────────────────────────────

#[test]
fn font_face_basico() {
    let css = r#"
        @font-face {
            font-family: "My Font";
            src: url("/fonts/my.woff2") format("woff2");
        }
    "#;
    let faces = engine(css).font_faces().to_vec();
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].family, "My Font");
    assert_eq!(faces[0].sources.len(), 1);
    assert_eq!(faces[0].sources[0].url.as_deref(), Some("/fonts/my.woff2"));
    assert_eq!(faces[0].sources[0].format.as_deref(), Some("woff2"));
}

#[test]
fn font_face_src_lista_y_local() {
    let css = r#"
        @font-face {
            font-family: Inter;
            src: local("Inter"), url(inter.woff2) format("woff2"), url(inter.ttf) format("truetype");
            font-weight: 100 900;
            font-style: italic;
            font-display: swap;
            unicode-range: U+0000-00FF, U+2000-206F;
        }
    "#;
    let f = engine(css).font_faces().to_vec();
    assert_eq!(f.len(), 1);
    let r = &f[0];
    assert_eq!(r.family, "Inter");
    assert_eq!(r.sources.len(), 3);
    assert_eq!(r.sources[0].local.as_deref(), Some("Inter"));
    assert_eq!(r.sources[1].url.as_deref(), Some("inter.woff2"));
    assert_eq!(r.sources[2].format.as_deref(), Some("truetype"));
    assert_eq!(r.weight.as_deref(), Some("100 900"));
    assert_eq!(r.style.as_deref(), Some("italic"));
    assert_eq!(r.display.as_deref(), Some("swap"));
    assert_eq!(r.unicode_range.as_deref(), Some("U+0000-00FF, U+2000-206F"));
}

#[test]
fn font_face_multiple_mismo_family() {
    // Dos @font-face con el mismo family (rangos/pesos distintos) = ambos.
    let css = r#"
        @font-face { font-family: Roboto; src: url(roboto-reg.woff2); font-weight: 400; }
        @font-face { font-family: Roboto; src: url(roboto-bold.woff2); font-weight: 700; }
    "#;
    let f = engine(css).font_faces().to_vec();
    assert_eq!(f.len(), 2);
    assert_eq!(f[0].weight.as_deref(), Some("400"));
    assert_eq!(f[1].weight.as_deref(), Some("700"));
}

#[test]
fn font_face_metricas_override() {
    let css = r#"
        @font-face {
            font-family: Fallback;
            src: local("Arial");
            ascent-override: 90%;
            descent-override: 20%;
            line-gap-override: 0%;
            size-adjust: 110%;
            font-feature-settings: "liga" 1;
            font-variation-settings: "wght" 700;
        }
    "#;
    let f = engine(css).font_faces().to_vec();
    let r = &f[0];
    assert_eq!(r.ascent_override.as_deref(), Some("90%"));
    assert_eq!(r.descent_override.as_deref(), Some("20%"));
    assert_eq!(r.line_gap_override.as_deref(), Some("0%"));
    assert_eq!(r.size_adjust.as_deref(), Some("110%"));
    assert_eq!(r.feature_settings.as_deref(), Some("\"liga\" 1"));
    assert_eq!(r.variation_settings.as_deref(), Some("\"wght\" 700"));
}

#[test]
fn font_face_invalido_se_descarta() {
    // sin font-family → descartado.
    let sin_family = engine("@font-face { src: url(x.woff2); }");
    assert!(sin_family.font_faces().is_empty());
    // sin src válido → descartado.
    let sin_src = engine("@font-face { font-family: X; }");
    assert!(sin_src.font_faces().is_empty());
    // family vacío → descartado.
    let vacio = engine(r#"@font-face { font-family: ""; src: url(x.woff2); }"#);
    assert!(vacio.font_faces().is_empty());
}

#[test]
fn font_face_convive_con_reglas_normales() {
    // El @font-face no rompe la cascada de las reglas normales del mismo sheet.
    let css = r#"
        @font-face { font-family: Brand; src: url(brand.woff2); }
        p { color: rgb(10, 20, 30); }
    "#;
    let eng = engine(css);
    assert_eq!(eng.font_faces().len(), 1);
    // y la regla `p` sigue presente (no se tragó el parser de at-rule).
    assert_eq!(eng.font_faces()[0].family, "Brand");
}

// ── Fase 7.923 — @property (Houdini) ───────────────────────────────────────

#[test]
fn at_property_basico() {
    let css = r#"
        @property --my-color {
            syntax: "<color>";
            inherits: false;
            initial-value: rebeccapurple;
        }
    "#;
    let p = engine(css).registered_properties().to_vec();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].name, "--my-color");
    assert_eq!(p[0].syntax, "<color>");
    assert!(!p[0].inherits);
    assert_eq!(p[0].initial_value.as_deref(), Some("rebeccapurple"));
}

#[test]
fn at_property_universal_sin_initial() {
    // syntax "*" no exige initial-value.
    let css = r#"@property --x { syntax: "*"; inherits: true; }"#;
    let p = engine(css).registered_properties().to_vec();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].syntax, "*");
    assert!(p[0].inherits);
    assert!(p[0].initial_value.is_none());
}

#[test]
fn at_property_invalido_se_descarta() {
    // nombre sin `--` → no es custom property → descartado.
    assert!(engine(r#"@property foo { syntax: "<length>"; inherits: false; initial-value: 0px; }"#)
        .registered_properties()
        .is_empty());
    // falta syntax → descartado.
    assert!(engine("@property --x { inherits: false; }")
        .registered_properties()
        .is_empty());
    // syntax tipado sin initial-value → descartado.
    assert!(engine(r#"@property --x { syntax: "<length>"; inherits: false; }"#)
        .registered_properties()
        .is_empty());
}

#[test]
fn at_property_y_font_face_coexisten() {
    let css = r#"
        @font-face { font-family: F; src: url(f.woff2); }
        @property --gap { syntax: "<length>"; inherits: false; initial-value: 8px; }
        @property --hue { syntax: "<angle>"; inherits: true; initial-value: 0deg; }
    "#;
    let eng = engine(css);
    assert_eq!(eng.font_faces().len(), 1);
    assert_eq!(eng.registered_properties().len(), 2);
    assert_eq!(eng.registered_properties()[0].name, "--gap");
    assert_eq!(eng.registered_properties()[1].initial_value.as_deref(), Some("0deg"));
}

// ── Fase 7.924 — @counter-style ────────────────────────────────────────────

#[test]
fn counter_style_cyclic() {
    let css = r#"
        @counter-style thumbs {
            system: cyclic;
            symbols: "\1F44D";
            suffix: " ";
        }
    "#;
    let c = engine(css).counter_styles().to_vec();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].name, "thumbs");
    assert_eq!(c[0].system.as_deref(), Some("cyclic"));
    assert_eq!(c[0].symbols.as_deref(), Some("\"\\1F44D\""));
    assert_eq!(c[0].suffix.as_deref(), Some("\" \""));
}

#[test]
fn counter_style_additive_y_range() {
    let css = r#"
        @counter-style roman {
            system: additive;
            additive-symbols: 10 X, 5 V, 1 I;
            range: 1 49;
            pad: 2 "0";
            negative: "-";
            fallback: decimal;
            speak-as: numbers;
        }
    "#;
    let c = engine(css).counter_styles().to_vec();
    let r = &c[0];
    assert_eq!(r.system.as_deref(), Some("additive"));
    assert_eq!(r.additive_symbols.as_deref(), Some("10 X, 5 V, 1 I"));
    assert_eq!(r.range.as_deref(), Some("1 49"));
    assert_eq!(r.pad.as_deref(), Some("2 \"0\""));
    assert_eq!(r.negative.as_deref(), Some("\"-\""));
    assert_eq!(r.fallback.as_deref(), Some("decimal"));
    assert_eq!(r.speak_as.as_deref(), Some("numbers"));
}

#[test]
fn counter_style_invalido_se_descarta() {
    // sin system/symbols/additive-symbols → no define nada → descartado.
    assert!(engine("@counter-style x { suffix: \".\"; }")
        .counter_styles()
        .is_empty());
    // nombre con `--` no es un counter-style válido (reservado a @property).
    assert!(engine("@counter-style --x { system: cyclic; symbols: \"a\"; }")
        .counter_styles()
        .is_empty());
}

// ── Fase 7.925 — @page (Paged Media) ───────────────────────────────────────

#[test]
fn page_sin_selector() {
    let css = r#"
        @page {
            size: A4 landscape;
            margin: 2cm;
            marks: crop cross;
            bleed: 6pt;
        }
    "#;
    let p = engine(css).page_rules().to_vec();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].selector, "");
    assert_eq!(p[0].size.as_deref(), Some("A4 landscape"));
    assert_eq!(p[0].marks.as_deref(), Some("crop cross"));
    assert_eq!(p[0].bleed.as_deref(), Some("6pt"));
    // margin queda en declarations crudas.
    assert!(p[0].declarations.iter().any(|(k, v)| k == "margin" && v == "2cm"));
}

#[test]
fn page_con_selector_y_orientacion() {
    let css = r#"
        @page :first { margin-top: 10cm; }
        @page chapter { size: letter; page-orientation: rotate-left; }
    "#;
    let p = engine(css).page_rules().to_vec();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].selector, ":first");
    assert!(p[0].declarations.iter().any(|(k, v)| k == "margin-top" && v == "10cm"));
    assert_eq!(p[1].selector, "chapter");
    assert_eq!(p[1].size.as_deref(), Some("letter"));
    assert_eq!(p[1].page_orientation.as_deref(), Some("rotate-left"));
}

#[test]
fn page_ignora_margin_at_rules_anidadas() {
    // Las margin-at-rules anidadas no se modelan pero NO deben ensuciar
    // declarations ni romper el parseo del resto del bloque.
    let css = r#"
        @page {
            size: A4;
            @top-center { content: "título"; }
            margin: 1cm;
        }
    "#;
    let p = engine(css).page_rules().to_vec();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].size.as_deref(), Some("A4"));
    // ningún par contiene basura de la at-rule anidada.
    assert!(p[0].declarations.iter().all(|(k, _)| !k.contains('@') && !k.contains('{')));
}

// ── Fase 7.926 — @layer (Cascade Layers): aplanado, ya no se dropea ─────────

#[test]
fn layer_bloque_aplica_reglas() {
    // Antes: las reglas dentro de @layer se dropeaban (p quedaba negro).
    // Ahora: se aplanan y aplican.
    assert_eq!(p_color("@layer base { p { color: rgb(10, 20, 30); } }"), (10, 20, 30));
}

#[test]
fn layer_anonimo_y_anidado() {
    // capa anónima
    assert_eq!(p_color("@layer { p { color: rgb(1, 2, 3); } }"), (1, 2, 3));
    // capas anidadas
    assert_eq!(
        p_color("@layer outer { @layer inner { p { color: rgb(4, 5, 6); } } }"),
        (4, 5, 6)
    );
}

#[test]
fn layer_statement_no_rompe() {
    // La forma statement `@layer a, b;` (sólo declara orden) no aporta reglas
    // pero no debe romper el parseo de las reglas que siguen.
    assert_eq!(
        p_color("@layer reset, base; @layer base { p { color: rgb(7, 8, 9); } }"),
        (7, 8, 9)
    );
}

#[test]
fn layer_convive_con_regla_normal() {
    // Una regla normal posterior pisa la de la capa (simplificación: orden de
    // fuente, no prioridad de capa — documentado).
    assert_eq!(
        p_color("@layer base { p { color: rgb(10, 10, 10); } } p { color: rgb(99, 99, 99); }"),
        (99, 99, 99)
    );
}

// ── Fase 7.933 — pseudo-clases inertes: la regla parsea, no se tira ─────────

#[test]
fn pseudo_clases_estandar_parsean() {
    // Antes: cualquier pseudo desconocida tiraba la regla entera.
    for sel in [
        "input:valid", "input:invalid", "input:placeholder-shown",
        "a:active", "a:visited", "div:target", "section:scope",
        "dialog:modal", "details:open", ":fullscreen", "video:playing",
        "input:user-invalid", "[popover]:popover-open", "input:default",
        "input:indeterminate", "input:autofill",
    ] {
        assert!(parse_selector(sel).is_some(), "debería parsear: {sel}");
    }
    // Funcionales reconocidas (inertes).
    for sel in ["div:dir(rtl)", "x-card:state(active)", ":host(.dark)", ":host-context(.rtl)"] {
        assert!(parse_selector(sel).is_some(), "debería parsear: {sel}");
    }
    // Pseudo desconocida de verdad SÍ invalida (comportamiento de browser).
    assert!(parse_selector(":totally-made-up").is_none());
    assert!(parse_selector(":foo-bar(baz)").is_none());
}

#[test]
fn nth_child_of_selector() {
    // `:nth-child(An+B of S)` parsea An+B Y la lista `S` (Fase 7.1211).
    let s = parse_selector(":nth-child(2 of .item)").expect("parsea");
    // el An+B se preserva (2 → a=0, b=2) y `of` queda con el selector parseado.
    assert!(s.compounds.iter().any(|c| c
        .pseudos
        .iter()
        .any(|p| matches!(p, Pseudo::NthChild { a: 0, b: 2, of: Some(_) }))));
    // sin `of`, queda None.
    let plain = parse_selector(":nth-child(2)").expect("parsea");
    assert!(plain.compounds.iter().any(|c| c
        .pseudos
        .iter()
        .any(|p| matches!(p, Pseudo::NthChild { a: 0, b: 2, of: None }))));
    assert!(parse_selector(":nth-last-child(odd of li)").is_some());
    // `S` inválido tira la regla (como cualquier selector inválido).
    assert!(parse_selector(":nth-child(1 of :totally-made-up)").is_none());
}

// ── Fase 7.934 — pseudo-elementos: parsean (no tiran la regla), inertes ─────

#[test]
fn pseudo_elementos_modernos_parsean() {
    use crate::style::PseudoElement;
    // ::before / ::after siguen generando su variante con box.
    assert_eq!(parse_selector("p::before").unwrap().pseudo_element, Some(PseudoElement::Before));
    assert_eq!(parse_selector("p:after").unwrap().pseudo_element, Some(PseudoElement::After));
    // Los modernos no-renderizados → Other (parsea, inerte).
    for sel in [
        "input::placeholder", "::selection", "li::marker", "p::first-line",
        "p::first-letter", "dialog::backdrop", "input::file-selector-button",
        "details::details-content", "::target-text", "::spelling-error",
        "::grammar-error", "::highlight(foo)", "x-el::part(label)",
        "::view-transition-group(root)", "::slotted(span)",
    ] {
        let s = parse_selector(sel).unwrap_or_else(|| panic!("debería parsear: {sel}"));
        assert_eq!(s.pseudo_element, Some(PseudoElement::Other), "{sel}");
    }
    // Legacy single-colon ::first-line/::first-letter.
    assert_eq!(parse_selector("p:first-line").unwrap().pseudo_element, Some(PseudoElement::Other));
}

#[test]
fn pseudo_element_no_renderizado_no_contamina_elemento() {
    // `::selection { color }` no debe pintar el color del <p> real.
    let css = "p::selection { color: rgb(1,2,3) } p { color: rgb(9,9,9) }";
    assert_eq!(p_color(css), (9, 9, 9));
}

// ── Fase 7.935 — selector de nesting `&` de nivel superior parsea ──────────

#[test]
fn nesting_amp_top_level() {
    // `&` solo (CSS Nesting top-level ≈ :scope) y combinaciones.
    assert!(parse_selector("&").is_some());
    assert!(parse_selector("&:hover").is_some());
    assert!(parse_selector("& > .child").is_some());
    assert!(parse_selector("&.active").is_some());
    // El nesting normal (con padre) sigue expandiéndose textualmente: el
    // `&:hover` no aplica al <p> en reposo, pero la decl del nivel padre sí.
    assert_eq!(p_color("p { &:hover { color: rgb(1,2,3) } color: rgb(7,8,9) }"), (7, 8, 9));
}

// ── Fase 7.936 — @container / @scope aplanan sus reglas (antes drop total) ──

#[test]
fn container_query_aplana_reglas() {
    // Antes: el @container entero se saltaba → el <p> quedaba sin estilo.
    assert_eq!(
        p_color("@container (min-width: 1px) { p { color: rgb(3,3,3) } }"),
        (3, 3, 3)
    );
    // Con nombre de contenedor.
    assert_eq!(
        p_color("@container card (min-width: 400px) { p { color: rgb(4,4,4) } }"),
        (4, 4, 4)
    );
}

#[test]
fn scope_aplana_reglas() {
    // @scope (root) { ... } aplana; `:scope` interno es inerte-true.
    assert_eq!(
        p_color("@scope (.card) to (.inner) { p { color: rgb(5,5,5) } }"),
        (5, 5, 5)
    );
    assert_eq!(
        p_color("@scope (body) { :scope p { color: rgb(6,6,6) } }"),
        (6, 6, 6)
    );
}

#[test]
fn starting_style_no_aplica_en_reposo() {
    // @starting-style NO debe aplanarse (sólo vale al aparecer): el <p> en
    // reposo conserva su color base, no el de @starting-style.
    assert_eq!(
        p_color("p { color: rgb(8,8,8) } @starting-style { p { color: rgb(1,1,1) } }"),
        (8, 8, 8)
    );
}

// ── Fase 7.937-7.938 — :is/:where/:not/:has con selectores COMPLEJOS ───────

// Computa el color del <p class="t"> dentro de la estructura dada.
fn p_color_in(html_body: &str, css: &str) -> (u8, u8, u8) {
    let html = format!("<html><head><style>{css}</style></head><body>{html_body}</body></html>");
    let dom = DomTree::parse(&html);
    let eng = StyleEngine::from_dom(&dom);
    let mut found = None;
    crate::dom::walk(&dom.document(), &mut |n| {
        if crate::dom::element_name(n).as_deref() == Some("p")
            && crate::dom::attr(n, "class").as_deref() == Some("t")
        {
            found = Some(n.clone());
        }
    });
    let cs = eng.compute(found.as_ref().expect("hay p.t"));
    (cs.color.r, cs.color.g, cs.color.b)
}

#[test]
fn is_where_not_complejos_parsean() {
    for sel in [
        ":is(.a .b)", ":is(a > b)", ":where(header .logo, nav a)",
        ":not(.a .b)", ":not(a > b, .c + .d)", "div:has(.a > .b)",
        "div:has(.a .b)", ":has(:not(.x))", "a:is(:hover, :focus)",
        "li:nth-child(even)", "input[type='text' i]",
    ] {
        assert!(parse_selector(sel).is_some(), "debería parsear: {sel}");
    }
}

#[test]
fn is_complejo_matchea() {
    // :is(.box .t) — el <p class="t"> es descendiente de .box → aplica.
    assert_eq!(
        p_color_in("<div class='box'><p class='t'></p></div>", ":is(.box .t) { color: rgb(3,3,3) }"),
        (3, 3, 3)
    );
    // sin el ancestro .box, no aplica.
    assert_eq!(
        p_color_in("<div><p class='t'></p></div>", ":is(.box .t) { color: rgb(3,3,3) } p { color: rgb(9,9,9) }"),
        (9, 9, 9)
    );
}

#[test]
fn not_complejo_matchea() {
    // p.t:not(.box .t) — el p.t NO está bajo .box → :not se cumple → aplica.
    assert_eq!(
        p_color_in("<div><p class='t'></p></div>", "p.t:not(.box p) { color: rgb(4,4,4) }"),
        (4, 4, 4)
    );
    // bajo .box → :not(.box p) falla → no aplica, queda el fallback.
    assert_eq!(
        p_color_in("<div class='box'><p class='t'></p></div>",
            "p.t:not(.box p) { color: rgb(4,4,4) } p { color: rgb(9,9,9) }"),
        (9, 9, 9)
    );
}

#[test]
fn has_complejo_matchea() {
    // div:has(.a > .b) aplica a un <p class="t"> que es hijo de ese div?
    // Estructura: div.wrap > p.t  y  div.wrap tiene .a > .b adentro.
    let body = "<div class='wrap'><p class='t'></p><div class='a'><span class='b'></span></div></div>";
    // .wrap:has(.a > .b) .t  → el p.t recibe color.
    assert_eq!(
        p_color_in(body, ".wrap:has(.a > .b) .t { color: rgb(5,5,5) }"),
        (5, 5, 5)
    );
    // si la relación es .a .b (descendiente) en vez de hijo directo, y no hay
    // hijo directo, :has(.a > .c) no matchea.
    assert_eq!(
        p_color_in(body, ".wrap:has(.a > .c) .t { color: rgb(5,5,5) } p { color: rgb(9,9,9) }"),
        (9, 9, 9)
    );
}

// ── Fase 7.939 — :is()/:where() son forgiving-selector-lists ───────────────

#[test]
fn is_where_forgiving() {
    // Parte inválida se descarta; la válida queda y el :is() NO se invalida.
    let s = parse_selector(":where(.a, !!!nope)").expect("forgiving: no invalida");
    let _ = s;
    // p.t:is(.t, $$bad) sigue aplicando vía la parte válida .t.
    assert_eq!(
        p_color_in("<p class='t'></p>", "p:is(.t, $$bad) { color: rgb(2,2,2) }"),
        (2, 2, 2)
    );
    // :is() con TODO inválido → lista vacía → matchea nada (no rompe el parse).
    assert!(parse_selector("p:is(@@@, ###)").is_some());
    assert_eq!(
        p_color_in("<p class='t'></p>", "p:is(@@@) { color: rgb(2,2,2) } p { color: rgb(9,9,9) }"),
        (9, 9, 9)
    );
}

// ── Fase 7.940 — :dir(rtl|ltr) evaluado de verdad (atributo dir heredado) ──

#[test]
fn dir_pseudo_real() {
    // <p class="t" dir="rtl"> → :dir(rtl) aplica.
    assert_eq!(
        p_color_in("<p class='t' dir='rtl'></p>", "p:dir(rtl) { color: rgb(1,2,3) } p { color: rgb(9,9,9) }"),
        (1, 2, 3)
    );
    // dir heredado del ancestro.
    assert_eq!(
        p_color_in("<div dir='rtl'><p class='t'></p></div>", "p:dir(rtl) { color: rgb(1,2,3) } p { color: rgb(9,9,9) }"),
        (1, 2, 3)
    );
    // ltr por default (sin dir) → :dir(rtl) NO aplica, :dir(ltr) sí.
    assert_eq!(
        p_color_in("<p class='t'></p>", "p:dir(rtl) { color: rgb(1,2,3) } p { color: rgb(9,9,9) }"),
        (9, 9, 9)
    );
    assert_eq!(
        p_color_in("<p class='t'></p>", "p:dir(ltr) { color: rgb(4,5,6) }"),
        (4, 5, 6)
    );
}
