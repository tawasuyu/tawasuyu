//! Tests del motor de estilo (grupo 13, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn background_capas_multiples_shorthand_y_longhand() {
        // Fase 7.206 — la lista `background: a, b` reparte la capa 0 en los
        // campos sueltos y las capas 2..N en background_extra_layers.
        let compute = |css: &str| {
            let html = format!(
                "<html><head><style>div {{ {css} }}</style></head><body><div></div></body></html>"
            );
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            eng.compute(&dom.find("div").unwrap())
        };

        // Shorthand: capa 0 (arriba) = url(top) no-repeat center/cover; capa
        // extra = url(bottom) repeat-x con defaults de size/position.
        let s = compute("background: url(top.png) no-repeat center / cover, url(bottom.png) repeat-x");
        assert_eq!(s.background_image_url.as_deref(), Some("top.png"));
        assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
        assert_eq!(s.background_size, BackgroundSize::Cover);
        assert_eq!(s.background_extra_layers.len(), 1);
        let ex = &s.background_extra_layers[0];
        assert_eq!(ex.image, BackgroundImage::Url("bottom.png".into()));
        assert_eq!(ex.repeat, BackgroundRepeat::RepeatX);
        assert_eq!(ex.size, BackgroundSize::Auto); // default
        assert_eq!((ex.position.x, ex.position.y), (LengthVal::Pct(0.0), LengthVal::Pct(0.0)));

        // Gradiente arriba de una imagen, y color sólo en la última capa.
        let s = compute("background: linear-gradient(red, blue), url(img.png) green");
        assert!(s.background_gradient.is_some()); // capa 0 = gradiente
        assert_eq!(s.background, Some(Color::rgb(0, 128, 0))); // color de la última capa
        assert_eq!(s.background_extra_layers.len(), 1);
        assert_eq!(s.background_extra_layers[0].image, BackgroundImage::Url("img.png".into()));

        // Una sola capa resetea las extra (la shorthand siempre emite la lista).
        let s = compute("background-image: url(a.png), url(b.png); background: blue");
        assert!(s.background_extra_layers.is_empty());
        assert_eq!(s.background, Some(Color::rgb(0, 0, 255)));

        // Longhand `background-image` con varias capas.
        let s = compute("background-image: url(a.png), url(b.png), url(c.png)");
        assert_eq!(s.background_image_url.as_deref(), Some("a.png"));
        assert_eq!(s.background_extra_layers.len(), 2);
        assert_eq!(s.background_extra_layers[0].image, BackgroundImage::Url("b.png".into()));
        assert_eq!(s.background_extra_layers[1].image, BackgroundImage::Url("c.png".into()));
    }

    #[test]
    fn background_capas_extra_resueltas_viajan_al_box() {
        // La capa extra de gradiente se resuelve y viaja al BoxNode (las url()
        // que no resuelven se descartan; el gradiente siempre pinta).
        let eng = crate::Engine::new();
        let html = r#"<html><body>
            <div id="d" style="background: url(x.png) no-repeat, linear-gradient(red, blue)"></div>
        </body></html>"#;
        let doc = eng.load_html("about:test", html);
        let mut layers = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                layers = Some(b.background_extra_layers.len());
                // El gradiente de la capa extra está presente.
                assert!(b.background_extra_layers.iter().any(|l| l.gradient.is_some()));
            }
        });
        assert_eq!(layers, Some(1));
    }

    #[test]
    fn parsea_padding_individual_4_lados() {
        let html = r#"<html><head><style>
            div {
                padding-top: 1px;
                padding-right: 2px;
                padding-bottom: 3px;
                padding-left: 4px;
            }
        </style></head><body><div>x</div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.padding.top, 1.0);
        assert_eq!(s.padding.right, 2.0);
        assert_eq!(s.padding.bottom, 3.0);
        assert_eq!(s.padding.left, 4.0);
    }

    #[test]
    fn parsea_position_y_insets() {
        let html = r#"<html><head><style>
            div { position: absolute; top: 10px; left: 50%; bottom: auto; right: 20px }
        </style></head><body><div></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.position, Position::Absolute);
        assert!(matches!(s.inset_top, LengthVal::Px(10.0)));
        assert!(matches!(s.inset_left, LengthVal::Pct(50.0)));
        assert!(matches!(s.inset_bottom, LengthVal::Auto));
        assert!(matches!(s.inset_right, LengthVal::Px(20.0)));

        let dom2 = DomTree::parse(r#"<html><body><nav style="position:sticky"></nav></body></html>"#);
        let eng2 = StyleEngine::from_dom(&dom2);
        let n = dom2.find("nav").unwrap();
        assert_eq!(eng2.compute(&n).position, Position::Sticky);
    }

    #[test]
    fn parsea_transforms_cadena() {
        let t = parse_transforms("translate(10px, 20px) scale(2) rotate(45deg)").unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], Transform::Translate(10.0, 20.0));
        assert_eq!(t[1], Transform::Scale(2.0, 2.0));
        assert_eq!(t[2], Transform::Rotate(45.0));

        let t = parse_transforms("translateX(5px) scaleY(0.5) rotate(0.5turn)").unwrap();
        assert_eq!(t[0], Transform::Translate(5.0, 0.0));
        assert_eq!(t[1], Transform::Scale(1.0, 0.5));
        assert_eq!(t[2], Transform::Rotate(180.0));

        assert!(parse_transforms("none").unwrap().is_empty());
    }

    #[test]
    fn parsea_translate_porcentaje() {
        // El truco de centrado: translate(-50%, -50%) → TranslatePct(-50,-50).
        let t = parse_transforms("translate(-50%, -50%)").unwrap();
        assert_eq!(t, vec![Transform::TranslatePct(-50.0, -50.0)]);
        // Mixto px/% → dos transforms (conmutan): TranslatePct + Translate.
        let t = parse_transforms("translate(10px, 50%)").unwrap();
        assert_eq!(t, vec![Transform::TranslatePct(0.0, 50.0), Transform::Translate(10.0, 0.0)]);
        // translateY(%) y combinado con otra función.
        let t = parse_transforms("translate(-50%, -50%) rotate(45deg)").unwrap();
        assert_eq!(t[0], Transform::TranslatePct(-50.0, -50.0));
        assert_eq!(t[1], Transform::Rotate(45.0));
        // translateX(%) → eje X.
        let t = parse_transforms("translateX(100%)").unwrap();
        assert_eq!(t, vec![Transform::TranslatePct(100.0, 0.0)]);
        // Pura px sigue dando Translate.
        let t = parse_transforms("translate(10px, 20px)").unwrap();
        assert_eq!(t, vec![Transform::Translate(10.0, 20.0)]);
        // La prop individual `translate:` también: pura % → TranslatePct.
        let t = parse_declarations("translate: -50% -50%", &HashMap::new());
        assert!(t.iter().any(|d| matches!(d.kind,
            DeclKind::Translate(Some(Transform::TranslatePct(x, y))) if x == -50.0 && y == -50.0)));
    }

    #[test]
    fn parsea_transforms_skew_y_matrix() {
        // skew(x), skew(x, y), skewX, skewY (ángulos con unidad).
        let t = parse_transforms("skew(10deg) skew(10deg, 20deg) skewX(0.25turn) skewY(15deg)").unwrap();
        assert_eq!(t[0], Transform::Skew(10.0, 0.0));
        assert_eq!(t[1], Transform::Skew(10.0, 20.0));
        assert_eq!(t[2], Transform::Skew(90.0, 0.0)); // 0.25turn = 90deg
        assert_eq!(t[3], Transform::Skew(0.0, 15.0));
        // matrix(a,b,c,d,e,f) — afín 2D completa.
        let t = parse_transforms("matrix(1, 0, 0, 1, 30, 40)").unwrap();
        assert_eq!(t[0], Transform::Matrix(1.0, 0.0, 0.0, 1.0, 30.0, 40.0));
        // matrix con escala/rotación.
        let t = parse_transforms("matrix(2, 0, 0, 0.5, 0, 0)").unwrap();
        assert_eq!(t[0], Transform::Matrix(2.0, 0.0, 0.0, 0.5, 0.0, 0.0));
        // matrix con aridad incorrecta → None.
        assert!(parse_transforms("matrix(1, 0, 0)").is_none());
    }

    #[test]
    fn parsea_text_shadow_simple_y_multiple() {
        let sh = parse_text_shadows("2px 3px 4px red").unwrap();
        assert_eq!(sh.len(), 1);
        assert_eq!(sh[0].offset_x, 2.0);
        assert_eq!(sh[0].offset_y, 3.0);
        assert_eq!(sh[0].blur_px, 4.0);
        assert_eq!(sh[0].color, Color::rgb(255, 0, 0));

        let sh = parse_text_shadows("1px 1px black, -1px -1px white").unwrap();
        assert_eq!(sh.len(), 2);
        assert_eq!(sh[0].color, Color::BLACK);
        assert_eq!(sh[1].color, Color::WHITE);
        assert_eq!(sh[1].offset_x, -1.0);

        let sh = parse_text_shadows("none").unwrap();
        assert!(sh.is_empty());
    }

    #[test]
    fn parsea_vertical_align() {
        assert_eq!(parse_vertical_align("baseline"), Some(VerticalAlign::Baseline));
        assert_eq!(parse_vertical_align("middle"), Some(VerticalAlign::Middle));
        assert_eq!(parse_vertical_align("text-top"), Some(VerticalAlign::Top));
        assert_eq!(parse_vertical_align("super"), Some(VerticalAlign::Super));
    }

    #[test]
    fn parsea_visibility_y_pointer_events_heredan() {
        let html = r#"<html><head><style>
            .h { visibility: hidden; pointer-events: none }
        </style></head><body>
          <div class="h"><p>oculto</p></div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let p = dom.find("p").unwrap();
        let d_style = eng.compute_with_parent(&d, None);
        let p_style = eng.compute_with_parent(&p, Some(&d_style));
        assert_eq!(p_style.visibility, Visibility::Hidden);
        assert_eq!(p_style.pointer_events, PointerEvents::None);
    }

    #[test]
    fn parsea_text_indent_y_word_spacing_heredan() {
        let html = r#"<html><head><style>
            p { text-indent: 30px; word-spacing: 5px }
        </style></head><body><p>x <span>y</span></p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let span = dom.find("span").unwrap();
        let p_style = eng.compute(&p);
        let span_style = eng.compute_with_parent(&span, Some(&p_style));
        assert_eq!(p_style.text_indent, 30.0);
        assert_eq!(p_style.word_spacing, 5.0);
        assert_eq!(span_style.word_spacing, 5.0);
        assert_eq!(span_style.text_indent, 30.0);
    }

    #[test]
    fn parsea_letter_spacing_hereda_y_normal_es_cero() {
        let html = r#"<html><head><style>
            p { letter-spacing: 2px }
            .tight { letter-spacing: normal }
        </style></head><body>
            <p>x <span>y</span></p>
            <div class="tight">z</div>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p_style = eng.compute(&dom.find("p").unwrap());
        let span_style = eng.compute_with_parent(&dom.find("span").unwrap(), Some(&p_style));
        assert_eq!(p_style.letter_spacing, 2.0);
        // Hereda al inline hijo.
        assert_eq!(span_style.letter_spacing, 2.0);
        // `normal` ⇒ 0px.
        let tight = eng.compute(&dom.find("div").unwrap());
        assert_eq!(tight.letter_spacing, 0.0);
    }

    #[test]
    fn parsea_display_grid_y_template() {
        let html = r#"<html><head><style>
            .grid {
                display: grid;
                grid-template-columns: 100px 1fr 2fr;
                grid-template-rows: repeat(3, auto);
                grid-gap: 8px 16px;
            }
        </style></head><body><div class="grid"></div></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let d = dom.find("div").unwrap();
        let s = eng.compute(&d);
        assert_eq!(s.display, Display::Grid);
        assert_eq!(s.grid_template_columns.len(), 3);
        assert!(matches!(s.grid_template_columns[0], GridTrackSize::Px(100.0)));
        assert!(matches!(s.grid_template_columns[1], GridTrackSize::Fr(1.0)));
        assert!(matches!(s.grid_template_columns[2], GridTrackSize::Fr(2.0)));
        assert_eq!(s.grid_template_rows.len(), 3);
        assert!(matches!(s.grid_template_rows[0], GridTrackSize::Auto));
        assert_eq!(s.gap_row, 8.0);
        assert_eq!(s.gap_column, 16.0);
    }

    #[test]
    fn unidades_viewport_resuelven() {
        assert_eq!(parse_length_px("50vw"), Some(640.0));
        assert_eq!(parse_length_px("25vh"), Some(200.0));
        assert_eq!(parse_length_px("10vmin"), Some(80.0));
        assert_eq!(parse_length_px("10vmax"), Some(128.0));
    }

    #[test]
    fn viewport_scope_cambia_y_restaura_la_resolucion() {
        // Fuera de scope: DEFAULT_VIEWPORT (1280×800).
        assert_eq!(parse_length_px("50vw"), Some(640.0));
        {
            let _g = ViewportScope::new(Viewport { width: 800.0, height: 600.0, dpr: 1.0 });
            assert_eq!(parse_length_px("50vw"), Some(400.0));
            assert_eq!(parse_length_px("50vh"), Some(300.0));
            assert_eq!(parse_length_px("50vmin"), Some(300.0));
            assert_eq!(parse_length_px("50vmax"), Some(400.0));
            // Anida: el scope interno gana y el externo se recupera al salir.
            {
                let _g2 = ViewportScope::new(Viewport { width: 200.0, height: 200.0, dpr: 1.0 });
                assert_eq!(parse_length_px("50vw"), Some(100.0));
            }
            assert_eq!(parse_length_px("50vw"), Some(400.0));
        }
        // Al dropear el guard, vuelve a DEFAULT.
        assert_eq!(parse_length_px("50vw"), Some(640.0));
    }

    #[test]
    fn media_query_filtra_segun_viewport() {
        assert!(!evaluate_media_query("(max-width: 600px)", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query("(min-width: 1024px)", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query(
            "(min-width: 800px) and (max-width: 1920px)",
            DEFAULT_VIEWPORT,
        ));
        assert!(!evaluate_media_query("print", DEFAULT_VIEWPORT));
        assert!(evaluate_media_query("screen", DEFAULT_VIEWPORT));

        let html = r#"<html><head><style>
            @media (max-width: 600px) { p { color: red } }
            @media (min-width: 1024px) { p { color: blue } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn media_query_orientation_resolution_y_combinadores() {
        let portrait = Viewport { width: 400.0, height: 900.0, dpr: 1.0 };
        let landscape = Viewport { width: 900.0, height: 400.0, dpr: 1.0 };
        let retina = Viewport { width: 900.0, height: 400.0, dpr: 2.0 };

        // orientation.
        assert!(evaluate_media_query("(orientation: portrait)", portrait));
        assert!(!evaluate_media_query("(orientation: portrait)", landscape));
        assert!(evaluate_media_query("(orientation: landscape)", landscape));

        // resolution (dppx / x / dpi).
        assert!(evaluate_media_query("(min-resolution: 2dppx)", retina));
        assert!(!evaluate_media_query("(min-resolution: 2dppx)", landscape));
        assert!(evaluate_media_query("(min-resolution: 2x)", retina));
        assert!(evaluate_media_query("(min-resolution: 192dpi)", retina));
        assert!(evaluate_media_query("(max-resolution: 1dppx)", landscape));

        // Lista OR (`,`): matchea si cualquiera lo hace.
        assert!(evaluate_media_query("(max-width: 100px), (orientation: landscape)", landscape));
        assert!(!evaluate_media_query("(max-width: 100px), (max-height: 100px)", landscape));

        // `not` invierte la query completa.
        assert!(evaluate_media_query("not (max-width: 100px)", landscape));
        assert!(!evaluate_media_query("not (orientation: landscape)", landscape));

        // Preferencias: reportamos tema claro y sin reducción de movimiento.
        assert!(evaluate_media_query("(prefers-color-scheme: light)", landscape));
        assert!(!evaluate_media_query("(prefers-color-scheme: dark)", landscape));
        assert!(evaluate_media_query("(prefers-reduced-motion: no-preference)", landscape));

        // `and` mezclando dimensión + orientación + resolución.
        assert!(evaluate_media_query(
            "screen and (min-width: 800px) and (orientation: landscape) and (min-resolution: 2dppx)",
            retina,
        ));
        assert!(!evaluate_media_query(
            "screen and (min-width: 800px) and (min-resolution: 2dppx)",
            landscape, // dpr 1.0 → falla la última
        ));

        // aspect-ratio (W/H y número). landscape = 900/400 = 2.25.
        assert!(evaluate_media_query("(min-aspect-ratio: 16/9)", landscape)); // 2.25 >= 1.77
        assert!(!evaluate_media_query("(min-aspect-ratio: 16/9)", portrait)); // 0.44 < 1.77
        assert!(evaluate_media_query("(max-aspect-ratio: 1/1)", portrait)); // 0.44 <= 1.0
        assert!(!evaluate_media_query("(max-aspect-ratio: 1/1)", landscape)); // 2.25 > 1.0
        assert!(evaluate_media_query("(min-aspect-ratio: 2)", landscape)); // 2.25 >= 2

        // Feature desconocida no descalifica (lenient, igual que antes).
        assert!(evaluate_media_query("(quantum-foam: 3)", landscape));
    }

    #[test]
    fn from_dom_with_viewport_selecciona_media_por_ancho_real() {
        let html = r#"<html><head><style>
            p { color: green }
            @media (max-width: 600px) { p { color: red } }
            @media (min-width: 601px) { p { color: blue } }
        </style></head><body><p>x</p></body></html>"#;
        let dom = DomTree::parse(html);

        // Viewport angosto → gana la regla red.
        let eng = StyleEngine::from_dom_with_viewport(&dom, Viewport { width: 500.0, height: 800.0, dpr: 1.0 });
        let p = dom.find("p").unwrap();
        assert_eq!(eng.compute(&p).color, Color::rgb(255, 0, 0), "ancho 500 → red");

        // Viewport ancho → gana la regla blue.
        let eng = StyleEngine::from_dom_with_viewport(&dom, Viewport { width: 1200.0, height: 800.0, dpr: 1.0 });
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255), "ancho 1200 → blue");

        // `from_dom` sin viewport cae en DEFAULT_VIEWPORT (1280) → blue.
        let eng = StyleEngine::from_dom(&dom);
        assert_eq!(eng.compute(&p).color, Color::rgb(0, 0, 255), "default 1280 → blue");
    }

    #[test]
    fn attr_selector_flag_case_insensitive() {
        let html = r#"<html><head><style>
            [data-x="hello" i] { color: rgb(0,0,255) }
            [type="EMAIL"] { color: rgb(255,0,0) }
            [href^="HTTP" i] { color: rgb(0,128,0) }
        </style></head><body>
            <p id="a" data-x="HELLO">a</p>
            <input id="c" type="email">
            <input id="d" type="EMAIL">
            <a id="e" href="https://x">e</a>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // `[data-x="hello" i]` matchea "HELLO" (insensible).
        assert_eq!(eng.compute(&by_id("a")).color, Color::rgb(0, 0, 255));
        // `[type="EMAIL"]` SIN flag es case-sensitive: "email" no matchea.
        assert_ne!(eng.compute(&by_id("c")).color, Color::rgb(255, 0, 0));
        // "EMAIL" exacto sí matchea.
        assert_eq!(eng.compute(&by_id("d")).color, Color::rgb(255, 0, 0));
        // Prefijo con flag i: `[href^="HTTP" i]` matchea "https://x".
        assert_eq!(eng.compute(&by_id("e")).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn css_nesting_expande_y_aplica() {
        let html = r#"<html><head><style>
            .card {
                color: rgb(1,1,1);
                .title { color: rgb(0,0,255) }
                &.active { color: rgb(0,128,0) }
            }
            .menu { & > li { color: rgb(255,0,0) } }
        </style></head><body>
            <div id="c1" class="card"><span id="t" class="title">t</span></div>
            <div id="c2" class="card active">a</div>
            <ul class="menu"><li id="li1">x</li></ul>
        </body></html>"#;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let by_id = |id: &str| -> Handle {
            let mut found = None;
            crate::dom::walk(&dom.document(), &mut |n| {
                if crate::dom::attr(n, "id").as_deref() == Some(id) {
                    found = Some(n.clone());
                }
            });
            found.unwrap()
        };
        // Declaración propia del padre.
        assert_eq!(eng.compute(&by_id("c1")).color, Color::rgb(1, 1, 1));
        // Anidada descendiente implícita: `.card .title`.
        assert_eq!(eng.compute(&by_id("t")).color, Color::rgb(0, 0, 255));
        // `&.active` → `.card.active` (mayor especificidad gana al padre).
        assert_eq!(eng.compute(&by_id("c2")).color, Color::rgb(0, 128, 0));
        // `& > li` → `.menu > li`.
        assert_eq!(eng.compute(&by_id("li1")).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn media_query_sintaxis_de_rango() {
        // DEFAULT_VIEWPORT = 1280 × 800, dpr 1.
        let vp = DEFAULT_VIEWPORT;
        // `feature op value`.
        assert!(evaluate_media_query("(width >= 600px)", vp));
        assert!(!evaluate_media_query("(width <= 600px)", vp));
        assert!(evaluate_media_query("(width >= 1280px)", vp));
        assert!(!evaluate_media_query("(width > 1280px)", vp));
        assert!(evaluate_media_query("(width < 2000px)", vp));
        // `value op feature` (orden invertido).
        assert!(evaluate_media_query("(600px < width)", vp));
        assert!(!evaluate_media_query("(2000px < width)", vp));
        // Rango de dos lados.
        assert!(evaluate_media_query("(400px <= width <= 1500px)", vp));
        assert!(!evaluate_media_query("(400px <= width <= 800px)", vp));
        // Sin espacios.
        assert!(evaluate_media_query("(width>=600px)", vp));
        // height + combinación con `and`.
        assert!(evaluate_media_query("(height < 1000px) and (width > 1000px)", vp));
        // El path `feature: value` clásico sigue funcionando (regresión).
        assert!(evaluate_media_query("(min-width: 600px)", vp));
        assert!(!evaluate_media_query("(max-width: 600px)", vp));
    }

    #[test]
    fn ua_body_lleva_margin_8() {
        // Cualquier página sin CSS de autor debe arrancar con el body
        // margin: 8px (default del browser real).
        let html = "<html><body>x</body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let body = dom.find("body").unwrap();
        let s = eng.compute(&body);
        assert_eq!(s.margin, Sides::all(8.0));
    }

    #[test]
    fn ua_h3_h4_h5_h6_tienen_tamanos_propios() {
        // Antes h3+ caían al default 16 (igual que `<p>`). Ahora cada
        // nivel tiene tamaño y margin propios.
        for (tag, expected) in
            [("h3", 19.0), ("h4", 16.0), ("h5", 13.0), ("h6", 11.0)]
        {
            let html = format!("<html><body><{tag}>x</{tag}></body></html>");
            let dom = DomTree::parse(&html);
            let eng = StyleEngine::from_dom(&dom);
            let node = dom.find(tag).unwrap();
            let s = eng.compute(&node);
            assert_eq!(s.font_size, expected, "{tag} font-size");
        }
    }

    #[test]
    fn ua_ul_y_ol_padding_left_para_bullets() {
        let html = "<html><body><ul><li>x</li></ul></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let ul = dom.find("ul").unwrap();
        let s = eng.compute(&ul);
        assert_eq!(s.padding.left, 40.0);
    }

    #[test]
    fn ua_a_color_azul_default() {
        let html = "<html><body><a href=#>link</a></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let a = dom.find("a").unwrap();
        let s = eng.compute(&a);
        assert_eq!(s.color, Color::rgb(0, 0, 238));
    }

    #[test]
    fn ua_svg_y_canvas_inline_block_video_none() {
        // SVG y `<canvas>` se renderizan (primitivas vía vello / comandos 2D
        // del runtime), así que quedan como inline-block (Fase 7.196 cableó
        // canvas). math/video/audio/etc. siguen ocultos hasta tener renderer.
        let html = "<html><body><svg></svg><canvas></canvas><video></video></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let svg = dom.find("svg").unwrap();
        let canvas = dom.find("canvas").unwrap();
        let video = dom.find("video").unwrap();
        assert_eq!(eng.compute(&svg).display, Display::InlineBlock);
        assert_eq!(eng.compute(&canvas).display, Display::InlineBlock);
        assert_eq!(eng.compute(&video).display, Display::None);
    }

    #[test]
    fn ua_table_layout_minimo() {
        let html = "<html><body><table><tr><td>a</td><td>b</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let table = dom.find("table").unwrap();
        let tr = dom.find("tr").unwrap();
        let td = dom.find("td").unwrap();
        assert_eq!(eng.compute(&table).display, Display::Block);
        // tr es Flex row para que td/td queden lado a lado.
        assert_eq!(eng.compute(&tr).display, Display::Flex);
        // td es InlineBlock para que el row de flex no le dé 100% width.
        assert_eq!(eng.compute(&td).display, Display::InlineBlock);
    }

    #[test]
    fn ua_table_cells_tienen_border_y_padding() {
        // Tablas sin CSS de autor deben mostrar bordes para que la grilla
        // se vea — sino tablas sin estilo (Wikipedia raw, RFC docs, etc.)
        // colapsan visualmente.
        let html = "<html><body><table><tr><th>h</th><td>d</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let th = dom.find("th").unwrap();
        let td = dom.find("td").unwrap();
        let s_th = eng.compute(&th);
        let s_td = eng.compute(&td);
        assert_eq!(s_th.border_widths.top, 1.0);
        assert!(s_th.border_colors.top.is_some());
        assert_eq!(s_td.border_widths.top, 1.0);
        assert_eq!(s_th.padding, Sides::all(4.0));
        assert_eq!(s_td.padding, Sides::all(4.0));
        // `<th>` lleva un bg gris claro para destacarlo como header.
        assert_eq!(s_th.background, Some(Color::rgb(242, 242, 242)));
    }

    #[test]
    fn ua_colgroup_y_col_ocultos() {
        // `<colgroup><col>` son metadatos de columna — no se renderean.
        let html = "<html><body><table><colgroup><col><col></colgroup><tr><td>x</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let colgroup = dom.find("colgroup").unwrap();
        let col = dom.find("col").unwrap();
        assert_eq!(eng.compute(&colgroup).display, Display::None);
        assert_eq!(eng.compute(&col).display, Display::None);
    }

    #[test]
    fn ua_caption_centrado() {
        let html = "<html><body><table><caption>Tabla X</caption><tr><td>a</td></tr></table></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let cap = dom.find("caption").unwrap();
        let s = eng.compute(&cap);
        assert_eq!(s.display, Display::Block);
        assert_eq!(s.text_align, TextAlign::Center);
    }

    #[test]
    fn ua_sub_y_sup_aplican_vertical_align() {
        let html = "<html><body><p>H<sub>2</sub>O y E=mc<sup>2</sup></p></body></html>";
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let sub = dom.find("sub").unwrap();
        let sup = dom.find("sup").unwrap();
        assert_eq!(eng.compute(&sub).vertical_align, VerticalAlign::Sub);
        assert_eq!(eng.compute(&sup).vertical_align, VerticalAlign::Super);
    }

