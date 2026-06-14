//! Tests del motor de estilo (grupo 11, extraído de `style/mod.rs`, regla #1).
use super::super::*;

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
        // Fase 7.867 — `georgian` ya no se descarta: se aproxima a `Decimal`
        // (estilo numérico). Un keyword realmente desconocido sí da None.
        assert_eq!(parse_list_style_type("georgian"), Some(ListStyleType::Decimal));
        assert_eq!(parse_list_style_type("no-existe-tal-estilo"), None);
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
    fn parsea_hue_unidades_y_none() {
        // 0.5turn = 180deg = cyan; 200grad = 180deg; π rad = 180deg.
        let cyan = Color::rgb(0, 255, 255);
        assert_eq!(parse_color("hsl(0.5turn 100% 50%)").unwrap(), cyan);
        assert_eq!(parse_color("hsl(200grad 100% 50%)").unwrap(), cyan);
        assert_eq!(parse_color("hsl(3.14159265rad 100% 50%)").unwrap(), cyan);
        // `none` en hue ⇒ 0deg = rojo.
        assert_eq!(parse_color("hwb(none 0% 0%)").unwrap(), Color::rgb(255, 0, 0));
    }

    #[test]
    fn parsea_hwb() {
        // hwb sin blancura ni negrura = hue puro.
        assert_eq!(parse_color("hwb(0 0% 0%)").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(parse_color("hwb(120 0% 0%)").unwrap(), Color::rgb(0, 255, 0));
        // 50% blancura clarea el rojo.
        assert_eq!(parse_color("hwb(0 50% 0%)").unwrap(), Color::rgb(255, 128, 128));
        // 50% negrura lo oscurece.
        assert_eq!(parse_color("hwb(0 0% 50%)").unwrap(), Color::rgb(128, 0, 0));
        // W+B ≥ 100% ⇒ gris W/(W+B).
        assert_eq!(parse_color("hwb(0 100% 100%)").unwrap(), Color::rgb(128, 128, 128));
        // Alpha por slash.
        assert_eq!(parse_color("hwb(0 0% 0% / 0.5)").unwrap(), Color { r: 255, g: 0, b: 0, a: 128 });
    }

    #[test]
    fn parsea_oklab_y_oklch() {
        // Blanco y negro son deterministas.
        assert_eq!(parse_color("oklab(1 0 0)").unwrap(), Color::rgb(255, 255, 255));
        assert_eq!(parse_color("oklab(0 0 0)").unwrap(), Color::rgb(0, 0, 0));
        assert_eq!(parse_color("oklch(1 0 0)").unwrap(), Color::rgb(255, 255, 255));
        // Alpha + `none` en lightness.
        assert_eq!(parse_color("oklch(none 0 0 / 0.5)").unwrap(), Color { r: 0, g: 0, b: 0, a: 128 });
        // Rojo sRGB ≈ oklch(0.628 0.2577 29.23) — tolerancia.
        let red = parse_color("oklch(0.628 0.2577 29.23)").unwrap();
        assert!(red.r > 245 && red.g < 25 && red.b < 25, "oklch rojo: {red:?}");
        // Porcentajes: L 100% = 1.0.
        assert_eq!(parse_color("oklch(100% 0 0)").unwrap(), Color::rgb(255, 255, 255));
    }

    #[test]
    fn parsea_lab_y_lch() {
        // Blanco D50 y negro.
        let white = parse_color("lab(100 0 0)").unwrap();
        assert!(white.r >= 253 && white.g >= 253 && white.b >= 253, "lab blanco: {white:?}");
        assert_eq!(parse_color("lab(0 0 0)").unwrap(), Color::rgb(0, 0, 0));
        let white_lch = parse_color("lch(100 0 0)").unwrap();
        assert!(white_lch.r >= 253 && white_lch.g >= 253 && white_lch.b >= 253);
        // Rojo sRGB ≈ lab(54.29 80.81 69.89) — tolerancia.
        let red = parse_color("lab(54.29 80.81 69.89)").unwrap();
        assert!(red.r > 245 && red.g < 25 && red.b < 25, "lab rojo: {red:?}");
    }

    #[test]
    fn parsea_color_func() {
        // srgb directo.
        assert_eq!(parse_color("color(srgb 1 0 0)").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(parse_color("color(srgb 0 1 0)").unwrap(), Color::rgb(0, 255, 0));
        // srgb-linear pasa por la gamma sRGB al codificar.
        assert_eq!(parse_color("color(srgb-linear 1 1 1)").unwrap(), Color::rgb(255, 255, 255));
        let mid = parse_color("color(srgb-linear 0.5 0.5 0.5)").unwrap();
        assert!((mid.r as i32 - 188).abs() <= 1, "srgb-linear 0.5: {mid:?}");
        // display-p3: blanco = blanco; verde P3 puro recorta al gamut sRGB.
        assert_eq!(parse_color("color(display-p3 1 1 1)").unwrap(), Color::rgb(255, 255, 255));
        assert_eq!(parse_color("color(display-p3 0 1 0)").unwrap(), Color::rgb(0, 255, 0));
        // Alpha.
        assert_eq!(parse_color("color(srgb 1 0 0 / 0.5)").unwrap(), Color { r: 255, g: 0, b: 0, a: 128 });
        // Fase 7.868 — rec2020/a98/prophoto/xyz ahora SÍ se resuelven (vía
        // pivote XYZ). rec2020 rojo puro recorta al gamut sRGB ≈ rojo saturado.
        let r2020 = parse_color("color(rec2020 1 0 0)").unwrap();
        assert!(r2020.r > 200 && r2020.g < 60 && r2020.b < 60, "rec2020 rojo: {r2020:?}");
        // Un espacio realmente inexistente sigue dando None.
        assert!(parse_color("color(no-tal-espacio 1 0 0)").is_none());
    }

    #[test]
    fn parsea_color_mix() {
        // 50/50 en sRGB.
        assert_eq!(parse_color("color-mix(in srgb, red, blue)").unwrap(), Color::rgb(128, 0, 128));
        assert_eq!(parse_color("color-mix(in srgb, white, black)").unwrap(), Color::rgb(128, 128, 128));
        // Porcentaje en el primer color.
        assert_eq!(parse_color("color-mix(in srgb, red 25%, blue)").unwrap(), Color::rgb(64, 0, 191));
        // Porcentaje en el segundo color (equivalente).
        assert_eq!(parse_color("color-mix(in srgb, red, blue 75%)").unwrap(), Color::rgb(64, 0, 191));
        // Ambos porcentajes se normalizan (20+20 → 50/50).
        assert_eq!(parse_color("color-mix(in srgb, red 20%, blue 20%)").unwrap(), Color::rgb(128, 0, 128));
        // Alpha se interpola.
        let alpha = parse_color("color-mix(in srgb, #ff000000, #ff0000ff)").unwrap();
        assert_eq!(alpha, Color { r: 255, g: 0, b: 0, a: 128 });
        // Espacio no soportado degrada a sRGB (no rompe el parseo).
        assert_eq!(parse_color("color-mix(in jzazbz, red, blue)").unwrap(), Color::rgb(128, 0, 128));
    }

    #[test]
    fn parsea_color_mix_perceptual() {
        // En oklab/oklch el mix de rojo y azul da un púrpura perceptual
        // (ambos canales presentes, verde bajo). Tolerancia.
        let ok = parse_color("color-mix(in oklab, red, blue)").unwrap();
        assert!(ok.r > 40 && ok.b > 40 && ok.g < 90, "oklab mix: {ok:?}");
        // oklch parsea y produce un color válido distinto del negro.
        let oklch = parse_color("color-mix(in oklch, red, blue)").unwrap();
        assert!(oklch.r as u32 + oklch.g as u32 + oklch.b as u32 > 0, "oklch mix: {oklch:?}");
        // Mezclar un color consigo mismo lo deja igual (sanity).
        assert_eq!(parse_color("color-mix(in oklab, red, red)").unwrap().r, 255);
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

