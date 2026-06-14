//! Tests del motor de estilo (grupo 08, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn shapes_text_combine_ruby_align_fase_7_444_448() {
        // Cinco props: shape-outside (opaco), shape-margin, shape-image-threshold,
        // text-combine-upright (NO heredan), ruby-align (HEREDA).

        // 1) Parsers de cabecera.
        assert_eq!(parse_alpha_value("0.5"), Some(0.5));
        assert_eq!(parse_alpha_value("75%"), Some(0.75));
        assert_eq!(parse_alpha_value("1.5"), Some(1.0)); // clamp
        assert_eq!(parse_alpha_value("-0.3"), Some(0.0)); // clamp
        assert_eq!(parse_alpha_value("nope"), None);
        assert_eq!(
            parse_text_combine_upright("none"),
            Some(TextCombineUpright::None)
        );
        assert_eq!(
            parse_text_combine_upright("ALL"),
            Some(TextCombineUpright::All)
        );
        assert_eq!(
            parse_text_combine_upright("digits"),
            Some(TextCombineUpright::Digits(2))
        );
        assert_eq!(
            parse_text_combine_upright("digits 4"),
            Some(TextCombineUpright::Digits(4))
        );
        assert_eq!(parse_text_combine_upright("digits nope"), None);
        assert_eq!(parse_ruby_align("start"), Some(RubyAlign::Start));
        assert_eq!(
            parse_ruby_align("SPACE-BETWEEN"),
            Some(RubyAlign::SpaceBetween)
        );
        assert_eq!(parse_ruby_align("nope"), None);

        // 2) E2E.
        let html = r##"<html><head><style>
            body {
                shape-outside: circle(50%);
                shape-margin: 12px;
                shape-image-threshold: 0.4;
                text-combine-upright: digits 3;
                ruby-align: center
            }
            #hijo {}
            #override { shape-margin: 50%; ruby-align: start }
        </style></head><body>
            <div id="hijo"></div>
            <div id="override"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut hijos = Vec::new();
        let mut overrides = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
            if crate::dom::attr(n, "id").as_deref() == Some("hijo") {
                hijos.push(n.clone());
            }
            if crate::dom::attr(n, "id").as_deref() == Some("override") {
                overrides.push(n.clone());
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.shape_outside.as_deref(), Some("circle(50%)"));
        assert_eq!(body_cs.shape_margin, LengthVal::Px(12.0));
        assert!((body_cs.shape_image_threshold - 0.4).abs() < 1e-5);
        assert_eq!(
            body_cs.text_combine_upright,
            TextCombineUpright::Digits(3)
        );
        assert_eq!(body_cs.ruby_align, RubyAlign::Center);
        // Hijo: shape-* y text-combine-upright NO heredan; ruby-align SÍ.
        let hijo_cs = eng.compute_with_parent(&hijos[0], Some(&body_cs));
        assert_eq!(hijo_cs.shape_outside, None);
        assert_eq!(hijo_cs.shape_margin, LengthVal::Px(0.0));
        assert_eq!(hijo_cs.shape_image_threshold, 0.0);
        assert_eq!(hijo_cs.text_combine_upright, TextCombineUpright::None);
        assert_eq!(hijo_cs.ruby_align, RubyAlign::Center);
        // Override: cambia shape-margin (pct) y ruby-align.
        let ov_cs = eng.compute_with_parent(&overrides[0], Some(&body_cs));
        assert_eq!(ov_cs.shape_margin, LengthVal::Pct(50.0));
        assert_eq!(ov_cs.ruby_align, RubyAlign::Start);
    }

    #[test]
    fn background_position_xy_grid_auto_fase_7_439_443() {
        // Cinco props: background-position-{x,y} + grid-auto-{flow,columns,rows}.
        // Las dos primeras reescriben sólo un eje del BackgroundPosition; las
        // tres siguientes pueblan campos nuevos en ComputedStyle (no heredan).

        // 1) Parsers de cabecera.
        assert_eq!(parse_background_position_x("left"), Some(LengthVal::Pct(0.0)));
        assert_eq!(parse_background_position_x("CENTER"), Some(LengthVal::Pct(50.0)));
        assert_eq!(parse_background_position_x("right"), Some(LengthVal::Pct(100.0)));
        assert_eq!(parse_background_position_x("25%"), Some(LengthVal::Pct(25.0)));
        assert_eq!(parse_background_position_x("10px"), Some(LengthVal::Px(10.0)));
        assert_eq!(parse_background_position_x("top"), None);
        assert_eq!(parse_background_position_y("top"), Some(LengthVal::Pct(0.0)));
        assert_eq!(parse_background_position_y("bottom"), Some(LengthVal::Pct(100.0)));
        assert_eq!(parse_background_position_y("33%"), Some(LengthVal::Pct(33.0)));
        assert_eq!(parse_background_position_y("left"), None);
        assert_eq!(parse_grid_auto_flow("row"), Some(GridAutoFlow::Row));
        assert_eq!(parse_grid_auto_flow("column"), Some(GridAutoFlow::Column));
        assert_eq!(parse_grid_auto_flow("dense"), Some(GridAutoFlow::RowDense));
        assert_eq!(parse_grid_auto_flow("row dense"), Some(GridAutoFlow::RowDense));
        assert_eq!(
            parse_grid_auto_flow("dense column"),
            Some(GridAutoFlow::ColumnDense)
        );
        assert_eq!(parse_grid_auto_flow("nope"), None);

        // 2) E2E.
        let html = r##"<html><head><style>
            #a { background-position: left top;
                 background-position-x: 25%;
                 background-position-y: 80% }
            #b { background-position-y: bottom }
            #c { grid-auto-flow: column dense;
                 grid-auto-columns: 100px 200px;
                 grid-auto-rows: 50px auto }
        </style></head><body>
            <div id="a"></div>
            <div id="b"></div>
            <div id="c"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by = |id: &str| {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // #a — los longhands -x/-y pisan al shorthand previo.
        let cs_a = eng.compute(&by("a"));
        assert_eq!(cs_a.background_position.x, LengthVal::Pct(25.0));
        assert_eq!(cs_a.background_position.y, LengthVal::Pct(80.0));
        // #b — sólo se cambia Y; X queda en default (0%).
        let cs_b = eng.compute(&by("b"));
        assert_eq!(cs_b.background_position.x, LengthVal::Pct(0.0));
        assert_eq!(cs_b.background_position.y, LengthVal::Pct(100.0));
        // #c — grid auto flow + tracks implícitos.
        let cs_c = eng.compute(&by("c"));
        assert_eq!(cs_c.grid_auto_flow, GridAutoFlow::ColumnDense);
        assert_eq!(
            cs_c.grid_auto_columns,
            vec![GridTrackSize::Px(100.0), GridTrackSize::Px(200.0)]
        );
        assert_eq!(
            cs_c.grid_auto_rows,
            vec![GridTrackSize::Px(50.0), GridTrackSize::Auto]
        );
    }

    #[test]
    fn contain_intrinsic_size_fase_7_434_438() {
        // Cinco props nuevas: contain-intrinsic-{width,height,block-size,
        // inline-size} (longhands) + contain-intrinsic-size (shorthand).
        // En LTR horizontal: block-size = height, inline-size = width.

        // 1) Parser de cabecera (todas las formas).
        assert_eq!(
            parse_contain_intrinsic_size("none"),
            Some(ContainIntrinsicSize::None)
        );
        assert_eq!(
            parse_contain_intrinsic_size("200px"),
            Some(ContainIntrinsicSize::Length(200.0))
        );
        assert_eq!(
            parse_contain_intrinsic_size("auto none"),
            Some(ContainIntrinsicSize::AutoNone)
        );
        assert_eq!(
            parse_contain_intrinsic_size("auto 150px"),
            Some(ContainIntrinsicSize::AutoLength(150.0))
        );
        // Rechazos: `auto` solo, basura, dos lengths.
        assert_eq!(parse_contain_intrinsic_size("auto"), None);
        assert_eq!(parse_contain_intrinsic_size("nope"), None);
        assert_eq!(parse_contain_intrinsic_size("100px 200px"), None);

        // 2) E2E — longhands físicos + lógicos + shorthand.
        let html = r##"<html><head><style>
            #a { contain-intrinsic-width: 300px;
                 contain-intrinsic-height: auto 240px }
            #b { contain-intrinsic-block-size: 180px;
                 contain-intrinsic-inline-size: auto none }
            #c { contain-intrinsic-size: 400px }
            #d { contain-intrinsic-size: 500px auto 360px }
            #e { contain-intrinsic-size: auto 150px none }
        </style></head><body>
            <div id="a"></div>
            <div id="b"></div>
            <div id="c"></div>
            <div id="d"></div>
            <div id="e"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by = |id: &str| {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // #a — longhands físicos.
        let cs_a = eng.compute(&by("a"));
        assert_eq!(cs_a.contain_intrinsic_width, ContainIntrinsicSize::Length(300.0));
        assert_eq!(
            cs_a.contain_intrinsic_height,
            ContainIntrinsicSize::AutoLength(240.0)
        );
        // #b — longhands lógicos mapean a height/width en LTR horizontal.
        let cs_b = eng.compute(&by("b"));
        assert_eq!(cs_b.contain_intrinsic_height, ContainIntrinsicSize::Length(180.0));
        assert_eq!(cs_b.contain_intrinsic_width, ContainIntrinsicSize::AutoNone);
        // #c — shorthand de 1 valor aplica a ambos.
        let cs_c = eng.compute(&by("c"));
        assert_eq!(cs_c.contain_intrinsic_width, ContainIntrinsicSize::Length(400.0));
        assert_eq!(cs_c.contain_intrinsic_height, ContainIntrinsicSize::Length(400.0));
        // #d — shorthand de 2 lados: width=500px, height=auto 360px.
        let cs_d = eng.compute(&by("d"));
        assert_eq!(cs_d.contain_intrinsic_width, ContainIntrinsicSize::Length(500.0));
        assert_eq!(
            cs_d.contain_intrinsic_height,
            ContainIntrinsicSize::AutoLength(360.0)
        );
        // #e — shorthand con auto en el primer lado: width=auto 150px, height=none.
        let cs_e = eng.compute(&by("e"));
        assert_eq!(
            cs_e.contain_intrinsic_width,
            ContainIntrinsicSize::AutoLength(150.0)
        );
        assert_eq!(cs_e.contain_intrinsic_height, ContainIntrinsicSize::None);
    }

    #[test]
    fn hyphenate_text_size_emoji_fase_7_429_433() {
        // Parsers de cabecera + cascada heredable. Cinco props nuevas en
        // bloque: hyphenate-character, hyphenate-limit-chars,
        // text-size-adjust, line-height-step, font-variant-emoji.

        // 1) Parsers aislados.
        assert_eq!(parse_hyphenate_character("auto"), None);
        assert_eq!(
            parse_hyphenate_character("\"\u{2010}\""),
            Some("\u{2010}".to_string())
        );
        assert_eq!(
            parse_hyphenate_limit_chars("auto"),
            Some(HyphenateLimitChars::default())
        );
        assert_eq!(
            parse_hyphenate_limit_chars("6 3 2"),
            Some(HyphenateLimitChars {
                total: Some(6),
                start: Some(3),
                end: Some(2),
            })
        );
        // Sólo total — el resto queda en `None` (= auto).
        assert_eq!(
            parse_hyphenate_limit_chars("6"),
            Some(HyphenateLimitChars { total: Some(6), start: None, end: None })
        );
        // `auto` por columna (CSS Text 4).
        assert_eq!(
            parse_hyphenate_limit_chars("5 auto 2"),
            Some(HyphenateLimitChars {
                total: Some(5),
                start: None,
                end: Some(2),
            })
        );
        assert_eq!(parse_hyphenate_limit_chars("a"), None);
        assert_eq!(parse_text_size_adjust("auto"), Some(TextSizeAdjust::Auto));
        assert_eq!(parse_text_size_adjust("NONE"), Some(TextSizeAdjust::None));
        assert_eq!(parse_text_size_adjust("85%"), Some(TextSizeAdjust::Pct(85.0)));
        assert_eq!(parse_text_size_adjust("100"), None);
        assert_eq!(
            parse_font_variant_emoji("emoji"),
            Some(FontVariantEmoji::Emoji)
        );
        assert_eq!(
            parse_font_variant_emoji("UNICODE"),
            Some(FontVariantEmoji::Unicode)
        );
        assert_eq!(parse_font_variant_emoji("nope"), None);

        // 2) E2E + herencia. Body declara las 5; div hijo NO redeclara →
        // los hereda. Sibling con override no afecta el primero.
        let html = r##"<html><head><style>
            body {
                hyphenate-character: "-";
                hyphenate-limit-chars: 6 3 2;
                text-size-adjust: 80%;
                line-height-step: 24px;
                font-variant-emoji: text;
            }
            div.heredero {}
            div.override-emoji { font-variant-emoji: emoji }
        </style></head><body>
            <div class="heredero"></div>
            <div class="override-emoji"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut herederos = Vec::new();
        let mut overrides = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
            if crate::dom::element_name(n).as_deref() == Some("div") {
                match crate::dom::attr(n, "class").as_deref() {
                    Some("heredero") => herederos.push(n.clone()),
                    Some("override-emoji") => overrides.push(n.clone()),
                    _ => {}
                }
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.hyphenate_character.as_deref(), Some("-"));
        assert_eq!(body_cs.hyphenate_limit_chars.total, Some(6));
        assert_eq!(body_cs.hyphenate_limit_chars.start, Some(3));
        assert_eq!(body_cs.hyphenate_limit_chars.end, Some(2));
        assert_eq!(body_cs.text_size_adjust, TextSizeAdjust::Pct(80.0));
        assert_eq!(body_cs.line_height_step, 24.0);
        assert_eq!(body_cs.font_variant_emoji, FontVariantEmoji::Text);
        // Heredero recibe TODAS.
        let heredero_cs = eng.compute_with_parent(&herederos[0], Some(&body_cs));
        assert_eq!(heredero_cs.hyphenate_character.as_deref(), Some("-"));
        assert_eq!(heredero_cs.hyphenate_limit_chars.start, Some(3));
        assert_eq!(heredero_cs.text_size_adjust, TextSizeAdjust::Pct(80.0));
        assert_eq!(heredero_cs.line_height_step, 24.0);
        assert_eq!(heredero_cs.font_variant_emoji, FontVariantEmoji::Text);
        // Override sólo cambia emoji; resto sigue heredado.
        let over_cs = eng.compute_with_parent(&overrides[0], Some(&body_cs));
        assert_eq!(over_cs.font_variant_emoji, FontVariantEmoji::Emoji);
        assert_eq!(over_cs.hyphenate_character.as_deref(), Some("-"));
        assert_eq!(over_cs.line_height_step, 24.0);
    }

    #[test]
    fn scroll_padding_inline_offset_fase_7_424_428() {
        // Cierre del eje inline de `scroll-padding` (longhands + shorthand)
        // y arranque de `offset-path` + `offset-distance` (CSS Motion Path).
        let html = r##"<html><head><style>
            #a { scroll-padding-inline-start: 3px; scroll-padding-inline-end: 5px }
            #b { scroll-padding-inline: 7% }
            #c { offset-path: path('M 0 0 L 100 100'); offset-distance: 40% }
            #d { offset-path: none }
        </style></head><body>
            <div id="a"></div>
            <div id="b"></div>
            <div id="c"></div>
            <div id="d"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut by_id: std::collections::HashMap<String, _> = Default::default();
        crate::dom::walk(&dom.document(), &mut |n| {
            if let Some(id) = crate::dom::attr(n, "id") {
                by_id.insert(id, n.clone());
            }
        });
        let cs_a = eng.compute(&by_id["a"]);
        assert_eq!(cs_a.scroll_padding.left, LengthVal::Px(3.0));
        assert_eq!(cs_a.scroll_padding.right, LengthVal::Px(5.0));
        let cs_b = eng.compute(&by_id["b"]);
        assert_eq!(cs_b.scroll_padding.left, LengthVal::Pct(7.0));
        assert_eq!(cs_b.scroll_padding.right, LengthVal::Pct(7.0));
        let cs_c = eng.compute(&by_id["c"]);
        assert_eq!(
            cs_c.offset_path.as_deref(),
            Some("path('M 0 0 L 100 100')")
        );
        assert_eq!(cs_c.offset_distance, LengthVal::Pct(40.0));
        let cs_d = eng.compute(&by_id["d"]);
        assert_eq!(cs_d.offset_path, None);
        // `offset-distance` no tocado → default `Px(0)`.
        assert_eq!(cs_d.offset_distance, LengthVal::Px(0.0));
    }

    #[test]
    fn scroll_margin_inline_padding_block_fase_7_419_423() {
        // Cierre del lógico de `scroll-margin` (inline-end + shorthand) y
        // arranque de `scroll-padding-block` longhands + shorthand. En LTR
        // horizontal: inline=X (left/right), block=Y (top/bottom).
        let html = r##"<html><head><style>
            #a { scroll-margin-inline-end: 7px }
            #b { scroll-margin-inline: 9px 11px }
            #c { scroll-padding-block-start: 3px; scroll-padding-block-end: 13% }
            #d { scroll-padding-block: 17px }
        </style></head><body>
            <div id="a"></div>
            <div id="b"></div>
            <div id="c"></div>
            <div id="d"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut by_id: std::collections::HashMap<String, _> = Default::default();
        crate::dom::walk(&dom.document(), &mut |n| {
            if let Some(id) = crate::dom::attr(n, "id") {
                by_id.insert(id, n.clone());
            }
        });
        let cs_a = eng.compute(&by_id["a"]);
        assert_eq!(cs_a.scroll_margin.right, 7.0);
        let cs_b = eng.compute(&by_id["b"]);
        assert_eq!(cs_b.scroll_margin.left, 9.0);
        assert_eq!(cs_b.scroll_margin.right, 11.0);
        let cs_c = eng.compute(&by_id["c"]);
        assert_eq!(cs_c.scroll_padding.top, LengthVal::Px(3.0));
        assert_eq!(cs_c.scroll_padding.bottom, LengthVal::Pct(13.0));
        let cs_d = eng.compute(&by_id["d"]);
        // 1 valor → mismo top y bottom.
        assert_eq!(cs_d.scroll_padding.top, LengthVal::Px(17.0));
        assert_eq!(cs_d.scroll_padding.bottom, LengthVal::Px(17.0));
        // Lados no tocados quedan en `Auto` (default de scroll-padding).
        assert_eq!(cs_d.scroll_padding.left, LengthVal::Auto);
    }

    #[test]
    fn overscroll_behavior_block_fase_7_413() {
        // `overscroll-behavior-block` mapea al longhand físico `-y`
        // (eje vertical en LTR horizontal). NO toca el `-x`.
        let html = r##"<html><head><style>
            body { overscroll-behavior-block: contain }
        </style></head><body></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
        });
        let cs = eng.compute(&bodies[0]);
        assert_eq!(cs.overscroll_behavior_y, OverscrollBehavior::Contain);
        // `-x` no afectado → queda en default `Auto`.
        assert_eq!(cs.overscroll_behavior_x, OverscrollBehavior::Auto);
    }

    #[test]
    fn text_decoration_color_y_style() {
        // Parser de longhands sueltos.
        assert_eq!(
            parse_text_decoration_style("dotted"),
            Some(TextDecorationStyle::Dotted)
        );
        assert_eq!(parse_text_decoration_style("WAVY"), Some(TextDecorationStyle::Wavy));
        assert_eq!(parse_text_decoration_style("zigzag"), None);

        let html = r##"<html><head><style>
            p.full { text-decoration: underline dotted red }
            p.color { text-decoration-color: rgb(0,128,0) }
            p.style { text-decoration-style: dashed }
            p.cc { color: blue; text-decoration: line-through currentColor }
            p.plain { color: red }
        </style></head><body>
            <p class="full">a</p>
            <p class="color">b</p>
            <p class="style">c</p>
            <p class="cc">d</p>
            <p class="plain">e</p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 5);
        // Shorthand: line + style + color de un mismo `text-decoration`.
        let full = eng.compute(&ps[0]);
        assert_eq!(full.text_decoration, TextDecorationLine::Underline);
        assert_eq!(full.text_decoration_style, TextDecorationStyle::Dotted);
        assert_eq!(full.text_decoration_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Longhand de color suelto (no toca line/style).
        let color = eng.compute(&ps[1]);
        assert_eq!(color.text_decoration_color.map(|c| (c.r, c.g, c.b)), Some((0, 128, 0)));
        assert_eq!(color.text_decoration_style, TextDecorationStyle::Solid);
        // Longhand de style suelto.
        assert_eq!(eng.compute(&ps[2]).text_decoration_style, TextDecorationStyle::Dashed);
        // `currentColor` explícito → None (el render sigue al `color`).
        let cc = eng.compute(&ps[3]);
        assert_eq!(cc.text_decoration, TextDecorationLine::LineThrough);
        assert_eq!(cc.text_decoration_color, None);
        // Sin declarar → defaults (color None = currentColor, style Solid).
        let plain = eng.compute(&ps[4]);
        assert_eq!(plain.text_decoration_color, None);
        assert_eq!(plain.text_decoration_style, TextDecorationStyle::Solid);
    }

    #[test]
    fn outline_style_dashed_dotted() {
        let html = r##"<html><head><style>
            div.sh { outline: 2px dashed red }
            div.ls { outline-color: blue; outline-width: 3px; outline-style: dotted }
            div.db { outline: 4px double green }
            div.none { outline: 1px solid black; outline-style: none }
            div.plain { outline: 1px solid black }
        </style></head><body>
            <div class="sh"></div><div class="ls"></div><div class="db"></div>
            <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 5);
        let sh = eng.compute(&divs[0]).outline;
        assert_eq!(sh.style, BorderLineStyle::Dashed);
        assert!(sh.style_active);
        assert_eq!(sh.width, 2.0);
        assert_eq!(eng.compute(&divs[1]).outline.style, BorderLineStyle::Dotted);
        assert_eq!(eng.compute(&divs[2]).outline.style, BorderLineStyle::Double);
        // `outline-style: none` apaga (style_active=false).
        assert!(!eng.compute(&divs[3]).outline.style_active);
        // Default → Solid.
        assert_eq!(eng.compute(&divs[4]).outline.style, BorderLineStyle::Solid);
    }

    #[test]
    fn border_style_dashed_dotted_double() {
        // Parser del keyword → patrón visual.
        assert_eq!(parse_border_line_style("dashed"), Some(BorderLineStyle::Dashed));
        assert_eq!(parse_border_line_style("DOTTED"), Some(BorderLineStyle::Dotted));
        assert_eq!(parse_border_line_style("double"), Some(BorderLineStyle::Double));
        // Estilos 3D (desde Fase 7.237) — mapean a sus variantes.
        assert_eq!(parse_border_line_style("groove"), Some(BorderLineStyle::Groove));
        assert_eq!(parse_border_line_style("RIDGE"), Some(BorderLineStyle::Ridge));
        assert_eq!(parse_border_line_style("inset"), Some(BorderLineStyle::Inset));
        assert_eq!(parse_border_line_style("outset"), Some(BorderLineStyle::Outset));
        assert_eq!(parse_border_line_style("zigzag"), None);

        let html = r##"<html><head><style>
            div.sh { border: 2px dashed red }
            div.ls { border-width: 3px; border-color: blue; border-style: dotted }
            div.db { border: 4px double green }
            div.none { border: 1px solid black; border-style: none }
            div.plain { border: 1px solid black }
        </style></head><body>
            <div class="sh"></div><div class="ls"></div><div class="db"></div>
            <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 5);
        // Shorthand `border: 2px dashed red`.
        let sh = eng.compute(&divs[0]);
        assert_eq!(sh.border_style, BorderLineStyle::Dashed);
        assert_eq!(sh.border_widths.top, 2.0);
        // Longhand `border-style: dotted` (sobre width/color sueltos).
        assert_eq!(eng.compute(&divs[1]).border_style, BorderLineStyle::Dotted);
        // `double`.
        assert_eq!(eng.compute(&divs[2]).border_style, BorderLineStyle::Double);
        // `border-style: none` desactiva el border (width→0) — el patrón
        // queda como estaba (Solid) pero no se pinta.
        let nb = eng.compute(&divs[3]);
        assert_eq!(nb.border_widths.top, 0.0);
        // Sin estilo explícito → Solid default.
        assert_eq!(eng.compute(&divs[4]).border_style, BorderLineStyle::Solid);
    }

    #[test]
    fn border_style_3d_fase_7_237() {
        // Los 4 estilos 3D llegan a `ComputedStyle.border_style` por
        // shorthand y longhand. El render por par de lados se prueba
        // visualmente — acá sólo el mapeo.
        let html = r##"<html><head><style>
            div.gr { border: 4px groove #888 }
            div.rg { border: 4px ridge #888 }
            div.ins { border: 4px inset #888 }
            div.out { border: 4px outset #888 }
            div.lh { border: 4px solid #888; border-style: groove }
        </style></head><body>
            <div class="gr"></div><div class="rg"></div>
            <div class="ins"></div><div class="out"></div>
            <div class="lh"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 5);
        assert_eq!(eng.compute(&divs[0]).border_style, BorderLineStyle::Groove);
        assert_eq!(eng.compute(&divs[1]).border_style, BorderLineStyle::Ridge);
        assert_eq!(eng.compute(&divs[2]).border_style, BorderLineStyle::Inset);
        assert_eq!(eng.compute(&divs[3]).border_style, BorderLineStyle::Outset);
        // El longhand `border-style: groove` pisa el `solid` del
        // shorthand previo.
        assert_eq!(eng.compute(&divs[4]).border_style, BorderLineStyle::Groove);
        // Y el width sobrevive (border-style: groove no apaga el border).
        assert_eq!(eng.compute(&divs[4]).border_widths.top, 4.0);
    }

    #[test]
    fn text_decoration_thickness_y_underline_offset() {
        let html = r##"<html><head><style>
            p.t { text-decoration: underline; text-decoration-thickness: 3px }
            p.o { text-decoration: underline; text-underline-offset: 2px }
            p.auto { text-decoration: underline; text-decoration-thickness: auto;
                     text-underline-offset: auto }
            p.ff { text-decoration-thickness: from-font }
            p.plain { text-decoration: underline }
        </style></head><body>
            <p class="t">a</p><p class="o">b</p><p class="auto">c</p>
            <p class="ff">d</p><p class="plain">e</p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(ps.len(), 5);
        assert_eq!(eng.compute(&ps[0]).text_decoration_thickness, Some(3.0));
        assert_eq!(eng.compute(&ps[1]).text_underline_offset, Some(2.0));
        // `auto` explícito → None (default derivado).
        let a = eng.compute(&ps[2]);
        assert_eq!(a.text_decoration_thickness, None);
        assert_eq!(a.text_underline_offset, None);
        // `from-font` → None (igual que auto en nuestro modelo).
        assert_eq!(eng.compute(&ps[3]).text_decoration_thickness, None);
        // Sin declarar → None ambos.
        let plain = eng.compute(&ps[4]);
        assert_eq!(plain.text_decoration_thickness, None);
        assert_eq!(plain.text_underline_offset, None);
    }

    #[test]
    fn font_size_acepta_calc_y_clamp() {
        // Tipografía fluida: font-size con funciones matemáticas de
        // unidades absolutas resuelve en parse-time.
        let html = r#"<html><head><style>
            .a{font-size:calc(10px + 6px)}
            .b{font-size:clamp(1rem, 2rem, 3rem)}
            .c{font-size:min(30px, 20px)}
        </style></head><body>
            <p class="a">a</p><p class="b">b</p><p class="c">c</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).font_size, 16.0); // 10+6
        assert_eq!(eng.compute(&ps[1]).font_size, 32.0); // 2rem = 32px
        assert_eq!(eng.compute(&ps[2]).font_size, 20.0); // min
    }

    #[test]
    fn font_shorthand_expande_longhands() {
        // `font:` shorthand reparte style/weight/size/line-height/family.
        let html = r#"<html><head><style>
            .a{font:italic bold 20px/1.5 "Helvetica", sans-serif}
            .b{font:16px serif}
            .c{font:300 2rem monospace}
            .d{font:caption}
        </style></head><body>
            <p class="a">a</p><p class="b">b</p>
            <p class="c">c</p><p class="d">d</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        // .a — todos los ejes presentes.
        let a = eng.compute(&ps[0]);
        assert_eq!(a.font_style, FontStyle::Italic);
        assert_eq!(a.font_weight, 700);
        assert_eq!(a.font_size, 20.0);
        assert!((a.line_height.unwrap() - 1.5).abs() < 1e-6);
        assert_eq!(a.font_family.as_deref(), Some(r#""Helvetica", sans-serif"#));
        // .b — sólo size + family; el resto queda en defaults heredados.
        let b = eng.compute(&ps[1]);
        assert_eq!(b.font_size, 16.0);
        assert_eq!(b.font_style, FontStyle::Normal);
        assert_eq!(b.font_family.as_deref(), Some("serif"));
        // .c — weight numérico + rem.
        let c = eng.compute(&ps[2]);
        assert_eq!(c.font_weight, 300);
        assert_eq!(c.font_size, 32.0);
        assert_eq!(c.font_family.as_deref(), Some("monospace"));
        // .d — fuente de sistema (`font: caption`): Fase 7.863 aplica el
        // tamaño de una fuente de UI estándar (13px), ya no se descarta.
        assert_eq!(eng.compute(&ps[3]).font_size, 13.0);
    }

    #[test]
    fn css_wide_keywords_inherit_initial_unset() {
        let html = r#"<html><head><style>
            .bg{background-color:inherit}
            .initc{color:initial}
            .unsbg{background-color:unset}
            .unsc{color:unset}
            .dispinh{display:inherit}
        </style></head><body>
            <div style="color:red; background-color:blue; display:block">
                <span class="bg">a</span><span class="initc">b</span>
                <span class="unsbg">c</span><span class="unsc">d</span>
                <span class="dispinh">e</span>
            </div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut div = None;
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("div") => div = Some(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        let parent = eng.compute(div.as_ref().unwrap());
        assert_eq!(parent.color, Color::rgb(255, 0, 0));
        assert_eq!(parent.background, Some(Color::rgb(0, 0, 255)));
        let c = |i: usize| eng.compute_with_parent(&spans[i], Some(&parent));
        // background-color: inherit fuerza herencia de una prop NO heredable.
        assert_eq!(c(0).background, Some(Color::rgb(0, 0, 255)));
        // color: initial resetea al default (negro), ignorando la herencia.
        assert_eq!(c(1).color, Color::BLACK);
        // background-color: unset = initial (no heredable) → None.
        assert_eq!(c(2).background, None);
        // color: unset = inherit (heredable) → rojo del padre.
        assert_eq!(c(3).color, Color::rgb(255, 0, 0));
        // display: inherit toma el block del padre (un span sería inline).
        assert_eq!(c(4).display, Display::Block);
    }

