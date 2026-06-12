//! Tests del motor de estilo (grupo 03, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn column_span_fase_7_282() {
        assert_eq!(parse_column_span("none"), Some(ColumnSpan::None));
        assert_eq!(parse_column_span("ALL"), Some(ColumnSpan::All));
        assert_eq!(parse_column_span("partial"), None);

        let html = r##"<html><head><style>
            body { column-span: all }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.column_span, ColumnSpan::All);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_span,
            ColumnSpan::None
        );
    }

    #[test]
    fn break_inside_fase_7_283() {
        assert_eq!(parse_break_inside("auto"), Some(BreakInside::Auto));
        assert_eq!(parse_break_inside("avoid"), Some(BreakInside::Avoid));
        assert_eq!(parse_break_inside("AVOID-PAGE"), Some(BreakInside::AvoidPage));
        assert_eq!(parse_break_inside("avoid-column"), Some(BreakInside::AvoidColumn));
        assert_eq!(parse_break_inside("avoid-region"), Some(BreakInside::AvoidRegion));
        assert_eq!(parse_break_inside("nope"), None);

        // Alias legacy `page-break-inside`.
        let html = r##"<html><head><style>
            body { break-inside: avoid }
            div.legacy { page-break-inside: avoid }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.break_inside, BreakInside::Avoid);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).break_inside,
            BreakInside::Avoid
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).break_inside,
            BreakInside::Auto
        );
    }

    #[test]
    fn table_layout_fase_7_284() {
        assert_eq!(parse_table_layout("auto"), Some(TableLayout::Auto));
        assert_eq!(parse_table_layout("FIXED"), Some(TableLayout::Fixed));
        assert_eq!(parse_table_layout("nope"), None);

        let html = r##"<html><head><style>
            body { table-layout: fixed }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.table_layout, TableLayout::Fixed);
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).table_layout,
            TableLayout::Auto
        );
    }

    #[test]
    fn border_collapse_fase_7_285() {
        assert_eq!(parse_border_collapse("separate"), Some(BorderCollapse::Separate));
        assert_eq!(parse_border_collapse("COLLAPSE"), Some(BorderCollapse::Collapse));
        assert_eq!(parse_border_collapse("merge"), None);

        let html = r##"<html><head><style>
            body { border-collapse: collapse }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.border_collapse, BorderCollapse::Collapse);
        // SÍ se hereda → div sin propio gana el del padre.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).border_collapse,
            BorderCollapse::Collapse
        );
    }

    #[test]
    fn border_spacing_fase_7_286() {
        assert_eq!(parse_border_spacing("5px"), Some((5.0, 5.0)));
        assert_eq!(parse_border_spacing("5px 10px"), Some((5.0, 10.0)));
        assert!(parse_border_spacing("5px 10px 15px").is_none());

        let html = r##"<html><head><style>
            body { border-spacing: 3px 7px }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.border_spacing_h, 3.0);
        assert_eq!(body_cs.border_spacing_v, 7.0);
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(plain.border_spacing_h, 3.0);
        assert_eq!(plain.border_spacing_v, 7.0);
    }

    #[test]
    fn caption_side_fase_7_287() {
        assert_eq!(parse_caption_side("top"), Some(CaptionSide::Top));
        assert_eq!(parse_caption_side("BOTTOM"), Some(CaptionSide::Bottom));
        // Logicals se aplastan.
        assert_eq!(parse_caption_side("block-start"), Some(CaptionSide::Top));
        assert_eq!(parse_caption_side("inline-end"), Some(CaptionSide::Bottom));
        assert_eq!(parse_caption_side("middle"), None);

        let html = r##"<html><head><style>
            body { caption-side: bottom }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.caption_side, CaptionSide::Bottom);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).caption_side,
            CaptionSide::Bottom
        );
    }

    #[test]
    fn empty_cells_fase_7_288() {
        assert_eq!(parse_empty_cells("show"), Some(EmptyCells::Show));
        assert_eq!(parse_empty_cells("HIDE"), Some(EmptyCells::Hide));
        assert_eq!(parse_empty_cells("nope"), None);

        let html = r##"<html><head><style>
            body { empty-cells: hide }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.empty_cells, EmptyCells::Hide);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).empty_cells,
            EmptyCells::Hide
        );
    }

    #[test]
    fn break_before_fase_7_289() {
        assert_eq!(parse_break_between("auto"), Some(BreakBetween::Auto));
        assert_eq!(parse_break_between("AVOID-PAGE"), Some(BreakBetween::AvoidPage));
        assert_eq!(parse_break_between("page"), Some(BreakBetween::Page));
        assert_eq!(parse_break_between("recto"), Some(BreakBetween::Recto));
        assert_eq!(parse_break_between("column"), Some(BreakBetween::Column));
        assert_eq!(parse_break_between("avoid-region"), Some(BreakBetween::AvoidRegion));
        assert_eq!(parse_break_between("nope"), None);

        // Alias legacy `page-break-before`.
        let html = r##"<html><head><style>
            body { break-before: page }
            div.legacy { page-break-before: always }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.break_before, BreakBetween::Page);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).break_before,
            BreakBetween::Always
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).break_before,
            BreakBetween::Auto
        );
    }

    #[test]
    fn break_after_fase_7_290() {
        // Mismo parser, comparte el dominio con break-before.
        let html = r##"<html><head><style>
            body { break-after: avoid-column }
            div.legacy { page-break-after: left }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.break_after, BreakBetween::AvoidColumn);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).break_after,
            BreakBetween::Left
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).break_after,
            BreakBetween::Auto
        );
    }

    #[test]
    fn orphans_fase_7_291() {
        assert_eq!(parse_positive_int("3"), Some(3));
        assert_eq!(parse_positive_int("0"), None);
        assert_eq!(parse_positive_int("-1"), None);
        assert_eq!(parse_positive_int("nope"), None);

        let html = r##"<html><head><style>
            body { orphans: 4 }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.orphans, 4);
        // SÍ se hereda.
        assert_eq!(eng.compute_with_parent(&divs[0], Some(&body_cs)).orphans, 4);
    }

    #[test]
    fn widows_fase_7_292() {
        let html = r##"<html><head><style>
            body { widows: 5 }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.widows, 5);
        // SÍ se hereda.
        assert_eq!(eng.compute_with_parent(&divs[0], Some(&body_cs)).widows, 5);
    }

    #[test]
    fn color_scheme_fase_7_293() {
        assert_eq!(parse_color_scheme("normal"), Some(ColorScheme::NORMAL));
        assert_eq!(
            parse_color_scheme("light dark"),
            Some(ColorScheme { light: true, dark: true, only: false })
        );
        assert_eq!(
            parse_color_scheme("only LIGHT"),
            Some(ColorScheme { light: true, dark: false, only: true })
        );
        // Duplicado descarta.
        assert!(parse_color_scheme("light light").is_none());
        // Token desconocido descarta.
        assert!(parse_color_scheme("light sepia").is_none());
        // `only` solo no es válido.
        assert!(parse_color_scheme("only").is_none());

        let html = r##"<html><head><style>
            body { color-scheme: light dark }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(body_cs.color_scheme.light && body_cs.color_scheme.dark);
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(plain.color_scheme.light && plain.color_scheme.dark);
    }

    #[test]
    fn list_style_position_fase_7_294() {
        assert_eq!(parse_list_style_position("outside"), Some(ListStylePosition::Outside));
        assert_eq!(parse_list_style_position("INSIDE"), Some(ListStylePosition::Inside));
        assert_eq!(parse_list_style_position("middle"), None);

        let html = r##"<html><head><style>
            body { list-style-position: inside }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.list_style_position, ListStylePosition::Inside);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).list_style_position,
            ListStylePosition::Inside
        );
    }

    #[test]
    fn list_style_image_fase_7_295() {
        assert_eq!(parse_list_style_image("none"), None);
        assert_eq!(parse_list_style_image(""), None);
        assert_eq!(
            parse_list_style_image("url(bullet.png)"),
            Some("bullet.png".to_string())
        );
        assert_eq!(
            parse_list_style_image("url(\"b.svg\")"),
            Some("b.svg".to_string())
        );
        // Subset: no aceptamos gradients ni image() todavía.
        assert_eq!(parse_list_style_image("linear-gradient(red, blue)"), None);

        let html = r##"<html><head><style>
            body { list-style-image: url(bullet.png) }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.list_style_image, Some("bullet.png".to_string()));
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).list_style_image,
            Some("bullet.png".to_string())
        );
    }

    #[test]
    fn list_style_shorthand_fase_7_296() {
        // Shorthand cubre los 3 longhands en orden libre.
        let html = r##"<html><head><style>
            body { list-style: square inside url(b.png) }
            div.none { list-style: none }
            div.plain {}
        </style></head><body>
          <div class="none"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.list_style_type, ListStyleType::Square);
        assert_eq!(body_cs.list_style_position, ListStylePosition::Inside);
        assert_eq!(body_cs.list_style_image, Some("b.png".to_string()));
        // `list-style: none` apaga type + image.
        let none = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(none.list_style_type, ListStyleType::None);
        assert_eq!(none.list_style_image, None);
        // position no se toca con `none` — sigue heredando del padre.
        assert_eq!(none.list_style_position, ListStylePosition::Inside);
    }

    #[test]
    fn counter_set_fase_7_297() {
        // Default = 0 (a diferencia de counter-increment cuyo default es 1).
        let html = r##"<html><head><style>
            body { counter-set: page 1 chapter }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.counter_set.len(), 2);
        assert_eq!(body_cs.counter_set[0], ("page".to_string(), 1));
        assert_eq!(body_cs.counter_set[1], ("chapter".to_string(), 0));
        // NO se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(plain.counter_set.is_empty());
    }

    #[test]
    fn quotes_fase_7_298() {
        assert_eq!(parse_quotes("auto"), Quotes::Auto);
        assert_eq!(parse_quotes("none"), Quotes::None);
        let q = parse_quotes(r#""«" "»" "‹" "›""#);
        assert_eq!(
            q,
            Quotes::Pairs(vec![
                ("«".to_string(), "»".to_string()),
                ("‹".to_string(), "›".to_string()),
            ])
        );
        // Impares (sin cierre) → cae a Auto.
        assert_eq!(parse_quotes(r#""«" "»" "‹""#), Quotes::Auto);
        // Sin comillas → cae a Auto.
        assert_eq!(parse_quotes("foo bar"), Quotes::Auto);

        let html = r##"<html><head><style>
            body { quotes: "“" "”" "‘" "’" }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        if let Quotes::Pairs(pairs) = &body_cs.quotes {
            assert_eq!(pairs.len(), 2);
            assert_eq!(pairs[0], ("“".to_string(), "”".to_string()));
            assert_eq!(pairs[1], ("‘".to_string(), "’".to_string()));
        } else {
            panic!("esperaba Pairs, vino {:?}", body_cs.quotes);
        }
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(matches!(plain.quotes, Quotes::Pairs(_)));
    }

    #[test]
    fn text_underline_position_fase_7_299() {
        assert_eq!(parse_text_underline_position("auto"), Some(TextUnderlinePosition::Auto));
        assert_eq!(
            parse_text_underline_position("FROM-FONT"),
            Some(TextUnderlinePosition::FromFont)
        );
        assert_eq!(parse_text_underline_position("under"), Some(TextUnderlinePosition::Under));
        assert_eq!(parse_text_underline_position("left"), Some(TextUnderlinePosition::Left));
        assert_eq!(parse_text_underline_position("right"), Some(TextUnderlinePosition::Right));
        assert_eq!(parse_text_underline_position("middle"), None);

        let html = r##"<html><head><style>
            body { text-underline-position: under }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_underline_position, TextUnderlinePosition::Under);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_underline_position,
            TextUnderlinePosition::Under
        );
    }

    #[test]
    fn text_justify_fase_7_300() {
        assert_eq!(parse_text_justify("auto"), Some(TextJustify::Auto));
        assert_eq!(parse_text_justify("INTER-WORD"), Some(TextJustify::InterWord));
        assert_eq!(
            parse_text_justify("inter-character"),
            Some(TextJustify::InterCharacter)
        );
        assert_eq!(parse_text_justify("distribute"), Some(TextJustify::Distribute));
        assert_eq!(parse_text_justify("nope"), None);

        let html = r##"<html><head><style>
            body { text-justify: inter-word }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_justify, TextJustify::InterWord);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_justify,
            TextJustify::InterWord
        );
    }

    #[test]
    fn print_color_adjust_fase_7_301() {
        assert_eq!(
            parse_print_color_adjust("economy"),
            Some(PrintColorAdjust::Economy)
        );
        assert_eq!(parse_print_color_adjust("EXACT"), Some(PrintColorAdjust::Exact));
        assert_eq!(parse_print_color_adjust("nope"), None);

        // Alias legacy `color-adjust` debería rutear igual.
        let html = r##"<html><head><style>
            body { print-color-adjust: exact }
            div.legacy { color-adjust: economy }
            div.plain {}
        </style></head><body>
          <div class="legacy"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.print_color_adjust, PrintColorAdjust::Exact);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).print_color_adjust,
            PrintColorAdjust::Economy
        );
        // SÍ se hereda → div.plain hereda Exact del body.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).print_color_adjust,
            PrintColorAdjust::Exact
        );
    }

    #[test]
    fn forced_color_adjust_fase_7_302() {
        assert_eq!(parse_forced_color_adjust("auto"), Some(ForcedColorAdjust::Auto));
        assert_eq!(parse_forced_color_adjust("NONE"), Some(ForcedColorAdjust::None));
        assert_eq!(
            parse_forced_color_adjust("preserve"),
            Some(ForcedColorAdjust::Preserve)
        );
        assert_eq!(parse_forced_color_adjust("nope"), None);

        let html = r##"<html><head><style>
            body { forced-color-adjust: none }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.forced_color_adjust, ForcedColorAdjust::None);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).forced_color_adjust,
            ForcedColorAdjust::None
        );
    }

    #[test]
    fn line_clamp_fase_7_303() {
        assert_eq!(parse_line_clamp("none"), None);
        assert_eq!(parse_line_clamp("3"), Some(3));
        assert_eq!(parse_line_clamp("0"), None);
        assert_eq!(parse_line_clamp("nope"), None);

        let html = r##"<html><head><style>
            body { -webkit-line-clamp: 2 }
            div.std { line-clamp: 5 }
            div.plain {}
        </style></head><body>
          <div class="std"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.line_clamp, Some(2));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).line_clamp,
            Some(5)
        );
        // NO se hereda → default None.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).line_clamp,
            None
        );
    }

    #[test]
    fn font_variant_caps_fase_7_304() {
        assert_eq!(parse_font_variant_caps("normal"), Some(FontVariantCaps::Normal));
        assert_eq!(parse_font_variant_caps("SMALL-CAPS"), Some(FontVariantCaps::SmallCaps));
        assert_eq!(
            parse_font_variant_caps("titling-caps"),
            Some(FontVariantCaps::TitlingCaps)
        );
        assert_eq!(parse_font_variant_caps("nope"), None);

        let html = r##"<html><head><style>
            body { font-variant-caps: small-caps }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_variant_caps, FontVariantCaps::SmallCaps);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).font_variant_caps,
            FontVariantCaps::SmallCaps
        );
    }

    #[test]
    fn font_variant_numeric_fase_7_305() {
        assert_eq!(
            parse_font_variant_numeric("normal"),
            Some(FontVariantNumeric::default())
        );
        let n = parse_font_variant_numeric("tabular-nums lining-nums").unwrap();
        assert!(n.tabular_nums && n.lining_nums);
        assert!(!n.proportional_nums && !n.oldstyle_nums);
        // Mutuamente excluyentes.
        assert!(parse_font_variant_numeric("lining-nums oldstyle-nums").is_none());
        assert!(parse_font_variant_numeric("tabular-nums proportional-nums").is_none());
        assert!(parse_font_variant_numeric("diagonal-fractions stacked-fractions").is_none());
        // Token desconocido descarta.
        assert!(parse_font_variant_numeric("tabular-nums bogus").is_none());

        let html = r##"<html><head><style>
            body { font-variant-numeric: tabular-nums slashed-zero }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(body_cs.font_variant_numeric.tabular_nums);
        assert!(body_cs.font_variant_numeric.slashed_zero);
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(plain.font_variant_numeric.tabular_nums);
        assert!(plain.font_variant_numeric.slashed_zero);
    }

    #[test]
    fn font_variant_ligatures_fase_7_306() {
        assert_eq!(
            parse_font_variant_ligatures("normal"),
            Some(FontVariantLigatures::Normal)
        );
        assert_eq!(
            parse_font_variant_ligatures("NONE"),
            Some(FontVariantLigatures::None)
        );
        if let Some(FontVariantLigatures::Custom(l)) =
            parse_font_variant_ligatures("common-ligatures discretionary-ligatures contextual")
        {
            assert!(l.common_ligatures && l.discretionary_ligatures && l.contextual);
            assert!(!l.no_common_ligatures);
        } else {
            panic!("esperaba Custom");
        }
        // on/off del mismo grupo es inválido.
        assert!(parse_font_variant_ligatures("common-ligatures no-common-ligatures").is_none());
        // Token desconocido descarta.
        assert!(parse_font_variant_ligatures("common-ligatures bogus").is_none());

        let html = r##"<html><head><style>
            body { font-variant-ligatures: discretionary-ligatures }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(matches!(body_cs.font_variant_ligatures, FontVariantLigatures::Custom(_)));
        // SÍ se hereda.
        assert!(matches!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).font_variant_ligatures,
            FontVariantLigatures::Custom(_)
        ));
    }

    #[test]
    fn font_variant_east_asian_fase_7_307() {
        assert_eq!(
            parse_font_variant_east_asian("normal"),
            Some(FontVariantEastAsian::default())
        );
        let e = parse_font_variant_east_asian("jis90 ruby full-width").unwrap();
        assert!(e.jis90 && e.ruby && e.full_width);
        // 2 JIS forms simultaneous = inválido.
        assert!(parse_font_variant_east_asian("jis78 jis83").is_none());
        // full-width + proportional-width = inválido.
        assert!(
            parse_font_variant_east_asian("full-width proportional-width").is_none()
        );
        // Token desconocido descarta.
        assert!(parse_font_variant_east_asian("ruby bogus").is_none());

        let html = r##"<html><head><style>
            body { font-variant-east-asian: simplified ruby }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(body_cs.font_variant_east_asian.simplified);
        assert!(body_cs.font_variant_east_asian.ruby);
        // SÍ se hereda.
        let plain = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(plain.font_variant_east_asian.simplified && plain.font_variant_east_asian.ruby);
    }

    #[test]
    fn font_variant_position_fase_7_308() {
        assert_eq!(
            parse_font_variant_position("normal"),
            Some(FontVariantPosition::Normal)
        );
        assert_eq!(parse_font_variant_position("SUB"), Some(FontVariantPosition::Sub));
        assert_eq!(parse_font_variant_position("super"), Some(FontVariantPosition::Super));
        assert_eq!(parse_font_variant_position("nope"), None);

        let html = r##"<html><head><style>
            body { font-variant-position: sub }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_variant_position, FontVariantPosition::Sub);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).font_variant_position,
            FontVariantPosition::Sub
        );
    }

    #[test]
    fn text_emphasis_style_fase_7_309() {
        assert_eq!(parse_text_emphasis_style("none"), Some(TextEmphasisStyle::None));
        assert_eq!(
            parse_text_emphasis_style("filled circle"),
            Some(TextEmphasisStyle::Mark {
                fill: TextEmphasisFill::Filled,
                shape: TextEmphasisShape::Circle,
            })
        );
        // Sólo shape → fill default Filled.
        assert_eq!(
            parse_text_emphasis_style("triangle"),
            Some(TextEmphasisStyle::Mark {
                fill: TextEmphasisFill::Filled,
                shape: TextEmphasisShape::Triangle,
            })
        );
        // Sólo fill → shape default Dot.
        assert_eq!(
            parse_text_emphasis_style("open"),
            Some(TextEmphasisStyle::Mark {
                fill: TextEmphasisFill::Open,
                shape: TextEmphasisShape::Dot,
            })
        );
        // String literal.
        assert_eq!(
            parse_text_emphasis_style(r#""★""#),
            Some(TextEmphasisStyle::Custom("★".to_string()))
        );
        // Duplicado y desconocido descartan.
        assert!(parse_text_emphasis_style("filled open").is_none());
        assert!(parse_text_emphasis_style("circle dot").is_none());
        assert!(parse_text_emphasis_style("nope").is_none());

        let html = r##"<html><head><style>
            body { text-emphasis-style: open circle }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.text_emphasis_style,
            TextEmphasisStyle::Mark {
                fill: TextEmphasisFill::Open,
                shape: TextEmphasisShape::Circle,
            }
        );
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_emphasis_style,
            body_cs.text_emphasis_style.clone()
        );
    }

    #[test]
    fn text_emphasis_color_fase_7_310() {
        let html = r##"<html><head><style>
            body { text-emphasis-color: rgb(0,128,0) }
            div.cc { text-emphasis-color: currentColor }
            div.plain {}
        </style></head><body>
          <div class="cc"></div><div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.text_emphasis_color.map(|c| (c.r, c.g, c.b)),
            Some((0, 128, 0))
        );
        // `currentColor` → None.
        let cc = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(cc.text_emphasis_color, None);
        // SÍ se hereda → div.plain hereda el verde del body.
        let plain = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(plain.text_emphasis_color.map(|c| (c.r, c.g, c.b)), Some((0, 128, 0)));
    }

    #[test]
    fn text_emphasis_position_fase_7_311() {
        assert_eq!(
            parse_text_emphasis_position("over right"),
            Some(TextEmphasisPosition { over: true, right: true })
        );
        // Orden libre.
        assert_eq!(
            parse_text_emphasis_position("LEFT under"),
            Some(TextEmphasisPosition { over: false, right: false })
        );
        // Solo un token → el otro queda en default.
        assert_eq!(
            parse_text_emphasis_position("under"),
            Some(TextEmphasisPosition { over: false, right: true })
        );
        // Duplicado o desconocido descartan.
        assert!(parse_text_emphasis_position("over over").is_none());
        assert!(parse_text_emphasis_position("middle").is_none());

        let html = r##"<html><head><style>
            body { text-emphasis-position: under left }
            div.plain {}
        </style></head><body><div class="plain"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("div") => divs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.text_emphasis_position,
            TextEmphasisPosition { over: false, right: false }
        );
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_emphasis_position,
            body_cs.text_emphasis_position
        );
    }

