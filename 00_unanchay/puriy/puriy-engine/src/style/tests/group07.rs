//! Tests del motor de estilo (grupo 07, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn mask_origin_fase_7_402() {
        assert_eq!(
            parse_mask_origin("border-box"),
            Some(MaskOrigin::BorderBox)
        );
        assert_eq!(
            parse_mask_origin("CONTENT-BOX"),
            Some(MaskOrigin::ContentBox)
        );
        assert_eq!(
            parse_mask_origin("stroke-box"),
            Some(MaskOrigin::StrokeBox)
        );
        // `no-clip` NO es válido en mask-origin (sólo en mask-clip).
        assert_eq!(parse_mask_origin("no-clip"), None);

        let html = r##"<html><head><style>
            body { mask-origin: padding-box }
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
        assert_eq!(cs.mask_origin, MaskOrigin::PaddingBox);
    }

    #[test]
    fn mask_repeat_fase_7_403() {
        let html = r##"<html><head><style>
            body { mask-repeat: no-repeat }
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
        assert_eq!(body_cs.mask_repeat, BackgroundRepeat::NoRepeat);
        // NO hereda — reset a `Repeat`.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mask_repeat,
            BackgroundRepeat::Repeat
        );
        // mask-repeat NO toca background-repeat (mismo tipo, campos distintos).
        assert_eq!(body_cs.background_repeat, BackgroundRepeat::Repeat);
    }

    #[test]
    fn mask_position_fase_7_404() {
        let html = r##"<html><head><style>
            body { mask-position: 25% 75% }
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
        assert_eq!(body_cs.mask_position.x, LengthVal::Pct(25.0));
        assert_eq!(body_cs.mask_position.y, LengthVal::Pct(75.0));
        // NO hereda — reset al default (0% 0%).
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(div_cs.mask_position.x, LengthVal::Pct(0.0));
        assert_eq!(div_cs.mask_position.y, LengthVal::Pct(0.0));
        // background-position queda intacto.
        assert_eq!(body_cs.background_position.x, LengthVal::Pct(0.0));
    }

    #[test]
    fn mask_size_fase_7_405() {
        let html = r##"<html><head><style>
            body { mask-size: cover }
            div.contain { mask-size: contain }
            div.plain {}
        </style></head><body>
            <div class="contain"></div>
            <div class="plain"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut contains = Vec::new();
        let mut plains = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
            if crate::dom::element_name(n).as_deref() == Some("div") {
                let class = crate::dom::attr(n, "class");
                if class.as_deref() == Some("contain") {
                    contains.push(n.clone());
                } else if class.as_deref() == Some("plain") {
                    plains.push(n.clone());
                }
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.mask_size, BackgroundSize::Cover);
        let contain_cs = eng.compute_with_parent(&contains[0], Some(&body_cs));
        assert_eq!(contain_cs.mask_size, BackgroundSize::Contain);
        // NO hereda — reset a `Auto`.
        let plain_cs = eng.compute_with_parent(&plains[0], Some(&body_cs));
        assert_eq!(plain_cs.mask_size, BackgroundSize::Auto);
    }

    #[test]
    fn container_name_fase_7_406() {
        assert_eq!(
            parse_ident_list_or_none("none"),
            Some(Vec::new())
        );

        let html = r##"<html><head><style>
            body { container-name: sidebar main }
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
        assert_eq!(body_cs.container_name, vec!["sidebar", "main"]);
        // NO hereda — vacío.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(div_cs.container_name.is_empty());
    }

    #[test]
    fn container_type_fase_7_407() {
        assert_eq!(
            parse_container_type("normal"),
            Some(ContainerType::Normal)
        );
        assert_eq!(
            parse_container_type("SIZE"),
            Some(ContainerType::Size)
        );
        assert_eq!(
            parse_container_type("inline-size"),
            Some(ContainerType::InlineSize)
        );
        assert_eq!(parse_container_type("scroll-state"), None);

        let html = r##"<html><head><style>
            body { container-type: inline-size }
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
        assert_eq!(body_cs.container_type, ContainerType::InlineSize);
        // NO hereda — reset a `Normal`.
        let div_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(div_cs.container_type, ContainerType::Normal);
    }

    #[test]
    fn container_shorthand_fase_7_408() {
        // Sólo name (sin `/`).
        let html_a = r##"<html><head><style>
            body { container: card }
        </style></head><body></body></html>"##;
        let dom_a = DomTree::parse(html_a);
        let eng_a = StyleEngine::from_dom(&dom_a);
        let mut bodies_a = Vec::new();
        crate::dom::walk(&dom_a.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies_a.push(n.clone());
            }
        });
        let cs_a = eng_a.compute(&bodies_a[0]);
        assert_eq!(cs_a.container_name, vec!["card"]);
        assert_eq!(cs_a.container_type, ContainerType::Normal);

        // Name + type (`name / type`).
        let html_b = r##"<html><head><style>
            body { container: sidebar / inline-size }
        </style></head><body></body></html>"##;
        let dom_b = DomTree::parse(html_b);
        let eng_b = StyleEngine::from_dom(&dom_b);
        let mut bodies_b = Vec::new();
        crate::dom::walk(&dom_b.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies_b.push(n.clone());
            }
        });
        let cs_b = eng_b.compute(&bodies_b[0]);
        assert_eq!(cs_b.container_name, vec!["sidebar"]);
        assert_eq!(cs_b.container_type, ContainerType::InlineSize);

        // `none / size` — name vacío, type Size.
        let html_c = r##"<html><head><style>
            body { container: none / size }
        </style></head><body></body></html>"##;
        let dom_c = DomTree::parse(html_c);
        let eng_c = StyleEngine::from_dom(&dom_c);
        let mut bodies_c = Vec::new();
        crate::dom::walk(&dom_c.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies_c.push(n.clone());
            }
        });
        let cs_c = eng_c.compute(&bodies_c[0]);
        assert!(cs_c.container_name.is_empty());
        assert_eq!(cs_c.container_type, ContainerType::Size);
    }

    #[test]
    fn border_radius_logico_fase_7_409_412() {
        // En LTR horizontal: block-start = top, block-end = bottom,
        // inline-start = left, inline-end = right. El primer eje es block.
        let html = r##"<html><head><style>
            body {
                border-start-start-radius: 1px;
                border-start-end-radius: 2px;
                border-end-start-radius: 3px;
                border-end-end-radius: 4px;
            }
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
        assert_eq!(cs.border_radii.top_left, 1.0);
        assert_eq!(cs.border_radii.top_right, 2.0);
        assert_eq!(cs.border_radii.bottom_left, 3.0);
        assert_eq!(cs.border_radii.bottom_right, 4.0);
    }

    #[test]
    fn overscroll_behavior_inline_fase_7_414() {
        // `overscroll-behavior-inline` mapea al longhand físico `-x`
        // (eje horizontal en LTR horizontal). NO toca el `-y`.
        let html = r##"<html><head><style>
            body { overscroll-behavior-inline: none }
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
        assert_eq!(cs.overscroll_behavior_x, OverscrollBehavior::None);
        assert_eq!(cs.overscroll_behavior_y, OverscrollBehavior::Auto);
    }

    #[test]
    fn scroll_margin_block_inline_fase_7_415_418() {
        // Longhands `-block-start/-end` + `-inline-start` y shorthand
        // `scroll-margin-block`. En LTR horizontal: block=Y, inline=X,
        // start=top/left, end=bottom/right.
        let html = r##"<html><head><style>
            #a { scroll-margin-block-start: 5px; scroll-margin-block-end: 7px }
            #b { scroll-margin-block: 9px 11px }
            #c { scroll-margin-block: 13px }
            #d { scroll-margin-inline-start: 15px }
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
        assert_eq!(cs_a.scroll_margin.top, 5.0);
        assert_eq!(cs_a.scroll_margin.bottom, 7.0);
        let cs_b = eng.compute(&by_id["b"]);
        assert_eq!(cs_b.scroll_margin.top, 9.0);
        assert_eq!(cs_b.scroll_margin.bottom, 11.0);
        let cs_c = eng.compute(&by_id["c"]);
        // 1 valor → mismo en ambos lados.
        assert_eq!(cs_c.scroll_margin.top, 13.0);
        assert_eq!(cs_c.scroll_margin.bottom, 13.0);
        let cs_d = eng.compute(&by_id["d"]);
        assert_eq!(cs_d.scroll_margin.left, 15.0);
        // Lados no tocados quedan en default (0).
        assert_eq!(cs_d.scroll_margin.right, 0.0);
    }

    #[test]
    fn block_step_fase_7_454_458() {
        // Cinco props: block-step-{size,insert,align,round} (longhands) +
        // block-step (shorthand). NO heredan.

        // 1) Parser de cabecera para -size.
        assert_eq!(parse_block_step_size("none"), Some(BlockStepSize::None));
        assert_eq!(
            parse_block_step_size("24px"),
            Some(BlockStepSize::Length(24.0))
        );
        assert_eq!(parse_block_step_size("bad"), None);

        // 2) E2E — longhands + shorthand.
        let html = r##"<html><head><style>
            #a { block-step-size: 16px;
                 block-step-insert: padding-box;
                 block-step-align: center;
                 block-step-round: down }
            #b { block-step: 20px }
            #c { block-step: down center 32px padding-box }
            #d { block-step: 16px 32px }
            #e {}
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
        // #a — longhands.
        let cs_a = eng.compute(&by("a"));
        assert_eq!(cs_a.block_step_size, BlockStepSize::Length(16.0));
        assert_eq!(cs_a.block_step_insert, BlockStepInsert::PaddingBox);
        assert_eq!(cs_a.block_step_align, BlockStepAlign::Center);
        assert_eq!(cs_a.block_step_round, BlockStepRound::Down);
        // #b — shorthand 1 token (sólo size); el resto se reinicia a defaults.
        let cs_b = eng.compute(&by("b"));
        assert_eq!(cs_b.block_step_size, BlockStepSize::Length(20.0));
        assert_eq!(cs_b.block_step_insert, BlockStepInsert::MarginBox);
        assert_eq!(cs_b.block_step_align, BlockStepAlign::Auto);
        assert_eq!(cs_b.block_step_round, BlockStepRound::Up);
        // #c — shorthand 4 tokens en orden distinto al canónico.
        let cs_c = eng.compute(&by("c"));
        assert_eq!(cs_c.block_step_size, BlockStepSize::Length(32.0));
        assert_eq!(cs_c.block_step_insert, BlockStepInsert::PaddingBox);
        assert_eq!(cs_c.block_step_align, BlockStepAlign::Center);
        assert_eq!(cs_c.block_step_round, BlockStepRound::Down);
        // #d — shorthand inválido (dos lengths) → no emite NADA, todo en default.
        let cs_d = eng.compute(&by("d"));
        assert_eq!(cs_d.block_step_size, BlockStepSize::None);
        assert_eq!(cs_d.block_step_insert, BlockStepInsert::MarginBox);
        assert_eq!(cs_d.block_step_align, BlockStepAlign::Auto);
        assert_eq!(cs_d.block_step_round, BlockStepRound::Up);
        // #e — defaults puros.
        let cs_e = eng.compute(&by("e"));
        assert_eq!(cs_e.block_step_size, BlockStepSize::None);
    }

    #[test]
    fn timelines_fontsyn_pos_interactivity_fase_7_469_473() {
        // Cinco props: view-timeline-inset (Fase 7.469), font-synthesis-
        // position (Fase 7.470), scroll-timeline shorthand (Fase 7.471),
        // view-timeline shorthand (Fase 7.472), interactivity (Fase 7.473).
        // HEREDAN: font-synthesis-position (toda la familia hereda),
        // interactivity.

        // 1) Parsers de cabecera para los dos shorthands de timeline.
        assert_eq!(
            parse_scroll_view_timeline_short("--t"),
            Some((Some("--t".to_string()), TimelineAxis::Block))
        );
        assert_eq!(
            parse_scroll_view_timeline_short("inline --t"),
            Some((Some("--t".to_string()), TimelineAxis::Inline))
        );
        assert_eq!(
            parse_scroll_view_timeline_short("none x"),
            Some((None, TimelineAxis::X))
        );
        // Redundancia rechaza.
        assert_eq!(parse_scroll_view_timeline_short("block inline"), None);
        // Vacío rechaza.
        assert_eq!(parse_scroll_view_timeline_short(""), None);

        assert_eq!(
            parse_view_timeline_short("--t inline 10px 20px"),
            Some((
                Some("--t".to_string()),
                TimelineAxis::Inline,
                LengthVal::Px(10.0),
                LengthVal::Px(20.0),
            ))
        );
        assert_eq!(
            parse_view_timeline_short("auto"),
            Some((None, TimelineAxis::Block, LengthVal::Px(0.0), LengthVal::Px(0.0)))
        );
        // Tres insets rechaza.
        assert_eq!(
            parse_view_timeline_short("10px 20px 30px"),
            None
        );

        // 2) E2E.
        let html = r##"<html><head><style>
            #a { view-timeline-inset: 10px 30%;
                 font-synthesis-position: none;
                 scroll-timeline: --st inline;
                 view-timeline: --vt y 5px;
                 interactivity: inert }
            #b { view-timeline: --b }
            #c { view-timeline-inset: 8px }
            #d {}
        </style></head><body>
            <div id="a"><span id="a-child"></span></div>
            <div id="b"></div>
            <div id="c"></div>
            <div id="d"></div>
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
        // #a — longhands + ambos shorthands. NOTA: view-timeline-inset:10px 30%
        // arranca el bloque, pero view-timeline (shorthand) viene DESPUÉS en
        // la cascada y reescribe inset a 5px/5px (1 valor → ambos lados).
        let cs_a = eng.compute(&by("a"));
        assert!(!cs_a.font_synthesis.position);
        assert_eq!(cs_a.view_timeline_inset_start, LengthVal::Px(5.0));
        assert_eq!(cs_a.view_timeline_inset_end, LengthVal::Px(5.0));
        assert_eq!(cs_a.scroll_timeline_name.as_deref(), Some("--st"));
        assert_eq!(cs_a.scroll_timeline_axis, TimelineAxis::Inline);
        assert_eq!(cs_a.view_timeline_name.as_deref(), Some("--vt"));
        assert_eq!(cs_a.view_timeline_axis, TimelineAxis::Y);
        assert_eq!(cs_a.interactivity, Interactivity::Inert);
        // El hijo HEREDA interactivity y font-synthesis (incluyendo el axis
        // `position`); NO hereda view-timeline-{name,axis,inset}.
        let cs_a_child = eng.compute_with_parent(&by("a-child"), Some(&cs_a));
        assert_eq!(cs_a_child.interactivity, Interactivity::Inert);
        assert!(!cs_a_child.font_synthesis.position);
        assert_eq!(cs_a_child.view_timeline_name, None);
        assert_eq!(cs_a_child.view_timeline_axis, TimelineAxis::Block);
        assert_eq!(cs_a_child.view_timeline_inset_start, LengthVal::Px(0.0));
        // #b — view-timeline shorthand sólo con name; axis=Block, inset=0.
        let cs_b = eng.compute(&by("b"));
        assert_eq!(cs_b.view_timeline_name.as_deref(), Some("--b"));
        assert_eq!(cs_b.view_timeline_axis, TimelineAxis::Block);
        assert_eq!(cs_b.view_timeline_inset_start, LengthVal::Px(0.0));
        assert_eq!(cs_b.view_timeline_inset_end, LengthVal::Px(0.0));
        // #c — view-timeline-inset 1 valor → ambos lados.
        let cs_c = eng.compute(&by("c"));
        assert_eq!(cs_c.view_timeline_inset_start, LengthVal::Px(8.0));
        assert_eq!(cs_c.view_timeline_inset_end, LengthVal::Px(8.0));
        // #d — defaults puros.
        let cs_d = eng.compute(&by("d"));
        assert_eq!(cs_d.interactivity, Interactivity::Auto);
        assert!(cs_d.font_synthesis.position);
        assert_eq!(cs_d.view_timeline_inset_start, LengthVal::Px(0.0));
        assert_eq!(cs_d.view_timeline_axis, TimelineAxis::Block);
    }

    #[test]
    fn anchor_position_try_fase_7_459_463() {
        // Cinco props: position-visibility (Fase 7.459), position-try-order
        // (Fase 7.460), position-try-fallbacks (Fase 7.461), position-try
        // shorthand (Fase 7.462), position-area (Fase 7.463). NO heredan.

        // 1) Parser de cabecera para fallbacks.
        assert_eq!(parse_position_try_fallbacks("none"), Some(Vec::new()));
        assert_eq!(
            parse_position_try_fallbacks("--top, --bottom"),
            Some(vec!["--top".to_string(), "--bottom".to_string()])
        );
        assert_eq!(
            parse_position_try_fallbacks("flip-block flip-inline, --side"),
            Some(vec![
                "flip-block flip-inline".to_string(),
                "--side".to_string(),
            ])
        );
        assert_eq!(parse_position_try_fallbacks(""), None);
        assert_eq!(parse_position_try_fallbacks("--a, , --b"), None);

        // 2) E2E — longhands + shorthand + position-area opaco.
        let html = r##"<html><head><style>
            #a { position-visibility: anchors-visible;
                 position-try-order: most-height;
                 position-try-fallbacks: --top, --bottom;
                 position-area: top span-all }
            #b { position-try: --right }
            #c { position-try: most-width --r1, --r2 }
            #d { position-area: none }
            #e {}
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
        // #a — longhands.
        let cs_a = eng.compute(&by("a"));
        assert_eq!(
            cs_a.position_visibility,
            PositionVisibility::AnchorsVisible
        );
        assert_eq!(cs_a.position_try_order, PositionTryOrder::MostHeight);
        assert_eq!(
            cs_a.position_try_fallbacks,
            vec!["--top".to_string(), "--bottom".to_string()]
        );
        assert_eq!(cs_a.position_area.as_deref(), Some("top span-all"));
        // #b — shorthand sin order explícito → Normal + fallbacks.
        let cs_b = eng.compute(&by("b"));
        assert_eq!(cs_b.position_try_order, PositionTryOrder::Normal);
        assert_eq!(cs_b.position_try_fallbacks, vec!["--right".to_string()]);
        // #c — shorthand con order + dos fallbacks.
        let cs_c = eng.compute(&by("c"));
        assert_eq!(cs_c.position_try_order, PositionTryOrder::MostWidth);
        assert_eq!(
            cs_c.position_try_fallbacks,
            vec!["--r1".to_string(), "--r2".to_string()]
        );
        // #d — position-area: none → None.
        let cs_d = eng.compute(&by("d"));
        assert_eq!(cs_d.position_area, None);
        // #e — defaults puros.
        let cs_e = eng.compute(&by("e"));
        assert_eq!(cs_e.position_visibility, PositionVisibility::Always);
        assert_eq!(cs_e.position_try_order, PositionTryOrder::Normal);
        assert!(cs_e.position_try_fallbacks.is_empty());
        assert_eq!(cs_e.position_area, None);
    }

    #[test]
    fn animation_range_transitions_fase_7_464_468() {
        // Cinco props: animation-range-start (Fase 7.464), animation-range-end
        // (Fase 7.465), animation-range shorthand (Fase 7.466), transition-
        // behavior (Fase 7.467), interpolate-size (Fase 7.468). Sólo
        // interpolate-size HEREDA.

        // 1) Parser de cabecera.
        assert_eq!(parse_animation_range("normal"), Some(AnimationRange::Normal));
        assert_eq!(
            parse_animation_range("cover"),
            Some(AnimationRange::Named {
                phase: AnimationRangePhase::Cover,
                offset_pct: None,
            })
        );
        assert_eq!(
            parse_animation_range("entry 20%"),
            Some(AnimationRange::Named {
                phase: AnimationRangePhase::Entry,
                offset_pct: Some(20.0),
            })
        );
        assert_eq!(
            parse_animation_range("50%"),
            Some(AnimationRange::Length(LengthVal::Pct(50.0)))
        );
        assert_eq!(
            parse_animation_range("100px"),
            Some(AnimationRange::Length(LengthVal::Px(100.0)))
        );
        // Rechazo: dos longitudes, fase desconocida, vacío.
        assert_eq!(parse_animation_range("10px 20px"), None);
        assert_eq!(parse_animation_range("foo"), None);
        assert_eq!(parse_animation_range(""), None);
        // entry sin %: el segundo token NO es % → falla (no es length-or-pct).
        assert_eq!(parse_animation_range("entry foo"), None);

        // 2) E2E.
        let html = r##"<html><head><style>
            #a { animation-range-start: cover;
                 animation-range-end: contain 80%;
                 transition-behavior: allow-discrete;
                 interpolate-size: allow-keywords }
            #b { animation-range: entry 0% exit 100% }
            #c { animation-range: 50% }
            #d {}
        </style></head><body>
            <div id="a"><span id="a-child"></span></div>
            <div id="b"></div>
            <div id="c"></div>
            <div id="d"></div>
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
        // #a — longhands.
        let cs_a = eng.compute(&by("a"));
        assert_eq!(
            cs_a.animation_range_start,
            AnimationRange::Named {
                phase: AnimationRangePhase::Cover,
                offset_pct: None,
            }
        );
        assert_eq!(
            cs_a.animation_range_end,
            AnimationRange::Named {
                phase: AnimationRangePhase::Contain,
                offset_pct: Some(80.0),
            }
        );
        assert_eq!(cs_a.transition_behavior, TransitionBehavior::AllowDiscrete);
        assert_eq!(cs_a.interpolate_size, InterpolateSize::AllowKeywords);
        // El hijo HEREDA interpolate-size pero NO transition-behavior.
        let cs_a_child = eng.compute_with_parent(&by("a-child"), Some(&cs_a));
        assert_eq!(cs_a_child.interpolate_size, InterpolateSize::AllowKeywords);
        assert_eq!(
            cs_a_child.transition_behavior,
            TransitionBehavior::Normal
        );
        // El hijo TAMPOCO hereda animation-range-{start,end}.
        assert_eq!(cs_a_child.animation_range_start, AnimationRange::Normal);
        // #b — shorthand con start y end ambos compuestos (fase + offset).
        let cs_b = eng.compute(&by("b"));
        assert_eq!(
            cs_b.animation_range_start,
            AnimationRange::Named {
                phase: AnimationRangePhase::Entry,
                offset_pct: Some(0.0),
            }
        );
        assert_eq!(
            cs_b.animation_range_end,
            AnimationRange::Named {
                phase: AnimationRangePhase::Exit,
                offset_pct: Some(100.0),
            }
        );
        // #c — shorthand de 1 lado: end ≡ start.
        let cs_c = eng.compute(&by("c"));
        assert_eq!(
            cs_c.animation_range_start,
            AnimationRange::Length(LengthVal::Pct(50.0))
        );
        assert_eq!(
            cs_c.animation_range_end,
            AnimationRange::Length(LengthVal::Pct(50.0))
        );
        // #d — defaults puros.
        let cs_d = eng.compute(&by("d"));
        assert_eq!(cs_d.animation_range_start, AnimationRange::Normal);
        assert_eq!(cs_d.animation_range_end, AnimationRange::Normal);
        assert_eq!(cs_d.transition_behavior, TransitionBehavior::Normal);
        assert_eq!(cs_d.interpolate_size, InterpolateSize::NumericOnly);
    }

    #[test]
    fn offset_extras_ruby_overhang_fase_7_449_453() {
        // Cinco props: offset-rotate, offset-anchor, offset-position,
        // object-view-box (NO heredan), ruby-overhang (HEREDA).

        // 1) Parsers de cabecera — offset-rotate.
        assert_eq!(
            parse_offset_rotate("auto"),
            Some(OffsetRotate { auto: true, reverse: false, angle_deg: 0.0 })
        );
        assert_eq!(
            parse_offset_rotate("reverse"),
            Some(OffsetRotate { auto: false, reverse: true, angle_deg: 0.0 })
        );
        assert_eq!(
            parse_offset_rotate("90deg"),
            Some(OffsetRotate { auto: false, reverse: false, angle_deg: 90.0 })
        );
        assert_eq!(
            parse_offset_rotate("auto 45deg"),
            Some(OffsetRotate { auto: true, reverse: false, angle_deg: 45.0 })
        );
        assert_eq!(
            parse_offset_rotate("0.5turn"),
            Some(OffsetRotate { auto: false, reverse: false, angle_deg: 180.0 })
        );
        // Rechazos: vacío, dos keywords, dos ángulos.
        assert_eq!(parse_offset_rotate(""), None);
        assert_eq!(parse_offset_rotate("auto reverse"), None);
        assert_eq!(parse_offset_rotate("90deg 45deg"), None);

        // 2) E2E.
        let html = r##"<html><head><style>
            body { ruby-overhang: none }
            #a {
                offset-rotate: auto 30deg;
                offset-anchor: 10% 25%;
                offset-position: center top;
                object-view-box: inset(10% 20% round 5px)
            }
            #b { offset-anchor: auto; offset-position: normal; object-view-box: none }
            #c {}
        </style></head><body>
            <div id="a"></div>
            <div id="b"></div>
            <div id="c"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let by = |id: &str| {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.ruby_overhang, RubyOverhang::None);
        // #a — todos los longhands aplican.
        let cs_a = eng.compute_with_parent(&by("a"), Some(&body_cs));
        assert_eq!(
            cs_a.offset_rotate,
            OffsetRotate { auto: true, reverse: false, angle_deg: 30.0 }
        );
        assert_eq!(
            cs_a.offset_anchor,
            Some(BackgroundPosition { x: LengthVal::Pct(10.0), y: LengthVal::Pct(25.0) })
        );
        // `center top` → ejes (50%, 0%).
        assert_eq!(
            cs_a.offset_position,
            Some(BackgroundPosition { x: LengthVal::Pct(50.0), y: LengthVal::Pct(0.0) })
        );
        assert_eq!(
            cs_a.object_view_box.as_deref(),
            Some("inset(10% 20% round 5px)")
        );
        // ruby-overhang HEREDA.
        assert_eq!(cs_a.ruby_overhang, RubyOverhang::None);
        // #b — auto/normal/none.
        let cs_b = eng.compute_with_parent(&by("b"), Some(&body_cs));
        assert_eq!(cs_b.offset_anchor, None);
        assert_eq!(cs_b.offset_position, None);
        assert_eq!(cs_b.object_view_box, None);
        // #c — default offset-rotate (auto, 0).
        let cs_c = eng.compute_with_parent(&by("c"), Some(&body_cs));
        assert_eq!(cs_c.offset_rotate, OffsetRotate::default());
        // ruby-overhang sigue heredado.
        assert_eq!(cs_c.ruby_overhang, RubyOverhang::None);
    }

