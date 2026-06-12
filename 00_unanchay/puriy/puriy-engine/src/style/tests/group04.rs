//! Tests del motor de estilo (grupo 04, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn text_emphasis_shorthand_fase_7_312() {
        let html = r##"<html><head><style>
            body { text-emphasis: filled triangle red }
            div.none { text-emphasis: none }
            div.style_only { text-emphasis: open circle }
            div.color_only { text-emphasis: blue }
            div.plain {}
        </style></head><body>
          <div class="none"></div><div class="style_only"></div>
          <div class="color_only"></div><div class="plain"></div>
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
            body_cs.text_emphasis_style,
            TextEmphasisStyle::Mark {
                fill: TextEmphasisFill::Filled,
                shape: TextEmphasisShape::Triangle,
            }
        );
        assert_eq!(body_cs.text_emphasis_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // `text-emphasis: none` apaga style + preserva color heredado del body.
        let none = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(none.text_emphasis_style, TextEmphasisStyle::None);
        assert_eq!(none.text_emphasis_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Sólo style: el style override pero el color sigue siendo el del body.
        let so = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert!(matches!(so.text_emphasis_style, TextEmphasisStyle::Mark { .. }));
        assert_eq!(so.text_emphasis_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Sólo color: el style hereda (Mark triangle), el color override.
        let co = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert!(matches!(co.text_emphasis_style, TextEmphasisStyle::Mark { .. }));
        assert_eq!(co.text_emphasis_color.map(|c| (c.r, c.g, c.b)), Some((0, 0, 255)));
    }

    #[test]
    fn ruby_position_fase_7_313() {
        assert_eq!(parse_ruby_position("over"), Some(RubyPosition::Over));
        assert_eq!(parse_ruby_position("UNDER"), Some(RubyPosition::Under));
        assert_eq!(
            parse_ruby_position("inter-character"),
            Some(RubyPosition::InterCharacter)
        );
        assert_eq!(parse_ruby_position("alternate"), Some(RubyPosition::Alternate));
        assert_eq!(parse_ruby_position("nope"), None);

        let html = r##"<html><head><style>
            body { ruby-position: under }
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
        assert_eq!(body_cs.ruby_position, RubyPosition::Under);
        // SÍ se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).ruby_position,
            RubyPosition::Under
        );
    }

    #[test]
    fn transform_origin_fase_7_314() {
        // Default = 50% 50% 0.
        assert_eq!(
            parse_transform_origin("center"),
            Some(TransformOrigin {
                x: LengthVal::Pct(50.0),
                y: LengthVal::Pct(50.0),
                z: 0.0
            })
        );
        // 1 keyword vertical → fija Y, X queda en 50%.
        assert_eq!(
            parse_transform_origin("top"),
            Some(TransformOrigin {
                x: LengthVal::Pct(50.0),
                y: LengthVal::Pct(0.0),
                z: 0.0
            })
        );
        // 1 length → fija X.
        assert_eq!(
            parse_transform_origin("10px"),
            Some(TransformOrigin {
                x: LengthVal::Px(10.0),
                y: LengthVal::Pct(50.0),
                z: 0.0
            })
        );
        // 2 tokens, orden invertido (`top left` → x=left, y=top).
        assert_eq!(
            parse_transform_origin("top left"),
            Some(TransformOrigin {
                x: LengthVal::Pct(0.0),
                y: LengthVal::Pct(0.0),
                z: 0.0
            })
        );
        // 3 tokens: el 3º es Z en px.
        assert_eq!(
            parse_transform_origin("right bottom 5px"),
            Some(TransformOrigin {
                x: LengthVal::Pct(100.0),
                y: LengthVal::Pct(100.0),
                z: 5.0
            })
        );
        // Eje Z en `%` → inválido.
        assert_eq!(parse_transform_origin("center center 5%"), None);
        // Más de 3 tokens → inválido.
        assert_eq!(parse_transform_origin("1px 2px 3px 4px"), None);

        // E2E: NO se hereda (transforms y su origen son por-elemento).
        let html = r##"<html><head><style>
            body { transform-origin: 10px 20px }
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
            body_cs.transform_origin,
            TransformOrigin {
                x: LengthVal::Px(10.0),
                y: LengthVal::Px(20.0),
                z: 0.0
            }
        );
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        // NO hereda — vuelve al default 50% 50% 0.
        assert_eq!(div_cs.transform_origin, TransformOrigin::default());
    }

    #[test]
    fn transform_style_fase_7_315() {
        assert_eq!(parse_transform_style("flat"), Some(TransformStyle::Flat));
        assert_eq!(
            parse_transform_style("PRESERVE-3D"),
            Some(TransformStyle::Preserve3d)
        );
        assert_eq!(parse_transform_style("nope"), None);

        let html = r##"<html><head><style>
            body { transform-style: preserve-3d }
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
        assert_eq!(body_cs.transform_style, TransformStyle::Preserve3d);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).transform_style,
            TransformStyle::Flat
        );
    }

    #[test]
    fn perspective_fase_7_316() {
        assert_eq!(parse_perspective("none"), Some(None));
        assert_eq!(parse_perspective("NONE"), Some(None));
        assert_eq!(parse_perspective("500px"), Some(Some(500.0)));
        // No negativo.
        assert_eq!(parse_perspective("-10px"), None);
        // `%` no es length-en-px.
        assert_eq!(parse_perspective("50%"), None);
        assert_eq!(parse_perspective("nope"), None);

        let html = r##"<html><head><style>
            body { perspective: 800px }
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
        assert_eq!(body_cs.perspective, Some(800.0));
        // NO hereda — vuelve a None.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).perspective,
            None
        );
    }

    #[test]
    fn perspective_origin_fase_7_317() {
        assert_eq!(
            parse_perspective_origin("center"),
            Some(PerspectiveOrigin {
                x: LengthVal::Pct(50.0),
                y: LengthVal::Pct(50.0)
            })
        );
        assert_eq!(
            parse_perspective_origin("top"),
            Some(PerspectiveOrigin {
                x: LengthVal::Pct(50.0),
                y: LengthVal::Pct(0.0)
            })
        );
        // Orden invertido: `top left` → x=left, y=top.
        assert_eq!(
            parse_perspective_origin("top left"),
            Some(PerspectiveOrigin {
                x: LengthVal::Pct(0.0),
                y: LengthVal::Pct(0.0)
            })
        );
        assert_eq!(
            parse_perspective_origin("25% 75%"),
            Some(PerspectiveOrigin {
                x: LengthVal::Pct(25.0),
                y: LengthVal::Pct(75.0)
            })
        );
        // 3 tokens → inválido (no hay eje Z).
        assert_eq!(parse_perspective_origin("center center 5px"), None);

        let html = r##"<html><head><style>
            body { perspective-origin: 20px 40px }
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
            body_cs.perspective_origin,
            PerspectiveOrigin { x: LengthVal::Px(20.0), y: LengthVal::Px(40.0) }
        );
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).perspective_origin,
            PerspectiveOrigin::default()
        );
    }

    #[test]
    fn backface_visibility_fase_7_318() {
        assert_eq!(
            parse_backface_visibility("visible"),
            Some(BackfaceVisibility::Visible)
        );
        assert_eq!(
            parse_backface_visibility("HIDDEN"),
            Some(BackfaceVisibility::Hidden)
        );
        assert_eq!(parse_backface_visibility("nope"), None);

        let html = r##"<html><head><style>
            body { backface-visibility: hidden }
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
        assert_eq!(body_cs.backface_visibility, BackfaceVisibility::Hidden);
        // NO hereda — vuelve a Visible.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).backface_visibility,
            BackfaceVisibility::Visible
        );
    }

    #[test]
    fn scrollbar_width_fase_7_319() {
        assert_eq!(parse_scrollbar_width("auto"), Some(ScrollbarWidth::Auto));
        assert_eq!(parse_scrollbar_width("THIN"), Some(ScrollbarWidth::Thin));
        assert_eq!(parse_scrollbar_width("none"), Some(ScrollbarWidth::None));
        assert_eq!(parse_scrollbar_width("nope"), None);

        let html = r##"<html><head><style>
            body { scrollbar-width: thin }
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
        assert_eq!(body_cs.scrollbar_width, ScrollbarWidth::Thin);
        // SÍ hereda (CSS Scrollbars 1).
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scrollbar_width,
            ScrollbarWidth::Thin
        );
    }

    #[test]
    fn scrollbar_color_fase_7_320() {
        assert_eq!(parse_scrollbar_color("auto"), Some(None));
        // Dos colores — keyword.
        let two = parse_scrollbar_color("red blue").unwrap().unwrap();
        assert_eq!((two.thumb.r, two.thumb.g, two.thumb.b), (255, 0, 0));
        assert_eq!((two.track.r, two.track.g, two.track.b), (0, 0, 255));
        // Dos colores — rgb(...) con espacios internos.
        let rgb = parse_scrollbar_color("rgb(10,20,30) rgb(40,50,60)")
            .unwrap()
            .unwrap();
        assert_eq!((rgb.thumb.r, rgb.thumb.g, rgb.thumb.b), (10, 20, 30));
        assert_eq!((rgb.track.r, rgb.track.g, rgb.track.b), (40, 50, 60));
        // Uno solo (falta track) → inválido.
        assert_eq!(parse_scrollbar_color("red"), None);
        assert_eq!(parse_scrollbar_color("nope"), None);

        let html = r##"<html><head><style>
            body { scrollbar-color: red blue }
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
        let pair = body_cs.scrollbar_color.unwrap();
        assert_eq!((pair.thumb.r, pair.thumb.g, pair.thumb.b), (255, 0, 0));
        // SÍ hereda.
        let div_pair = eng
            .compute_with_parent(&divs[0], Some(&body_cs))
            .scrollbar_color
            .unwrap();
        assert_eq!((div_pair.track.r, div_pair.track.g, div_pair.track.b), (0, 0, 255));
    }

    #[test]
    fn scrollbar_gutter_fase_7_321() {
        assert_eq!(parse_scrollbar_gutter("auto"), Some(ScrollbarGutter::AUTO));
        assert_eq!(parse_scrollbar_gutter("stable"), Some(ScrollbarGutter::STABLE));
        assert_eq!(
            parse_scrollbar_gutter("stable both-edges"),
            Some(ScrollbarGutter::STABLE_BOTH)
        );
        // Orden libre (`both-edges stable`).
        assert_eq!(
            parse_scrollbar_gutter("both-edges stable"),
            Some(ScrollbarGutter::STABLE_BOTH)
        );
        // `both-edges` solo (sin `stable`) → inválido.
        assert_eq!(parse_scrollbar_gutter("both-edges"), None);
        assert_eq!(parse_scrollbar_gutter("nope"), None);

        let html = r##"<html><head><style>
            body { scrollbar-gutter: stable both-edges }
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
        assert_eq!(body_cs.scrollbar_gutter, ScrollbarGutter::STABLE_BOTH);
        // NO hereda — vuelve a auto.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scrollbar_gutter,
            ScrollbarGutter::AUTO
        );
    }

    #[test]
    fn overflow_anchor_fase_7_322() {
        assert_eq!(parse_overflow_anchor("auto"), Some(OverflowAnchor::Auto));
        assert_eq!(parse_overflow_anchor("NONE"), Some(OverflowAnchor::None));
        assert_eq!(parse_overflow_anchor("nope"), None);

        let html = r##"<html><head><style>
            body { overflow-anchor: none }
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
        assert_eq!(body_cs.overflow_anchor, OverflowAnchor::None);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).overflow_anchor,
            OverflowAnchor::Auto
        );
    }

    #[test]
    fn overflow_clip_margin_fase_7_323() {
        // length sola → padding-box.
        assert_eq!(
            parse_overflow_clip_margin("10px"),
            Some(Some(OverflowClipMargin {
                visual_box: VisualBox::PaddingBox,
                length: 10.0
            }))
        );
        // visual-box solo → length 0.
        assert_eq!(
            parse_overflow_clip_margin("content-box"),
            Some(Some(OverflowClipMargin {
                visual_box: VisualBox::ContentBox,
                length: 0.0
            }))
        );
        // Ambos.
        assert_eq!(
            parse_overflow_clip_margin("border-box 5px"),
            Some(Some(OverflowClipMargin {
                visual_box: VisualBox::BorderBox,
                length: 5.0
            }))
        );
        // Orden libre.
        assert_eq!(
            parse_overflow_clip_margin("5px border-box"),
            Some(Some(OverflowClipMargin {
                visual_box: VisualBox::BorderBox,
                length: 5.0
            }))
        );
        // `0px` solo → reset (None).
        assert_eq!(parse_overflow_clip_margin("0px"), Some(None));
        // Negativo descarta.
        assert_eq!(parse_overflow_clip_margin("-1px"), None);
        // Dos visual-box descarta.
        assert_eq!(parse_overflow_clip_margin("border-box content-box"), None);
        // Vacío descarta.
        assert_eq!(parse_overflow_clip_margin(""), None);

        let html = r##"<html><head><style>
            body { overflow-clip-margin: content-box 8px }
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
            body_cs.overflow_clip_margin,
            Some(OverflowClipMargin {
                visual_box: VisualBox::ContentBox,
                length: 8.0
            })
        );
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).overflow_clip_margin,
            None
        );
    }

    #[test]
    fn text_align_last_fase_7_324() {
        assert_eq!(parse_text_align_last("auto"), Some(TextAlignLast::Auto));
        assert_eq!(parse_text_align_last("START"), Some(TextAlignLast::Start));
        assert_eq!(parse_text_align_last("justify"), Some(TextAlignLast::Justify));
        assert_eq!(parse_text_align_last("center"), Some(TextAlignLast::Center));
        assert_eq!(parse_text_align_last("nope"), None);

        let html = r##"<html><head><style>
            body { text-align-last: justify }
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
        assert_eq!(body_cs.text_align_last, TextAlignLast::Justify);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_align_last,
            TextAlignLast::Justify
        );
    }

    #[test]
    fn text_wrap_fase_7_325() {
        assert_eq!(parse_text_wrap("wrap"), Some(TextWrap::Wrap));
        assert_eq!(parse_text_wrap("NOWRAP"), Some(TextWrap::Nowrap));
        assert_eq!(parse_text_wrap("balance"), Some(TextWrap::Balance));
        assert_eq!(parse_text_wrap("pretty"), Some(TextWrap::Pretty));
        assert_eq!(parse_text_wrap("stable"), Some(TextWrap::Stable));
        assert_eq!(parse_text_wrap("nope"), None);

        let html = r##"<html><head><style>
            body { text-wrap: balance }
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
        assert_eq!(body_cs.text_wrap, TextWrap::Balance);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_wrap,
            TextWrap::Balance
        );
    }

    #[test]
    fn line_break_fase_7_326() {
        assert_eq!(parse_line_break("auto"), Some(LineBreak::Auto));
        assert_eq!(parse_line_break("LOOSE"), Some(LineBreak::Loose));
        assert_eq!(parse_line_break("normal"), Some(LineBreak::Normal));
        assert_eq!(parse_line_break("strict"), Some(LineBreak::Strict));
        assert_eq!(parse_line_break("anywhere"), Some(LineBreak::Anywhere));
        assert_eq!(parse_line_break("nope"), None);

        let html = r##"<html><head><style>
            body { line-break: strict }
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
        assert_eq!(body_cs.line_break, LineBreak::Strict);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).line_break,
            LineBreak::Strict
        );
    }

    #[test]
    fn hanging_punctuation_fase_7_327() {
        assert_eq!(
            parse_hanging_punctuation("none"),
            Some(HangingPunctuation::default())
        );
        // first solo.
        assert_eq!(
            parse_hanging_punctuation("first"),
            Some(HangingPunctuation { first: true, ..Default::default() })
        );
        // first + force-end + last (orden libre).
        assert_eq!(
            parse_hanging_punctuation("last force-end first"),
            Some(HangingPunctuation {
                first: true,
                force_end: true,
                allow_end: false,
                last: true
            })
        );
        // allow-end solo.
        assert_eq!(
            parse_hanging_punctuation("allow-end"),
            Some(HangingPunctuation { allow_end: true, ..Default::default() })
        );
        // force-end + allow-end → excluyentes, descarta.
        assert_eq!(parse_hanging_punctuation("force-end allow-end"), None);
        // Duplicado descarta.
        assert_eq!(parse_hanging_punctuation("first first"), None);
        // Token desconocido descarta.
        assert_eq!(parse_hanging_punctuation("first foo"), None);
        // Vacío descarta.
        assert_eq!(parse_hanging_punctuation(""), None);

        let html = r##"<html><head><style>
            body { hanging-punctuation: first last }
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
        assert!(body_cs.hanging_punctuation.first);
        assert!(body_cs.hanging_punctuation.last);
        // SÍ hereda.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(div_cs.hanging_punctuation.first);
        assert!(div_cs.hanging_punctuation.last);
    }

    #[test]
    fn text_decoration_skip_ink_fase_7_328() {
        assert_eq!(
            parse_text_decoration_skip_ink("auto"),
            Some(TextDecorationSkipInk::Auto)
        );
        assert_eq!(
            parse_text_decoration_skip_ink("NONE"),
            Some(TextDecorationSkipInk::None)
        );
        assert_eq!(
            parse_text_decoration_skip_ink("all"),
            Some(TextDecorationSkipInk::All)
        );
        assert_eq!(parse_text_decoration_skip_ink("nope"), None);

        let html = r##"<html><head><style>
            body { text-decoration-skip-ink: none }
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
        assert_eq!(body_cs.text_decoration_skip_ink, TextDecorationSkipInk::None);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .text_decoration_skip_ink,
            TextDecorationSkipInk::None
        );
    }

    #[test]
    fn font_optical_sizing_fase_7_329() {
        assert_eq!(
            parse_font_optical_sizing("auto"),
            Some(FontOpticalSizing::Auto)
        );
        assert_eq!(
            parse_font_optical_sizing("NONE"),
            Some(FontOpticalSizing::None)
        );
        assert_eq!(parse_font_optical_sizing("nope"), None);

        let html = r##"<html><head><style>
            body { font-optical-sizing: none }
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
        assert_eq!(body_cs.font_optical_sizing, FontOpticalSizing::None);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).font_optical_sizing,
            FontOpticalSizing::None
        );
    }

    #[test]
    fn font_synthesis_weight_fase_7_330() {
        // Longhand 1/3: weight.
        assert_eq!(parse_auto_or_none("auto"), Some(true));
        assert_eq!(parse_auto_or_none("none"), Some(false));
        assert_eq!(parse_auto_or_none("nope"), None);

        let html = r##"<html><head><style>
            body { font-synthesis-weight: none }
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
        assert!(!body_cs.font_synthesis.weight);
        // Los otros 2 axes siguen en true (default).
        assert!(body_cs.font_synthesis.style);
        assert!(body_cs.font_synthesis.small_caps);
        // SÍ hereda (toda la struct).
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(!div_cs.font_synthesis.weight);
        assert!(div_cs.font_synthesis.style);
    }

    #[test]
    fn font_synthesis_style_fase_7_331() {
        let html = r##"<html><head><style>
            body { font-synthesis-style: none }
        </style></head><body></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(body_cs.font_synthesis.weight);
        assert!(!body_cs.font_synthesis.style);
        assert!(body_cs.font_synthesis.small_caps);
    }

    #[test]
    fn font_synthesis_small_caps_fase_7_332() {
        let html = r##"<html><head><style>
            body { font-synthesis-small-caps: none }
        </style></head><body></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!(body_cs.font_synthesis.weight);
        assert!(body_cs.font_synthesis.style);
        assert!(!body_cs.font_synthesis.small_caps);
    }

    #[test]
    fn font_synthesis_shorthand_fase_7_333() {
        // `none` apaga los 3.
        assert_eq!(
            parse_font_synthesis_shorthand("none"),
            Some(FontSynthesis::NONE)
        );
        // `weight` solo: weight=true, los demás false.
        assert_eq!(
            parse_font_synthesis_shorthand("weight"),
            Some(FontSynthesis {
                weight: true,
                style: false,
                small_caps: false,
                position: false,
            })
        );
        // Combinación orden libre.
        assert_eq!(
            parse_font_synthesis_shorthand("small-caps weight"),
            Some(FontSynthesis {
                weight: true,
                style: false,
                small_caps: true,
                position: false,
            })
        );
        // Los 4 (CSS Fonts 4 — Fase 7.470 agrega `position`).
        assert_eq!(
            parse_font_synthesis_shorthand("weight style small-caps position"),
            Some(FontSynthesis {
                weight: true,
                style: true,
                small_caps: true,
                position: true,
            })
        );
        // Duplicado descarta.
        assert_eq!(parse_font_synthesis_shorthand("weight weight"), None);
        // Token desconocido descarta.
        assert_eq!(parse_font_synthesis_shorthand("weight foo"), None);
        // Vacío descarta.
        assert_eq!(parse_font_synthesis_shorthand(""), None);

        // E2E via shorthand: `font-synthesis: style` apaga weight y small-caps.
        let html = r##"<html><head><style>
            body { font-synthesis: style small-caps }
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
        assert!(!body_cs.font_synthesis.weight);
        assert!(body_cs.font_synthesis.style);
        assert!(body_cs.font_synthesis.small_caps);
        // SÍ hereda.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(!div_cs.font_synthesis.weight);
        assert!(div_cs.font_synthesis.style);
    }

    #[test]
    fn font_size_adjust_fase_7_334() {
        // `none`.
        assert_eq!(parse_font_size_adjust("none"), Some(FontSizeAdjust::None));
        // `<num>` solo → métrica default `ex-height`.
        assert_eq!(
            parse_font_size_adjust("0.5"),
            Some(FontSizeAdjust::Value(FontMetric::ExHeight, 0.5))
        );
        // `from-font` solo → métrica default.
        assert_eq!(
            parse_font_size_adjust("from-font"),
            Some(FontSizeAdjust::FromFont(FontMetric::ExHeight))
        );
        // `<metric> <num>`.
        assert_eq!(
            parse_font_size_adjust("cap-height 0.7"),
            Some(FontSizeAdjust::Value(FontMetric::CapHeight, 0.7))
        );
        // `<metric> from-font`.
        assert_eq!(
            parse_font_size_adjust("ic-width from-font"),
            Some(FontSizeAdjust::FromFont(FontMetric::IcWidth))
        );
        // Negativo descarta.
        assert_eq!(parse_font_size_adjust("-0.5"), None);
        // Métrica desconocida descarta.
        assert_eq!(parse_font_size_adjust("foo 0.5"), None);

        let html = r##"<html><head><style>
            body { font-size-adjust: cap-height 0.7 }
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
            body_cs.font_size_adjust,
            FontSizeAdjust::Value(FontMetric::CapHeight, 0.7)
        );
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).font_size_adjust,
            FontSizeAdjust::Value(FontMetric::CapHeight, 0.7)
        );
    }

    #[test]
    fn image_orientation_fase_7_335() {
        // Keywords.
        assert_eq!(
            parse_image_orientation("from-image"),
            Some(ImageOrientation::FromImage)
        );
        assert_eq!(parse_image_orientation("NONE"), Some(ImageOrientation::None));
        assert_eq!(
            parse_image_orientation("flip"),
            Some(ImageOrientation::Angle { degrees: 0.0, flip: true })
        );
        // `<angle>` solo.
        assert_eq!(
            parse_image_orientation("90deg"),
            Some(ImageOrientation::Angle { degrees: 90.0, flip: false })
        );
        // `<angle> flip` y `flip <angle>` (orden libre).
        assert_eq!(
            parse_image_orientation("180deg flip"),
            Some(ImageOrientation::Angle { degrees: 180.0, flip: true })
        );
        assert_eq!(
            parse_image_orientation("flip 270deg"),
            Some(ImageOrientation::Angle { degrees: 270.0, flip: true })
        );
        // Unidades alternativas.
        let half_turn = parse_image_orientation("0.5turn").unwrap();
        match half_turn {
            ImageOrientation::Angle { degrees, flip } => {
                assert!((degrees - 180.0).abs() < 1e-3);
                assert!(!flip);
            }
            _ => panic!("expected Angle"),
        }
        // Sin unidad y distinto de 0 descarta.
        assert_eq!(parse_image_orientation("90"), None);
        // 0 sin unidad sí.
        assert_eq!(
            parse_image_orientation("0"),
            Some(ImageOrientation::Angle { degrees: 0.0, flip: false })
        );
        // Token desconocido descarta.
        assert_eq!(parse_image_orientation("nope"), None);

        let html = r##"<html><head><style>
            body { image-orientation: none }
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
        assert_eq!(body_cs.image_orientation, ImageOrientation::None);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).image_orientation,
            ImageOrientation::None
        );
    }

    #[test]
    fn place_items_fase_7_336() {
        // 1 token aplica a los 2 ejes.
        let (a, j) = parse_place_items("center").unwrap();
        assert_eq!(a, AlignItems::Center);
        assert_eq!(j, AlignItems::Center);
        // 2 tokens distintos.
        let (a, j) = parse_place_items("start end").unwrap();
        assert_eq!(a, AlignItems::Start);
        assert_eq!(j, AlignItems::End);
        // 3 tokens descarta.
        assert!(parse_place_items("a b c").is_none());
        assert!(parse_place_items("nope").is_none());

        // E2E: `place-items: center stretch` setea align-items=center,
        // justify-items=stretch (uno solo) en un grid.
        let html = r##"<html><head><style>
            .grid { display: grid; place-items: center stretch }
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
        assert_eq!(cs.align_items, AlignItems::Center);
        assert_eq!(cs.justify_items, Some(AlignItems::Stretch));
    }

    #[test]
    fn place_content_fase_7_337() {
        // 1 token comparte.
        let (a, j) = parse_place_content("center").unwrap();
        assert_eq!(a, AlignContent::Center);
        assert_eq!(j, JustifyContent::Center);
        // 2 tokens.
        let (a, j) = parse_place_content("start end").unwrap();
        assert_eq!(a, AlignContent::Start);
        assert_eq!(j, JustifyContent::End);
        assert!(parse_place_content("nope").is_none());

        let html = r##"<html><head><style>
            .grid { display: grid; place-content: start end }
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
        assert_eq!(cs.align_content, AlignContent::Start);
        assert_eq!(cs.justify_content, JustifyContent::End);
    }

    #[test]
    fn place_self_fase_7_338() {
        // 1 token comparte.
        let (a, j) = parse_place_self("center").unwrap();
        assert_eq!(a, AlignSelf::Center);
        assert_eq!(j, AlignSelf::Center);
        // 2 tokens.
        let (a, j) = parse_place_self("start end").unwrap();
        assert_eq!(a, AlignSelf::Start);
        assert_eq!(j, AlignSelf::End);
        assert!(parse_place_self("nope").is_none());

        let html = r##"<html><head><style>
            .item { place-self: center end }
        </style></head><body><div class="item"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let cs = eng.compute(&divs[0]);
        assert_eq!(cs.align_self, AlignSelf::Center);
        assert_eq!(cs.justify_self, AlignSelf::End);
    }

    #[test]
    fn animation_timeline_fase_7_339() {
        assert_eq!(parse_timeline_ref("auto"), Some(TimelineRef::Auto));
        assert_eq!(parse_timeline_ref("NONE"), Some(TimelineRef::None));
        assert_eq!(
            parse_timeline_ref("--scroller"),
            Some(TimelineRef::Named("--scroller".to_string()))
        );
        assert_eq!(parse_timeline_ref(""), None);

        let html = r##"<html><head><style>
            body { animation-timeline: --scroller }
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
            body_cs.animation_timeline,
            TimelineRef::Named("--scroller".to_string())
        );
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).animation_timeline,
            TimelineRef::Auto
        );
    }

    #[test]
    fn scroll_timeline_name_fase_7_340() {
        assert_eq!(parse_dashed_ident_or_none("none"), Some(None));
        assert_eq!(
            parse_dashed_ident_or_none("--my-tl"),
            Some(Some("--my-tl".to_string()))
        );
        assert_eq!(parse_dashed_ident_or_none(""), None);

        let html = r##"<html><head><style>
            body { scroll-timeline-name: --tl }
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
        assert_eq!(body_cs.scroll_timeline_name, Some("--tl".to_string()));
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_timeline_name,
            None
        );
    }

    #[test]
    fn scroll_timeline_axis_fase_7_341() {
        assert_eq!(parse_timeline_axis("block"), Some(TimelineAxis::Block));
        assert_eq!(parse_timeline_axis("INLINE"), Some(TimelineAxis::Inline));
        assert_eq!(parse_timeline_axis("x"), Some(TimelineAxis::X));
        assert_eq!(parse_timeline_axis("y"), Some(TimelineAxis::Y));
        assert_eq!(parse_timeline_axis("nope"), None);

        let html = r##"<html><head><style>
            body { scroll-timeline-axis: inline }
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
        assert_eq!(body_cs.scroll_timeline_axis, TimelineAxis::Inline);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_timeline_axis,
            TimelineAxis::Block
        );
    }

