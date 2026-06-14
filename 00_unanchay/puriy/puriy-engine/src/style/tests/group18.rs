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

// ── Fase 7.918 — alignment-baseline hanging + corner-shape longhands (Ola B) ─

#[test]
fn alignment_baseline_hanging() {
    // SVG 1.1: `hanging` aceptado (aprox. a text-top).
    assert!(decls("alignment-baseline: hanging")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AlignmentBaseline(AlignmentBaseline::TextTop))));
}

#[test]
fn corner_shape_longhands() {
    // Los longhands por esquina (físicos, por-lado, lógicos) se aceptan y
    // colapsan al campo opaco corner_shape. Antes dropeaban.
    let longhands = [
        "corner-top-left-shape",
        "corner-bottom-right-shape",
        "corner-top-shape",
        "corner-left-shape",
        "corner-block-start-shape",
        "corner-inline-end-shape",
        "corner-start-start-shape",
        "corner-end-end-shape",
    ];
    for lh in longhands {
        let css = format!("{lh}: bevel");
        assert!(
            decls(&css).iter().any(|d| matches!(
                &d.kind,
                DeclKind::CornerShape(Some(s)) if s == "bevel"
            )),
            "{lh} debería aceptarse como CornerShape opaco"
        );
    }
    // `round` sigue colapsando a None en un longhand
    assert!(decls("corner-top-left-shape: round")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::CornerShape(None))));
}

// ── Fase 7.919 — CSS Speech (Ola C) ────────────────────────────────────────

#[test]
fn voice_family_stress_duration() {
    assert!(decls("voice-family: female")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::VoiceFamily(Some(s)) if s == "female")));
    assert!(decls("voice-family: preserve")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::VoiceFamily(None))));
    assert!(decls("voice-stress: strong")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::VoiceStress(Some(s)) if s == "strong")));
    assert!(decls("voice-stress: normal")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::VoiceStress(None))));
    assert!(decls("voice-duration: 2s")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::VoiceDuration(Some(s)) if s == "2s")));
    assert!(decls("voice-duration: auto")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::VoiceDuration(None))));
}

#[test]
fn speak_as_combinacion() {
    // combo `spell-out digits` válido → primer keyword no-normal (degradado).
    assert!(decls("speak-as: spell-out digits")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::SpeakAs(SpeakAs::SpellOut))));
    assert!(decls("speak-as: digits literal-punctuation")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::SpeakAs(SpeakAs::Digits))));
    // un solo keyword sigue funcionando
    assert!(decls("speak-as: no-punctuation")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::SpeakAs(SpeakAs::NoPunctuation))));
    // token inválido rechaza todo
    assert!(decls("speak-as: spell-out foobar").is_empty());
}

#[test]
fn pause_rest_shorthands() {
    // `pause: 20ms 40ms` → before=20ms, after=40ms.
    let p = decls("pause: 20ms 40ms");
    assert!(p.iter().any(|d| matches!(&d.kind, DeclKind::PauseBefore(Some(s)) if s == "20ms")));
    assert!(p.iter().any(|d| matches!(&d.kind, DeclKind::PauseAfter(Some(s)) if s == "40ms")));
    // 1 valor → ambos lados iguales.
    let p1 = decls("pause: weak");
    assert!(p1.iter().any(|d| matches!(&d.kind, DeclKind::PauseBefore(Some(s)) if s == "weak")));
    assert!(p1.iter().any(|d| matches!(&d.kind, DeclKind::PauseAfter(Some(s)) if s == "weak")));
    // `rest` análogo, con `none` → None.
    let r = decls("rest: none 1s");
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RestBefore(None))));
    assert!(r.iter().any(|d| matches!(&d.kind, DeclKind::RestAfter(Some(s)) if s == "1s")));
}

// ── Fase 7.920 — CSS Gap Decorations: row-rule + rule shorthands (Ola D) ────

#[test]
fn row_rule_longhands() {
    assert!(decls("row-rule-width: 2px")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::RowRuleWidth(w) if (w - 2.0).abs() < 0.01)));
    assert!(decls("row-rule-color: red")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::RowRuleColor(Some(_)))));
    assert!(decls("row-rule-color: currentColor")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::RowRuleColor(None))));
    // row-rule-style activa + patrón
    let s = decls("row-rule-style: dashed");
    assert!(s.iter().any(|d| matches!(d.kind, DeclKind::RowRuleStyleActive(true))));
    assert!(s.iter().any(|d| matches!(d.kind, DeclKind::RowRuleStylePattern(BorderLineStyle::Dashed))));
}

#[test]
fn row_rule_shorthand() {
    let r = decls("row-rule: 2px solid red");
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RowRuleWidth(w) if (w - 2.0).abs() < 0.01)));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RowRuleColor(Some(_)))));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RowRuleStyleActive(true))));
    // no toca el eje de columnas
    assert!(!r.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleWidth(_))));
}

#[test]
fn rule_shorthand_ambos_ejes() {
    // `rule` fija filas Y columnas.
    let r = decls("rule: 3px dotted blue");
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleWidth(w) if (w - 3.0).abs() < 0.01)));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RowRuleWidth(w) if (w - 3.0).abs() < 0.01)));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleStylePattern(BorderLineStyle::Dotted))));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RowRuleStylePattern(BorderLineStyle::Dotted))));
    // sub-shorthands `rule-width` / `rule-color`
    let w = decls("rule-width: 4px");
    assert!(w.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleWidth(v) if (v - 4.0).abs() < 0.01)));
    assert!(w.iter().any(|d| matches!(d.kind, DeclKind::RowRuleWidth(v) if (v - 4.0).abs() < 0.01)));
    let c = decls("rule-color: red");
    assert!(c.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleColor(Some(_)))));
    assert!(c.iter().any(|d| matches!(d.kind, DeclKind::RowRuleColor(Some(_)))));
}

// ── Fase 7.921 — justify-items legacy (Ola E) ──────────────────────────────

#[test]
fn justify_items_legacy() {
    assert!(decls("justify-items: legacy center")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::JustifyItems(AlignItems::Center))));
    assert!(decls("justify-items: legacy left")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::JustifyItems(AlignItems::Start))));
    // orden inverso
    assert!(decls("justify-items: right legacy")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::JustifyItems(AlignItems::End))));
    // valores normales intactos
    assert!(decls("justify-items: center")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::JustifyItems(AlignItems::Center))));
}

// ── Fase 7.927 — CSS Text 4 wrap-* + Speech speak moderno + list-style ' ────

#[test]
fn wrap_between_y_inside() {
    assert!(decls("wrap-before: avoid")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WrapBefore(WrapBetween::Avoid))));
    assert!(decls("wrap-after: avoid-flex")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WrapAfter(WrapBetween::AvoidFlex))));
    assert!(decls("wrap-before: line")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WrapBefore(WrapBetween::Line))));
    assert!(decls("wrap-inside: avoid")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WrapInside(WrapInside::Avoid))));
    // valor inválido se dropea
    assert!(decls("wrap-inside: line").is_empty());
}

#[test]
fn speak_moderno_css_speech() {
    assert!(decls("speak: never")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Speak(Speak::Never))));
    assert!(decls("speak: always")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Speak(Speak::Always))));
    assert!(decls("speak: auto")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Speak(Speak::Auto))));
    // CSS 2.1 sigue válido
    assert!(decls("speak: spell-out")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Speak(Speak::SpellOut))));
}

#[test]
fn list_style_type_comilla_simple() {
    // marcador string con comilla simple (antes sólo doble).
    assert!(decls("list-style-type: '→'")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::ListStyleType(ListStyleType::Disc))));
}

// ── Fase 7.928 — CSS Scroll Snap 2: scroll-start + scroll-start-target ──────

#[test]
fn scroll_start_family() {
    assert!(decls("scroll-start: 100px")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::ScrollStart(Some(s)) if s == "100px")));
    assert!(decls("scroll-start-block: center")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::ScrollStartBlock(Some(s)) if s == "center")));
    assert!(decls("scroll-start-inline: start")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::ScrollStartInline(Some(s)) if s == "start")));
    // sentinel inicial → None
    assert!(decls("scroll-start: auto")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::ScrollStart(None))));
    // scroll-start-target: sentinel `none`
    assert!(decls("scroll-start-target: auto")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::ScrollStartTarget(Some(s)) if s == "auto")));
    assert!(decls("scroll-start-target-block: auto")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::ScrollStartTargetBlock(Some(s)) if s == "auto")));
    assert!(decls("scroll-start-target: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::ScrollStartTarget(None))));
}

// ── Fase 7.929 — CSS Gap Decorations: gap-rule* aliases de rule* ───────────

#[test]
fn gap_rule_aliases() {
    // `gap-rule` fija ambos ejes (como `rule`).
    let r = decls("gap-rule: 2px solid red");
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleWidth(w) if (w - 2.0).abs() < 0.01)));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::RowRuleWidth(w) if (w - 2.0).abs() < 0.01)));
    // sub-shorthands
    assert!(decls("gap-rule-color: red")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::RowRuleColor(Some(_)))));
    assert!(decls("gap-rule-width: 3px")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::ColumnRuleWidth(w) if (w - 3.0).abs() < 0.01)));
    let s = decls("gap-rule-style: solid");
    assert!(s.iter().any(|d| matches!(d.kind, DeclKind::ColumnRuleStyleActive(true))));
    assert!(s.iter().any(|d| matches!(d.kind, DeclKind::RowRuleStyleActive(true))));
}
