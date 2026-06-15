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
