//! Tests del motor de estilo (grupo 14, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn supports_query_filtra_por_parser() {
        assert!(evaluate_supports_query("(display: flex)"));
        assert!(evaluate_supports_query("(color: red)"));
        assert!(!evaluate_supports_query("(display: garbage)"));

        let html = r#"<html><head><style>
            @supports (display: flex) { p { color: green } }
            @supports (display: garbage) { p { color: red } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn supports_query_and_or_not_selector() {
        // and: ambas soportadas.
        assert!(evaluate_supports_query("(display: grid) and (color: red)"));
        assert!(!evaluate_supports_query("(display: grid) and (frobnicate: 1)"));
        // or: alguna soportada.
        assert!(evaluate_supports_query("(display: grid) or (frobnicate: 1)"));
        assert!(!evaluate_supports_query("(frob: 1) or (nicate: 2)"));
        // not.
        assert!(evaluate_supports_query("not (frobnicate: 1)"));
        assert!(!evaluate_supports_query("not (display: grid)"));
        // selector(): soportado si el selector parsea.
        assert!(evaluate_supports_query("selector(.a > .b)"));
        // agrupación anidada.
        assert!(evaluate_supports_query("((display: grid))"));
        assert!(evaluate_supports_query("(display: grid) and ((color: red) or (frob: 1))"));
        // @supports con `and` aplica el bloque end-to-end.
        let html = r#"<html><head><style>
            @supports (display: grid) and (color: red) { p { color: rgb(0,0,255) } }
            @supports (display: grid) and (frob: 1) { p { color: rgb(255,0,0) } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    // === Fase B1: @keyframes ===

    #[test]
    fn keyframes_from_to_se_parsean() {
        let html = r#"<html><head><style>
            @keyframes fade {
                from { opacity: 0; }
                to { opacity: 1; }
            }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("fade").expect("keyframes fade ausente");
        assert_eq!(kf.steps.len(), 2);
        assert_eq!(kf.steps[0].offset, 0.0);
        assert_eq!(kf.steps[0].declarations, vec![("opacity".into(), "0".into())]);
        assert_eq!(kf.steps[1].offset, 1.0);
        assert_eq!(kf.steps[1].declarations, vec![("opacity".into(), "1".into())]);
    }

    #[test]
    fn keyframes_porcentajes_y_orden() {
        // Pasos declarados fuera de orden deben quedar ordenados por offset.
        let html = r#"<html><head><style>
            @keyframes slide {
                100% { left: 100px; }
                0% { left: 0px; }
                50% { left: 40px; top: 10px; }
            }
        </style></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("slide").unwrap();
        let offsets: Vec<f32> = kf.steps.iter().map(|s| s.offset).collect();
        assert_eq!(offsets, vec![0.0, 0.5, 1.0]);
        // El paso del 50% conserva las dos declaraciones en orden.
        assert_eq!(
            kf.steps[1].declarations,
            vec![("left".into(), "40px".into()), ("top".into(), "10px".into())]
        );
    }

    #[test]
    fn keyframes_selector_multiple_comparte_decls() {
        // `0%, 100% { ... }` genera dos pasos con las mismas decls.
        let html = r#"<html><head><style>
            @keyframes pulse {
                0%, 100% { transform: scale(1); }
                50% { transform: scale(1.2); }
            }
        </style></head><body></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let kf = eng.keyframes().get("pulse").unwrap();
        assert_eq!(kf.steps.len(), 3);
        assert_eq!(kf.steps[0].offset, 0.0);
        assert_eq!(kf.steps[2].offset, 1.0);
        assert_eq!(kf.steps[0].declarations, kf.steps[2].declarations);
    }

    #[test]
    fn keyframes_prefijo_vendor_y_no_rompe_reglas_normales() {
        // `@-webkit-keyframes` se captura igual; y las reglas normales
        // alrededor del at-rule siguen aplicándose.
        let html = r#"<html><head><style>
            p { color: red; }
            @-webkit-keyframes spin { from { opacity: 0 } to { opacity: 1 } }
            p { color: green; }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        assert!(eng.keyframes().contains_key("spin"));
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    // === Fase B2: animation shorthand ===

    fn anim_de(decl: &str) -> AnimationBinding {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        eng.compute(&p).animation.expect("animation ausente")
    }

    #[test]
    fn animation_shorthand_completo() {
        let a = anim_de("animation: spin 2s ease-in-out 0.5s infinite alternate forwards");
        assert_eq!(a.name, "spin");
        assert_eq!(a.duration_s, 2.0);
        assert_eq!(a.timing, EasingFunction::EaseInOut);
        assert_eq!(a.delay_s, 0.5);
        assert_eq!(a.iterations, AnimationIterations::Infinite);
        assert_eq!(a.direction, AnimationDirection::Alternate);
        assert_eq!(a.fill_mode, AnimationFillMode::Forwards);
    }

    #[test]
    fn animation_orden_laxo_y_defaults() {
        // Tokens en orden no canónico + count numérico + ms.
        let a = anim_de("animation: 200ms linear 3 fade");
        assert_eq!(a.name, "fade");
        assert!((a.duration_s - 0.2).abs() < 1e-6);
        assert_eq!(a.timing, EasingFunction::Linear);
        assert_eq!(a.iterations, AnimationIterations::Count(3.0));
        assert_eq!(a.delay_s, 0.0);
        assert_eq!(a.direction, AnimationDirection::Normal);
        assert_eq!(a.fill_mode, AnimationFillMode::None);
    }

    #[test]
    fn animation_cubic_bezier_no_se_parte_por_comas() {
        let a = anim_de("animation: bounce 1s cubic-bezier(0.1, 0.7, 1.0, 0.1)");
        assert_eq!(a.name, "bounce");
        assert_eq!(a.duration_s, 1.0);
        assert_eq!(a.timing, EasingFunction::CubicBezier(0.1, 0.7, 1.0, 0.1));
    }

    #[test]
    fn animation_none_limpia() {
        let html = r#"<html><body><p style="animation: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).animation, None);
    }

    // === Fase B3: transition shorthand ===

    fn trans_de(decl: &str) -> Vec<TransitionBinding> {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        eng.compute(&p).transitions
    }

    #[test]
    fn transition_simple() {
        let t = trans_de("transition: opacity 200ms ease");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "opacity");
        assert!((t[0].duration_s - 0.2).abs() < 1e-6);
        assert_eq!(t[0].timing, EasingFunction::Ease);
        assert_eq!(t[0].delay_s, 0.0);
    }

    #[test]
    fn transition_lista_multiple() {
        let t = trans_de("transition: opacity 200ms ease, transform 0.3s ease-in 0.1s");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].property, "opacity");
        assert_eq!(t[1].property, "transform");
        assert!((t[1].duration_s - 0.3).abs() < 1e-6);
        assert_eq!(t[1].timing, EasingFunction::EaseIn);
        assert!((t[1].delay_s - 0.1).abs() < 1e-6);
    }

    #[test]
    fn transition_default_property_es_all() {
        // Sin nombre de propiedad, default `all` (CSS spec).
        let t = trans_de("transition: 1s");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "all");
        assert_eq!(t[0].duration_s, 1.0);
        assert_eq!(t[0].timing, EasingFunction::Ease);
    }

    #[test]
    fn transition_steps_y_none() {
        let t = trans_de("transition: width 2s steps(4, end)");
        assert_eq!(t[0].timing, EasingFunction::Steps(4, false));

        let html = r#"<html><body><p style="transition: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert!(eng.compute(&p).transitions.is_empty());
    }

    #[test]
    fn opera_presto_aliases_fase_7_941_945() {
        // -o-* (Opera Presto legacy) deben enrutar al mismo almacén que el estándar.
        let html = r#"<html><body>
            <img style="-o-object-fit: cover; -o-object-position: right bottom">
            <p style="-o-text-overflow: ellipsis; -o-tab-size: 4; -o-transform: translateX(10px)">x</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let img = dom.find("img").unwrap();
        let s = eng.compute(&img);
        assert_eq!(s.object_fit, Some(ObjectFit::Cover));
        assert!(s.object_position.is_some());
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.text_overflow, TextOverflow::Ellipsis);
        assert_eq!(s.tab_size, TabSize::Chars(4));
        assert_eq!(s.transforms.len(), 1);
    }

    #[test]
    fn opera_presto_anim_trans_aliases_fase_7_946_950() {
        // -o-transition / -o-animation (shorthands) + longhands de animation.
        let t = trans_de("-o-transition: opacity 200ms ease");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].property, "opacity");

        let html = r#"<html><body><p style="-o-animation-name: spin; -o-animation-duration: 3s; -o-animation-timing-function: linear">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let a = eng.compute(&p).animation.expect("binding de animation");
        assert_eq!(a.name, "spin");
        assert_eq!(a.duration_s, 3.0);
        assert_eq!(a.timing, EasingFunction::Linear);

        let html2 = r#"<html><body><p style="-o-animation: pulse 2s">x</p></body></html>"#;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let p2 = dom2.find("p").unwrap();
        assert_eq!(eng2.compute(&p2).animation.unwrap().name, "pulse");
    }

    #[test]
    fn opera_presto_origin_anim_border_select_fase_7_951_955() {
        let html = r#"<html><body><p style="
            -o-transform-origin: 10px 20px;
            -o-animation-iteration-count: infinite;
            -o-animation-fill-mode: forwards;
            -o-border-image: url(b.png) 30 round;
            -o-user-select: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.transform_origin.x, LengthVal::Px(10.0));
        assert_eq!(s.transform_origin.y, LengthVal::Px(20.0));
        let a = s.animation.expect("binding");
        assert_eq!(a.iterations, AnimationIterations::Infinite);
        assert_eq!(a.fill_mode, AnimationFillMode::Forwards);
        assert!(s.border_image.is_some());
        assert_eq!(s.user_select, UserSelect::None);
    }

    #[test]
    fn khtml_legacy_aliases_fase_7_956_960() {
        // -khtml-* (Konqueror / early Safari, los prefijos más viejos).
        let html = r#"<html><body><p style="
            -khtml-opacity: 0.5;
            -khtml-border-radius: 8px;
            -khtml-box-shadow: 2px 2px 4px black;
            -khtml-box-sizing: border-box;
            -khtml-user-select: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.opacity, 0.5);
        assert_eq!(s.border_radii.top_left, 8.0);
        assert_eq!(s.box_shadows.len(), 1);
        assert_eq!(s.box_sizing, BoxSizing::BorderBox);
        assert_eq!(s.user_select, UserSelect::None);
    }

    #[test]
    fn vendor_misc_aliases_fase_7_961_965() {
        // KHTML user-modify/appearance + scroll-snap-type (webkit/ms) + -ms-flex.
        let html = r#"<html><body><p style="
            -khtml-user-modify: read-write;
            -khtml-appearance: none;
            -ms-scroll-snap-type: x mandatory;
            -ms-flex: 2 3 auto">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert!(s.user_modify.is_some());
        assert_eq!(s.appearance, Appearance::None);
        assert!(s.scroll_snap_type.0.is_some());
        assert_eq!(s.flex_grow, 2.0);
        assert_eq!(s.flex_shrink, 3.0);
    }

    #[test]
    fn spatial_nav_exclusions_plumb_fase_7_966_970() {
        let html = r#"<html><body><p style="
            spatial-navigation-action: focus;
            spatial-navigation-contain: contain;
            spatial-navigation-function: grid;
            wrap-flow: both;
            wrap-through: none">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let s = eng.compute(&dom.find("p").unwrap());
        assert_eq!(s.spatial_navigation_action.as_deref(), Some("focus"));
        assert_eq!(s.spatial_navigation_contain.as_deref(), Some("contain"));
        assert_eq!(s.spatial_navigation_function.as_deref(), Some("grid"));
        assert_eq!(s.wrap_flow.as_deref(), Some("both"));
        assert_eq!(s.wrap_through.as_deref(), Some("none"));
        // El sentinel (valor initial) colapsa a None.
        let html2 = r#"<html><body><p style="spatial-navigation-action: auto; wrap-through: wrap">x</p></body></html>"#;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let s2 = eng2.compute(&dom2.find("p").unwrap());
        assert_eq!(s2.spatial_navigation_action, None);
        assert_eq!(s2.wrap_through, None);
    }

    #[test]
    fn regions_marks_textalignall_plumb_fase_7_971_975() {
        let html = r#"<html><body>
            <div style="flow-into: article; mark-before: url(a.wav); text-align-all: justify">
                <span>child</span>
            </div>
            <p style="flow-from: article; mark-after: url(b.wav)">x</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = eng.compute(&dom.find("div").unwrap());
        assert_eq!(div.flow_into.as_deref(), Some("article"));
        assert_eq!(div.mark_before.as_deref(), Some("url(a.wav)"));
        assert_eq!(div.text_align_all.as_deref(), Some("justify"));
        let p = eng.compute(&dom.find("p").unwrap());
        assert_eq!(p.flow_from.as_deref(), Some("article"));
        assert_eq!(p.mark_after.as_deref(), Some("url(b.wav)"));
        // text-align-all HEREDA: con el div como padre, el span lo recibe.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&div));
        assert_eq!(span.text_align_all.as_deref(), Some("justify"));
        // flow-into NO hereda.
        assert_eq!(span.flow_into, None);
    }

    #[test]
    fn viewport_descriptors_plumb_fase_7_976_980() {
        let html = r#"<html><body><p style="
            min-zoom: 0.5;
            max-zoom: 200%;
            user-zoom: fixed;
            viewport-fit: cover;
            ime-mode: disabled">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let s = eng.compute(&dom.find("p").unwrap());
        assert_eq!(s.min_zoom.as_deref(), Some("0.5"));
        assert_eq!(s.max_zoom.as_deref(), Some("200%"));
        assert_eq!(s.user_zoom.as_deref(), Some("fixed"));
        assert_eq!(s.viewport_fit.as_deref(), Some("cover"));
        assert_eq!(s.ime_mode.as_deref(), Some("disabled"));
    }

    #[test]
    fn svg_speech_legacy_plumb_fase_7_981_985() {
        let html = r#"<html><body>
            <div style="kerning: 2px; enable-background: new; color-profile: sRGB; voice-range: x-high; text-security: circle">
                <span>c</span>
            </div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.kerning.as_deref(), Some("2px"));
        assert_eq!(t.enable_background.as_deref(), Some("new"));
        // opaque_or_sentinel conserva el case original del valor.
        assert_eq!(t.color_profile.as_deref(), Some("sRGB"));
        assert_eq!(t.voice_range.as_deref(), Some("x-high"));
        assert_eq!(t.text_security.as_deref(), Some("circle"));
        // kerning / color-profile / voice-range HEREDAN (div como padre);
        // enable-background NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.kerning.as_deref(), Some("2px"));
        assert_eq!(span.color_profile.as_deref(), Some("sRGB"));
        assert_eq!(span.voice_range.as_deref(), Some("x-high"));
        assert_eq!(span.enable_background, None);
    }

    #[test]
    fn shapes_inline_linegrid_plumb_fase_7_986_990() {
        let html = r#"<html><body><p style="
            shape-padding: 10px;
            line-fit-edge: cap alphabetic;
            inline-sizing: stretch;
            box-snap: first;
            copy-into: foo content()">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let s = eng.compute(&dom.find("p").unwrap());
        assert_eq!(s.shape_padding.as_deref(), Some("10px"));
        assert_eq!(s.line_fit_edge.as_deref(), Some("cap alphabetic"));
        assert_eq!(s.inline_sizing.as_deref(), Some("stretch"));
        assert_eq!(s.box_snap.as_deref(), Some("first"));
        assert_eq!(s.copy_into.as_deref(), Some("foo content()"));
        // sentinel (initial) → None.
        let html2 = r#"<html><body><p style="box-snap: none; inline-sizing: normal">x</p></body></html>"#;
        let dom2 = DomTree::parse(html2);
        let eng2 = StyleEngine::from_dom(&dom2);
        let s2 = eng2.compute(&dom2.find("p").unwrap());
        assert_eq!(s2.box_snap, None);
        assert_eq!(s2.inline_sizing, None);
    }

    #[test]
    fn line_stacking_plumb_fase_7_991_995() {
        let html = r#"<html><body><div style="
            line-stacking: inline-line-height exclude-ruby consider-shifts;
            line-stacking-ruby: include-ruby;
            line-stacking-shift: disregard-shifts;
            line-stacking-strategy: grid-height;
            inline-box-align: 3"><span>c</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = eng.compute(&dom.find("div").unwrap());
        assert_eq!(d.line_stacking_ruby.as_deref(), Some("include-ruby"));
        assert_eq!(d.line_stacking_shift.as_deref(), Some("disregard-shifts"));
        assert_eq!(d.line_stacking_strategy.as_deref(), Some("grid-height"));
        assert_eq!(d.inline_box_align.as_deref(), Some("3"));
        // line-stacking-* HEREDAN; inline-box-align NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&d));
        assert_eq!(span.line_stacking_ruby.as_deref(), Some("include-ruby"));
        assert_eq!(span.line_stacking_strategy.as_deref(), Some("grid-height"));
        assert_eq!(span.inline_box_align, None);
    }

    #[test]
    fn alignment_textheight_dropinitial_plumb_fase_7_996_1000() {
        let html = r#"<html><body><div style="
            alignment-adjust: central;
            text-height: font-size;
            drop-initial-size: 3;
            drop-initial-value: 2;
            drop-initial-before-align: x-height"><span>c</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = eng.compute(&dom.find("div").unwrap());
        assert_eq!(d.alignment_adjust.as_deref(), Some("central"));
        assert_eq!(d.text_height.as_deref(), Some("font-size"));
        assert_eq!(d.drop_initial_size.as_deref(), Some("3"));
        assert_eq!(d.drop_initial_value.as_deref(), Some("2"));
        assert_eq!(d.drop_initial_before_align.as_deref(), Some("x-height"));
        // text-height HEREDA; alignment-adjust / drop-initial-* NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&d));
        assert_eq!(span.text_height.as_deref(), Some("font-size"));
        assert_eq!(span.alignment_adjust, None);
        assert_eq!(span.drop_initial_size, None);
    }

    #[test]
    fn dropinitial_rest_legacy_plumb_fase_7_1001_1005() {
        let html = r#"<html><body><div style="
            drop-initial-after-align: central;
            drop-initial-before-adjust: text-before-edge;
            drop-initial-after-adjust: text-after-edge;
            block-progression: rl;
            snap-height: 8px 2"><span>c</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = eng.compute(&dom.find("div").unwrap());
        assert_eq!(d.drop_initial_after_align.as_deref(), Some("central"));
        assert_eq!(d.drop_initial_before_adjust.as_deref(), Some("text-before-edge"));
        assert_eq!(d.drop_initial_after_adjust.as_deref(), Some("text-after-edge"));
        assert_eq!(d.block_progression.as_deref(), Some("rl"));
        assert_eq!(d.snap_height.as_deref(), Some("8px 2"));
        // block-progression / snap-height HEREDAN; drop-initial-* NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&d));
        assert_eq!(span.block_progression.as_deref(), Some("rl"));
        assert_eq!(span.snap_height.as_deref(), Some("8px 2"));
        assert_eq!(span.drop_initial_after_align, None);
    }

    // Computa un único `<p style="...">` y devuelve su ComputedStyle.
    fn style_of(decl: &str) -> ComputedStyle {
        let html = format!("<html><body><p style=\"{decl}\">x</p></body></html>");
        let dom = DomTree::parse(&html);
        let eng = StyleEngine::from_dom(&dom);
        eng.compute(&dom.find("p").unwrap())
    }

    #[test]
    fn epub_aliases_fase_7_1006_1012() {
        // Cada -epub-* debe producir el MISMO computed que su estándar.
        assert_eq!(style_of("-epub-hyphens: auto").hyphens, style_of("hyphens: auto").hyphens);
        assert_eq!(style_of("-epub-text-transform: uppercase").text_transform,
                   style_of("text-transform: uppercase").text_transform);
        assert_eq!(style_of("-epub-ruby-position: over").ruby_position,
                   style_of("ruby-position: over").ruby_position);
        assert_eq!(style_of("-epub-line-break: strict").line_break,
                   style_of("line-break: strict").line_break);
        assert_eq!(style_of("-epub-text-align-last: justify").text_align_last,
                   style_of("text-align-last: justify").text_align_last);
        assert_eq!(style_of("-epub-text-emphasis-position: over right").text_emphasis_position,
                   style_of("text-emphasis-position: over right").text_emphasis_position);
        assert_eq!(style_of("-epub-text-emphasis: dot red").text_emphasis_style,
                   style_of("text-emphasis: dot red").text_emphasis_style);
        // Y el alias efectivamente parseó algo (no quedó en el default).
        assert_ne!(style_of("-epub-text-transform: uppercase").text_transform,
                   ComputedStyle::default().text_transform);
    }

    #[test]
    fn moz_textdecoration_aliases_fase_7_1013_1015() {
        assert_eq!(style_of("-moz-text-decoration-line: underline").text_decoration,
                   style_of("text-decoration-line: underline").text_decoration);
        assert_eq!(style_of("-moz-text-decoration-color: red").text_decoration_color,
                   style_of("text-decoration-color: red").text_decoration_color);
        assert!(style_of("-moz-text-decoration-color: red").text_decoration_color.is_some());
        assert_eq!(style_of("-moz-text-decoration-style: dashed").text_decoration_style,
                   style_of("text-decoration-style: dashed").text_decoration_style);
    }

    #[test]
    fn ms_webkit_misc_aliases_fase_7_1016_1020() {
        assert_eq!(style_of("-ms-word-break: break-all").word_break,
                   style_of("word-break: break-all").word_break);
        assert_eq!(style_of("-ms-text-overflow: ellipsis").text_overflow,
                   style_of("text-overflow: ellipsis").text_overflow);
        assert_eq!(style_of("-ms-text-combine-horizontal: all").text_combine_upright,
                   style_of("text-combine-upright: all").text_combine_upright);
        assert_eq!(style_of("-ms-high-contrast-adjust: none").forced_color_adjust,
                   style_of("forced-color-adjust: none").forced_color_adjust);
        assert_eq!(style_of("-webkit-hyphenate-limit-lines: 3").hyphenate_limit_lines,
                   style_of("hyphenate-limit-lines: 3").hyphenate_limit_lines);
        assert_eq!(style_of("-webkit-hyphenate-limit-lines: 3").hyphenate_limit_lines, Some(3));
    }

    #[test]
    fn scroll_snap_margin_padding_aliases_fase_7_1021_1030() {
        // Los nombres legacy scroll-snap-margin*/scroll-snap-padding*
        // (CSS Scroll Snap v0) deben producir el MISMO computed que los
        // estándar scroll-margin*/scroll-padding*.
        assert_eq!(style_of("scroll-snap-margin: 10px").scroll_margin.top,
                   style_of("scroll-margin: 10px").scroll_margin.top);
        assert_eq!(style_of("scroll-snap-margin-top: 7px").scroll_margin.top,
                   style_of("scroll-margin-top: 7px").scroll_margin.top);
        assert_eq!(style_of("scroll-snap-margin-right: 7px").scroll_margin.right,
                   style_of("scroll-margin-right: 7px").scroll_margin.right);
        assert_eq!(style_of("scroll-snap-margin-bottom: 7px").scroll_margin.bottom,
                   style_of("scroll-margin-bottom: 7px").scroll_margin.bottom);
        assert_eq!(style_of("scroll-snap-margin-left: 7px").scroll_margin.left,
                   style_of("scroll-margin-left: 7px").scroll_margin.left);
        assert_eq!(style_of("scroll-snap-padding: 5px").scroll_padding.top,
                   style_of("scroll-padding: 5px").scroll_padding.top);
        assert_eq!(style_of("scroll-snap-padding-top: 3px").scroll_padding.top,
                   style_of("scroll-padding-top: 3px").scroll_padding.top);
        assert_eq!(style_of("scroll-snap-padding-right: 3px").scroll_padding.right,
                   style_of("scroll-padding-right: 3px").scroll_padding.right);
        assert_eq!(style_of("scroll-snap-padding-bottom: 3px").scroll_padding.bottom,
                   style_of("scroll-padding-bottom: 3px").scroll_padding.bottom);
        assert_eq!(style_of("scroll-snap-padding-left: 3px").scroll_padding.left,
                   style_of("scroll-padding-left: 3px").scroll_padding.left);
        // Y efectivamente parseó (no quedó en default).
        assert_ne!(style_of("scroll-snap-margin-top: 7px").scroll_margin.top,
                   ComputedStyle::default().scroll_margin.top);
    }

    #[test]
    fn scroll_snap_v0_plumb_fase_7_1031_1034() {
        // CSS Scroll Snap v0 (deprecado): plumb opaco. Sentinel initial → None.
        assert_eq!(style_of("scroll-snap-points-x: repeat(100px)").scroll_snap_points_x,
                   Some("repeat(100px)".to_string()));
        assert_eq!(style_of("scroll-snap-points-y: repeat(50%)").scroll_snap_points_y,
                   Some("repeat(50%)".to_string()));
        assert_eq!(style_of("scroll-snap-destination: 50% 50%").scroll_snap_destination,
                   Some("50% 50%".to_string()));
        assert_eq!(style_of("scroll-snap-coordinate: 0 0, 100% 100%").scroll_snap_coordinate,
                   Some("0 0, 100% 100%".to_string()));
        // Sentinel = valor initial → None.
        assert_eq!(style_of("scroll-snap-points-x: none").scroll_snap_points_x, None);
        assert_eq!(style_of("scroll-snap-coordinate: none").scroll_snap_coordinate, None);
        assert_eq!(style_of("scroll-snap-destination: 0px 0px").scroll_snap_destination, None);
    }

    #[test]
    fn moz_gecko_props_plumb_fase_7_1035_1042() {
        let html = r#"<html><body><div style="
            -moz-orient: vertical;
            -moz-user-focus: ignore;
            -moz-user-input: disabled;
            -moz-window-dragging: drag;
            -moz-float-edge: margin-box;
            -moz-force-broken-image-icon: 1;
            -moz-image-region: rect(0 8px 8px 0);
            -moz-binding: url(b.xml#x)
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.moz_orient.as_deref(), Some("vertical"));
        assert_eq!(t.moz_user_focus.as_deref(), Some("ignore"));
        assert_eq!(t.moz_user_input.as_deref(), Some("disabled"));
        assert_eq!(t.moz_window_dragging.as_deref(), Some("drag"));
        assert_eq!(t.moz_float_edge.as_deref(), Some("margin-box"));
        assert_eq!(t.moz_force_broken_image_icon.as_deref(), Some("1"));
        assert_eq!(t.moz_image_region.as_deref(), Some("rect(0 8px 8px 0)"));
        assert_eq!(t.moz_binding.as_deref(), Some("url(b.xml#x)"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-moz-orient: inline").moz_orient, None);
        assert_eq!(style_of("-moz-float-edge: content-box").moz_float_edge, None);
        // -moz-user-focus / -moz-user-input / -moz-image-region HEREDAN;
        // el resto NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.moz_user_focus.as_deref(), Some("ignore"));
        assert_eq!(span.moz_user_input.as_deref(), Some("disabled"));
        assert_eq!(span.moz_image_region.as_deref(), Some("rect(0 8px 8px 0)"));
        assert_eq!(span.moz_orient, None);
        assert_eq!(span.moz_binding, None);
    }

    #[test]
    fn moz_outline_radius_plumb_fase_7_1043_1047() {
        let s = style_of(
            "-moz-outline-radius: 5px; \
             -moz-outline-radius-topleft: 1px; \
             -moz-outline-radius-topright: 2px; \
             -moz-outline-radius-bottomleft: 3px; \
             -moz-outline-radius-bottomright: 4px",
        );
        assert_eq!(s.moz_outline_radius.as_deref(), Some("5px"));
        assert_eq!(s.moz_outline_radius_topleft.as_deref(), Some("1px"));
        assert_eq!(s.moz_outline_radius_topright.as_deref(), Some("2px"));
        assert_eq!(s.moz_outline_radius_bottomleft.as_deref(), Some("3px"));
        assert_eq!(s.moz_outline_radius_bottomright.as_deref(), Some("4px"));
        // Sentinel `0` (initial) → None.
        assert_eq!(style_of("-moz-outline-radius: 0").moz_outline_radius, None);
        assert_eq!(style_of("-moz-outline-radius-topleft: 0").moz_outline_radius_topleft, None);
    }

    #[test]
    fn svg_masking_scrollsnap_v0_plumb_fase_7_1048_1051() {
        assert_eq!(style_of("buffered-rendering: static").buffered_rendering,
                   Some("static".to_string()));
        assert_eq!(style_of("mask-source-type: luminance").mask_source_type,
                   Some("luminance".to_string()));
        assert_eq!(style_of("scroll-snap-type-x: mandatory").scroll_snap_type_x,
                   Some("mandatory".to_string()));
        assert_eq!(style_of("scroll-snap-type-y: proximity").scroll_snap_type_y,
                   Some("proximity".to_string()));
        // Sentinel = initial → None.
        assert_eq!(style_of("buffered-rendering: auto").buffered_rendering, None);
        assert_eq!(style_of("mask-source-type: auto").mask_source_type, None);
        assert_eq!(style_of("scroll-snap-type-x: none").scroll_snap_type_x, None);
    }

    #[test]
    fn ms_legacy_props_plumb_fase_7_1052_1057() {
        assert_eq!(style_of("-ms-overflow-style: -ms-autohiding-scrollbar").ms_overflow_style,
                   Some("-ms-autohiding-scrollbar".to_string()));
        assert_eq!(style_of("-ms-scroll-chaining: none").ms_scroll_chaining,
                   Some("none".to_string()));
        assert_eq!(style_of("-ms-content-zooming: zoom").ms_content_zooming,
                   Some("zoom".to_string()));
        assert_eq!(style_of("-ms-scroll-rails: none").ms_scroll_rails,
                   Some("none".to_string()));
        assert_eq!(style_of("-ms-flex-align: center").ms_flex_align,
                   Some("center".to_string()));
        assert_eq!(style_of("-ms-flex-pack: justify").ms_flex_pack,
                   Some("justify".to_string()));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-overflow-style: auto").ms_overflow_style, None);
        assert_eq!(style_of("-ms-scroll-chaining: chained").ms_scroll_chaining, None);
        assert_eq!(style_of("-ms-flex-pack: start").ms_flex_pack, None);
    }

    #[test]
    fn moz_misc_props_plumb_fase_7_1058_1062() {
        let html = r#"<html><body><div style="
            -moz-context-properties: fill stroke;
            -moz-stack-sizing: ignore;
            -moz-text-blink: blink;
            -moz-default-appearance: button;
            -moz-box-flexgroup: 2
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.moz_context_properties.as_deref(), Some("fill stroke"));
        assert_eq!(t.moz_stack_sizing.as_deref(), Some("ignore"));
        assert_eq!(t.moz_text_blink.as_deref(), Some("blink"));
        assert_eq!(t.moz_default_appearance.as_deref(), Some("button"));
        assert_eq!(t.moz_box_flexgroup.as_deref(), Some("2"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-moz-context-properties: none").moz_context_properties, None);
        assert_eq!(style_of("-moz-box-flexgroup: 1").moz_box_flexgroup, None);
        // -moz-context-properties / -moz-text-blink HEREDAN; el resto NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.moz_context_properties.as_deref(), Some("fill stroke"));
        assert_eq!(span.moz_text_blink.as_deref(), Some("blink"));
        assert_eq!(span.moz_stack_sizing, None);
        assert_eq!(span.moz_default_appearance, None);
    }

    #[test]
    fn css_stroke_props_plumb_fase_7_1063_1072() {
        let html = r#"<html><body><div style="
            stroke-align: outset;
            stroke-break: slice;
            stroke-color: red;
            stroke-image: linear-gradient(red, blue);
            stroke-origin: content-box;
            stroke-position: center;
            stroke-repeat: round;
            stroke-size: 10px;
            stroke-dash-corner: 2px;
            stroke-dash-justify: compress
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.stroke_align.as_deref(), Some("outset"));
        assert_eq!(t.stroke_break.as_deref(), Some("slice"));
        assert_eq!(t.stroke_color_css.as_deref(), Some("red"));
        assert_eq!(t.stroke_image.as_deref(), Some("linear-gradient(red, blue)"));
        assert_eq!(t.stroke_origin.as_deref(), Some("content-box"));
        assert_eq!(t.stroke_position.as_deref(), Some("center"));
        assert_eq!(t.stroke_repeat.as_deref(), Some("round"));
        assert_eq!(t.stroke_size.as_deref(), Some("10px"));
        assert_eq!(t.stroke_dash_corner.as_deref(), Some("2px"));
        assert_eq!(t.stroke_dash_justify.as_deref(), Some("compress"));
        // Sentinel = initial → None.
        assert_eq!(style_of("stroke-image: none").stroke_image, None);
        assert_eq!(style_of("stroke-color: currentcolor").stroke_color_css, None);
        assert_eq!(style_of("stroke-align: center").stroke_align, None);
        // HEREDAN (tradición SVG).
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.stroke_align.as_deref(), Some("outset"));
        assert_eq!(span.stroke_color_css.as_deref(), Some("red"));
        assert_eq!(span.stroke_dash_justify.as_deref(), Some("compress"));
    }

    #[test]
    fn css_fill_props_plumb_fase_7_1073_1079() {
        let html = r#"<html><body><div style="
            fill-break: slice;
            fill-color: red;
            fill-image: url(p.png);
            fill-origin: content-box;
            fill-position: center;
            fill-size: cover;
            fill-repeat: space
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.fill_break.as_deref(), Some("slice"));
        assert_eq!(t.fill_color_css.as_deref(), Some("red"));
        assert_eq!(t.fill_image.as_deref(), Some("url(p.png)"));
        assert_eq!(t.fill_origin.as_deref(), Some("content-box"));
        assert_eq!(t.fill_position.as_deref(), Some("center"));
        assert_eq!(t.fill_size.as_deref(), Some("cover"));
        assert_eq!(t.fill_repeat.as_deref(), Some("space"));
        // Sentinel = initial → None.
        assert_eq!(style_of("fill-image: none").fill_image, None);
        assert_eq!(style_of("fill-color: black").fill_color_css, None);
        // HEREDAN (tradición SVG).
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.fill_color_css.as_deref(), Some("red"));
        assert_eq!(span.fill_repeat.as_deref(), Some("space"));
        // El paint SVG `fill` sigue independiente del plumb fill-color.
        assert_eq!(style_of("fill-color: red").fill, ComputedStyle::default().fill);
    }

    #[test]
    fn animation_trigger_longhands_plumb_fase_7_1080_1087() {
        let s = style_of(
            "animation-trigger-behavior: repeat; \
             animation-trigger-timeline: --t; \
             animation-trigger-range: entry 0% exit 100%; \
             animation-trigger-range-start: entry 0%; \
             animation-trigger-range-end: exit 100%; \
             animation-trigger-exit-range: cover; \
             animation-trigger-exit-range-start: cover 0%; \
             animation-trigger-exit-range-end: cover 100%",
        );
        assert_eq!(s.animation_trigger_behavior.as_deref(), Some("repeat"));
        assert_eq!(s.animation_trigger_timeline.as_deref(), Some("--t"));
        assert_eq!(s.animation_trigger_range.as_deref(), Some("entry 0% exit 100%"));
        assert_eq!(s.animation_trigger_range_start.as_deref(), Some("entry 0%"));
        assert_eq!(s.animation_trigger_range_end.as_deref(), Some("exit 100%"));
        assert_eq!(s.animation_trigger_exit_range.as_deref(), Some("cover"));
        assert_eq!(s.animation_trigger_exit_range_start.as_deref(), Some("cover 0%"));
        assert_eq!(s.animation_trigger_exit_range_end.as_deref(), Some("cover 100%"));
        // Sentinel = initial → None.
        assert_eq!(style_of("animation-trigger-behavior: once").animation_trigger_behavior, None);
        assert_eq!(style_of("animation-trigger-timeline: auto").animation_trigger_timeline, None);
        assert_eq!(style_of("animation-trigger-range: normal").animation_trigger_range, None);
    }

    #[test]
    fn webkit_legacy_box_columnbreak_fase_7_1088_1092() {
        // -webkit-box-lines / -webkit-box-flex-group: plumb opaco.
        assert_eq!(style_of("-webkit-box-lines: multiple").webkit_box_lines,
                   Some("multiple".to_string()));
        assert_eq!(style_of("-webkit-box-flex-group: 3").webkit_box_flex_group,
                   Some("3".to_string()));
        assert_eq!(style_of("-webkit-box-lines: single").webkit_box_lines, None);
        // -webkit-column-break-* deben dar el MISMO computed que break-*.
        assert_eq!(style_of("-webkit-column-break-before: always").break_before,
                   style_of("break-before: always").break_before);
        assert_eq!(style_of("-webkit-column-break-after: avoid").break_after,
                   style_of("break-after: avoid").break_after);
        assert_eq!(style_of("-webkit-column-break-inside: avoid").break_inside,
                   style_of("break-inside: avoid").break_inside);
        // Y efectivamente parsearon (no quedaron en default).
        assert_ne!(style_of("-webkit-column-break-before: always").break_before,
                   ComputedStyle::default().break_before);
    }

    #[test]
    fn ie_scrollbar_colors_plumb_fase_7_1093_1100() {
        let html = r#"<html><body><div style="
            scrollbar-base-color: #ccc;
            scrollbar-face-color: #ddd;
            scrollbar-track-color: #eee;
            scrollbar-arrow-color: black;
            scrollbar-shadow-color: gray;
            scrollbar-highlight-color: white;
            scrollbar-3dlight-color: silver;
            scrollbar-darkshadow-color: #333
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.scrollbar_base_color.as_deref(), Some("#ccc"));
        assert_eq!(t.scrollbar_face_color.as_deref(), Some("#ddd"));
        assert_eq!(t.scrollbar_track_color.as_deref(), Some("#eee"));
        assert_eq!(t.scrollbar_arrow_color.as_deref(), Some("black"));
        assert_eq!(t.scrollbar_shadow_color.as_deref(), Some("gray"));
        assert_eq!(t.scrollbar_highlight_color.as_deref(), Some("white"));
        assert_eq!(t.scrollbar_3dlight_color.as_deref(), Some("silver"));
        assert_eq!(t.scrollbar_darkshadow_color.as_deref(), Some("#333"));
        // HEREDAN.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.scrollbar_base_color.as_deref(), Some("#ccc"));
        assert_eq!(span.scrollbar_darkshadow_color.as_deref(), Some("#333"));
    }

    #[test]
    fn ms_grid_plumb_fase_7_1101_1108() {
        let s = style_of(
            "-ms-grid-columns: 1fr 200px; \
             -ms-grid-rows: auto 1fr; \
             -ms-grid-column: 2; \
             -ms-grid-row: 3; \
             -ms-grid-column-span: 2; \
             -ms-grid-row-span: 4; \
             -ms-grid-column-align: center; \
             -ms-grid-row-align: end",
        );
        assert_eq!(s.ms_grid_columns.as_deref(), Some("1fr 200px"));
        assert_eq!(s.ms_grid_rows.as_deref(), Some("auto 1fr"));
        assert_eq!(s.ms_grid_column.as_deref(), Some("2"));
        assert_eq!(s.ms_grid_row.as_deref(), Some("3"));
        assert_eq!(s.ms_grid_column_span.as_deref(), Some("2"));
        assert_eq!(s.ms_grid_row_span.as_deref(), Some("4"));
        assert_eq!(s.ms_grid_column_align.as_deref(), Some("center"));
        assert_eq!(s.ms_grid_row_align.as_deref(), Some("end"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-grid-columns: none").ms_grid_columns, None);
        assert_eq!(style_of("-ms-grid-column: 1").ms_grid_column, None);
        assert_eq!(style_of("-ms-grid-row-align: stretch").ms_grid_row_align, None);
    }

    #[test]
    fn ms_exclusions_regions_text_plumb_fase_7_1109_1116() {
        let html = r#"<html><body><div style="
            -ms-touch-select: none;
            -ms-text-autospace: ideograph-alpha;
            -ms-wrap-flow: both;
            -ms-wrap-margin: 10px;
            -ms-wrap-through: none;
            -ms-flow-into: region1;
            -ms-flow-from: region1;
            -ms-hyphenate-limit-chars: 6 3 3
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.ms_touch_select.as_deref(), Some("none"));
        assert_eq!(t.ms_text_autospace.as_deref(), Some("ideograph-alpha"));
        assert_eq!(t.ms_wrap_flow.as_deref(), Some("both"));
        assert_eq!(t.ms_wrap_margin.as_deref(), Some("10px"));
        assert_eq!(t.ms_wrap_through.as_deref(), Some("none"));
        assert_eq!(t.ms_flow_into.as_deref(), Some("region1"));
        assert_eq!(t.ms_flow_from.as_deref(), Some("region1"));
        assert_eq!(t.ms_hyphenate_limit_chars.as_deref(), Some("6 3 3"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-wrap-flow: auto").ms_wrap_flow, None);
        assert_eq!(style_of("-ms-flow-into: none").ms_flow_into, None);
        // -ms-text-autospace / -ms-hyphenate-limit-chars HEREDAN; resto NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.ms_text_autospace.as_deref(), Some("ideograph-alpha"));
        assert_eq!(span.ms_hyphenate_limit_chars.as_deref(), Some("6 3 3"));
        assert_eq!(span.ms_wrap_flow, None);
        assert_eq!(span.ms_flow_into, None);
    }

    #[test]
    fn moz_outline_aliases_webkit_misc_fase_7_1117_1122() {
        // -moz-outline-* deben dar el MISMO computed que outline-*.
        assert_eq!(style_of("-moz-outline-width: 3px").outline.width,
                   style_of("outline-width: 3px").outline.width);
        assert_eq!(style_of("-moz-outline-style: dashed").outline.style,
                   style_of("outline-style: dashed").outline.style);
        assert_eq!(style_of("-moz-outline-color: red").outline.color,
                   style_of("outline-color: red").outline.color);
        assert_eq!(style_of("-moz-outline-offset: 4px").outline.offset,
                   style_of("outline-offset: 4px").outline.offset);
        // `invert` y currentColor pasan por el brazo guardado.
        assert_eq!(style_of("-moz-outline-color: invert").current_color,
                   style_of("outline-color: invert").current_color);
        // Parseó de verdad (no quedó en default).
        assert_ne!(style_of("-moz-outline-style: dashed").outline.style,
                   ComputedStyle::default().outline.style);
        // WebKit misc opaco.
        let html = r#"<html><body><div style="
            -webkit-mask-attachment: fixed;
            -webkit-text-decorations-in-effect: underline
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.webkit_mask_attachment.as_deref(), Some("fixed"));
        assert_eq!(t.webkit_text_decorations_in_effect.as_deref(), Some("underline"));
        assert_eq!(style_of("-webkit-mask-attachment: scroll").webkit_mask_attachment, None);
        // -webkit-text-decorations-in-effect HEREDA; -webkit-mask-attachment NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.webkit_text_decorations_in_effect.as_deref(), Some("underline"));
        assert_eq!(span.webkit_mask_attachment, None);
    }

    #[test]
    fn border_clip_limit_plumb_fase_7_1123_1128() {
        let s = style_of(
            "border-clip: 0 10px; \
             border-clip-top: 25%; \
             border-clip-right: 1em 2em; \
             border-clip-bottom: 50%; \
             border-clip-left: 5px; \
             border-limit: all",
        );
        assert_eq!(s.border_clip.as_deref(), Some("0 10px"));
        assert_eq!(s.border_clip_top.as_deref(), Some("25%"));
        assert_eq!(s.border_clip_right.as_deref(), Some("1em 2em"));
        assert_eq!(s.border_clip_bottom.as_deref(), Some("50%"));
        assert_eq!(s.border_clip_left.as_deref(), Some("5px"));
        assert_eq!(s.border_limit.as_deref(), Some("all"));
        // Sentinel = initial → None.
        assert_eq!(style_of("border-clip: normal").border_clip, None);
        assert_eq!(style_of("border-clip-top: normal").border_clip_top, None);
        assert_eq!(style_of("border-limit: round").border_limit, None);
    }

    #[test]
    fn webkit_apple_legacy_plumb_fase_7_1129_1135() {
        let html = r#"<html><body><div style="
            -webkit-marquee: left infinite;
            -webkit-region-fragment: break;
            -webkit-svg-shadow: 2px 2px 4px black;
            -webkit-text-zoom: reset;
            -apple-pay-button-style: black;
            -apple-pay-button-type: buy;
            -apple-color-filter: apple-invert-lightness()
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.webkit_marquee.as_deref(), Some("left infinite"));
        assert_eq!(t.webkit_region_fragment.as_deref(), Some("break"));
        assert_eq!(t.webkit_svg_shadow.as_deref(), Some("2px 2px 4px black"));
        assert_eq!(t.webkit_text_zoom.as_deref(), Some("reset"));
        assert_eq!(t.apple_pay_button_style.as_deref(), Some("black"));
        assert_eq!(t.apple_pay_button_type.as_deref(), Some("buy"));
        assert_eq!(t.apple_color_filter.as_deref(), Some("apple-invert-lightness()"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-webkit-region-fragment: auto").webkit_region_fragment, None);
        assert_eq!(style_of("-apple-pay-button-style: white").apple_pay_button_style, None);
        // -webkit-text-zoom HEREDA; -webkit-marquee NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.webkit_text_zoom.as_deref(), Some("reset"));
        assert_eq!(span.webkit_marquee, None);
    }

    #[test]
    fn moz_mathml_props_plumb_fase_7_1136_1140() {
        let html = r#"<html><body><div style="
            -moz-script-level: 2;
            -moz-math-display: block;
            -moz-script-min-size: 6pt;
            -moz-script-size-multiplier: 0.8;
            -moz-presentation-level: 1
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.moz_script_level.as_deref(), Some("2"));
        assert_eq!(t.moz_math_display.as_deref(), Some("block"));
        assert_eq!(t.moz_script_min_size.as_deref(), Some("6pt"));
        assert_eq!(t.moz_script_size_multiplier.as_deref(), Some("0.8"));
        assert_eq!(t.moz_presentation_level.as_deref(), Some("1"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-moz-script-level: 0").moz_script_level, None);
        assert_eq!(style_of("-moz-math-display: inline").moz_math_display, None);
        assert_eq!(style_of("-moz-script-size-multiplier: 0.71").moz_script_size_multiplier, None);
        // Todas HEREDAN (layout matemático).
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.moz_script_level.as_deref(), Some("2"));
        assert_eq!(span.moz_math_display.as_deref(), Some("block"));
        assert_eq!(span.moz_presentation_level.as_deref(), Some("1"));
    }

    #[test]
    fn webkit_line_marquee_mark_plumb_fase_7_1141_1146() {
        let html = r#"<html><body><div style="
            -webkit-line-align: edges;
            -webkit-line-box-contain: block glyphs;
            -webkit-line-snap: contain;
            marquee-play-count: 3;
            mark: url(a.wav) 2;
            text-combine-mode: horizontal
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.webkit_line_align.as_deref(), Some("edges"));
        assert_eq!(t.webkit_line_box_contain.as_deref(), Some("block glyphs"));
        assert_eq!(t.webkit_line_snap.as_deref(), Some("contain"));
        assert_eq!(t.marquee_play_count.as_deref(), Some("3"));
        assert_eq!(t.mark.as_deref(), Some("url(a.wav) 2"));
        assert_eq!(t.text_combine_mode.as_deref(), Some("horizontal"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-webkit-line-align: none").webkit_line_align, None);
        assert_eq!(style_of("marquee-play-count: infinite").marquee_play_count, None);
        assert_eq!(style_of("mark: none").mark, None);
        // line-align/-box-contain/-snap, mark y text-combine-mode HEREDAN;
        // marquee-play-count NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.webkit_line_align.as_deref(), Some("edges"));
        assert_eq!(span.mark.as_deref(), Some("url(a.wav) 2"));
        assert_eq!(span.text_combine_mode.as_deref(), Some("horizontal"));
        assert_eq!(span.marquee_play_count, None);
    }

    #[test]
    fn ms_layout_grid_plumb_fase_7_1147_1151() {
        let html = r#"<html><body><div style="
            -ms-layout-grid: both loose 18px 12px;
            -ms-layout-grid-char: 16px;
            -ms-layout-grid-line: 20px;
            -ms-layout-grid-mode: line;
            -ms-layout-grid-type: strict
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.ms_layout_grid.as_deref(), Some("both loose 18px 12px"));
        assert_eq!(t.ms_layout_grid_char.as_deref(), Some("16px"));
        assert_eq!(t.ms_layout_grid_line.as_deref(), Some("20px"));
        assert_eq!(t.ms_layout_grid_mode.as_deref(), Some("line"));
        assert_eq!(t.ms_layout_grid_type.as_deref(), Some("strict"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-layout-grid-mode: both").ms_layout_grid_mode, None);
        assert_eq!(style_of("-ms-layout-grid-type: loose").ms_layout_grid_type, None);
        // Todas HEREDAN (layout CJK).
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.ms_layout_grid_char.as_deref(), Some("16px"));
        assert_eq!(span.ms_layout_grid_type.as_deref(), Some("strict"));
    }

    #[test]
    fn ms_content_zoom_plumb_fase_7_1152_1158() {
        let s = style_of(
            "-ms-content-zoom-chaining: chained; \
             -ms-content-zoom-limit: 100% 500%; \
             -ms-content-zoom-limit-max: 500%; \
             -ms-content-zoom-limit-min: 50%; \
             -ms-content-zoom-snap: mandatory snapInterval(0%, 100%); \
             -ms-content-zoom-snap-points: snapList(0%, 100%); \
             -ms-content-zoom-snap-type: proximity",
        );
        assert_eq!(s.ms_content_zoom_chaining.as_deref(), Some("chained"));
        assert_eq!(s.ms_content_zoom_limit.as_deref(), Some("100% 500%"));
        assert_eq!(s.ms_content_zoom_limit_max.as_deref(), Some("500%"));
        assert_eq!(s.ms_content_zoom_limit_min.as_deref(), Some("50%"));
        assert_eq!(s.ms_content_zoom_snap.as_deref(), Some("mandatory snapInterval(0%, 100%)"));
        assert_eq!(s.ms_content_zoom_snap_points.as_deref(), Some("snapList(0%, 100%)"));
        assert_eq!(s.ms_content_zoom_snap_type.as_deref(), Some("proximity"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-content-zoom-chaining: none").ms_content_zoom_chaining, None);
        assert_eq!(style_of("-ms-content-zoom-limit-max: 400%").ms_content_zoom_limit_max, None);
        assert_eq!(style_of("-ms-content-zoom-snap-type: none").ms_content_zoom_snap_type, None);
    }

    #[test]
    fn ms_scroll_limit_snap_plumb_fase_7_1159_1168() {
        let s = style_of(
            "-ms-scroll-limit: 0 0 500px 800px; \
             -ms-scroll-limit-x-max: 500px; \
             -ms-scroll-limit-x-min: 10px; \
             -ms-scroll-limit-y-max: 800px; \
             -ms-scroll-limit-y-min: 20px; \
             -ms-scroll-snap-points-x: snapInterval(0px, 100px); \
             -ms-scroll-snap-points-y: snapList(0px, 50px); \
             -ms-scroll-snap-x: mandatory snapInterval(0px, 100px); \
             -ms-scroll-snap-y: proximity snapList(0px); \
             -ms-scroll-translation: vertical-to-horizontal",
        );
        assert_eq!(s.ms_scroll_limit.as_deref(), Some("0 0 500px 800px"));
        assert_eq!(s.ms_scroll_limit_x_max.as_deref(), Some("500px"));
        assert_eq!(s.ms_scroll_limit_x_min.as_deref(), Some("10px"));
        assert_eq!(s.ms_scroll_limit_y_max.as_deref(), Some("800px"));
        assert_eq!(s.ms_scroll_limit_y_min.as_deref(), Some("20px"));
        assert_eq!(s.ms_scroll_snap_points_x.as_deref(), Some("snapInterval(0px, 100px)"));
        assert_eq!(s.ms_scroll_snap_points_y.as_deref(), Some("snapList(0px, 50px)"));
        assert_eq!(s.ms_scroll_snap_x.as_deref(), Some("mandatory snapInterval(0px, 100px)"));
        assert_eq!(s.ms_scroll_snap_y.as_deref(), Some("proximity snapList(0px)"));
        assert_eq!(s.ms_scroll_translation.as_deref(), Some("vertical-to-horizontal"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-scroll-limit-x-max: auto").ms_scroll_limit_x_max, None);
        assert_eq!(style_of("-ms-scroll-limit-y-min: 0").ms_scroll_limit_y_min, None);
        assert_eq!(style_of("-ms-scroll-translation: none").ms_scroll_translation, None);
    }

    #[test]
    fn ms_aliases_estandar_fase_7_1169_1176() {
        // Cada -ms-* debe producir el MISMO computed que su estándar.
        assert_eq!(style_of("-ms-line-break: strict").line_break,
                   style_of("line-break: strict").line_break);
        assert_eq!(style_of("-ms-text-justify: distribute").text_justify,
                   style_of("text-justify: distribute").text_justify);
        assert_eq!(style_of("-ms-text-underline-position: under").text_underline_position,
                   style_of("text-underline-position: under").text_underline_position);
        assert_eq!(style_of("-ms-zoom: 1.5").zoom, style_of("zoom: 1.5").zoom);
        assert_eq!(style_of("-ms-scrollbar-base-color: #ccc").scrollbar_base_color,
                   style_of("scrollbar-base-color: #ccc").scrollbar_base_color);
        assert_eq!(style_of("-ms-scrollbar-face-color: #ddd").scrollbar_face_color,
                   style_of("scrollbar-face-color: #ddd").scrollbar_face_color);
        assert_eq!(style_of("-ms-scrollbar-track-color: #eee").scrollbar_track_color,
                   style_of("scrollbar-track-color: #eee").scrollbar_track_color);
        // Y parsearon de verdad (no quedaron en default).
        assert_ne!(style_of("-ms-line-break: strict").line_break,
                   ComputedStyle::default().line_break);
        assert_eq!(style_of("-ms-zoom: 1.5").zoom, Some("1.5".to_string()));
        assert_eq!(style_of("-ms-scrollbar-base-color: #ccc").scrollbar_base_color,
                   Some("#ccc".to_string()));
    }

    #[test]
    fn ms_opaque_misc_plumb_fase_7_1177_1183() {
        let html = r#"<html><body><div style="
            -ms-interpolation-mode: bicubic;
            -ms-block-progression: rl;
            -ms-text-kashida-space: 50%;
            -ms-accelerator: true;
            -ms-behavior: url(htc.htc);
            -ms-filter: progid:DXImageTransform.Microsoft.gradient();
            -ms-writing-mode: tb-rl
        "><span>x</span></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let t = eng.compute(&dom.find("div").unwrap());
        assert_eq!(t.ms_interpolation_mode.as_deref(), Some("bicubic"));
        assert_eq!(t.ms_block_progression.as_deref(), Some("rl"));
        assert_eq!(t.ms_text_kashida_space.as_deref(), Some("50%"));
        assert_eq!(t.ms_accelerator.as_deref(), Some("true"));
        assert_eq!(t.ms_behavior.as_deref(), Some("url(htc.htc)"));
        assert_eq!(t.ms_filter.as_deref(), Some("progid:DXImageTransform.Microsoft.gradient()"));
        assert_eq!(t.ms_writing_mode.as_deref(), Some("tb-rl"));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-interpolation-mode: nearest-neighbor").ms_interpolation_mode, None);
        assert_eq!(style_of("-ms-accelerator: false").ms_accelerator, None);
        assert_eq!(style_of("-ms-writing-mode: lr-tb").ms_writing_mode, None);
        // -ms-text-kashida-space / -ms-writing-mode HEREDAN; el resto NO.
        let span = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&t));
        assert_eq!(span.ms_text_kashida_space.as_deref(), Some("50%"));
        assert_eq!(span.ms_writing_mode.as_deref(), Some("tb-rl"));
        assert_eq!(span.ms_filter, None);
        assert_eq!(span.ms_interpolation_mode, None);
    }

    #[test]
    fn moz_animation_longhand_aliases_fase_7_1184_1191() {
        // Cada -moz-animation-* longhand alimenta el mismo AnimationBinding que
        // el estándar; el binding resultante debe ser idéntico.
        assert_eq!(style_of("-moz-animation-name: spin").animation,
                   style_of("animation-name: spin").animation);
        assert_eq!(style_of("-moz-animation-duration: 2s").animation,
                   style_of("animation-duration: 2s").animation);
        assert_eq!(style_of("-moz-animation-timing-function: ease-in").animation,
                   style_of("animation-timing-function: ease-in").animation);
        assert_eq!(style_of("-moz-animation-iteration-count: 3").animation,
                   style_of("animation-iteration-count: 3").animation);
        assert_eq!(style_of("-moz-animation-fill-mode: both").animation,
                   style_of("animation-fill-mode: both").animation);
        assert_eq!(style_of("-moz-animation-direction: reverse").animation,
                   style_of("animation-direction: reverse").animation);
        assert_eq!(style_of("-moz-animation-play-state: paused").animation,
                   style_of("animation-play-state: paused").animation);
        assert_eq!(style_of("-moz-animation-delay: 0.5s").animation,
                   style_of("animation-delay: 0.5s").animation);
        // Y parsearon de verdad (nombre quedó en el binding).
        assert_eq!(style_of("-moz-animation-name: spin").animation.map(|b| b.name),
                   Some("spin".to_string()));
    }

    #[test]
    fn o_transition_moz_misc_aliases_fase_7_1192_1197() {
        // -o-transition-* longhands == estándar (junto a -webkit-/-moz- ya existentes).
        assert_eq!(style_of("-o-transition-property: opacity").transitions,
                   style_of("transition-property: opacity").transitions);
        assert_eq!(style_of("-o-transition-duration: 2s").transitions,
                   style_of("transition-duration: 2s").transitions);
        assert_eq!(style_of("-o-transition-delay: 0.3s").transitions,
                   style_of("transition-delay: 0.3s").transitions);
        assert_eq!(style_of("-o-transition-timing-function: ease-out").transitions,
                   style_of("transition-timing-function: ease-out").transitions);
        // -moz-font-language-override == estándar.
        assert_eq!(style_of("-moz-font-language-override: 'SRB'").font_language_override,
                   style_of("font-language-override: 'SRB'").font_language_override);
        // -moz-outline (shorthand) == outline: expande width/style/color.
        let moz = style_of("-moz-outline: 2px dashed red");
        let std = style_of("outline: 2px dashed red");
        assert_eq!(moz.outline.width, std.outline.width);
        assert_eq!(moz.outline.style, std.outline.style);
        assert_eq!(moz.outline.color, std.outline.color);
        assert_ne!(moz.outline.style, ComputedStyle::default().outline.style);
    }

    #[test]
    fn ms_flex_2012_remnants_fase_7_1198_1200() {
        // -ms-flex-flow (alias) == flex-flow: expande direction + wrap.
        assert_eq!(style_of("-ms-flex-flow: column wrap").flex_direction,
                   style_of("flex-flow: column wrap").flex_direction);
        assert_eq!(style_of("-ms-flex-flow: column wrap").flex_wrap,
                   style_of("flex-flow: column wrap").flex_wrap);
        assert_ne!(style_of("-ms-flex-flow: column wrap").flex_direction,
                   ComputedStyle::default().flex_direction);
        // -ms-flex-item-align / -ms-flex-line-pack: plumb opaco (IE10 2012).
        assert_eq!(style_of("-ms-flex-item-align: center").ms_flex_item_align,
                   Some("center".to_string()));
        assert_eq!(style_of("-ms-flex-line-pack: justify").ms_flex_line_pack,
                   Some("justify".to_string()));
        // Sentinel = initial → None.
        assert_eq!(style_of("-ms-flex-item-align: auto").ms_flex_item_align, None);
        assert_eq!(style_of("-ms-flex-line-pack: stretch").ms_flex_line_pack, None);
    }
