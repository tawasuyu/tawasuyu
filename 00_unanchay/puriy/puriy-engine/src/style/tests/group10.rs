//! Tests del motor de estilo (grupo 10, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn parsea_line_height() {
        let s = parse_stylesheet(
            "p { line-height: 1.5 } h1 { line-height: 32px }",
            &HashMap::new(),
            DEFAULT_VIEWPORT,
        );
        // 1.5 → 1.5
        assert!(matches!(s[0].decls[0].kind, DeclKind::LineHeight(v) if (v - 1.5).abs() < 1e-6));
        // 32px sobre font-size 16px estimado → 2.0
        assert!(matches!(s[1].decls[0].kind, DeclKind::LineHeight(v) if (v - 2.0).abs() < 1e-6));
    }

    #[test]
    fn computa_width_y_text_align() {
        let html = r#"<html><head><style>
            .narrow{max-width:600px;text-align:center;line-height:1.6}
        </style></head><body><div class="narrow">x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert_eq!(st.max_width, LengthVal::Px(600.0));
        assert_eq!(st.text_align, TextAlign::Center);
        assert!((st.line_height.unwrap() - 1.6).abs() < 1e-6);
    }

    #[test]
    fn hereda_color_y_font_size_del_padre() {
        // `<p style="color:red; font-size:20px">foo <em>bar</em></p>` —
        // el `<em>` no tiene regla propia pero hereda color y tamaño.
        let html = r#"<html><body><p style="color:red; font-size:20px">foo<em>bar</em></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.color, Color::rgb(255, 0, 0));
        let em = dom.find("em").unwrap();
        let em_style = eng.compute_with_parent(&em, Some(&p_style));
        assert_eq!(em_style.color, Color::rgb(255, 0, 0));
        assert!((em_style.font_size - 20.0).abs() < 1e-6);
    }

    #[test]
    fn no_hereda_propiedades_no_heredables() {
        // background y margin/padding NO heredan.
        let html = r#"<html><body><div style="background:red; margin:30px"><p>x</p></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let div_style = eng.compute_with_parent(&div, None);
        assert_eq!(div_style.background, Some(Color::rgb(255, 0, 0)));
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, Some(&div_style));
        assert_eq!(p_style.background, None);
        // margin del <p> es 12px (UA default), no 30px del padre.
        assert!((p_style.margin.top - 12.0).abs() < 1e-6);
        assert!((p_style.margin.bottom - 12.0).abs() < 1e-6);
    }

    #[test]
    fn font_weight_bold_local_no_propaga_a_padre_no_bold() {
        // Un `<b>` dentro de `<p>` no-bold sigue siendo bold.
        let html = "<html><body><p>foo<b>bar</b></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let p_style = eng.compute_with_parent(&p, None);
        assert_eq!(p_style.font_weight, 400);
        let b = dom.find("b").unwrap();
        let b_style = eng.compute_with_parent(&b, Some(&p_style));
        assert_eq!(b_style.font_weight, 700);
    }

    #[test]
    fn box_tree_propaga_color_a_hoja_de_texto() {
        // Verifica el bug original: el text leaf debe heredar el color
        // del `<p>` padre.
        let html = r#"<html><body><p style="color: #00ff00">verde</p></body></html>"#;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut leaf_colors = Vec::new();
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("verde") {
                leaf_colors.push(b.color);
            }
        });
        assert_eq!(leaf_colors.len(), 1);
        assert_eq!(leaf_colors[0], Color::rgb(0, 0xff, 0));
    }

    #[test]
    fn specificity_calculada_correctamente() {
        // `body p` = 0,0,2 → 2
        let s1 = parse_selector("body p").unwrap();
        assert_eq!(s1.specificity(), 2);
        // `.menu li` = 0,1,1 → 11
        let s2 = parse_selector(".menu li").unwrap();
        assert_eq!(s2.specificity(), 11);
        // `#hero` = 1,0,0 → 100
        let s3 = parse_selector("#hero").unwrap();
        assert_eq!(s3.specificity(), 100);
        // `a.btn[href^="https"]:first-child` = 0,3,1 → 31
        let s4 = parse_selector(r#"a.btn[href^="https"]:first-child"#).unwrap();
        assert_eq!(s4.specificity(), 31);
        // `nav > a#x.y` = 1,1,2 → 112
        let s5 = parse_selector("nav > a#x.y").unwrap();
        assert_eq!(s5.specificity(), 112);
    }

    #[test]
    fn id_vence_a_tag_aunque_llegue_antes() {
        // `#hero { color: blue }` está ANTES que `body p { color: red }`
        // en el stylesheet — sin especificidad, el último (rojo) ganaba.
        // Con especificidad, el #id (100 > 2) gana azul.
        let html = r#"<html><head><style>
            #hero { color: blue }
            body p { color: red }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn clase_vence_a_tag() {
        // `.alert` (10) > `p` (1) aunque ambos matcheen.
        let html = r#"<html><head><style>
            .alert { color: red }
            p { color: blue }
        </style></head><body><p class="alert">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn inline_style_vence_a_id() {
        // Inline tiene especificidad implícita 1000 — gana sobre `#hero`.
        let html = r##"<html><head><style>
            #hero { color: blue }
        </style></head><body><p id="hero" style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn empate_de_especificidad_gana_el_ultimo() {
        // Dos selectores con misma especificidad: gana el que llega después.
        let html = r#"<html><head><style>
            p { color: red }
            p { color: blue }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn important_vence_normal_de_mayor_especificidad() {
        // `body p { color: red !important }` (spec=2) debe vencer a
        // `#hero { color: blue }` (spec=100) — important rompe la
        // jerarquía de especificidad dentro del mismo origen.
        let html = r#"<html><head><style>
            body p { color: red !important }
            #hero { color: blue }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn important_inline_vence_important_de_id() {
        // Inline !important vence cualquier !important de selector.
        let html = r##"<html><head><style>
            #hero { color: red !important }
        </style></head><body><p id="hero" style="color: green !important">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn normal_inline_pierde_contra_important_de_regla() {
        // Inline normal (1000) pierde contra !important de cualquier selector.
        let html = r##"<html><head><style>
            p { color: red !important }
        </style></head><body><p style="color: green">x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn parsea_border_shorthand() {
        let html = r#"<html><head><style>
            .a { border: 2px solid #ff0000 }
            .b { border: 1px dashed blue !important }
            .c { border: none }
            .d { border-radius: 8px }
        </style></head><body>
          <div class="a"></div><div class="b"></div>
          <div class="c"></div><div class="d"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 4);
        let a = eng.compute(&divs[0]);
        assert!((a.border_widths.top - 2.0).abs() < 1e-6);
        assert_eq!(a.border_colors.top, Some(Color::rgb(255, 0, 0)));
        let b = eng.compute(&divs[1]);
        assert!((b.border_widths.top - 1.0).abs() < 1e-6);
        assert_eq!(b.border_colors.top, Some(Color::rgb(0, 0, 255)));
        let c = eng.compute(&divs[2]);
        assert_eq!(c.border_colors.top, None); // `none` deshabilita
        assert!((c.border_widths.top - 0.0).abs() < 1e-6);
        let d = eng.compute(&divs[3]);
        assert!((d.border_radii.top_left - 8.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_per_side() {
        // `border-top: 2px solid red` setea sólo el top; `border-bottom-color`
        // sólo el color del bottom; `border-right-width` sólo el ancho derecho.
        let html = r#"<html><head><style>
            div {
                border-top: 2px solid #ff0000;
                border-bottom-color: #0000ff;
                border-bottom-width: 4px;
                border-bottom-style: solid;
                border-right-width: 1px;
                border-right-color: #00ff00;
                border-right-style: solid;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let s = eng.compute(&div);
        // Top: del shorthand
        assert!((s.border_widths.top - 2.0).abs() < 1e-6);
        assert_eq!(s.border_colors.top, Some(Color::rgb(255, 0, 0)));
        // Bottom: 3 longhand
        assert!((s.border_widths.bottom - 4.0).abs() < 1e-6);
        assert_eq!(s.border_colors.bottom, Some(Color::rgb(0, 0, 255)));
        // Right: 3 longhand
        assert!((s.border_widths.right - 1.0).abs() < 1e-6);
        assert_eq!(s.border_colors.right, Some(Color::rgb(0, 0xff, 0)));
        // Left: no se tocó
        assert_eq!(s.border_widths.left, 0.0);
        assert_eq!(s.border_colors.left, None);
    }

    #[test]
    fn parsea_border_radius_per_corner() {
        let html = r#"<html><head><style>
            div {
                border-top-left-radius: 4px;
                border-top-right-radius: 8px;
                border-bottom-right-radius: 12px;
                border-bottom-left-radius: 16px;
            }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let s = eng.compute(&div);
        assert!((s.border_radii.top_left - 4.0).abs() < 1e-6);
        assert!((s.border_radii.top_right - 8.0).abs() < 1e-6);
        assert!((s.border_radii.bottom_right - 12.0).abs() < 1e-6);
        assert!((s.border_radii.bottom_left - 16.0).abs() < 1e-6);
    }

    #[test]
    fn parsea_border_propiedades_individuales() {
        let html = r#"<html><head><style>
            div { border-width: 3px; border-color: #00ff00; border-style: solid; border-radius: 5px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let div = dom.find("div").unwrap();
        let st = eng.compute(&div);
        assert!((st.border_widths.top - 3.0).abs() < 1e-6);
        assert_eq!(st.border_colors.top, Some(Color::rgb(0, 0xff, 0)));
        assert!((st.border_radii.top_left - 5.0).abs() < 1e-6);
    }

    #[test]
    fn hover_state_activa_regla_solo_cuando_corresponde() {
        // `.btn:hover { background: red }`: matchea con hover_active=true,
        // no matchea sin él.
        let html = r##"<html><head><style>
            .btn:hover { background: #ff0000 }
            .btn { background: #ffffff }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let base = eng.compute_with_parent_in_state(&a, None, false);
        let hover = eng.compute_with_parent_in_state(&a, None, true);
        assert_eq!(base.background, Some(Color::rgb(255, 255, 255)));
        assert_eq!(hover.background, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn hover_pseudo_aporta_a_specificity() {
        // `.btn:hover` debe tener specificity 0,2,0 → 20 (clase 10 + pseudo 10)
        let s = parse_selector(".btn:hover").unwrap();
        assert_eq!(s.specificity(), 20);
    }

    #[test]
    fn box_tree_expone_hover_background() {
        let html = r##"<html><head><style>
            .btn { background: white }
            .btn:hover { background: #ffaa00 }
        </style></head><body><a class="btn">x</a></body></html>"##;
        let eng = crate::Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut hover_bgs = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                hover_bgs.push(b.hover_background);
            }
        });
        assert_eq!(hover_bgs.len(), 1);
        assert_eq!(hover_bgs[0], Some(Color::rgb(0xff, 0xaa, 0)));
    }

    #[test]
    fn parsea_box_shadow_completo() {
        let html = r#"<html><head><style>
            .a { box-shadow: 2px 4px 8px 1px #000000 }
            .b { box-shadow: 1px 2px red }
            .c { box-shadow: none }
        </style></head><body>
          <div class="a"></div><div class="b"></div><div class="c"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let a_list = eng.compute(&divs[0]).box_shadows.clone();
        assert_eq!(a_list.len(), 1);
        let a = a_list[0];
        assert!((a.offset_x - 2.0).abs() < 1e-6);
        assert!((a.offset_y - 4.0).abs() < 1e-6);
        assert!((a.blur_px - 8.0).abs() < 1e-6);
        assert!((a.spread_px - 1.0).abs() < 1e-6);
        assert_eq!(a.color, Color::BLACK);
        assert!(!a.inset);
        let b = eng.compute(&divs[1]).box_shadows[0];
        assert_eq!(b.color, Color::rgb(255, 0, 0));
        assert!((b.blur_px - 0.0).abs() < 1e-6);
        assert!((b.spread_px - 0.0).abs() < 1e-6);
        assert!(eng.compute(&divs[2]).box_shadows.is_empty());
    }

    #[test]
    fn box_shadow_multi_e_inset_fase_7_236() {
        let html = r#"<html><head><style>
            .multi { box-shadow: 2px 2px #000, 4px 4px red, inset 1px 1px blue }
            .ins   { box-shadow: inset 3px 4px 5px 6px #00ff00 }
            .noop  { box-shadow: garbage }
        </style></head><body>
          <div class="multi"></div><div class="ins"></div><div class="noop"></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        let list = eng.compute(&divs[0]).box_shadows.clone();
        assert_eq!(list.len(), 3, "tres sombras en la lista");
        assert!(!list[0].inset && list[0].color == Color::BLACK);
        assert!(!list[1].inset && list[1].color == Color::rgb(255, 0, 0));
        assert!(list[2].inset && list[2].color == Color::rgb(0, 0, 255));
        let ins = eng.compute(&divs[1]).box_shadows[0];
        assert!(ins.inset);
        assert!((ins.offset_x - 3.0).abs() < 1e-6);
        assert!((ins.offset_y - 4.0).abs() < 1e-6);
        assert!((ins.blur_px - 5.0).abs() < 1e-6);
        assert!((ins.spread_px - 6.0).abs() < 1e-6);
        assert_eq!(ins.color, Color::rgb(0, 255, 0));
        assert!(eng.compute(&divs[2]).box_shadows.is_empty());
    }

    #[test]
    fn parse_nth_arg_acepta_formatos_comunes() {
        assert_eq!(parse_nth_arg("odd"), Some((2, 1)));
        assert_eq!(parse_nth_arg("even"), Some((2, 0)));
        assert_eq!(parse_nth_arg("3"), Some((0, 3)));
        assert_eq!(parse_nth_arg("n"), Some((1, 0)));
        assert_eq!(parse_nth_arg("2n"), Some((2, 0)));
        assert_eq!(parse_nth_arg("2n+1"), Some((2, 1)));
        assert_eq!(parse_nth_arg("3n -2"), Some((3, -2)));
        assert_eq!(parse_nth_arg("-n+3"), Some((-1, 3)));
        assert_eq!(parse_nth_arg("xyz"), None);
    }

    #[test]
    fn selector_nth_child_aplica() {
        // `li:nth-child(odd)` matchea li 1, 3 (1-indexed).
        let html = r#"<html><head><style>
            li:nth-child(odd) { color: #f00 }
            li:nth-child(2n) { color: #00f }
        </style></head><body><ul>
          <li>a</li><li>b</li><li>c</li><li>d</li>
        </ul></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(lis.len(), 4);
        assert_eq!(eng.compute(&lis[0]).color, Color::rgb(0xff, 0, 0)); // odd
        assert_eq!(eng.compute(&lis[1]).color, Color::rgb(0, 0, 0xff)); // even (2n)
        assert_eq!(eng.compute(&lis[2]).color, Color::rgb(0xff, 0, 0)); // odd
        assert_eq!(eng.compute(&lis[3]).color, Color::rgb(0, 0, 0xff)); // even
    }

    #[test]
    fn selector_nth_child_n_fija() {
        // `:nth-child(3)` matchea SÓLO la tercera.
        let html = r#"<html><head><style>
            li:nth-child(3) { color: #0a0 }
        </style></head><body><ul>
          <li>1</li><li>2</li><li>3</li><li>4</li>
        </ul></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut lis = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("li") {
                lis.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&lis[0]).color, Color::BLACK);
        assert_eq!(eng.compute(&lis[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&lis[2]).color, Color::rgb(0, 0xaa, 0));
        assert_eq!(eng.compute(&lis[3]).color, Color::BLACK);
    }

    #[test]
    fn selector_not_excluye() {
        // `p:not(.skip)` matchea todos los <p> excepto los con class skip.
        let html = r#"<html><head><style>
            p:not(.skip) { color: #f00 }
        </style></head><body>
          <p>uno</p>
          <p class="skip">dos</p>
          <p>tres</p>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut ps = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("p") {
                ps.push(n.clone());
            }
        });
        assert_eq!(eng.compute(&ps[0]).color, Color::rgb(0xff, 0, 0));
        assert_eq!(eng.compute(&ps[1]).color, Color::BLACK);
        assert_eq!(eng.compute(&ps[2]).color, Color::rgb(0xff, 0, 0));
    }

    #[test]
    fn specificity_not_aporta_la_del_argumento() {
        // `:not(#x)` aporta 100 (la del #id interno).
        let s = parse_selector(":not(#x)").unwrap();
        assert_eq!(s.specificity(), 100);
        // `a:not(.b)` aporta 1 (tag) + 10 (.b interno) = 11.
        let s = parse_selector("a:not(.b)").unwrap();
        assert_eq!(s.specificity(), 11);
    }

    #[test]
    fn not_anidado_se_acepta() {
        // Fase 7.938 — CSS Selectors 4 PERMITE `:not(:not(p))` (= elementos
        // que son `p`). Antes lo rechazábamos por una anti-recursión que ya no
        // hace falta: el matching de `:not` recurre acotado por el input.
        let s = parse_selector(":not(:not(p))").expect("Selectors 4 lo permite");
        // matchea un <p> real.
        let html = "<html><body><p></p><span></span></body></html>";
        let dom = DomTree::parse(html);
        let mut p = None;
        let mut span = None;
        crate::dom::walk(&dom.document(), &mut |n| match crate::dom::element_name(n).as_deref() {
            Some("p") => p = Some(n.clone()),
            Some("span") => span = Some(n.clone()),
            _ => {}
        });
        assert!(
            selector_matches_subject(&s, p.as_ref().unwrap(), false, false),
            ":not(:not(p)) debe matchear <p>"
        );
        assert!(
            !selector_matches_subject(&s, span.as_ref().unwrap(), false, false),
            ":not(:not(p)) NO matchea <span>"
        );
    }

    #[test]
    fn cascada_inline_sobrescribe() {
        let html = "<html><head><style>p { color: red }</style></head><body><p style='color:blue'>x</p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let style = eng.compute(&p);
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }

