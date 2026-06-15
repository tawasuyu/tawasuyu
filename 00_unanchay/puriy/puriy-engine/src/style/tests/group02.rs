//! Tests del motor de estilo (grupo 02, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn font_stretch_fase_7_252() {
        // Keywords.
        assert!((parse_font_stretch("normal").unwrap() - 1.0).abs() < 1e-3);
        assert!((parse_font_stretch("CONDENSED").unwrap() - 0.75).abs() < 1e-3);
        assert!((parse_font_stretch("ultra-expanded").unwrap() - 2.0).abs() < 1e-3);
        assert!((parse_font_stretch("ultra-condensed").unwrap() - 0.50).abs() < 1e-3);
        // Porcentaje.
        assert!((parse_font_stretch("125%").unwrap() - 1.25).abs() < 1e-3);
        // Clamp: 300% → 200%.
        assert!((parse_font_stretch("300%").unwrap() - 2.0).abs() < 1e-3);
        assert!((parse_font_stretch("10%").unwrap() - 0.5).abs() < 1e-3);
        assert_eq!(parse_font_stretch("nope"), None);

        let html = r##"<html><head><style>
            body { font-stretch: expanded }
            p.c { font-stretch: 75% }
            p.plain {}
        </style></head><body>
          <p class="c"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert!((body_cs.font_stretch - 1.25).abs() < 1e-3);
        assert!(
            (eng.compute_with_parent(&ps[0], Some(&body_cs)).font_stretch - 0.75).abs() < 1e-3
        );
        // Heredado.
        assert!(
            (eng.compute_with_parent(&ps[1], Some(&body_cs)).font_stretch - 1.25).abs() < 1e-3
        );
    }

    #[test]
    fn image_rendering_fase_7_253() {
        assert_eq!(parse_image_rendering("auto"), Some(ImageRendering::Auto));
        assert_eq!(parse_image_rendering("SMOOTH"), Some(ImageRendering::Smooth));
        assert_eq!(parse_image_rendering("crisp-edges"), Some(ImageRendering::CrispEdges));
        assert_eq!(parse_image_rendering("pixelated"), Some(ImageRendering::Pixelated));
        // Legacy CSS2 → Auto.
        assert_eq!(parse_image_rendering("optimizeSpeed"), Some(ImageRendering::Auto));
        assert_eq!(parse_image_rendering("nope"), None);

        let html = r##"<html><head><style>
            body { image-rendering: pixelated }
            img.over { image-rendering: smooth }
            img.plain {}
        </style></head><body>
          <img class="over"/><img class="plain"/>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut imgs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("img") => imgs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.image_rendering, ImageRendering::Pixelated);
        assert_eq!(
            eng.compute_with_parent(&imgs[0], Some(&body_cs)).image_rendering,
            ImageRendering::Smooth
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&imgs[1], Some(&body_cs)).image_rendering,
            ImageRendering::Pixelated
        );
    }

    #[test]
    fn mix_blend_mode_fase_7_254() {
        assert_eq!(parse_blend_mode("normal"), Some(BlendMode::Normal));
        assert_eq!(parse_blend_mode("MULTIPLY"), Some(BlendMode::Multiply));
        assert_eq!(parse_blend_mode("color-dodge"), Some(BlendMode::ColorDodge));
        assert_eq!(parse_blend_mode("plus-lighter"), Some(BlendMode::PlusLighter));
        assert_eq!(parse_blend_mode("nope"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { mix-blend-mode: multiply }
            div.s { mix-blend-mode: screen }
            div.plain {}
        </style></head><body>
          <div class="s"></div><div class="plain"></div>
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
        assert_eq!(body_cs.mix_blend_mode, BlendMode::Multiply);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mix_blend_mode,
            BlendMode::Screen
        );
        // NO se hereda → default `Normal`.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).mix_blend_mode,
            BlendMode::Normal
        );
    }

    #[test]
    fn background_blend_mode_fase_7_255() {
        // Lista de varios modos.
        let list = parse_blend_mode_list("multiply, screen, OVERLAY");
        assert_eq!(
            list,
            vec![BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay]
        );
        // Inválidos individuales caen a Normal (no rompen la lista).
        let list2 = parse_blend_mode_list("multiply, BANANA, color");
        assert_eq!(
            list2,
            vec![BlendMode::Multiply, BlendMode::Normal, BlendMode::Color]
        );

        let html = r##"<html><head><style>
            div.bg { background-blend-mode: multiply, screen }
        </style></head><body><div class="bg"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let cs = eng.compute(&divs[0]);
        assert_eq!(
            cs.background_blend_mode,
            vec![BlendMode::Multiply, BlendMode::Screen]
        );
    }

    #[test]
    fn isolation_fase_7_256() {
        assert_eq!(parse_isolation("auto"), Some(Isolation::Auto));
        assert_eq!(parse_isolation("ISOLATE"), Some(Isolation::Isolate));
        assert_eq!(parse_isolation("nope"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { isolation: isolate }
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
        assert_eq!(body_cs.isolation, Isolation::Isolate);
        // Default Auto en el hijo.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).isolation,
            Isolation::Auto
        );
    }

    #[test]
    fn will_change_fase_7_257() {
        // `auto` y `auto, x` se aplanan: `auto` se descarta.
        assert!(parse_will_change("auto").is_empty());
        assert_eq!(
            parse_will_change("scroll-position, contents"),
            vec![WillChangeHint::ScrollPosition, WillChangeHint::Contents]
        );
        // Property arbitraria conservada lowercase.
        assert_eq!(
            parse_will_change("Transform, OPACITY"),
            vec![
                WillChangeHint::Property("transform".to_string()),
                WillChangeHint::Property("opacity".to_string()),
            ]
        );

        // NO se hereda.
        let html = r##"<html><head><style>
            body { will-change: transform }
            div.over { will-change: scroll-position }
            div.plain {}
        </style></head><body>
          <div class="over"></div><div class="plain"></div>
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
            body_cs.will_change,
            vec![WillChangeHint::Property("transform".to_string())]
        );
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).will_change,
            vec![WillChangeHint::ScrollPosition]
        );
        // NO se hereda → vacío.
        assert!(
            eng.compute_with_parent(&divs[1], Some(&body_cs))
                .will_change
                .is_empty()
        );
    }

    #[test]
    fn appearance_fase_7_258() {
        assert_eq!(parse_appearance("none"), Some(Appearance::None));
        assert_eq!(parse_appearance("AUTO"), Some(Appearance::Auto));
        assert_eq!(parse_appearance("textfield"), Some(Appearance::Textfield));
        assert_eq!(
            parse_appearance("menulist-button"),
            Some(Appearance::MenulistButton)
        );
        // Compat legacy → Auto.
        assert_eq!(parse_appearance("searchfield"), Some(Appearance::Auto));
        assert_eq!(parse_appearance("nope"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { appearance: none }
            input.btn { -webkit-appearance: button }
            input.plain {}
        </style></head><body>
          <input class="btn"/><input class="plain"/>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut inputs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("input") => inputs.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.appearance, Appearance::None);
        assert_eq!(
            eng.compute_with_parent(&inputs[0], Some(&body_cs)).appearance,
            Appearance::Button
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&inputs[1], Some(&body_cs)).appearance,
            Appearance::Auto
        );
    }

    #[test]
    fn font_kerning_fase_7_259() {
        assert_eq!(parse_font_kerning("auto"), Some(FontKerning::Auto));
        assert_eq!(parse_font_kerning("NORMAL"), Some(FontKerning::Normal));
        assert_eq!(parse_font_kerning("none"), Some(FontKerning::None));
        assert_eq!(parse_font_kerning("xx"), None);

        let html = r##"<html><head><style>
            body { font-kerning: normal }
            p.off { font-kerning: none }
            p.plain {}
        </style></head><body>
          <p class="off"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_kerning, FontKerning::Normal);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).font_kerning,
            FontKerning::None
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).font_kerning,
            FontKerning::Normal
        );
    }

    #[test]
    fn font_feature_settings_fase_7_260() {
        // `normal` → vacío.
        assert!(parse_font_feature_settings("normal").is_empty());
        // Default value = 1.
        let r = parse_font_feature_settings("\"liga\"");
        assert_eq!(r, vec![FontFeatureSetting { tag: *b"liga", value: 1 }]);
        // on/off + número.
        let r2 = parse_font_feature_settings("\"liga\" on, \"smcp\" off, \"ss01\" 2");
        assert_eq!(
            r2,
            vec![
                FontFeatureSetting { tag: *b"liga", value: 1 },
                FontFeatureSetting { tag: *b"smcp", value: 0 },
                FontFeatureSetting { tag: *b"ss01", value: 2 },
            ]
        );
        // Tags inválidas (longitud) se descartan.
        let r3 = parse_font_feature_settings("\"abc\", \"lig\"");
        assert!(r3.is_empty());

        let html = r##"<html><head><style>
            body { font-feature-settings: "liga" on }
            p.over { font-feature-settings: "smcp" }
        </style></head><body>
          <p class="over"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(
            body_cs.font_feature_settings,
            vec![FontFeatureSetting { tag: *b"liga", value: 1 }]
        );
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).font_feature_settings,
            vec![FontFeatureSetting { tag: *b"smcp", value: 1 }]
        );
    }

    #[test]
    fn font_variation_settings_fase_7_261() {
        assert!(parse_font_variation_settings("normal").is_empty());
        let r = parse_font_variation_settings("\"wght\" 700, \"wdth\" 80, \"slnt\" -15.5");
        assert_eq!(r.len(), 3);
        assert_eq!(&r[0].tag, b"wght");
        assert!((r[0].value - 700.0).abs() < 1e-3);
        assert_eq!(&r[2].tag, b"slnt");
        assert!((r[2].value + 15.5).abs() < 1e-3);

        let html = r##"<html><head><style>
            body { font-variation-settings: "wght" 700 }
            p.plain {}
        </style></head><body><p class="plain"></p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_variation_settings.len(), 1);
        assert_eq!(&body_cs.font_variation_settings[0].tag, b"wght");
        // Heredado.
        let p_cs = eng.compute_with_parent(&ps[0], Some(&body_cs));
        assert_eq!(p_cs.font_variation_settings.len(), 1);
    }

    #[test]
    fn font_language_override_fase_7_262() {
        assert_eq!(parse_font_language_override("normal"), None);
        assert_eq!(
            parse_font_language_override("\"DEU\""),
            Some("DEU".to_string())
        );
        // Single-quote también.
        assert_eq!(
            parse_font_language_override("'TRK'"),
            Some("TRK".to_string())
        );
        // Sin comillas o vacío.
        assert_eq!(parse_font_language_override("DEU"), None);
        assert_eq!(parse_font_language_override("\"\""), None);

        let html = r##"<html><head><style>
            body { font-language-override: "DEU" }
            p.over { font-language-override: "ROM" }
            p.plain {}
        </style></head><body>
          <p class="over"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.font_language_override.as_deref(), Some("DEU"));
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs))
                .font_language_override
                .as_deref(),
            Some("ROM")
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs))
                .font_language_override
                .as_deref(),
            Some("DEU")
        );
    }

    #[test]
    fn text_rendering_fase_7_263() {
        assert_eq!(parse_text_rendering("auto"), Some(TextRendering::Auto));
        assert_eq!(
            parse_text_rendering("optimizeSpeed"),
            Some(TextRendering::OptimizeSpeed)
        );
        assert_eq!(
            parse_text_rendering("OptimizeLegibility"),
            Some(TextRendering::OptimizeLegibility)
        );
        assert_eq!(
            parse_text_rendering("geometricprecision"),
            Some(TextRendering::GeometricPrecision)
        );
        assert_eq!(parse_text_rendering("nope"), None);

        let html = r##"<html><head><style>
            body { text-rendering: optimizeLegibility }
            p.plain {}
        </style></head><body><p class="plain"></p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_rendering, TextRendering::OptimizeLegibility);
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).text_rendering,
            TextRendering::OptimizeLegibility
        );
    }

    #[test]
    fn filter_fase_7_264() {
        // `none` y vacío → vacío.
        assert!(parse_filter_list("none").is_empty());
        assert!(parse_filter_list("").is_empty());

        // Funciones simples.
        let r = parse_filter_list("blur(4px) brightness(120%) hue-rotate(45deg)");
        assert_eq!(r.len(), 3);
        assert!(matches!(r[0], FilterFn::Blur(v) if (v - 4.0).abs() < 1e-3));
        assert!(matches!(r[1], FilterFn::Brightness(v) if (v - 1.2).abs() < 1e-3));
        assert!(matches!(r[2], FilterFn::HueRotate(v) if (v - 45.0).abs() < 1e-3));

        // Número unitless + porcentaje.
        let r2 = parse_filter_list("opacity(0.5) saturate(50%) grayscale(1)");
        assert!(matches!(r2[0], FilterFn::Opacity(v) if (v - 0.5).abs() < 1e-3));
        assert!(matches!(r2[1], FilterFn::Saturate(v) if (v - 0.5).abs() < 1e-3));
        assert!(matches!(r2[2], FilterFn::Grayscale(v) if (v - 1.0).abs() < 1e-3));

        // hue-rotate con rad/turn.
        let r3 = parse_filter_list("hue-rotate(0.5turn) hue-rotate(3.14159rad)");
        assert!(matches!(r3[0], FilterFn::HueRotate(v) if (v - 180.0).abs() < 1e-1));
        assert!(matches!(r3[1], FilterFn::HueRotate(v) if (v - 180.0).abs() < 1.0));

        // drop-shadow reusa box-shadow.
        let r4 = parse_filter_list("drop-shadow(2px 3px red)");
        assert!(matches!(&r4[0], FilterFn::DropShadow(s) if (s.offset_x - 2.0).abs() < 1e-3));

        // Función desconocida descartada (sólo se queda la conocida).
        let r5 = parse_filter_list("nope(1) blur(2px) bogus(x)");
        assert_eq!(r5.len(), 1);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { filter: blur(2px) }
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
        assert_eq!(body_cs.filter.len(), 1);
        // NO se hereda → vacío.
        assert!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .filter
                .is_empty()
        );
    }

    #[test]
    fn backdrop_filter_fase_7_265() {
        let r = parse_filter_list("blur(8px) saturate(180%)");
        assert_eq!(r.len(), 2);

        let html = r##"<html><head><style>
            div.glass { -webkit-backdrop-filter: blur(10px) }
        </style></head><body><div class="glass"></div></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let cs = eng.compute(&divs[0]);
        assert_eq!(cs.backdrop_filter.len(), 1);
        assert!(matches!(cs.backdrop_filter[0], FilterFn::Blur(v) if (v - 10.0).abs() < 1e-3));
    }

    #[test]
    fn text_orientation_fase_7_266() {
        assert_eq!(parse_text_orientation("mixed"), Some(TextOrientation::Mixed));
        assert_eq!(parse_text_orientation("UPRIGHT"), Some(TextOrientation::Upright));
        assert_eq!(parse_text_orientation("sideways"), Some(TextOrientation::Sideways));
        assert_eq!(
            parse_text_orientation("sideways-right"),
            Some(TextOrientation::SidewaysRight)
        );
        assert_eq!(parse_text_orientation("nope"), None);

        let html = r##"<html><head><style>
            body { text-orientation: upright }
            p.over { text-orientation: sideways }
            p.plain {}
        </style></head><body>
          <p class="over"></p><p class="plain"></p>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("p") => ps.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.text_orientation, TextOrientation::Upright);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).text_orientation,
            TextOrientation::Sideways
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).text_orientation,
            TextOrientation::Upright
        );
    }

    #[test]
    fn overscroll_behavior_fase_7_267() {
        assert_eq!(parse_overscroll_behavior("auto"), Some(OverscrollBehavior::Auto));
        assert_eq!(parse_overscroll_behavior("CONTAIN"), Some(OverscrollBehavior::Contain));
        assert_eq!(parse_overscroll_behavior("none"), Some(OverscrollBehavior::None));
        assert_eq!(parse_overscroll_behavior("nope"), None);

        // Shorthand: `contain none` → x=contain, y=none. `auto` solo → x=y=auto.
        let html = r##"<html><head><style>
            body { overscroll-behavior: contain none }
            div.solo { overscroll-behavior: contain }
            div.split { overscroll-behavior-x: none; overscroll-behavior-y: auto }
            div.plain {}
        </style></head><body>
          <div class="solo"></div><div class="split"></div><div class="plain"></div>
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
        // 2-valor: x=contain, y=none.
        assert_eq!(body_cs.overscroll_behavior_x, OverscrollBehavior::Contain);
        assert_eq!(body_cs.overscroll_behavior_y, OverscrollBehavior::None);
        // 1-valor: x=y=contain.
        let solo_cs = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(solo_cs.overscroll_behavior_x, OverscrollBehavior::Contain);
        assert_eq!(solo_cs.overscroll_behavior_y, OverscrollBehavior::Contain);
        // Longhands separadas.
        let split_cs = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(split_cs.overscroll_behavior_x, OverscrollBehavior::None);
        assert_eq!(split_cs.overscroll_behavior_y, OverscrollBehavior::Auto);
        // NO se hereda → default Auto.
        let plain_cs = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert_eq!(plain_cs.overscroll_behavior_x, OverscrollBehavior::Auto);
        assert_eq!(plain_cs.overscroll_behavior_y, OverscrollBehavior::Auto);
    }

    #[test]
    fn scroll_snap_type_fase_7_268() {
        assert_eq!(parse_scroll_snap_type("none"), Some(ScrollSnapType(None)));
        assert_eq!(
            parse_scroll_snap_type("x"),
            Some(ScrollSnapType(Some((ScrollSnapAxis::X, ScrollSnapStrictness::Proximity))))
        );
        assert_eq!(
            parse_scroll_snap_type("y mandatory"),
            Some(ScrollSnapType(Some((ScrollSnapAxis::Y, ScrollSnapStrictness::Mandatory))))
        );
        assert_eq!(
            parse_scroll_snap_type("BOTH proximity"),
            Some(ScrollSnapType(Some((ScrollSnapAxis::Both, ScrollSnapStrictness::Proximity))))
        );
        assert_eq!(parse_scroll_snap_type("xy"), None);

        // NO se hereda.
        let html = r##"<html><head><style>
            body { scroll-snap-type: y mandatory }
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
            body_cs.scroll_snap_type,
            ScrollSnapType(Some((ScrollSnapAxis::Y, ScrollSnapStrictness::Mandatory)))
        );
        // NO se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_snap_type,
            ScrollSnapType(None)
        );
    }

    #[test]
    fn scroll_snap_align_fase_7_269() {
        assert_eq!(parse_scroll_snap_align("none"), Some(ScrollSnapAlign::None));
        assert_eq!(parse_scroll_snap_align("START"), Some(ScrollSnapAlign::Start));
        assert_eq!(parse_scroll_snap_align("end"), Some(ScrollSnapAlign::End));
        assert_eq!(parse_scroll_snap_align("center"), Some(ScrollSnapAlign::Center));
        assert_eq!(parse_scroll_snap_align("middle"), None);

        // Shorthand: 1 valor → ambos ejes; 2 valores → block + inline.
        let html = r##"<html><head><style>
            body { scroll-snap-align: start end }
            div.solo { scroll-snap-align: center }
            div.split { scroll-snap-align-block: end; scroll-snap-align-inline: start }
            div.plain {}
        </style></head><body>
          <div class="solo"></div><div class="split"></div><div class="plain"></div>
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
        assert_eq!(body_cs.scroll_snap_align_block, ScrollSnapAlign::Start);
        assert_eq!(body_cs.scroll_snap_align_inline, ScrollSnapAlign::End);
        let solo = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(solo.scroll_snap_align_block, ScrollSnapAlign::Center);
        assert_eq!(solo.scroll_snap_align_inline, ScrollSnapAlign::Center);
        let split = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(split.scroll_snap_align_block, ScrollSnapAlign::End);
        assert_eq!(split.scroll_snap_align_inline, ScrollSnapAlign::Start);
        // NO se hereda → default None.
        let plain = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert_eq!(plain.scroll_snap_align_block, ScrollSnapAlign::None);
        assert_eq!(plain.scroll_snap_align_inline, ScrollSnapAlign::None);
    }

    #[test]
    fn scroll_snap_stop_fase_7_270() {
        assert_eq!(parse_scroll_snap_stop("normal"), Some(ScrollSnapStop::Normal));
        assert_eq!(parse_scroll_snap_stop("ALWAYS"), Some(ScrollSnapStop::Always));
        assert_eq!(parse_scroll_snap_stop("never"), None);

        let html = r##"<html><head><style>
            body { scroll-snap-stop: always }
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
        assert_eq!(body_cs.scroll_snap_stop, ScrollSnapStop::Always);
        // NO se hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_snap_stop,
            ScrollSnapStop::Normal
        );
    }

    #[test]
    fn scroll_padding_fase_7_271() {
        // Sides 1–4 valores con `LengthVal`.
        assert_eq!(
            parse_sides_lp("10px"),
            Some(Sides {
                top: LengthVal::Px(10.0),
                right: LengthVal::Px(10.0),
                bottom: LengthVal::Px(10.0),
                left: LengthVal::Px(10.0),
            })
        );
        assert_eq!(
            parse_sides_lp("auto 5%"),
            Some(Sides {
                top: LengthVal::Auto,
                right: LengthVal::Pct(5.0),
                bottom: LengthVal::Auto,
                left: LengthVal::Pct(5.0),
            })
        );
        assert!(parse_sides_lp("nope").is_none());

        let html = r##"<html><head><style>
            body { scroll-padding: 10px 20px 30px 40px }
            div.lh { scroll-padding-top: 5px; scroll-padding-left: 15% }
            div.plain {}
        </style></head><body>
          <div class="lh"></div><div class="plain"></div>
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
        assert_eq!(body_cs.scroll_padding.top, LengthVal::Px(10.0));
        assert_eq!(body_cs.scroll_padding.right, LengthVal::Px(20.0));
        assert_eq!(body_cs.scroll_padding.bottom, LengthVal::Px(30.0));
        assert_eq!(body_cs.scroll_padding.left, LengthVal::Px(40.0));
        // Longhands sobre default (NO se hereda del body — empieza en auto).
        let lh = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(lh.scroll_padding.top, LengthVal::Px(5.0));
        assert_eq!(lh.scroll_padding.right, LengthVal::Auto);
        assert_eq!(lh.scroll_padding.bottom, LengthVal::Auto);
        assert_eq!(lh.scroll_padding.left, LengthVal::Pct(15.0));
        // NO se hereda → todos auto.
        let plain = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(plain.scroll_padding.top, LengthVal::Auto);
        assert_eq!(plain.scroll_padding.left, LengthVal::Auto);
    }

    #[test]
    fn scroll_margin_fase_7_272() {
        let html = r##"<html><head><style>
            body { scroll-margin: 8px 16px }
            div.full { scroll-margin: 1px 2px 3px 4px }
            div.lh { scroll-margin-top: 7px; scroll-margin-right: 9px }
            div.plain {}
        </style></head><body>
          <div class="full"></div><div class="lh"></div><div class="plain"></div>
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
        // 2-valor: top/bottom=8, left/right=16.
        assert_eq!(body_cs.scroll_margin.top, 8.0);
        assert_eq!(body_cs.scroll_margin.right, 16.0);
        assert_eq!(body_cs.scroll_margin.bottom, 8.0);
        assert_eq!(body_cs.scroll_margin.left, 16.0);
        // 4-valor explícito.
        let full = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(full.scroll_margin.top, 1.0);
        assert_eq!(full.scroll_margin.right, 2.0);
        assert_eq!(full.scroll_margin.bottom, 3.0);
        assert_eq!(full.scroll_margin.left, 4.0);
        // Longhands sobre default (NO hereda).
        let lh = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(lh.scroll_margin.top, 7.0);
        assert_eq!(lh.scroll_margin.right, 9.0);
        assert_eq!(lh.scroll_margin.bottom, 0.0);
        assert_eq!(lh.scroll_margin.left, 0.0);
        // NO se hereda → todo 0.
        let plain = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert_eq!(plain.scroll_margin.top, 0.0);
        assert_eq!(plain.scroll_margin.right, 0.0);
    }

    #[test]
    fn touch_action_fase_7_273() {
        assert_eq!(parse_touch_action("auto"), Some(TouchAction::Auto));
        assert_eq!(parse_touch_action("NONE"), Some(TouchAction::None));
        assert_eq!(parse_touch_action("manipulation"), Some(TouchAction::Manipulation));
        assert_eq!(
            parse_touch_action("pan-x"),
            Some(TouchAction::Pan { pan_x: true, pan_y: false, pinch_zoom: false })
        );
        // `pan-left` se aplasta a pan-x; `pan-up` a pan-y; combinable con pinch-zoom.
        assert_eq!(
            parse_touch_action("pan-left pan-up pinch-zoom"),
            Some(TouchAction::Pan { pan_x: true, pan_y: true, pinch_zoom: true })
        );
        // Token inválido descarta la regla entera.
        assert_eq!(parse_touch_action("pan-x bogus"), None);
        // Sin pan ni pinch-zoom no es válido (no debería pasar por el path
        // de palabras sueltas, pero por las dudas).
        assert_eq!(parse_touch_action(""), None);

        let html = r##"<html><head><style>
            body { touch-action: pan-y pinch-zoom }
            div.none { touch-action: none }
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
        assert_eq!(
            body_cs.touch_action,
            TouchAction::Pan { pan_x: false, pan_y: true, pinch_zoom: true }
        );
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).touch_action,
            TouchAction::None
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).touch_action,
            TouchAction::Auto
        );
    }

    #[test]
    fn clip_path_fase_7_274() {
        assert!(parse_clip_path("none").is_none());
        assert!(parse_clip_path("").is_none());
        // inset 1-valor.
        let r = parse_clip_path("inset(10px)").unwrap();
        assert_eq!(
            r,
            ClipPath::Inset { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0, radius: 0.0 }
        );
        // inset 4-valor con `round`.
        let r = parse_clip_path("inset(1px 2px 3px 4px round 5px)").unwrap();
        assert_eq!(
            r,
            ClipPath::Inset { top: 1.0, right: 2.0, bottom: 3.0, left: 4.0, radius: 5.0 }
        );
        // circle con radio px + centro.
        let r = parse_clip_path("circle(30px at 50% 50%)").unwrap();
        assert_eq!(
            r,
            ClipPath::Circle {
                radius: ClipRadius::Len(LengthVal::Px(30.0)),
                cx: LengthVal::Pct(50.0),
                cy: LengthVal::Pct(50.0)
            }
        );
        // circle con radio % (Fase 7.1221: antes no parseaba → clip perdido).
        let r = parse_clip_path("circle(50%)").unwrap();
        assert_eq!(
            r,
            ClipPath::Circle {
                radius: ClipRadius::Len(LengthVal::Pct(50.0)),
                cx: LengthVal::Pct(50.0),
                cy: LengthVal::Pct(50.0)
            }
        );
        // circle() vacío → closest-side (default de la spec); keyword explícito.
        assert_eq!(
            parse_clip_path("circle()").unwrap(),
            ClipPath::Circle {
                radius: ClipRadius::ClosestSide,
                cx: LengthVal::Pct(50.0),
                cy: LengthVal::Pct(50.0)
            }
        );
        assert!(matches!(
            parse_clip_path("circle(farthest-side at 0 0)").unwrap(),
            ClipPath::Circle { radius: ClipRadius::FarthestSide, .. }
        ));
        // ellipse default centro, radios px.
        let r = parse_clip_path("ellipse(20px 10px)").unwrap();
        assert_eq!(
            r,
            ClipPath::Ellipse {
                rx: ClipRadius::Len(LengthVal::Px(20.0)),
                ry: ClipRadius::Len(LengthVal::Px(10.0)),
                cx: LengthVal::Pct(50.0),
                cy: LengthVal::Pct(50.0)
            }
        );
        // ellipse radios % → resuelven contra ancho/alto en el compositor.
        let r = parse_clip_path("ellipse(25% 40%)").unwrap();
        assert_eq!(
            r,
            ClipPath::Ellipse {
                rx: ClipRadius::Len(LengthVal::Pct(25.0)),
                ry: ClipRadius::Len(LengthVal::Pct(40.0)),
                cx: LengthVal::Pct(50.0),
                cy: LengthVal::Pct(50.0)
            }
        );
        // ellipse con keywords de lado mixtos.
        assert_eq!(
            parse_clip_path("ellipse(farthest-side closest-side)").unwrap(),
            ClipPath::Ellipse {
                rx: ClipRadius::FarthestSide,
                ry: ClipRadius::ClosestSide,
                cx: LengthVal::Pct(50.0),
                cy: LengthVal::Pct(50.0)
            }
        );
        // polygon() (Fase 7.1223): triángulo px+%, fill-rule nonzero default.
        assert_eq!(
            parse_clip_path("polygon(0 0, 100% 0, 50% 100%)").unwrap(),
            ClipPath::Polygon {
                evenodd: false,
                points: vec![
                    (LengthVal::Px(0.0), LengthVal::Px(0.0)),
                    (LengthVal::Pct(100.0), LengthVal::Px(0.0)),
                    (LengthVal::Pct(50.0), LengthVal::Pct(100.0)),
                ]
            }
        );
        // polygon con fill-rule evenodd explícito.
        assert_eq!(
            parse_clip_path("polygon(evenodd, 0 0, 10px 20px)").unwrap(),
            ClipPath::Polygon {
                evenodd: true,
                points: vec![
                    (LengthVal::Px(0.0), LengthVal::Px(0.0)),
                    (LengthVal::Px(10.0), LengthVal::Px(20.0)),
                ]
            }
        );
        // polygon vacío o vértice con 1 sola coord → None.
        assert!(parse_clip_path("polygon()").is_none());
        assert!(parse_clip_path("polygon(0 0, 5px)").is_none());
        // path() (Fase 7.1224): string crudo con comillas simples o dobles.
        assert_eq!(
            parse_clip_path("path('M0 0 L10 0 L10 10 Z')").unwrap(),
            ClipPath::Path { evenodd: false, d: "M0 0 L10 0 L10 10 Z".to_string() }
        );
        assert_eq!(
            parse_clip_path("path(evenodd, \"M0 0 L5 5\")").unwrap(),
            ClipPath::Path { evenodd: true, d: "M0 0 L5 5".to_string() }
        );
        // path sin comillas o vacío → None.
        assert!(parse_clip_path("path(M0 0)").is_none());
        assert!(parse_clip_path("path('')").is_none());
        // Función desconocida → None.
        assert!(parse_clip_path("ray(45deg)").is_none());

        // e2e: body con clip-path, div sin → NO se hereda.
        let html = r##"<html><head><style>
            body { clip-path: circle(50px) }
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
        assert!(matches!(body_cs.clip_path, Some(ClipPath::Circle { .. })));
        assert!(eng.compute_with_parent(&divs[0], Some(&body_cs)).clip_path.is_none());
    }

    #[test]
    fn mask_image_fase_7_275() {
        assert!(parse_mask_image("none").is_none());
        assert!(parse_mask_image("").is_none());
        assert_eq!(
            parse_mask_image("url(mask.png)"),
            Some(MaskImage::Url("mask.png".to_string()))
        );
        assert_eq!(
            parse_mask_image("url(\"m.svg\")"),
            Some(MaskImage::Url("m.svg".to_string()))
        );
        // Lo que no es `url(...)` cae a None (subset).
        assert!(parse_mask_image("linear-gradient(red, blue)").is_none());

        // Alias `-webkit-mask` y shorthand `mask` redirigen al subset url-only.
        let html = r##"<html><head><style>
            body { mask: url(body.png) }
            div.legacy { -webkit-mask-image: url('legacy.png') }
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
        assert_eq!(body_cs.mask_image, Some(MaskImage::Url("body.png".to_string())));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mask_image,
            Some(MaskImage::Url("legacy.png".to_string()))
        );
        // NO se hereda.
        assert!(eng.compute_with_parent(&divs[1], Some(&body_cs)).mask_image.is_none());
    }

    #[test]
    fn content_visibility_fase_7_276() {
        assert_eq!(parse_content_visibility("visible"), Some(ContentVisibility::Visible));
        assert_eq!(parse_content_visibility("AUTO"), Some(ContentVisibility::Auto));
        assert_eq!(parse_content_visibility("hidden"), Some(ContentVisibility::Hidden));
        assert_eq!(parse_content_visibility("nope"), None);

        let html = r##"<html><head><style>
            body { content-visibility: auto }
            div.h { content-visibility: hidden }
            div.plain {}
        </style></head><body>
          <div class="h"></div><div class="plain"></div>
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
        assert_eq!(body_cs.content_visibility, ContentVisibility::Auto);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).content_visibility,
            ContentVisibility::Hidden
        );
        // NO se hereda → default Visible.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).content_visibility,
            ContentVisibility::Visible
        );
    }

    #[test]
    fn contain_fase_7_277() {
        // Keywords compuestos.
        assert_eq!(parse_contain("none"), Some(ContainFlags::default()));
        assert_eq!(parse_contain("STRICT"), Some(ContainFlags::STRICT));
        assert_eq!(parse_contain("content"), Some(ContainFlags::CONTENT));
        // Bitset libre.
        let mixed = parse_contain("layout paint").unwrap();
        assert!(mixed.layout && mixed.paint);
        assert!(!mixed.size && !mixed.style);
        // `inline-size` también.
        assert!(parse_contain("inline-size").unwrap().inline_size);
        // Token inválido descarta.
        assert!(parse_contain("bogus").is_none());

        let html = r##"<html><head><style>
            body { contain: strict }
            div.lp { contain: layout paint }
            div.plain {}
        </style></head><body>
          <div class="lp"></div><div class="plain"></div>
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
        assert_eq!(body_cs.contain, ContainFlags::STRICT);
        let lp = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert!(lp.contain.layout && lp.contain.paint && !lp.contain.size);
        // NO se hereda → all-false.
        assert!(eng.compute_with_parent(&divs[1], Some(&body_cs)).contain.is_none());
    }

    #[test]
    fn column_count_fase_7_278() {
        assert_eq!(parse_column_count("auto"), None);
        assert_eq!(parse_column_count("3"), Some(3));
        assert_eq!(parse_column_count("0"), None);
        assert_eq!(parse_column_count("-2"), None);
        assert_eq!(parse_column_count("nope"), None);

        let html = r##"<html><head><style>
            body { column-count: 4 }
            div.auto { column-count: auto }
            div.plain {}
        </style></head><body>
          <div class="auto"></div><div class="plain"></div>
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
        assert_eq!(body_cs.column_count, Some(4));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_count,
            None
        );
        // NO se hereda → default None.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).column_count,
            None
        );
    }

    #[test]
    fn column_width_fase_7_279() {
        let html = r##"<html><head><style>
            body { column-width: 200px }
            div.auto { column-width: auto }
            div.pct { column-width: 30% }
            div.plain {}
        </style></head><body>
          <div class="auto"></div><div class="pct"></div><div class="plain"></div>
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
        assert_eq!(body_cs.column_width, LengthVal::Px(200.0));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_width,
            LengthVal::Auto
        );
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).column_width,
            LengthVal::Pct(30.0)
        );
        // NO se hereda → default Auto.
        assert_eq!(
            eng.compute_with_parent(&divs[2], Some(&body_cs)).column_width,
            LengthVal::Auto
        );
    }

    #[test]
    fn column_rule_fase_7_280() {
        // Longhands sueltos + shorthand. `currentColor` → None (defer al render).
        let html = r##"<html><head><style>
            body { column-rule: 2px dashed red }
            div.lh { column-rule-color: blue; column-rule-width: 3px; column-rule-style: dotted }
            div.cc { column-rule: 1px solid currentColor }
            div.none { column-rule: 4px solid black; column-rule-style: none }
            div.plain {}
        </style></head><body>
          <div class="lh"></div><div class="cc"></div><div class="none"></div><div class="plain"></div>
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
        assert_eq!(body_cs.column_rule_width, 2.0);
        assert_eq!(body_cs.column_rule_style, BorderLineStyle::Dashed);
        assert!(body_cs.column_rule_style_active);
        assert_eq!(body_cs.column_rule_color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Longhands.
        let lh = eng.compute_with_parent(&divs[0], Some(&body_cs));
        assert_eq!(lh.column_rule_width, 3.0);
        assert_eq!(lh.column_rule_style, BorderLineStyle::Dotted);
        assert_eq!(lh.column_rule_color.map(|c| (c.r, c.g, c.b)), Some((0, 0, 255)));
        // currentColor → None.
        let cc = eng.compute_with_parent(&divs[1], Some(&body_cs));
        assert_eq!(cc.column_rule_color, None);
        // `column-rule-style: none` apaga.
        let none = eng.compute_with_parent(&divs[2], Some(&body_cs));
        assert!(!none.column_rule_style_active);
        // NO se hereda → defaults (width 0, color None, style_active false).
        let plain = eng.compute_with_parent(&divs[3], Some(&body_cs));
        assert_eq!(plain.column_rule_width, 0.0);
        assert!(!plain.column_rule_style_active);
        assert_eq!(plain.column_rule_color, None);
    }

    #[test]
    fn column_fill_fase_7_281() {
        assert_eq!(parse_column_fill("auto"), Some(ColumnFill::Auto));
        assert_eq!(parse_column_fill("BALANCE"), Some(ColumnFill::Balance));
        assert_eq!(parse_column_fill("balance-all"), Some(ColumnFill::BalanceAll));
        assert_eq!(parse_column_fill("nope"), None);

        let html = r##"<html><head><style>
            body { column-fill: auto }
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
        assert_eq!(body_cs.column_fill, ColumnFill::Auto);
        // NO se hereda → default Balance.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).column_fill,
            ColumnFill::Balance
        );
    }

