//! Tests de los longhands/shorthands cableados en Fase 7.816-7.825:
//! longhands `animation-*` (direction/play-state/delay), shorthands de
//! grid placement (`grid-row`/`grid-column`/`grid-area`) y longhands
//! `transition-*` (property/duration/timing/delay). Cubren tanto el parse
//! (DeclKind emitido) como la composición sobre `ComputedStyle` (el merge
//! sobre un único binding, que es la parte sutil).
use super::super::*;

fn decls(s: &str) -> Vec<Decl> {
    parse_declarations(s, &HashMap::new())
}

// ── Fase 7.816-7.818 — longhands animation ────────────────────────────────

#[test]
fn animation_direction_longhand() {
    let d = decls("animation-direction: alternate");
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AnimationDirection(AnimationDirection::Alternate))));
    assert!(decls("animation-direction: alternate-reverse").iter().any(|d| matches!(
        d.kind,
        DeclKind::AnimationDirection(AnimationDirection::AlternateReverse)
    )));
    assert!(decls("animation-direction: garbage").is_empty());
}

#[test]
fn animation_play_state_y_delay_longhand() {
    assert!(decls("animation-play-state: paused")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AnimationPlayState(AnimationPlayState::Paused))));
    assert!(decls("animation-delay: 0.5s")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AnimationDelay(v) if (v - 0.5).abs() < 1e-6)));
}

#[test]
fn animation_longhands_componen_un_binding() {
    let html = r#"<div style="animation-name: spin; animation-duration: 2s; animation-direction: alternate; animation-play-state: paused; animation-delay: 0.5s"></div>"#;
    let dom = DomTree::parse(html);
    let eng = StyleEngine::from_dom(&dom);
    let div = dom.find("div").unwrap();
    let a = eng.compute(&div).animation.expect("binding presente");
    assert_eq!(a.name, "spin");
    assert_eq!(a.duration_s, 2.0);
    assert_eq!(a.direction, AnimationDirection::Alternate);
    assert_eq!(a.play_state, AnimationPlayState::Paused);
    assert_eq!(a.delay_s, 0.5);
}

// ── Fase 7.819-7.821 — shorthands grid placement ──────────────────────────

#[test]
fn grid_column_y_row_shorthand() {
    let c = decls("grid-column: 1 / 3");
    assert!(c.iter().any(|d| matches!(&d.kind, DeclKind::GridColumnStart(Some(s)) if s == "1")));
    assert!(c.iter().any(|d| matches!(&d.kind, DeclKind::GridColumnEnd(Some(s)) if s == "3")));
    // `span N` en start deja el end en auto (None).
    let r = decls("grid-row: span 2");
    assert!(r.iter().any(|d| matches!(&d.kind, DeclKind::GridRowStart(Some(s)) if s == "span 2")));
    assert!(r.iter().any(|d| matches!(d.kind, DeclKind::GridRowEnd(None))));
    // Un custom-ident solo se replica al end.
    let n = decls("grid-column: header");
    assert!(n.iter().any(|d| matches!(&d.kind, DeclKind::GridColumnStart(Some(s)) if s == "header")));
    assert!(n.iter().any(|d| matches!(&d.kind, DeclKind::GridColumnEnd(Some(s)) if s == "header")));
}

#[test]
fn grid_area_shorthand_4_y_omision() {
    let full = decls("grid-area: 1 / 2 / 3 / 4");
    assert!(full.iter().any(|d| matches!(&d.kind, DeclKind::GridRowStart(Some(s)) if s == "1")));
    assert!(full.iter().any(|d| matches!(&d.kind, DeclKind::GridColumnStart(Some(s)) if s == "2")));
    assert!(full.iter().any(|d| matches!(&d.kind, DeclKind::GridRowEnd(Some(s)) if s == "3")));
    assert!(full.iter().any(|d| matches!(&d.kind, DeclKind::GridColumnEnd(Some(s)) if s == "4")));
    // `grid-area: a` (custom-ident) → los cuatro toman `a`.
    let one = decls("grid-area: a");
    let count_a = one
        .iter()
        .filter(|d| {
            matches!(&d.kind,
                DeclKind::GridRowStart(Some(s))
                | DeclKind::GridColumnStart(Some(s))
                | DeclKind::GridRowEnd(Some(s))
                | DeclKind::GridColumnEnd(Some(s)) if s == "a")
        })
        .count();
    assert_eq!(count_a, 4);
    // `grid-area: 1 / 2` → re=auto (1 no es ident), ce=auto (2 no es ident).
    let two = decls("grid-area: 1 / 2");
    assert!(two.iter().any(|d| matches!(d.kind, DeclKind::GridRowEnd(None))));
    assert!(two.iter().any(|d| matches!(d.kind, DeclKind::GridColumnEnd(None))));
}

// ── Fase 7.822-7.825 — longhands transition ───────────────────────────────

#[test]
fn transition_longhands_parse() {
    assert!(decls("transition-property: opacity")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::TransitionPropertyFirst(Some(p)) if p == "opacity")));
    assert!(decls("transition-property: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TransitionPropertyFirst(None))));
    assert!(decls("transition-duration: 0.3s")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TransitionDurationFirst(v) if (v - 0.3).abs() < 1e-6)));
    assert!(decls("transition-delay: 1s")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TransitionDelayFirst(v) if (v - 1.0).abs() < 1e-6)));
    assert!(decls("transition-timing-function: ease-in")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TransitionTimingFirst(_))));
}

#[test]
fn transition_longhands_componen_un_binding() {
    let html = r#"<div style="transition-property: opacity; transition-duration: 0.3s; transition-delay: 0.1s"></div>"#;
    let dom = DomTree::parse(html);
    let eng = StyleEngine::from_dom(&dom);
    let div = dom.find("div").unwrap();
    let c = eng.compute(&div);
    assert_eq!(c.transitions.len(), 1);
    assert_eq!(c.transitions[0].property, "opacity");
    assert_eq!(c.transitions[0].duration_s, 0.3);
    assert_eq!(c.transitions[0].delay_s, 0.1);
    // `transition-property: none` limpia la lista.
    let html2 = r#"<div id="x" style="transition-duration: 1s; transition-property: none"></div>"#;
    let dom2 = DomTree::parse(html2);
    let eng2 = StyleEngine::from_dom(&dom2);
    let d2 = dom2.find("div").unwrap();
    assert!(eng2.compute(&d2).transitions.is_empty());
}

// ── Fase 7.826-7.828 — props individuales translate/rotate/scale ──────────

#[test]
fn props_individuales_transform_parse() {
    assert!(decls("translate: 5px")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::Translate(Some(Transform::Translate(x, _))) if *x == 5.0)));
    assert!(decls("translate: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Translate(None))));
    // `50%` en scale = factor 0.5.
    assert!(decls("scale: 50%")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::Scale(Some(Transform::Scale(sx, _))) if (*sx - 0.5).abs() < 1e-6)));
    // `0.5turn` = 180deg.
    assert!(decls("rotate: 0.5turn")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::Rotate(Some(Transform::Rotate(deg))) if (*deg - 180.0).abs() < 1e-3)));
    // Eje no-Z explícito → sin rotación en el plano 2D.
    assert!(decls("rotate: y 45deg")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Rotate(Some(Transform::Rotate(deg))) if deg == 0.0)));
    assert!(decls("scale: garbage").is_empty());
}

#[test]
fn props_individuales_transform_componen_en_orden() {
    let html = r#"<div style="translate: 10px 20px; rotate: 45deg; scale: 2; transform: skewX(10deg)"></div>"#;
    let dom = DomTree::parse(html);
    let eng = StyleEngine::from_dom(&dom);
    let div = dom.find("div").unwrap();
    let t = eng.compute(&div).transforms;
    // Orden CSS Transforms 2: translate → rotate → scale → transform-list.
    assert_eq!(t.len(), 4);
    assert!(matches!(t[0], Transform::Translate(x, y) if x == 10.0 && y == 20.0));
    assert!(matches!(t[1], Transform::Rotate(d) if d == 45.0));
    assert!(matches!(t[2], Transform::Scale(sx, sy) if sx == 2.0 && sy == 2.0));
    assert!(matches!(t[3], Transform::Skew(_, _)));
}

// ── Fase 7.829 — shorthand font-variant ───────────────────────────────────

#[test]
fn font_variant_shorthand() {
    // Caso dominante: caps.
    assert!(decls("font-variant: small-caps")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontVariantCaps(FontVariantCaps::SmallCaps))));
    // Reparto multi-grupo: caps + numeric (dos tokens) + position.
    let d = decls("font-variant: small-caps tabular-nums oldstyle-nums super");
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontVariantCaps(FontVariantCaps::SmallCaps))));
    assert!(d
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::FontVariantNumeric(n) if n.tabular_nums && n.oldstyle_nums)));
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontVariantPosition(FontVariantPosition::Super))));
    // `normal` resetea (emite varios longhands a default).
    assert!(decls("font-variant: normal")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontVariantCaps(FontVariantCaps::Normal))));
    // Un token desconocido descarta el shorthand entero.
    assert!(decls("font-variant: small-caps bogus-token").is_empty());
}

// ── Fase 7.830 — `none` en max-width/height (y lógicos) → Auto ─────────────

#[test]
fn max_size_none_resetea_a_auto() {
    assert!(decls("max-width: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MaxWidth(LengthVal::Auto))));
    assert!(decls("max-height: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MaxHeight(LengthVal::Auto))));
    assert!(decls("max-inline-size: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MaxWidth(LengthVal::Auto))));
    assert!(decls("max-block-size: none")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MaxHeight(LengthVal::Auto))));
    // Y los valores normales siguen funcionando.
    assert!(decls("max-width: 300px")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MaxWidth(LengthVal::Px(p)) if p == 300.0)));
}

// ── Fase 7.831-7.836 — value-keywords en props existentes ─────────────────

#[test]
fn line_height_normal_resetea_a_none() {
    assert!(decls("line-height: normal")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::LineHeightNormal)));
    // Un número sigue produciendo LineHeight(f32).
    assert!(decls("line-height: 1.5")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6)));
    // Computa a None (normal) end-to-end.
    let html = r#"<p style="line-height: normal">x</p>"#;
    let dom = DomTree::parse(html);
    let eng = StyleEngine::from_dom(&dom);
    let p = dom.find("p").unwrap();
    assert_eq!(eng.compute(&p).line_height, None);
}

#[test]
fn overflow_overlay_y_dos_valores() {
    // overlay = alias legacy de auto → Hidden.
    assert!(decls("overflow: overlay")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Hidden))));
    // Dos valores: tomamos el 1er token (eje x) en el modelo de campo único.
    assert!(decls("overflow: hidden visible")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Hidden))));
    assert!(decls("overflow: visible hidden")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Visible))));
}

#[test]
fn keyword_fixes_varios() {
    // position: -webkit-sticky → sticky.
    assert!(decls("position: -webkit-sticky")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Position(Position::Sticky))));
    // gap: normal → 0.
    assert!(decls("gap: normal")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Gap { row, column } if row == 0.0 && column == 0.0)));
    // outline-style: auto → visible (OutlineStyle(true)).
    assert!(decls("outline-style: auto")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::OutlineStyle(true))));
}

// ── Fase 7.837-7.838 — border-width/-color multi-valor (TRBL) ─────────────

#[test]
fn border_width_multivalor_y_keywords() {
    // 1 token keyword (antes se descartaba) → global.
    assert!(decls("border-width: thick")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::BorderWidth(w) if w == 5.0)));
    // 3 valores con keywords → top/horiz/bottom per-side.
    let d = decls("border-width: thin medium thick");
    let w = |e: BorderEdge| {
        d.iter().find_map(|d| match d.kind {
            DeclKind::BorderSideWidth(side, v) if side == e => Some(v),
            _ => None,
        })
    };
    assert_eq!(w(BorderEdge::Top), Some(1.0));
    assert_eq!(w(BorderEdge::Right), Some(3.0));
    assert_eq!(w(BorderEdge::Bottom), Some(5.0));
    assert_eq!(w(BorderEdge::Left), Some(3.0)); // = right (horiz)
}

#[test]
fn border_color_multivalor() {
    // 2 valores → vert/horiz.
    let d = decls("border-color: red blue");
    let n_sides = d
        .iter()
        .filter(|d| matches!(d.kind, DeclKind::BorderSideColor(_, _)))
        .count();
    assert_eq!(n_sides, 4);
    // currentColor por lado se respeta.
    assert!(decls("border-color: red currentColor")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::CurrentColor(ColorTarget::BorderSide(_)))));
    // Token inválido descarta el shorthand entero.
    assert!(decls("border-color: red notacolor").is_empty());
}
