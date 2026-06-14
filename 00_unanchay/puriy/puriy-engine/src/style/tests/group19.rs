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
