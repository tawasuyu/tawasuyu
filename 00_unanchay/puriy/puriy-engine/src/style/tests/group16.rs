//! Tests de los gaps cerrados en Fase 7.846-7.848 (lote data-driven sobre el
//! sondeo de cobertura CSS): overflow lógico (`overflow-inline`/`-block`),
//! `calc()` en los shorthands de sides (`margin`/`padding`), y los shorthands
//! `grid`/`grid-template` (`none` + forma `<rows> / <columns>`).
use super::super::*;

fn decls(s: &str) -> Vec<Decl> {
    parse_declarations(s, &HashMap::new())
}

// ── Fase 7.846 — overflow lógico → físico (modelo de campo único) ──────────

#[test]
fn overflow_inline_block_logicos() {
    // `overflow-inline`/`-block` caen al mismo `Overflow` que x/y.
    assert!(decls("overflow-inline: scroll")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Hidden))));
    assert!(decls("overflow-block: hidden")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Hidden))));
    assert!(decls("overflow-inline: visible")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Visible))));
    // Dos valores: toma el 1er token (eje inline).
    assert!(decls("overflow-block: visible hidden")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Overflow(Overflow::Visible))));
    assert!(decls("overflow-inline: garbage").is_empty());
}

// ── Fase 7.847 — calc() en shorthands de sides ─────────────────────────────

#[test]
fn margin_calc_un_valor() {
    // `calc(1em + 2px)` = 16 + 2 = 18px (em = 16px). No debe partirse por los
    // espacios internos del calc.
    let d = decls("margin: calc(1em + 2px)");
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MarginTop(px) if (px - 18.0).abs() < 0.01)));
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MarginLeft(px) if (px - 18.0).abs() < 0.01)));
}

#[test]
fn margin_calc_mezcla_con_auto() {
    // `calc(10px + 5px) auto` → top/bottom = 15px, left/right = auto.
    let d = decls("margin: calc(10px + 5px) auto");
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MarginTop(px) if (px - 15.0).abs() < 0.01)));
    assert!(d
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MarginLeftAuto(true))));
}

#[test]
fn padding_calc() {
    // `padding` usa `parse_sides`; `calc(20px / 2)` = 10px en los 4 lados.
    let d = decls("padding: calc(20px / 2)");
    assert!(d.iter().any(|d| matches!(
        d.kind,
        DeclKind::Padding(s) if (s.top - 10.0).abs() < 0.01 && (s.left - 10.0).abs() < 0.01
    )));
}

#[test]
fn padding_calc_con_pct_se_descarta() {
    // calc con componente `%` no es representable en los f32 de sides → drop.
    assert!(decls("padding: calc(50% + 10px)").is_empty());
}

// ── Fase 7.848 — shorthands grid / grid-template ───────────────────────────

#[test]
fn grid_template_none() {
    for prop in ["grid", "grid-template"] {
        let d = decls(&format!("{prop}: none"));
        assert!(
            d.iter()
                .any(|d| matches!(&d.kind, DeclKind::GridTemplateRows(t) if t.is_empty())),
            "{prop}: none debe vaciar rows"
        );
        assert!(
            d.iter()
                .any(|d| matches!(&d.kind, DeclKind::GridTemplateColumns(t) if t.is_empty())),
            "{prop}: none debe vaciar columns"
        );
        assert!(
            d.iter().any(|d| matches!(d.kind, DeclKind::GridTemplateAreas(None))),
            "{prop}: none debe limpiar areas"
        );
    }
}

#[test]
fn grid_template_rows_slash_columns() {
    // `grid-template: auto / 1fr 1fr` → rows=[auto], columns=[1fr,1fr].
    let d = decls("grid-template: auto / 1fr 1fr");
    let rows = d.iter().find_map(|d| match &d.kind {
        DeclKind::GridTemplateRows(t) => Some(t.clone()),
        _ => None,
    });
    let cols = d.iter().find_map(|d| match &d.kind {
        DeclKind::GridTemplateColumns(t) => Some(t.clone()),
        _ => None,
    });
    assert_eq!(rows.map(|t| t.len()), Some(1));
    assert_eq!(cols.map(|t| t.len()), Some(2));
}

#[test]
fn grid_con_auto_flow_no_expande() {
    // La forma con `auto-flow` (sólo `grid`) está fuera de alcance: no debe
    // emitir templates espurios.
    assert!(decls("grid: auto-flow / 1fr").is_empty());
}

// ── Fase 7.849 — keywords de tamaño intrínseco ─────────────────────────────

#[test]
fn intrinsic_size_keywords_parsean() {
    for (val, expected) in [
        ("min-content", LengthVal::MinContent),
        ("max-content", LengthVal::MaxContent),
        ("fit-content", LengthVal::FitContent),
        ("fit-content(200px)", LengthVal::FitContent),
    ] {
        let d = decls(&format!("width: {val}"));
        assert!(
            d.iter().any(|d| matches!(d.kind, DeclKind::Width(lv) if lv == expected)),
            "width: {val} → {expected:?}"
        );
    }
    // También en height, min-* y max-* (este último vía parse_max_size).
    assert!(decls("height: min-content")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Height(LengthVal::MinContent))));
    assert!(decls("min-width: max-content")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MinWidth(LengthVal::MaxContent))));
    assert!(decls("max-width: fit-content")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MaxWidth(LengthVal::FitContent))));
    // Y por el alias lógico inline-size (despachado a Width).
    assert!(decls("inline-size: max-content")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Width(LengthVal::MaxContent))));
}
