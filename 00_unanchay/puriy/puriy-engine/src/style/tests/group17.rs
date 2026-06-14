//! Tests del lote data-driven Fase 7.852-7.860 (sondeo de cobertura CSS):
//! unidades de longitud completas (absolutas/viewport-dinámicas/container),
//! funciones matemáticas extra (`round`/`mod`/`rem`) y en más props, `minmax`
//! en grid, `border-radius` multivalor, `background-position` por borde, y
//! varios keywords/funciones que faltaban.
use super::super::*;

fn decls(s: &str) -> Vec<Decl> {
    parse_declarations(s, &HashMap::new())
}

/// Helper: primer `Width` como px (resuelto contra DEFAULT_VIEWPORT 1280×800).
fn width_px(s: &str) -> Option<f32> {
    decls(s).iter().find_map(|d| match d.kind {
        DeclKind::Width(LengthVal::Px(v)) => Some(v),
        _ => None,
    })
}

// ── Fase 7.852 — unidades de longitud ──────────────────────────────────────

#[test]
fn unidades_absolutas() {
    // 1in = 96px; 1cm = 96/2.54; 1pt = 96/72; 1pc = 16px.
    assert_eq!(width_px("width: 1in"), Some(96.0));
    assert!((width_px("width: 1cm").unwrap() - 96.0 / 2.54).abs() < 0.01);
    assert!((width_px("width: 10mm").unwrap() - 96.0 / 2.54).abs() < 0.01); // 10mm = 1cm
    assert_eq!(width_px("width: 1pc"), Some(16.0));
    assert!((width_px("width: 72pt").unwrap() - 96.0).abs() < 0.01); // 72pt = 1in
    assert!((width_px("width: 40q").unwrap() - 96.0 / 2.54).abs() < 0.01); // 40q = 1cm
}

#[test]
fn unidades_font_relativas_ch_ex() {
    // ch/ex ≈ 0.5em = 8px (sin métricas reales de fuente).
    assert_eq!(width_px("width: 1ch"), Some(8.0));
    assert_eq!(width_px("width: 2ex"), Some(16.0));
}

#[test]
fn unidades_viewport_dinamicas() {
    // svh/lvh/dvh colapsan a vh (sin UI dinámica). 50dvh de 800 = 400.
    assert_eq!(width_px("width: 50dvh"), Some(400.0));
    assert_eq!(width_px("width: 50svh"), Some(400.0));
    assert_eq!(width_px("width: 50lvh"), Some(400.0));
    // 50dvw de 1280 = 640.
    assert_eq!(width_px("width: 50dvw"), Some(640.0));
}

#[test]
fn unidades_container_query() {
    // Sin container real → viewport. cqw=ancho, cqh=alto, cqi=inline(ancho),
    // cqb=block(alto), cqmin/cqmax.
    assert_eq!(width_px("width: 10cqw"), Some(128.0)); // 10% de 1280
    assert_eq!(width_px("width: 10cqi"), Some(128.0));
    assert_eq!(width_px("width: 10cqh"), Some(80.0)); // 10% de 800
    assert_eq!(width_px("width: 10cqb"), Some(80.0));
    assert_eq!(width_px("width: 10cqmin"), Some(80.0)); // min(1280,800)=800
    assert_eq!(width_px("width: 10cqmax"), Some(128.0)); // max=1280
}

// ── Fase 7.853 — math fns en margin/padding/gap ────────────────────────────

#[test]
fn margin_top_acepta_max() {
    // max(0px, 1rem) = max(0, 16) = 16.
    assert!(decls("margin-top: max(0px, 1rem)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::MarginTop(v) if (v - 16.0).abs() < 0.01)));
}

#[test]
fn gap_acepta_min_y_normal() {
    // min(10px, 20px) = 10 en ambos ejes.
    let g = decls("gap: min(10px, 20px)");
    assert!(g
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Gap { row, column } if row == 10.0 && column == 10.0)));
    // `normal` → 0.
    assert!(decls("row-gap: normal")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::RowGap(v) if v == 0.0)));
}

// ── Fase 7.854 — round / mod / rem ─────────────────────────────────────────

#[test]
fn round_nearest() {
    // round(98px, 8px): múltiplos 96 y 104; 98 está más cerca de 96.
    assert_eq!(width_px("width: round(98px, 8px)"), Some(96.0));
    // Equidistante (100 entre 96 y 104): nearest redondea hacia +∞ = 104.
    assert_eq!(width_px("width: round(100px, 8px)"), Some(104.0));
    // round(up, 98px, 8px) = 104; round(down, 98px, 8px) = 96.
    assert_eq!(width_px("width: round(up, 98px, 8px)"), Some(104.0));
    assert_eq!(width_px("width: round(down, 98px, 8px)"), Some(96.0));
}

#[test]
fn mod_y_rem() {
    // mod(18px, 5px) = 3; rem(18px, 5px) = 3.
    assert_eq!(width_px("width: mod(18px, 5px)"), Some(3.0));
    assert_eq!(width_px("width: rem(18px, 5px)"), Some(3.0));
}

// ── Fase 7.855 — animation-timing-function: cubic-bezier/steps ──────────────

#[test]
fn animation_timing_cubic_bezier_y_steps() {
    assert!(decls("animation-timing-function: cubic-bezier(0.1, 0.7, 1, 0.1)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AnimationTimingFunction(EasingFunction::CubicBezier(..)))));
    assert!(decls("animation-timing-function: steps(4, end)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AnimationTimingFunction(EasingFunction::Steps(4, _)))));
}

// ── Fase 7.856 — text-align: justify-all + word-spacing: normal ─────────────

#[test]
fn justify_all_y_word_spacing_normal() {
    assert!(decls("text-align: justify-all")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TextAlign(TextAlign::Justify))));
    assert!(decls("word-spacing: normal")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::WordSpacing(v) if v == 0.0)));
}

// ── Fase 7.857 — margin-inline/-block: auto ────────────────────────────────

#[test]
fn margin_inline_auto_centra() {
    let d = decls("margin-inline: auto");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::MarginLeftAuto(true))));
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::MarginRightAuto(true))));
    // margin-block: auto → top/bottom 0 (no centra).
    let b = decls("margin-block: auto");
    assert!(b.iter().any(|x| matches!(x.kind, DeclKind::MarginTop(v) if v == 0.0)));
    assert!(b.iter().any(|x| matches!(x.kind, DeclKind::MarginBottom(v) if v == 0.0)));
    // Mezcla: margin-inline: auto 10px → left auto, right 10px.
    let m = decls("margin-inline: auto 10px");
    assert!(m.iter().any(|x| matches!(x.kind, DeclKind::MarginLeftAuto(true))));
    assert!(m.iter().any(|x| matches!(x.kind, DeclKind::MarginRight(v) if (v - 10.0).abs() < 0.01)));
}

// ── Fase 7.858 — border-radius multivalor + slash ──────────────────────────

#[test]
fn border_radius_dos_valores() {
    // 8px 16px → TL/BR=8, TR/BL=16.
    let d = decls("border-radius: 8px 16px");
    let r = |c: BorderCorner| {
        d.iter().find_map(|x| match x.kind {
            DeclKind::BorderCornerRadius(corner, v) if corner == c => Some(v),
            _ => None,
        })
    };
    assert_eq!(r(BorderCorner::TopLeft), Some(8.0));
    assert_eq!(r(BorderCorner::BottomRight), Some(8.0));
    assert_eq!(r(BorderCorner::TopRight), Some(16.0));
    assert_eq!(r(BorderCorner::BottomLeft), Some(16.0));
}

#[test]
fn border_radius_slash_usa_horizontal() {
    // 10px / 20px → el eje vertical se ignora; todas las esquinas = 10.
    let d = decls("border-radius: 10px / 20px");
    assert!(d
        .iter()
        .filter(|x| matches!(x.kind, DeclKind::BorderCornerRadius(..)))
        .count()
        == 4);
    assert!(d.iter().all(|x| match x.kind {
        DeclKind::BorderCornerRadius(_, v) => (v - 10.0).abs() < 0.01,
        _ => true,
    }));
}

// ── Fase 7.859 — minmax / fit-content / auto-fill en grid ──────────────────

#[test]
fn grid_minmax_toma_el_max() {
    // minmax(100px, 1fr) → aproxima al max (1fr).
    let d = decls("grid-template-columns: minmax(100px, 1fr)");
    assert!(d.iter().any(|x| matches!(&x.kind,
        DeclKind::GridTemplateColumns(v) if v == &vec![GridTrackSize::Fr(1.0)])));
    // minmax(100px, 200px) → max px.
    let d2 = decls("grid-template-columns: minmax(100px, 200px)");
    assert!(d2.iter().any(|x| matches!(&x.kind,
        DeclKind::GridTemplateColumns(v) if v == &vec![GridTrackSize::Px(200.0)])));
}

#[test]
fn grid_auto_fill_estima_columnas() {
    // repeat(auto-fill, minmax(200px, 1fr)) → floor(1280/200)=6 tracks de 1fr.
    let d = decls("grid-template-columns: repeat(auto-fill, minmax(200px, 1fr))");
    assert!(d.iter().any(|x| matches!(&x.kind,
        DeclKind::GridTemplateColumns(v) if v.len() == 6 && v.iter().all(|t| *t == GridTrackSize::Fr(1.0)))));
}

// ── Fase 7.860 — background-position por borde (3-4 tokens) ─────────────────

#[test]
fn background_position_cuatro_valores() {
    // right 10px bottom 20px → esquina inferior-derecha (offsets px → borde).
    let d = decls("background-position: right 10px bottom 20px");
    assert!(d.iter().any(|x| matches!(x.kind,
        DeclKind::BackgroundPosition(BackgroundPosition { x: LengthVal::Pct(px), y: LengthVal::Pct(py) })
            if px == 100.0 && py == 100.0)));
    // left 25% top → x=25%, y=0.
    let d2 = decls("background-position: left 25% top");
    assert!(d2.iter().any(|x| matches!(x.kind,
        DeclKind::BackgroundPosition(BackgroundPosition { x: LengthVal::Pct(px), y: LengthVal::Pct(py) })
            if px == 25.0 && py == 0.0)));
}

// ── Fase 7.861 — stretch / fill-available + vertical-align numérico ─────────

#[test]
fn width_stretch_y_fill_available() {
    for s in ["width: stretch", "width: -webkit-fill-available", "width: -moz-available"] {
        assert!(decls(s).iter().any(|d| matches!(d.kind, DeclKind::Width(LengthVal::Auto))), "{s}");
    }
}

#[test]
fn vertical_align_numerico_colapsa_a_baseline() {
    for s in ["vertical-align: 10px", "vertical-align: 50%", "vertical-align: -0.5em"] {
        assert!(decls(s)
            .iter()
            .any(|d| matches!(d.kind, DeclKind::VerticalAlign(VerticalAlign::Baseline))), "{s}");
    }
    // Un keyword sigue funcionando.
    assert!(decls("vertical-align: middle")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::VerticalAlign(VerticalAlign::Middle))));
    // Basura no parsea.
    assert!(decls("vertical-align: garbage").is_empty());
}

// ── Fase 7.862 — colores de sistema ────────────────────────────────────────

#[test]
fn colores_de_sistema() {
    let color_of = |s: &str| {
        decls(s).iter().find_map(|d| match d.kind {
            DeclKind::Color(c) => Some(c),
            _ => None,
        })
    };
    assert_eq!(color_of("color: Canvas"), Some(Color::WHITE));
    assert_eq!(color_of("color: CanvasText"), Some(Color::BLACK));
    assert_eq!(color_of("color: ActiveText"), Some(Color::rgb_const(255, 0, 0)));
    // Case-insensitive.
    assert_eq!(color_of("color: buttonface"), Some(Color::rgb_const(240, 240, 240)));
}

// ── Fase 7.863 — fuentes de sistema en el shorthand `font` ──────────────────

#[test]
fn font_system_keywords() {
    assert!(decls("font: caption")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontSize(v) if v == 13.0)));
    assert!(decls("font: small-caption")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontSize(v) if v == 11.0)));
    // El shorthand normal sigue andando.
    assert!(decls("font: italic bold 16px/1.5 serif")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FontSize(v) if v == 16.0)));
}

// ── Fase 7.864 — outline-color: invert ─────────────────────────────────────

#[test]
fn outline_color_invert() {
    assert!(decls("outline-color: invert")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::CurrentColor(ColorTarget::Outline))));
}

// ── Fase 7.865 — line-height: calc() ───────────────────────────────────────

#[test]
fn line_height_calc() {
    // calc(1.5) número crudo = multiplicador 1.5.
    assert!(decls("line-height: calc(1.5)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6)));
    // calc(24px) px → /16 = 1.5.
    assert!(decls("line-height: calc(20px + 4px)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6)));
}

// ── Fase 7.866 — background longhands con listas por coma ───────────────────

#[test]
fn background_longhands_comma_toman_primera_capa() {
    // background-size: cover, contain → toma `cover`.
    assert!(decls("background-size: cover, contain")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::BackgroundSize(BackgroundSize::Cover))));
    // background-repeat: no-repeat, repeat → toma `no-repeat`.
    assert!(decls("background-repeat: no-repeat, repeat")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::BackgroundRepeat(_))));
    // background-position: 50% 50%, top right → toma la 1ª capa.
    assert!(decls("background-position: 50% 50%, top right")
        .iter()
        .any(|d| matches!(d.kind,
            DeclKind::BackgroundPosition(BackgroundPosition { x: LengthVal::Pct(px), y: LengthVal::Pct(py) })
                if px == 50.0 && py == 50.0)));
}

// ── Fase 7.867 — list-style-type estilos extra → aproximación ──────────────

#[test]
fn list_style_type_extra_keywords() {
    let lst = |s: &str| {
        decls(s).iter().find_map(|d| match d.kind {
            DeclKind::ListStyleType(t) => Some(t),
            _ => None,
        })
    };
    assert_eq!(lst("list-style-type: georgian"), Some(ListStyleType::Decimal));
    assert_eq!(lst("list-style-type: decimal-leading-zero"), Some(ListStyleType::Decimal));
    assert_eq!(lst("list-style-type: lower-greek"), Some(ListStyleType::LowerAlpha));
    assert_eq!(lst("list-style-type: disclosure-open"), Some(ListStyleType::Disc));
}

// ── Fase 7.868 — color() gamut amplio ──────────────────────────────────────

#[test]
fn color_func_gamut_amplio() {
    // rec2020 rojo puro ≈ rojo sRGB saturado (clamp). No debe descartarse.
    assert!(decls("color: color(rec2020 1 0 0)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Color(_))));
    // xyz: el rojo sRGB tiene XYZ ≈ (0.4124, 0.2126, 0.0193).
    let c = decls("color: color(xyz 0.4124 0.2126 0.0193)")
        .iter()
        .find_map(|d| match d.kind { DeclKind::Color(c) => Some(c), _ => None });
    let c = c.unwrap();
    assert!(c.r > 200 && c.g < 60 && c.b < 60, "xyz rojo dio {c:?}");
    // a98-rgb / prophoto-rgb parsean.
    assert!(!decls("color: color(a98-rgb 1 0 0)").is_empty());
    assert!(!decls("color: color(prophoto-rgb 1 0 0)").is_empty());
}

// ── Fase 7.869 — env() ─────────────────────────────────────────────────────

#[test]
fn env_resuelve_a_cero_o_fallback() {
    // env(safe-area-inset-top) → 0px.
    assert!(decls("padding-top: env(safe-area-inset-top)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::PaddingTop(v) if v == 0.0)));
    // Con fallback → el fallback.
    assert!(decls("padding-top: env(safe-area-inset-top, 20px)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::PaddingTop(v) if (v - 20.0).abs() < 0.01)));
    // Dentro de calc.
    assert!(decls("padding-top: calc(env(safe-area-inset-top, 10px) + 5px)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::PaddingTop(v) if (v - 15.0).abs() < 0.01)));
}

// ── Fase 7.870 — image-set() / cross-fade() ────────────────────────────────

#[test]
fn image_set_y_cross_fade_toman_primera_url() {
    assert!(decls("background-image: image-set(url(a.png) 1x, url(b.png) 2x)")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::BackgroundImageUrl(u) if u == "a.png")));
    assert!(decls("background-image: cross-fade(url(x.png), url(y.png))")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::BackgroundImageUrl(u) if u == "x.png")));
}

// ── Fase 7.871 — funciones math abs/sign/sqrt/pow/hypot + constantes ───────

#[test]
fn math_abs_sign_y_constantes() {
    // abs(-10px) = 10px.
    assert_eq!(width_px("width: abs(-10px)"), Some(10.0));
    assert_eq!(width_px("width: calc(abs(-10px) + 5px)"), Some(15.0));
    // sign(-3) = -1 → * 10px = -10px.
    assert_eq!(width_px("width: calc(sign(-3) * 10px)"), Some(-10.0));
    // pi * 10px ≈ 31.4.
    assert!((width_px("width: calc(10px * pi)").unwrap() - 31.4159).abs() < 0.01);
    // e: 100 / e ≈ 36.8.
    assert!((width_px("width: calc(100px / e)").unwrap() - 36.7879).abs() < 0.01);
}

#[test]
fn math_sqrt_pow_hypot() {
    assert_eq!(width_px("width: calc(sqrt(16) * 1px)"), Some(4.0));
    assert_eq!(width_px("width: calc(pow(2, 3) * 1px)"), Some(8.0));
    // hypot(3px, 4px) = 5px.
    assert_eq!(width_px("width: hypot(3px, 4px)"), Some(5.0));
}

// ── Fase 7.872 — props <number> aceptan calc + flex-basis: content ─────────

#[test]
fn props_numero_aceptan_calc() {
    assert!(decls("opacity: calc(1 / 4)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Opacity(v) if (v - 0.25).abs() < 1e-6)));
    assert!(decls("z-index: calc(2 + 3)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::ZIndex(5))));
    assert!(decls("flex-grow: calc(1 + 1)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FlexGrow(v) if v == 2.0)));
    assert!(decls("flex-basis: content")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::FlexBasis(LengthVal::Auto))));
    assert!(decls("order: calc(0 - 1)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Order(-1))));
}

// ── Fase 7.873 — line-height %, outline-width kw, text-indent kw, t-transform ─

#[test]
fn varios_873() {
    // line-height: 150% = 1.5.
    assert!(decls("line-height: 150%")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6)));
    // outline-width: thin = 1px.
    assert!(decls("outline-width: thin")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::OutlineWidth(v) if v == 1.0)));
    // text-indent: 2em hanging → toma 2em (32px), ignora keyword.
    assert!(decls("text-indent: 2em hanging")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TextIndent(v) if (v - 32.0).abs() < 0.01)));
    // text-transform: full-width → None (no-op, no se descarta).
    assert!(decls("text-transform: full-width")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TextTransform(TextTransform::None))));
}

// ── Fase 7.874 — border-style multi-valor → 1er token ──────────────────────

#[test]
fn border_style_multivalor_toma_primero() {
    let d = decls("border-style: solid dotted");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::BorderEnabled(true))));
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::BorderStyleKind(BorderLineStyle::Solid))));
}

// ── Fase 7.875 — calc() con ángulos ────────────────────────────────────────

#[test]
fn rotate_prop_acepta_calc_angulos() {
    // 0.25turn (90°) + 30deg = 120°.
    let deg = |s: &str| {
        decls(s).iter().find_map(|d| match d.kind {
            DeclKind::Rotate(Some(Transform::Rotate(deg))) => Some(deg),
            _ => None,
        })
    };
    assert!((deg("rotate: calc(0.25turn + 30deg)").unwrap() - 120.0).abs() < 0.01);
    assert!((deg("rotate: calc(90deg / 2)").unwrap() - 45.0).abs() < 0.01);
    assert!((deg("rotate: max(45deg, 90deg)").unwrap() - 90.0).abs() < 0.01);
    // 1rad ≈ 57.2958°.
    assert!((deg("rotate: calc(1rad)").unwrap() - 57.2958).abs() < 0.01);
}

#[test]
fn transform_y_hue_aceptan_calc_angulos() {
    // transform: rotate(calc(45deg + 45deg)) = 90°.
    assert!(decls("transform: rotate(calc(45deg + 45deg))")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::Transforms(t)
            if t.iter().any(|x| matches!(x, Transform::Rotate(deg) if (deg - 90.0).abs() < 0.01)))));
    // skew(calc(10deg)) parsea.
    assert!(decls("transform: skew(calc(10deg))")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::Transforms(_))));
    // hue-rotate vía parse_hue calc-aware (filter).
    assert!(!decls("filter: hue-rotate(calc(90deg + 90deg))").is_empty());
}

// ── Fase 7.876 — aspect-ratio auto sufijo, rgb(none), pointer-events ───────

#[test]
fn varios_876() {
    // aspect-ratio: 4 / 3 auto → ratio 4/3 (auto sufijo descartado).
    assert!(decls("aspect-ratio: 4 / 3 auto")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AspectRatio(Some(r)) if (r - 4.0 / 3.0).abs() < 1e-6)));
    // rgb(none none none) → negro (none = 0).
    assert!(decls("color: rgb(none none none)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::Color(c) if c.r == 0 && c.g == 0 && c.b == 0)));
    // pointer-events: bounding-box → Auto.
    assert!(decls("pointer-events: bounding-box")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::PointerEvents(PointerEvents::Auto))));
}

// ── Fase 7.878 — color relativo rgb()/hsl() from ───────────────────────────

#[test]
fn color_relativo_rgb_y_hsl() {
    let col = |s: &str| decls(s).iter().find_map(|d| match d.kind {
        DeclKind::Color(c) => Some(c),
        _ => None,
    });
    // Identidad: rgb(from red r g b) = red.
    assert_eq!(col("color: rgb(from red r g b)"), Some(Color { r: 255, g: 0, b: 0, a: 255 }));
    // Cambiar sólo alpha.
    assert_eq!(
        col("color: rgb(from red r g b / 50%)"),
        Some(Color { r: 255, g: 0, b: 0, a: 128 })
    );
    // calc sobre un canal: rgb(from #336699 r g calc(b / 2)) → b=153/2≈76.
    let c = col("color: rgb(from #336699 r g calc(b / 2))").unwrap();
    assert_eq!((c.r, c.g), (0x33, 0x66));
    assert!((c.b as i32 - 76).abs() <= 1, "b = {}", c.b);
    // hsl identidad: hsl(from blue h s l) ≈ blue (round-trip HSL).
    let hb = col("color: hsl(from blue h s l)").unwrap();
    assert!(hb.b > 250 && hb.r < 5 && hb.g < 5, "hsl from blue = {hb:?}");
}

// ── Lote data-driven: float/clear, grid auto-flow, masonry, subgrid, ──────
//    animation-timeline scroll()/view(), text-box shorthand, d (SVG) ───────

#[test]
fn float_y_clear_incluyen_logicos() {
    let f = |s: &str| decls(s).iter().find_map(|d| match d.kind {
        DeclKind::Float(v) => Some(v),
        _ => None,
    });
    let c = |s: &str| decls(s).iter().find_map(|d| match d.kind {
        DeclKind::Clear(v) => Some(v),
        _ => None,
    });
    assert_eq!(f("float: left"), Some(Float::Left));
    assert_eq!(f("float: right"), Some(Float::Right));
    assert_eq!(f("float: inline-start"), Some(Float::InlineStart));
    assert_eq!(f("float: inline-end"), Some(Float::InlineEnd));
    assert_eq!(f("float: none"), Some(Float::None));
    assert_eq!(c("clear: both"), Some(Clear::Both));
    assert_eq!(c("clear: left"), Some(Clear::Left));
    assert_eq!(c("clear: inline-end"), Some(Clear::InlineEnd));
    // Valores inválidos dropean.
    assert!(decls("float: middle").is_empty());
    assert!(decls("clear: everything").is_empty());
}

#[test]
fn grid_shorthand_auto_flow() {
    // `auto-flow` a la izquierda → flujo row, template = columnas.
    let d = decls("grid: auto-flow / 1fr 1fr");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::GridAutoFlow(GridAutoFlow::Row))));
    assert!(d.iter().any(|x| matches!(&x.kind, DeclKind::GridAutoRows(t) if t.is_empty())));
    assert!(d.iter().any(|x| matches!(&x.kind,
        DeclKind::GridTemplateColumns(t) if t.len() == 2
            && matches!(t[0], GridTrackSize::Fr(f) if (f - 1.0).abs() < 1e-6))));
    // `auto-flow dense <tracks>` a la izquierda.
    let d = decls("grid: auto-flow dense 50px / 1fr");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::GridAutoFlow(GridAutoFlow::RowDense))));
    assert!(d.iter().any(|x| matches!(&x.kind,
        DeclKind::GridAutoRows(t) if matches!(t.as_slice(), [GridTrackSize::Px(p)] if (*p - 50.0).abs() < 1e-6))));
    // `auto-flow` a la derecha → flujo column, template = filas, auto-columns.
    let d = decls("grid: 1fr 1fr / auto-flow 100px");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::GridAutoFlow(GridAutoFlow::Column))));
    assert!(d.iter().any(|x| matches!(&x.kind,
        DeclKind::GridAutoColumns(t) if matches!(t.as_slice(), [GridTrackSize::Px(p)] if (*p - 100.0).abs() < 1e-6))));
    assert!(d.iter().any(|x| matches!(&x.kind, DeclKind::GridTemplateRows(t) if t.len() == 2)));
}

#[test]
fn masonry_auto_flow_y_tracks() {
    let maf = |s: &str| decls(s).iter().find_map(|d| match d.kind {
        DeclKind::MasonryAutoFlow(v) => Some(v),
        _ => None,
    });
    assert_eq!(
        maf("masonry-auto-flow: pack"),
        Some(MasonryAutoFlow { placement: MasonryPlacement::Pack, order: MasonryOrder::DefiniteFirst })
    );
    assert_eq!(
        maf("masonry-auto-flow: next ordered"),
        Some(MasonryAutoFlow { placement: MasonryPlacement::Next, order: MasonryOrder::Ordered })
    );
    // Orden libre.
    assert_eq!(
        maf("masonry-auto-flow: ordered next"),
        Some(MasonryAutoFlow { placement: MasonryPlacement::Next, order: MasonryOrder::Ordered })
    );
    // Componente repetido dropea.
    assert!(decls("masonry-auto-flow: pack next").is_empty());
    assert!(decls("masonry-auto-flow: bogus").is_empty());
    // justify-tracks / align-tracks: listas por coma.
    assert!(decls("justify-tracks: center").iter().any(|d| matches!(&d.kind,
        DeclKind::JustifyTracks(v) if v.as_slice() == [JustifyContent::Center])));
    assert!(decls("justify-tracks: start, end, center").iter().any(|d| matches!(&d.kind,
        DeclKind::JustifyTracks(v) if v.len() == 3 && v[1] == JustifyContent::End)));
    assert!(decls("align-tracks: end").iter().any(|d| matches!(&d.kind,
        DeclKind::AlignTracks(v) if v.as_slice() == [AlignContent::End])));
}

#[test]
fn grid_template_subgrid() {
    let d = decls("grid-template-columns: subgrid");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::GridTemplateColumnsSubgrid(true))));
    assert!(d.iter().any(|x| matches!(&x.kind, DeclKind::GridTemplateColumns(t) if t.is_empty())));
    // Líneas nombradas se descartan pero el subgrid se reconoce.
    let d = decls("grid-template-rows: subgrid [line-a] [line-b]");
    assert!(d.iter().any(|x| matches!(x.kind, DeclKind::GridTemplateRowsSubgrid(true))));
    // Un track-list normal NO marca subgrid (y, vía apply, lo resetea).
    assert!(decls("grid-template-columns: 1fr 1fr")
        .iter()
        .all(|x| !matches!(x.kind, DeclKind::GridTemplateColumnsSubgrid(_))));
}

#[test]
fn animation_timeline_scroll_y_view() {
    let tl = |s: &str| decls(s).iter().find_map(|d| match &d.kind {
        DeclKind::AnimationTimeline(v) => Some(v.clone()),
        _ => None,
    });
    assert_eq!(
        tl("animation-timeline: scroll(root block)"),
        Some(TimelineRef::Scroll { scroller: ScrollScroller::Root, axis: TimelineAxis::Block })
    );
    // scroll() vacío → defaults nearest/block.
    assert_eq!(
        tl("animation-timeline: scroll()"),
        Some(TimelineRef::Scroll { scroller: ScrollScroller::Nearest, axis: TimelineAxis::Block })
    );
    // view(axis) sin inset.
    assert_eq!(
        tl("animation-timeline: view(inline)"),
        Some(TimelineRef::View { axis: TimelineAxis::Inline, inset: None })
    );
    // view(axis inset) guarda inset opaco.
    assert_eq!(
        tl("animation-timeline: view(block auto)"),
        Some(TimelineRef::View { axis: TimelineAxis::Block, inset: Some("auto".to_string()) })
    );
    // <dashed-ident> sigue funcionando.
    assert_eq!(tl("animation-timeline: --mi-tl"), Some(TimelineRef::Named("--mi-tl".to_string())));
    // scroll con eje inválido dropea.
    assert!(tl("animation-timeline: scroll(diagonal)").is_none());
}

#[test]
fn text_box_shorthand() {
    let pair = |s: &str| {
        let d = decls(s);
        let t = d.iter().find_map(|x| match x.kind {
            DeclKind::TextBoxTrim(v) => Some(v),
            _ => None,
        });
        let e = d.iter().find_map(|x| match x.kind {
            DeclKind::TextBoxEdge(v) => Some(v),
            _ => None,
        });
        (t, e)
    };
    // trim + edge (2 keywords de edge).
    assert_eq!(
        pair("text-box: trim-both cap alphabetic"),
        (Some(TextBoxTrim::TrimBoth), Some(TextBoxEdge::Edge { over: TextEdge::Cap, under: TextEdge::Alphabetic }))
    );
    // solo trim → edge auto.
    assert_eq!(pair("text-box: trim-both"), (Some(TextBoxTrim::TrimBoth), Some(TextBoxEdge::Auto)));
    // solo edge → trim none.
    assert_eq!(
        pair("text-box: cap alphabetic"),
        (Some(TextBoxTrim::None), Some(TextBoxEdge::Edge { over: TextEdge::Cap, under: TextEdge::Alphabetic }))
    );
    // normal → ambos default.
    assert_eq!(pair("text-box: normal"), (Some(TextBoxTrim::None), Some(TextBoxEdge::Auto)));
}

#[test]
fn d_svg_geometry_como_css() {
    assert!(decls("d: path(\"M0 0 L10 10\")")
        .iter()
        .any(|x| matches!(&x.kind, DeclKind::D(Some(p)) if p.starts_with("path("))));
    assert!(decls("d: none").iter().any(|x| matches!(x.kind, DeclKind::D(None))));
    // Valor inválido dropea (no es path() ni none).
    assert!(decls("d: 5px").is_empty());
}

// ── Lote data-driven (cont.): linear() easing + page ──────────────────────

#[test]
fn linear_easing_y_page() {
    // `linear(...)` valida y colapsa al keyword Linear (plumb lossy).
    assert!(decls("transition-timing-function: linear(0, 0.5 50%, 1)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::TransitionTimingFirst(EasingFunction::Linear))));
    assert!(decls("animation-timing-function: linear(0, 1)")
        .iter()
        .any(|d| matches!(d.kind, DeclKind::AnimationTimingFunction(EasingFunction::Linear))));
    // linear() vacío o con basura dropea.
    assert!(decls("transition-timing-function: linear()").is_empty());
    assert!(decls("transition-timing-function: linear(foo)").is_empty());
    assert!(decls("transition-timing-function: linear(0 10% 20% 30%)").is_empty());
    // `page`: auto → None; <custom-ident> → Some.
    assert!(decls("page: auto").iter().any(|d| matches!(d.kind, DeclKind::Page(None))));
    assert!(decls("page: chapter")
        .iter()
        .any(|d| matches!(&d.kind, DeclKind::Page(Some(n)) if n == "chapter")));
    // multi-word inválido.
    assert!(decls("page: a b").is_empty());
}
