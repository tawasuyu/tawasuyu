//! Tests del motor de estilo (grupo 05, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn view_timeline_name_fase_7_342() {
        let html = r##"<html><head><style>
            body { view-timeline-name: --section }
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
        assert_eq!(cs.view_timeline_name, Some("--section".to_string()));
    }

    #[test]
    fn view_timeline_axis_fase_7_343() {
        let html = r##"<html><head><style>
            body { view-timeline-axis: y }
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
        assert_eq!(body_cs.view_timeline_axis, TimelineAxis::Y);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).view_timeline_axis,
            TimelineAxis::Block
        );
    }

    #[test]
    fn white_space_collapse_fase_7_344() {
        assert_eq!(
            parse_white_space_collapse("collapse"),
            Some(WhiteSpaceCollapse::Collapse)
        );
        assert_eq!(
            parse_white_space_collapse("PRESERVE"),
            Some(WhiteSpaceCollapse::Preserve)
        );
        assert_eq!(
            parse_white_space_collapse("preserve-breaks"),
            Some(WhiteSpaceCollapse::PreserveBreaks)
        );
        assert_eq!(
            parse_white_space_collapse("break-spaces"),
            Some(WhiteSpaceCollapse::BreakSpaces)
        );
        assert_eq!(parse_white_space_collapse("nope"), None);

        let html = r##"<html><head><style>
            body { white-space-collapse: preserve }
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
        assert_eq!(body_cs.white_space_collapse, WhiteSpaceCollapse::Preserve);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).white_space_collapse,
            WhiteSpaceCollapse::Preserve
        );
    }

    #[test]
    fn text_wrap_mode_fase_7_345() {
        assert_eq!(parse_text_wrap_mode("wrap"), Some(TextWrapMode::Wrap));
        assert_eq!(parse_text_wrap_mode("NOWRAP"), Some(TextWrapMode::Nowrap));
        assert_eq!(parse_text_wrap_mode("nope"), None);

        let html = r##"<html><head><style>
            body { text-wrap-mode: nowrap }
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
        assert_eq!(body_cs.text_wrap_mode, TextWrapMode::Nowrap);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_wrap_mode,
            TextWrapMode::Nowrap
        );
    }

    #[test]
    fn text_wrap_style_fase_7_346() {
        assert_eq!(parse_text_wrap_style("auto"), Some(TextWrapStyle::Auto));
        assert_eq!(parse_text_wrap_style("BALANCE"), Some(TextWrapStyle::Balance));
        assert_eq!(parse_text_wrap_style("pretty"), Some(TextWrapStyle::Pretty));
        assert_eq!(parse_text_wrap_style("stable"), Some(TextWrapStyle::Stable));
        assert_eq!(parse_text_wrap_style("nope"), None);

        let html = r##"<html><head><style>
            body { text-wrap-style: pretty }
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
        assert_eq!(body_cs.text_wrap_style, TextWrapStyle::Pretty);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_wrap_style,
            TextWrapStyle::Pretty
        );
    }

    #[test]
    fn text_spacing_trim_fase_7_347() {
        assert_eq!(
            parse_text_spacing_trim("normal"),
            Some(TextSpacingTrim::Normal)
        );
        assert_eq!(
            parse_text_spacing_trim("SPACE-ALL"),
            Some(TextSpacingTrim::SpaceAll)
        );
        assert_eq!(
            parse_text_spacing_trim("space-first"),
            Some(TextSpacingTrim::SpaceFirst)
        );
        assert_eq!(
            parse_text_spacing_trim("trim-start"),
            Some(TextSpacingTrim::TrimStart)
        );
        assert_eq!(parse_text_spacing_trim("nope"), None);

        let html = r##"<html><head><style>
            body { text-spacing-trim: trim-start }
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
        assert_eq!(body_cs.text_spacing_trim, TextSpacingTrim::TrimStart);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_spacing_trim,
            TextSpacingTrim::TrimStart
        );
    }

    #[test]
    fn text_box_trim_fase_7_348() {
        assert_eq!(parse_text_box_trim("none"), Some(TextBoxTrim::None));
        assert_eq!(parse_text_box_trim("TRIM-START"), Some(TextBoxTrim::TrimStart));
        assert_eq!(parse_text_box_trim("trim-end"), Some(TextBoxTrim::TrimEnd));
        assert_eq!(parse_text_box_trim("trim-both"), Some(TextBoxTrim::TrimBoth));
        assert_eq!(parse_text_box_trim("nope"), None);

        let html = r##"<html><head><style>
            body { text-box-trim: trim-both }
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
        assert_eq!(body_cs.text_box_trim, TextBoxTrim::TrimBoth);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_box_trim,
            TextBoxTrim::TrimBoth
        );
    }

    #[test]
    fn math_style_fase_7_349() {
        assert_eq!(parse_math_style("normal"), Some(MathStyle::Normal));
        assert_eq!(parse_math_style("COMPACT"), Some(MathStyle::Compact));
        assert_eq!(parse_math_style("nope"), None);

        let html = r##"<html><head><style>
            body { math-style: compact }
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
        assert_eq!(body_cs.math_style, MathStyle::Compact);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).math_style,
            MathStyle::Compact
        );
    }

    #[test]
    fn math_depth_fase_7_350() {
        assert_eq!(parse_math_depth("auto-add"), Some(MathDepth::Auto));
        assert_eq!(parse_math_depth("3"), Some(MathDepth::Value(3)));
        assert_eq!(parse_math_depth("-1"), Some(MathDepth::Value(-1)));
        assert_eq!(parse_math_depth("add(2)"), Some(MathDepth::Add(2)));
        assert_eq!(parse_math_depth("ADD(-3)"), Some(MathDepth::Add(-3)));
        assert_eq!(parse_math_depth("nope"), None);
        assert_eq!(parse_math_depth("add(foo)"), None);

        let html = r##"<html><head><style>
            body { math-depth: 2 }
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
        assert_eq!(cs.math_depth, MathDepth::Value(2));
    }

    #[test]
    fn math_shift_fase_7_351() {
        assert_eq!(parse_math_shift("normal"), Some(MathShift::Normal));
        assert_eq!(parse_math_shift("COMPACT"), Some(MathShift::Compact));
        assert_eq!(parse_math_shift("nope"), None);

        let html = r##"<html><head><style>
            body { math-shift: compact }
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
        assert_eq!(body_cs.math_shift, MathShift::Compact);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).math_shift,
            MathShift::Compact
        );
    }

    #[test]
    fn field_sizing_fase_7_352() {
        assert_eq!(parse_field_sizing("fixed"), Some(FieldSizing::Fixed));
        assert_eq!(parse_field_sizing("CONTENT"), Some(FieldSizing::Content));
        assert_eq!(parse_field_sizing("nope"), None);

        let html = r##"<html><head><style>
            body { field-sizing: content }
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
        assert_eq!(body_cs.field_sizing, FieldSizing::Content);
        // NO hereda — vuelve a Fixed.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).field_sizing,
            FieldSizing::Fixed
        );
    }

    #[test]
    fn text_box_edge_fase_7_353() {
        assert_eq!(parse_text_box_edge("auto"), Some(TextBoxEdge::Auto));
        // 1 token → over=under=text.
        assert_eq!(
            parse_text_box_edge("text"),
            Some(TextBoxEdge::Edge {
                over: TextEdge::Text,
                under: TextEdge::Text
            })
        );
        // 2 tokens distintos.
        assert_eq!(
            parse_text_box_edge("cap alphabetic"),
            Some(TextBoxEdge::Edge {
                over: TextEdge::Cap,
                under: TextEdge::Alphabetic
            })
        );
        // 3 tokens descarta.
        assert_eq!(parse_text_box_edge("text ex cap"), None);
        // Token desconocido descarta.
        assert_eq!(parse_text_box_edge("nope"), None);

        let html = r##"<html><head><style>
            body { text-box-edge: cap alphabetic }
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
            body_cs.text_box_edge,
            TextBoxEdge::Edge {
                over: TextEdge::Cap,
                under: TextEdge::Alphabetic
            }
        );
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_box_edge,
            TextBoxEdge::Edge {
                over: TextEdge::Cap,
                under: TextEdge::Alphabetic
            }
        );
    }

    #[test]
    fn anchor_name_fase_7_354() {
        assert_eq!(parse_ident_list_or_none("none"), Some(Vec::new()));
        assert_eq!(
            parse_ident_list_or_none("--a"),
            Some(vec!["--a".to_string()])
        );
        assert_eq!(
            parse_ident_list_or_none("--a --b --c"),
            Some(vec!["--a".to_string(), "--b".to_string(), "--c".to_string()])
        );
        assert_eq!(parse_ident_list_or_none(""), None);

        let html = r##"<html><head><style>
            body { anchor-name: --tip }
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
        assert_eq!(body_cs.anchor_name, vec!["--tip".to_string()]);
        // NO hereda.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(div_cs.anchor_name.is_empty());
    }

    #[test]
    fn position_anchor_fase_7_355() {
        assert_eq!(parse_ident_or_auto("auto"), Some(None));
        assert_eq!(
            parse_ident_or_auto("--tip"),
            Some(Some("--tip".to_string()))
        );
        assert_eq!(parse_ident_or_auto(""), None);

        let html = r##"<html><head><style>
            body { position-anchor: --tip }
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
        assert_eq!(body_cs.position_anchor, Some("--tip".to_string()));
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).position_anchor,
            None
        );
    }

    #[test]
    fn anchor_scope_fase_7_356() {
        assert_eq!(parse_anchor_scope("none"), Some(AnchorScope::None));
        assert_eq!(parse_anchor_scope("ALL"), Some(AnchorScope::All));
        assert_eq!(
            parse_anchor_scope("--a --b"),
            Some(AnchorScope::Names(vec![
                "--a".to_string(),
                "--b".to_string()
            ]))
        );
        assert_eq!(parse_anchor_scope(""), None);

        let html = r##"<html><head><style>
            body { anchor-scope: --tip }
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
            body_cs.anchor_scope,
            AnchorScope::Names(vec!["--tip".to_string()])
        );
        // SÍ hereda (CSS Anchor Positioning 1).
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).anchor_scope,
            AnchorScope::Names(vec!["--tip".to_string()])
        );
    }

    #[test]
    fn view_transition_name_fase_7_357() {
        let html = r##"<html><head><style>
            body { view-transition-name: hero }
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
        assert_eq!(body_cs.view_transition_name, Some("hero".to_string()));
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).view_transition_name,
            None
        );
    }

    #[test]
    fn view_transition_class_fase_7_358() {
        let html = r##"<html><head><style>
            body { view-transition-class: foo bar }
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
        assert_eq!(
            cs.view_transition_class,
            vec!["foo".to_string(), "bar".to_string()]
        );
    }

    #[test]
    fn font_palette_fase_7_359() {
        assert_eq!(parse_font_palette("normal"), Some(FontPalette::Normal));
        assert_eq!(parse_font_palette("LIGHT"), Some(FontPalette::Light));
        assert_eq!(parse_font_palette("dark"), Some(FontPalette::Dark));
        assert_eq!(
            parse_font_palette("--my-palette"),
            Some(FontPalette::Named("--my-palette".to_string()))
        );
        assert_eq!(parse_font_palette(""), None);

        let html = r##"<html><head><style>
            body { font-palette: --hi }
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
        assert_eq!(body_cs.font_palette, FontPalette::Named("--hi".to_string()));
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).font_palette,
            FontPalette::Named("--hi".to_string())
        );
    }

    #[test]
    fn font_variant_alternates_fase_7_360() {
        assert_eq!(
            parse_font_variant_alternates("normal"),
            Some(FontVariantAlternates::default())
        );
        // historical-forms solo.
        let hist = parse_font_variant_alternates("historical-forms").unwrap();
        assert!(hist.historical_forms);
        assert!(hist.functional.is_empty());
        // funcional stylistic(...).
        let s = parse_font_variant_alternates("stylistic(--swash)").unwrap();
        assert!(!s.historical_forms);
        assert_eq!(
            s.functional,
            vec![("stylistic".to_string(), "--swash".to_string())]
        );
        // combinado.
        let combo = parse_font_variant_alternates(
            "historical-forms stylistic(--a) styleset(--b)",
        )
        .unwrap();
        assert!(combo.historical_forms);
        assert_eq!(combo.functional.len(), 2);
        // duplicado historical-forms descarta.
        assert_eq!(
            parse_font_variant_alternates("historical-forms historical-forms"),
            None
        );
        // función desconocida descarta.
        assert_eq!(parse_font_variant_alternates("foo(--x)"), None);
        // función con paréntesis vacío descarta.
        assert_eq!(parse_font_variant_alternates("stylistic()"), None);

        let html = r##"<html><head><style>
            body { font-variant-alternates: historical-forms }
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
        assert!(body_cs.font_variant_alternates.historical_forms);
        // SÍ hereda.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(div_cs.font_variant_alternates.historical_forms);
    }

    #[test]
    fn columns_shorthand_fase_7_361() {
        // 2 auto.
        assert_eq!(
            parse_columns_shorthand("auto"),
            Some((LengthVal::Auto, None))
        );
        // length sola.
        assert_eq!(
            parse_columns_shorthand("200px"),
            Some((LengthVal::Px(200.0), None))
        );
        // integer solo.
        assert_eq!(
            parse_columns_shorthand("3"),
            Some((LengthVal::Auto, Some(3)))
        );
        // length + integer.
        assert_eq!(
            parse_columns_shorthand("200px 3"),
            Some((LengthVal::Px(200.0), Some(3)))
        );
        // orden libre.
        assert_eq!(
            parse_columns_shorthand("3 200px"),
            Some((LengthVal::Px(200.0), Some(3)))
        );
        // dos integers descarta.
        assert_eq!(parse_columns_shorthand("3 4"), None);
        // 0 columnas descarta.
        assert_eq!(parse_columns_shorthand("0"), None);

        let html = r##"<html><head><style>
            .grid { columns: 200px 3 }
        </style></head><body><div class="grid"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let cs = eng.compute(&divs[0]);
        assert_eq!(cs.column_width, LengthVal::Px(200.0));
        assert_eq!(cs.column_count, Some(3));
    }

    #[test]
    fn background_attachment_fase_7_362() {
        assert_eq!(
            parse_background_attachment("scroll"),
            Some(vec![BackgroundAttachment::Scroll])
        );
        assert_eq!(
            parse_background_attachment("FIXED"),
            Some(vec![BackgroundAttachment::Fixed])
        );
        // Lista por coma.
        assert_eq!(
            parse_background_attachment("scroll, fixed, local"),
            Some(vec![
                BackgroundAttachment::Scroll,
                BackgroundAttachment::Fixed,
                BackgroundAttachment::Local,
            ])
        );
        assert_eq!(parse_background_attachment("nope"), None);

        let html = r##"<html><head><style>
            body { background-attachment: fixed }
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
        assert_eq!(body_cs.background_attachment, vec![BackgroundAttachment::Fixed]);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).background_attachment,
            vec![BackgroundAttachment::Scroll]
        );
    }

    #[test]
    fn caret_shape_fase_7_363() {
        assert_eq!(parse_caret_shape("auto"), Some(CaretShape::Auto));
        assert_eq!(parse_caret_shape("BAR"), Some(CaretShape::Bar));
        assert_eq!(parse_caret_shape("block"), Some(CaretShape::Block));
        assert_eq!(parse_caret_shape("underscore"), Some(CaretShape::Underscore));
        assert_eq!(parse_caret_shape("nope"), None);

        let html = r##"<html><head><style>
            body { caret-shape: block }
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
        assert_eq!(body_cs.caret_shape, CaretShape::Block);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).caret_shape,
            CaretShape::Block
        );
    }

    #[test]
    fn baseline_source_fase_7_364() {
        assert_eq!(parse_baseline_source("auto"), Some(BaselineSource::Auto));
        assert_eq!(parse_baseline_source("FIRST"), Some(BaselineSource::First));
        assert_eq!(parse_baseline_source("last"), Some(BaselineSource::Last));
        assert_eq!(parse_baseline_source("nope"), None);

        let html = r##"<html><head><style>
            body { baseline-source: last }
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
        assert_eq!(body_cs.baseline_source, BaselineSource::Last);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).baseline_source,
            BaselineSource::Auto
        );
    }

    #[test]
    fn alignment_baseline_fase_7_365() {
        assert_eq!(
            parse_alignment_baseline("baseline"),
            Some(AlignmentBaseline::Baseline)
        );
        assert_eq!(
            parse_alignment_baseline("TEXT-BOTTOM"),
            Some(AlignmentBaseline::TextBottom)
        );
        assert_eq!(
            parse_alignment_baseline("central"),
            Some(AlignmentBaseline::Central)
        );
        assert_eq!(
            parse_alignment_baseline("mathematical"),
            Some(AlignmentBaseline::Mathematical)
        );
        assert_eq!(parse_alignment_baseline("nope"), None);

        let html = r##"<html><head><style>
            body { alignment-baseline: central }
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
        assert_eq!(body_cs.alignment_baseline, AlignmentBaseline::Central);
        // NO hereda — vuelve a Baseline.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).alignment_baseline,
            AlignmentBaseline::Baseline
        );
    }

    #[test]
    fn dominant_baseline_fase_7_366() {
        assert_eq!(
            parse_dominant_baseline("auto"),
            Some(DominantBaseline::Auto)
        );
        assert_eq!(
            parse_dominant_baseline("HANGING"),
            Some(DominantBaseline::Hanging)
        );
        assert_eq!(
            parse_dominant_baseline("mathematical"),
            Some(DominantBaseline::Mathematical)
        );
        assert_eq!(parse_dominant_baseline("nope"), None);

        let html = r##"<html><head><style>
            body { dominant-baseline: hanging }
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
        assert_eq!(body_cs.dominant_baseline, DominantBaseline::Hanging);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).dominant_baseline,
            DominantBaseline::Hanging
        );
    }

    #[test]
    fn paint_order_fase_7_367() {
        // `normal` = fill, stroke, markers.
        assert_eq!(parse_paint_order("normal"), Some(PaintOrder::default()));
        // 1 keyword completa con el orden canónico.
        assert_eq!(
            parse_paint_order("stroke"),
            Some(PaintOrder {
                one: PaintFragment::Stroke,
                two: PaintFragment::Fill,
                three: PaintFragment::Markers,
            })
        );
        // 2 keywords.
        assert_eq!(
            parse_paint_order("markers stroke"),
            Some(PaintOrder {
                one: PaintFragment::Markers,
                two: PaintFragment::Stroke,
                three: PaintFragment::Fill,
            })
        );
        // 3 keywords explícitos.
        assert_eq!(
            parse_paint_order("stroke markers fill"),
            Some(PaintOrder {
                one: PaintFragment::Stroke,
                two: PaintFragment::Markers,
                three: PaintFragment::Fill,
            })
        );
        // Duplicado descarta.
        assert_eq!(parse_paint_order("fill fill"), None);
        // Token desconocido descarta.
        assert_eq!(parse_paint_order("foo"), None);
        // Más de 3 descarta.
        assert_eq!(parse_paint_order("fill stroke markers fill"), None);

        let html = r##"<html><head><style>
            body { paint-order: stroke }
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
        assert_eq!(body_cs.paint_order.one, PaintFragment::Stroke);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).paint_order.one,
            PaintFragment::Stroke
        );
    }

    #[test]
    fn marker_side_fase_7_368() {
        assert_eq!(parse_marker_side("match-self"), Some(MarkerSide::MatchSelf));
        assert_eq!(
            parse_marker_side("MATCH-PARENT"),
            Some(MarkerSide::MatchParent)
        );
        assert_eq!(parse_marker_side("nope"), None);

        let html = r##"<html><head><style>
            body { marker-side: match-parent }
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
        assert_eq!(body_cs.marker_side, MarkerSide::MatchParent);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).marker_side,
            MarkerSide::MatchParent
        );
    }

    #[test]
    fn fill_fase_7_369() {
        assert_eq!(parse_svg_paint("none"), Some(SvgPaint::None));
        assert_eq!(parse_svg_paint("currentColor"), Some(SvgPaint::CurrentColor));
        let red = parse_svg_paint("red").unwrap();
        assert!(matches!(red, SvgPaint::Color(c) if (c.r,c.g,c.b)==(255,0,0)));
        assert_eq!(
            parse_svg_paint("url(#grad1)"),
            Some(SvgPaint::Url("#grad1".to_string()))
        );
        assert_eq!(parse_svg_paint("nope"), None);

        // E2E + cascada heredable.
        let html = r##"<html><head><style>
            body { fill: rgb(255,0,0) }
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
        assert!(matches!(body_cs.fill, SvgPaint::Color(c) if (c.r,c.g,c.b)==(255,0,0)));
        // SÍ hereda.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(matches!(div_cs.fill, SvgPaint::Color(c) if (c.r,c.g,c.b)==(255,0,0)));
    }

    #[test]
    fn stroke_fase_7_370() {
        let html = r##"<html><head><style>
            body { stroke: blue }
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
        assert!(matches!(cs.stroke, SvgPaint::Color(c) if (c.r,c.g,c.b)==(0,0,255)));
    }

    #[test]
    fn fill_opacity_fase_7_371() {
        // Número y % se parsean igual.
        assert_eq!(parse_svg_opacity("0.5"), Some(0.5));
        assert_eq!(parse_svg_opacity("50%"), Some(0.5));
        // Clamp.
        assert_eq!(parse_svg_opacity("2.5"), Some(1.0));
        assert_eq!(parse_svg_opacity("-1"), Some(0.0));
        assert_eq!(parse_svg_opacity("nope"), None);

        let html = r##"<html><head><style>
            body { fill-opacity: 0.5 }
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
        assert_eq!(body_cs.fill_opacity, 0.5);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).fill_opacity,
            0.5
        );
    }

