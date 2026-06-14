//! Tests del lote data-driven Fase 7.917+ (sondeo de cobertura CSS, 2ª tanda):
//! gaps de VALOR en props ya reconocidas — keywords nuevos de specs recientes
//! que el parser dropeaba. Detectados con el sondeo `probe::sondeo_drops`.
use super::super::*;

fn decls(s: &str) -> Vec<Decl> {
    parse_declarations(s, &std::collections::HashMap::new())
}

// ── Fase 7.917 — gaps de valor (Ola A) ─────────────────────────────────────

#[test]
fn word_break_auto_phrase() {
    // CSS Text 4: `auto-phrase` (japonés). Antes se dropeaba.
    assert!(decls("word-break: auto-phrase")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WordBreak(WordBreak::AutoPhrase))));
    // los valores previos siguen funcionando
    assert!(decls("word-break: break-all")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WordBreak(WordBreak::BreakAll))));
    assert!(decls("word-break: keep-all")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WordBreak(WordBreak::KeepAll))));
    // basura sigue dropeando
    assert!(decls("word-break: foobar").is_empty());
}

#[test]
fn alignment_baseline_edge_keywords() {
    // SVG 1.1 edge keywords mapean a los equivalentes de borde.
    let cases = [
        ("text-before-edge", AlignmentBaseline::TextTop),
        ("text-after-edge", AlignmentBaseline::TextBottom),
        ("before-edge", AlignmentBaseline::Top),
        ("after-edge", AlignmentBaseline::Bottom),
    ];
    for (kw, expected) in cases {
        let css = format!("alignment-baseline: {kw}");
        assert!(
            decls(&css)
                .iter()
                .any(|d| matches!(&d.kind, DeclKind::AlignmentBaseline(a) if *a == expected)),
            "{kw} debería mapear a {expected:?}"
        );
    }
    // los valores SVG 2 previos intactos
    assert!(decls("alignment-baseline: central")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AlignmentBaseline(AlignmentBaseline::Central))));
}

#[test]
fn text_decoration_line_correccion() {
    // CSS Text Decoration 4: spelling/grammar-error → subrayado (degradado).
    for kw in ["spelling-error", "grammar-error"] {
        let css = format!("text-decoration-line: {kw}");
        assert!(
            decls(&css)
                .iter()
                .any(|d| matches!(d.kind, DeclKind::TextDecoration(TextDecorationLine::Underline))),
            "{kw} debería aceptarse como Underline"
        );
    }
}

#[test]
fn background_clip_border_area() {
    // CSS Backgrounds 4: `border-area` aceptado (tratado como border-box).
    assert!(decls("background-clip: border-area")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::BackgroundClip(BackgroundClip::BorderBox))));
    // `text` (Fase 7.208) sigue distinto
    assert!(decls("background-clip: text")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::BackgroundClip(BackgroundClip::Text))));
}
