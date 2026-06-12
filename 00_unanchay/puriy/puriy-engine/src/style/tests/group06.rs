//! Tests del motor de estilo (grupo 06, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn stroke_opacity_fase_7_372() {
        let html = r##"<html><head><style>
            body { stroke-opacity: 25% }
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
        assert_eq!(cs.stroke_opacity, 0.25);
    }

    #[test]
    fn stroke_width_fase_7_373() {
        let html = r##"<html><head><style>
            body { stroke-width: 3px }
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
        assert_eq!(body_cs.stroke_width, LengthVal::Px(3.0));
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).stroke_width,
            LengthVal::Px(3.0)
        );
    }

    #[test]
    fn stroke_linecap_fase_7_374() {
        assert_eq!(parse_stroke_linecap("butt"), Some(StrokeLinecap::Butt));
        assert_eq!(parse_stroke_linecap("ROUND"), Some(StrokeLinecap::Round));
        assert_eq!(parse_stroke_linecap("square"), Some(StrokeLinecap::Square));
        assert_eq!(parse_stroke_linecap("nope"), None);

        let html = r##"<html><head><style>
            body { stroke-linecap: round }
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
        assert_eq!(body_cs.stroke_linecap, StrokeLinecap::Round);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).stroke_linecap,
            StrokeLinecap::Round
        );
    }

    #[test]
    fn stroke_linejoin_fase_7_375() {
        assert_eq!(parse_stroke_linejoin("miter"), Some(StrokeLinejoin::Miter));
        assert_eq!(parse_stroke_linejoin("BEVEL"), Some(StrokeLinejoin::Bevel));
        assert_eq!(parse_stroke_linejoin("arcs"), Some(StrokeLinejoin::Arcs));
        assert_eq!(
            parse_stroke_linejoin("miter-clip"),
            Some(StrokeLinejoin::MiterClip)
        );
        assert_eq!(parse_stroke_linejoin("nope"), None);

        let html = r##"<html><head><style>
            body { stroke-linejoin: bevel }
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
        assert_eq!(cs.stroke_linejoin, StrokeLinejoin::Bevel);
    }

    #[test]
    fn stroke_miterlimit_fase_7_376() {
        assert_eq!(parse_stroke_miterlimit("10"), Some(10.0));
        assert_eq!(parse_stroke_miterlimit("1"), Some(1.0));
        // <1 descarta.
        assert_eq!(parse_stroke_miterlimit("0.5"), None);
        assert_eq!(parse_stroke_miterlimit("nope"), None);

        let html = r##"<html><head><style>
            body { stroke-miterlimit: 8 }
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
        assert_eq!(body_cs.stroke_miterlimit, 8.0);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).stroke_miterlimit,
            8.0
        );
    }

    #[test]
    fn stroke_dasharray_fase_7_377() {
        assert_eq!(parse_stroke_dasharray("none"), Some(Vec::new()));
        // Separado por espacios.
        assert_eq!(
            parse_stroke_dasharray("5 10"),
            Some(vec![LengthVal::Px(5.0), LengthVal::Px(10.0)])
        );
        // Separado por comas.
        assert_eq!(
            parse_stroke_dasharray("5, 10, 15"),
            Some(vec![
                LengthVal::Px(5.0),
                LengthVal::Px(10.0),
                LengthVal::Px(15.0)
            ])
        );
        // Mezcla espacios y comas.
        assert_eq!(
            parse_stroke_dasharray("5 10, 15"),
            Some(vec![
                LengthVal::Px(5.0),
                LengthVal::Px(10.0),
                LengthVal::Px(15.0)
            ])
        );
        assert_eq!(parse_stroke_dasharray("foo"), None);

        let html = r##"<html><head><style>
            body { stroke-dasharray: 4 6 }
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
            body_cs.stroke_dasharray,
            vec![LengthVal::Px(4.0), LengthVal::Px(6.0)]
        );
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).stroke_dasharray,
            vec![LengthVal::Px(4.0), LengthVal::Px(6.0)]
        );
    }

    #[test]
    fn stroke_dashoffset_fase_7_378() {
        let html = r##"<html><head><style>
            body { stroke-dashoffset: 12px }
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
        assert_eq!(body_cs.stroke_dashoffset, LengthVal::Px(12.0));
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).stroke_dashoffset,
            LengthVal::Px(12.0)
        );
    }

    #[test]
    fn fill_rule_fase_7_379() {
        assert_eq!(parse_fill_rule("nonzero"), Some(FillRule::Nonzero));
        assert_eq!(parse_fill_rule("EVENODD"), Some(FillRule::Evenodd));
        assert_eq!(parse_fill_rule("nope"), None);

        let html = r##"<html><head><style>
            body { fill-rule: evenodd }
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
        assert_eq!(body_cs.fill_rule, FillRule::Evenodd);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).fill_rule,
            FillRule::Evenodd
        );
    }

    #[test]
    fn clip_rule_fase_7_380() {
        let html = r##"<html><head><style>
            body { clip-rule: evenodd }
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
        assert_eq!(body_cs.clip_rule, FillRule::Evenodd);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).clip_rule,
            FillRule::Evenodd
        );
    }

    #[test]
    fn color_interpolation_fase_7_381() {
        assert_eq!(
            parse_color_interpolation("auto"),
            Some(ColorInterpolation::Auto)
        );
        assert_eq!(
            parse_color_interpolation("SRGB"),
            Some(ColorInterpolation::SRgb)
        );
        assert_eq!(
            parse_color_interpolation("linearRGB"),
            Some(ColorInterpolation::LinearRgb)
        );
        assert_eq!(parse_color_interpolation("nope"), None);

        let html = r##"<html><head><style>
            body { color-interpolation: linearRGB }
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
        assert_eq!(body_cs.color_interpolation, ColorInterpolation::LinearRgb);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).color_interpolation,
            ColorInterpolation::LinearRgb
        );
    }

    #[test]
    fn shape_rendering_fase_7_382() {
        assert_eq!(parse_shape_rendering("auto"), Some(ShapeRendering::Auto));
        assert_eq!(
            parse_shape_rendering("optimizeSpeed"),
            Some(ShapeRendering::OptimizeSpeed)
        );
        assert_eq!(
            parse_shape_rendering("CRISPEDGES"),
            Some(ShapeRendering::CrispEdges)
        );
        assert_eq!(
            parse_shape_rendering("geometricPrecision"),
            Some(ShapeRendering::GeometricPrecision)
        );
        assert_eq!(parse_shape_rendering("nope"), None);

        let html = r##"<html><head><style>
            body { shape-rendering: crispEdges }
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
        assert_eq!(body_cs.shape_rendering, ShapeRendering::CrispEdges);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).shape_rendering,
            ShapeRendering::CrispEdges
        );
    }

    #[test]
    fn vector_effect_fase_7_383() {
        assert_eq!(parse_vector_effect("none"), Some(VectorEffect::None));
        assert_eq!(
            parse_vector_effect("non-scaling-stroke"),
            Some(VectorEffect::NonScalingStroke)
        );
        assert_eq!(
            parse_vector_effect("FIXED-POSITION"),
            Some(VectorEffect::FixedPosition)
        );
        assert_eq!(parse_vector_effect("nope"), None);

        let html = r##"<html><head><style>
            body { vector-effect: non-scaling-stroke }
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
        assert_eq!(body_cs.vector_effect, VectorEffect::NonScalingStroke);
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).vector_effect,
            VectorEffect::None
        );
    }

    #[test]
    fn flood_color_fase_7_384() {
        assert_eq!(parse_color_or_current("currentColor"), Some(None));
        let red = parse_color_or_current("red").unwrap().unwrap();
        assert_eq!((red.r, red.g, red.b), (255, 0, 0));

        let html = r##"<html><head><style>
            body { flood-color: red }
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
        let c = body_cs.flood_color.unwrap();
        assert_eq!((c.r, c.g, c.b), (255, 0, 0));
        // NO hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).flood_color,
            None
        );
    }

    #[test]
    fn flood_opacity_fase_7_385() {
        let html = r##"<html><head><style>
            body { flood-opacity: 0.4 }
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
        assert_eq!(body_cs.flood_opacity, 0.4);
        // NO hereda — vuelve a 1.0.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).flood_opacity,
            1.0
        );
    }

    #[test]
    fn lighting_color_fase_7_386() {
        let html = r##"<html><head><style>
            body { lighting-color: rgb(0, 255, 0) }
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
        let c = cs.lighting_color.unwrap();
        assert_eq!((c.r, c.g, c.b), (0, 255, 0));
    }

    #[test]
    fn stop_color_fase_7_387() {
        let html = r##"<html><head><style>
            body { stop-color: blue }
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
        let c = cs.stop_color.unwrap();
        assert_eq!((c.r, c.g, c.b), (0, 0, 255));
    }

    #[test]
    fn stop_opacity_fase_7_388() {
        let html = r##"<html><head><style>
            body { stop-opacity: 0.7 }
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
        assert_eq!(cs.stop_opacity, 0.7);
    }

    #[test]
    fn text_anchor_fase_7_389() {
        assert_eq!(parse_text_anchor("start"), Some(TextAnchor::Start));
        assert_eq!(parse_text_anchor("MIDDLE"), Some(TextAnchor::Middle));
        assert_eq!(parse_text_anchor("end"), Some(TextAnchor::End));
        assert_eq!(parse_text_anchor("foo"), None);

        let html = r##"<html><head><style>
            body { text-anchor: middle }
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
        assert_eq!(body_cs.text_anchor, TextAnchor::Middle);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).text_anchor,
            TextAnchor::Middle
        );
    }

    #[test]
    fn color_rendering_fase_7_390() {
        assert_eq!(
            parse_color_rendering("auto"),
            Some(ColorRendering::Auto)
        );
        assert_eq!(
            parse_color_rendering("optimizeSpeed"),
            Some(ColorRendering::OptimizeSpeed)
        );
        assert_eq!(
            parse_color_rendering("OPTIMIZEQUALITY"),
            Some(ColorRendering::OptimizeQuality)
        );
        assert_eq!(parse_color_rendering("nope"), None);

        let html = r##"<html><head><style>
            body { color-rendering: optimizeQuality }
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
        assert_eq!(body_cs.color_rendering, ColorRendering::OptimizeQuality);
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).color_rendering,
            ColorRendering::OptimizeQuality
        );
    }

    #[test]
    fn color_interpolation_filters_fase_7_391() {
        assert_eq!(
            parse_color_interpolation_filters("auto"),
            Some(ColorInterpolationFilters::Auto)
        );
        assert_eq!(
            parse_color_interpolation_filters("sRGB"),
            Some(ColorInterpolationFilters::SRgb)
        );
        assert_eq!(
            parse_color_interpolation_filters("linearRGB"),
            Some(ColorInterpolationFilters::LinearRgb)
        );
        assert_eq!(parse_color_interpolation_filters("rgb"), None);

        // Default es linearRGB (spec) — diferente de color-interpolation.
        let dom = DomTree::parse("<html><body></body></html>");
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies.push(n.clone());
            }
        });
        assert_eq!(
            eng.compute(&bodies[0]).color_interpolation_filters,
            ColorInterpolationFilters::LinearRgb
        );

        let html = r##"<html><head><style>
            body { color-interpolation-filters: sRGB }
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
            body_cs.color_interpolation_filters,
            ColorInterpolationFilters::SRgb
        );
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .color_interpolation_filters,
            ColorInterpolationFilters::SRgb
        );
    }

    #[test]
    fn glyph_orientation_vertical_fase_7_392() {
        assert_eq!(
            parse_glyph_orientation_vertical("auto"),
            Some(GlyphOrientationVertical::Auto)
        );
        assert_eq!(
            parse_glyph_orientation_vertical("0"),
            Some(GlyphOrientationVertical::Deg0)
        );
        assert_eq!(
            parse_glyph_orientation_vertical("90deg"),
            Some(GlyphOrientationVertical::Deg90)
        );
        assert_eq!(
            parse_glyph_orientation_vertical("180DEG"),
            Some(GlyphOrientationVertical::Deg180)
        );
        assert_eq!(
            parse_glyph_orientation_vertical("270"),
            Some(GlyphOrientationVertical::Deg270)
        );
        // Sólo los 4 ángulos rectos.
        assert_eq!(parse_glyph_orientation_vertical("45"), None);
        assert_eq!(parse_glyph_orientation_vertical("nope"), None);

        let html = r##"<html><head><style>
            body { glyph-orientation-vertical: 90deg }
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
            body_cs.glyph_orientation_vertical,
            GlyphOrientationVertical::Deg90
        );
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .glyph_orientation_vertical,
            GlyphOrientationVertical::Deg90
        );
    }

    #[test]
    fn transform_box_fase_7_393() {
        assert_eq!(
            parse_transform_box("content-box"),
            Some(TransformBox::ContentBox)
        );
        assert_eq!(
            parse_transform_box("border-box"),
            Some(TransformBox::BorderBox)
        );
        assert_eq!(
            parse_transform_box("fill-box"),
            Some(TransformBox::FillBox)
        );
        assert_eq!(
            parse_transform_box("stroke-box"),
            Some(TransformBox::StrokeBox)
        );
        assert_eq!(
            parse_transform_box("VIEW-BOX"),
            Some(TransformBox::ViewBox)
        );
        assert_eq!(parse_transform_box("none"), None);

        let html = r##"<html><head><style>
            body { transform-box: fill-box }
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
        assert_eq!(body_cs.transform_box, TransformBox::FillBox);
        // NO hereda — vuelve al default ViewBox.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).transform_box,
            TransformBox::ViewBox
        );
    }

    #[test]
    fn marker_start_fase_7_394() {
        assert_eq!(parse_marker_ref("none"), Some(None));
        assert_eq!(
            parse_marker_ref("url(#m)"),
            Some(Some("url(#m)".to_string()))
        );
        assert_eq!(parse_marker_ref("xxx"), None);
        assert_eq!(parse_marker_ref("url"), None);
        assert_eq!(parse_marker_ref("url(#m"), None);

        let html = r##"<html><head><style>
            body { marker-start: url(#arrow) }
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
        assert_eq!(body_cs.marker_start.as_deref(), Some("url(#arrow)"));
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .marker_start
                .as_deref(),
            Some("url(#arrow)")
        );
    }

    #[test]
    fn marker_mid_fase_7_395() {
        let html = r##"<html><head><style>
            body { marker-mid: url(#dot) }
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
        assert_eq!(body_cs.marker_mid.as_deref(), Some("url(#dot)"));
        // SÍ hereda.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs))
                .marker_mid
                .as_deref(),
            Some("url(#dot)")
        );
    }

    #[test]
    fn marker_end_fase_7_396() {
        let html = r##"<html><head><style>
            body { marker-end: url(#arrow2) }
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
        assert_eq!(cs.marker_end.as_deref(), Some("url(#arrow2)"));
    }

    #[test]
    fn marker_shorthand_fase_7_397() {
        // El shorthand `marker` setea los 3 a la vez.
        let html = r##"<html><head><style>
            body { marker: url(#tri) }
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
        assert_eq!(cs.marker_start.as_deref(), Some("url(#tri)"));
        assert_eq!(cs.marker_mid.as_deref(), Some("url(#tri)"));
        assert_eq!(cs.marker_end.as_deref(), Some("url(#tri)"));

        // `marker: none` apaga los 3.
        let html2 = r##"<html><head><style>
            body { marker: none }
        </style></head><body></body></html>"##;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let mut bodies2 = Vec::new();
        crate::dom::walk(&dom2.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("body") {
                bodies2.push(n.clone());
            }
        });
        let cs2 = eng2.compute(&bodies2[0]);
        assert!(cs2.marker_start.is_none());
        assert!(cs2.marker_mid.is_none());
        assert!(cs2.marker_end.is_none());
    }

    #[test]
    fn mask_type_fase_7_398() {
        assert_eq!(parse_mask_type("luminance"), Some(MaskType::Luminance));
        assert_eq!(parse_mask_type("ALPHA"), Some(MaskType::Alpha));
        assert_eq!(parse_mask_type("none"), None);

        let html = r##"<html><head><style>
            body { mask-type: alpha }
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
        assert_eq!(body_cs.mask_type, MaskType::Alpha);
        // NO hereda — vuelve a Luminance.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mask_type,
            MaskType::Luminance
        );
    }

    #[test]
    fn mask_mode_fase_7_399() {
        assert_eq!(parse_mask_mode("alpha"), Some(MaskMode::Alpha));
        assert_eq!(parse_mask_mode("LUMINANCE"), Some(MaskMode::Luminance));
        assert_eq!(
            parse_mask_mode("match-source"),
            Some(MaskMode::MatchSource)
        );
        assert_eq!(parse_mask_mode("nope"), None);

        let html = r##"<html><head><style>
            body { mask-mode: alpha }
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
        assert_eq!(body_cs.mask_mode, MaskMode::Alpha);
        // NO hereda — reset a `MatchSource`.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mask_mode,
            MaskMode::MatchSource
        );
    }

    #[test]
    fn mask_clip_fase_7_400() {
        assert_eq!(parse_mask_clip("border-box"), Some(MaskClip::BorderBox));
        assert_eq!(parse_mask_clip("PADDING-BOX"), Some(MaskClip::PaddingBox));
        assert_eq!(parse_mask_clip("fill-box"), Some(MaskClip::FillBox));
        assert_eq!(parse_mask_clip("no-clip"), Some(MaskClip::NoClip));
        assert_eq!(parse_mask_clip("nope"), None);

        let html = r##"<html><head><style>
            body { mask-clip: no-clip }
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
        assert_eq!(body_cs.mask_clip, MaskClip::NoClip);
        // NO hereda — reset a `BorderBox`.
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).mask_clip,
            MaskClip::BorderBox
        );
    }

    #[test]
    fn mask_composite_fase_7_401() {
        assert_eq!(
            parse_mask_composite("add"),
            Some(MaskComposite::Add)
        );
        assert_eq!(
            parse_mask_composite("SUBTRACT"),
            Some(MaskComposite::Subtract)
        );
        assert_eq!(
            parse_mask_composite("intersect"),
            Some(MaskComposite::Intersect)
        );
        assert_eq!(
            parse_mask_composite("exclude"),
            Some(MaskComposite::Exclude)
        );
        assert_eq!(parse_mask_composite("nope"), None);

        let html = r##"<html><head><style>
            body { mask-composite: exclude }
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
        assert_eq!(cs.mask_composite, MaskComposite::Exclude);
    }

