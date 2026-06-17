//! Tests del motor de estilo (grupo 01, extraído de `style/mod.rs`, regla #1).
use super::super::*;

    #[test]
    fn parsea_hex_color() {
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("red"), Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn parsea_radial_gradient() {
        let grad = |v: &str| match parse_background_image(v) {
            Some(DeclKind::BackgroundGradient(g)) => g,
            other => panic!("esperaba gradiente, {other:?}"),
        };
        // Sin prelude: default farthest-corner at center, 2 stops.
        let g = grad("radial-gradient(red, blue)");
        let spec = g.radial().expect("debe ser radial");
        assert_eq!(spec.size, RadialSize::FarthestCorner);
        assert_eq!(spec.cx, LengthVal::Pct(50.0));
        assert_eq!(spec.cy, LengthVal::Pct(50.0));
        assert_eq!(g.stops.len(), 2);
        // shape + size + posición.
        let g = grad("radial-gradient(circle closest-side at 30% 70%, red 0%, blue 100%)");
        let spec = g.radial().unwrap();
        assert_eq!(spec.size, RadialSize::ClosestSide);
        assert_eq!(spec.cx, LengthVal::Pct(30.0));
        assert_eq!(spec.cy, LengthVal::Pct(70.0));
        // Sólo `at <pos>` con keywords.
        let g = grad("radial-gradient(at top left, red, blue)");
        let spec = g.radial().unwrap();
        assert_eq!(spec.cx, LengthVal::Pct(0.0));
        assert_eq!(spec.cy, LengthVal::Pct(0.0));
        // El lineal sigue sin radial.
        assert!(grad("linear-gradient(to right, red, blue)").radial().is_none());
    }

    #[test]
    fn parsea_conic_gradient() {
        let grad = |v: &str| match parse_background_image(v) {
            Some(DeclKind::BackgroundGradient(g)) => g,
            other => panic!("esperaba gradiente, {other:?}"),
        };
        let conic = |g: &LinearGradient| match g.geometry {
            GradientGeometry::Conic { from_deg, cx, cy } => (from_deg, cx, cy),
            other => panic!("esperaba conic, {other:?}"),
        };
        // Sin prelude: from 0 at center.
        let (from, cx, cy) = conic(&grad("conic-gradient(red, blue)"));
        assert_eq!(from, 0.0);
        assert_eq!(cx, LengthVal::Pct(50.0));
        assert_eq!(cy, LengthVal::Pct(50.0));
        // from <angle> + at <pos>; turn → grados.
        let (from, cx, cy) = conic(&grad("conic-gradient(from 0.25turn at 20% 80%, red, blue)"));
        assert!((from - 90.0).abs() < 1e-3);
        assert_eq!(cx, LengthVal::Pct(20.0));
        assert_eq!(cy, LengthVal::Pct(80.0));
        // Sólo `from <deg>`.
        let (from, _, _) = conic(&grad("conic-gradient(from 45deg, red, blue)"));
        assert!((from - 45.0).abs() < 1e-3);
        assert_eq!(grad("conic-gradient(red, blue)").stops.len(), 2);

        // Posiciones de stop angulares: `90deg`/`0.25turn` → Px(grados); `%`
        // sigue siendo Pct. El render trata el eje cónico como 360°.
        let g = grad("conic-gradient(red 90deg, blue 0.25turn, lime 75%)");
        assert_eq!(g.stops[0].pos, Some(LengthVal::Px(90.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(90.0)));
        assert_eq!(g.stops[2].pos, Some(LengthVal::Pct(75.0)));
        // Doble posición angular `red 0deg 90deg` ⇒ dos stops.
        let g = grad("repeating-conic-gradient(red 0deg 90deg, blue 90deg 180deg)");
        assert_eq!(g.stops.len(), 4);
        assert_eq!(g.stops[0].pos, Some(LengthVal::Px(0.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(90.0)));
    }

    #[test]
    fn parsea_repeating_gradients_y_stops_px() {
        let grad = |v: &str| match parse_background_image(v) {
            Some(DeclKind::BackgroundGradient(g)) => g,
            other => panic!("esperaba gradiente, {other:?}"),
        };

        // `repeating-*` activa el flag; el no-repetido lo deja en false.
        assert!(grad("repeating-linear-gradient(red, blue 20px)").repeating);
        assert!(grad("repeating-radial-gradient(circle, red, blue 30px)").repeating);
        assert!(grad("repeating-conic-gradient(red, blue 25%)").repeating);
        assert!(!grad("linear-gradient(red, blue)").repeating);
        assert!(matches!(
            grad("repeating-linear-gradient(45deg, red, blue 10px)").geometry,
            GradientGeometry::Linear { .. }
        ));

        // Posiciones de stop: % → Pct, px → Px reales (no la vieja heurística
        // /100), `auto`/sin posición → None.
        let g = grad("linear-gradient(red 40%, blue 30px)");
        assert_eq!(g.stops[0].pos, Some(LengthVal::Pct(40.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(30.0)));

        // Doble posición `#ccc 0 10px` ⇒ dos stops del mismo color (franjas).
        let g = grad("repeating-linear-gradient(#ccc 0 10px, #fff 10px 20px)");
        assert_eq!(g.stops.len(), 4);
        assert_eq!(g.stops[0].color, g.stops[1].color);
        assert_eq!(g.stops[0].pos, Some(LengthVal::Px(0.0)));
        assert_eq!(g.stops[1].pos, Some(LengthVal::Px(10.0)));
        assert_eq!(g.stops[2].color, g.stops[3].color);
        assert_eq!(g.stops[3].pos, Some(LengthVal::Px(20.0)));
    }

    #[test]
    fn parsea_named_colors_extendidos() {
        // Tabla CSS3 completa: colores que antes dropeaban la declaración.
        assert_eq!(parse_color("coral"), Some(Color::rgb(255, 127, 80)));
        assert_eq!(parse_color("tomato"), Some(Color::rgb(255, 99, 71)));
        assert_eq!(parse_color("slateblue"), Some(Color::rgb(106, 90, 205)));
        assert_eq!(parse_color("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
        assert_eq!(parse_color("darkslategray"), Some(Color::rgb(47, 79, 79)));
        // Case-insensitive + variante grey.
        assert_eq!(parse_color("SteelBlue"), Some(Color::rgb(70, 130, 180)));
        assert_eq!(parse_color("dimgrey"), Some(Color::rgb(105, 105, 105)));
        // No-color sigue siendo None.
        assert_eq!(parse_color("notacolor"), None);
    }

    #[test]
    fn parsea_length() {
        assert_eq!(parse_length_px("12px"), Some(12.0));
        assert_eq!(parse_length_px("1.5em"), Some(24.0));
        assert_eq!(parse_length_px("0"), Some(0.0));
        assert_eq!(parse_length_px("xyz"), None);
    }

    #[test]
    fn parse_content_value_acepta_string_quoted() {
        assert_eq!(
            parse_content_value(r#""hola""#),
            Some(vec![ContentItem::Text("hola".into())])
        );
        assert_eq!(
            parse_content_value(r#"'mundo'"#),
            Some(vec![ContentItem::Text("mundo".into())])
        );
        assert_eq!(parse_content_value("none"), None);
        assert_eq!(parse_content_value("normal"), None);
        // Sin comillas y sin counter()/attr() → None.
        assert_eq!(parse_content_value("foo"), None);
    }

    #[test]
    fn parse_content_value_respeta_escapes() {
        assert_eq!(
            parse_content_value(r#""linea1\nlinea2""#),
            Some(vec![ContentItem::Text("linea1nlinea2".into())]) // \n no especial
        );
        assert_eq!(
            parse_content_value(r#""con \"quote\" adentro""#),
            Some(vec![ContentItem::Text(r#"con "quote" adentro"#.into())])
        );
    }

    #[test]
    fn parse_content_value_concat_counter_attr() {
        let items = parse_content_value(r#""Sección " counter(sec) ": " attr(data-title)"#)
            .expect("debería parsear");
        assert_eq!(
            items,
            vec![
                ContentItem::Text("Sección ".into()),
                ContentItem::Counter("sec".into()),
                ContentItem::Text(": ".into()),
                ContentItem::Attr("data-title".into()),
            ]
        );
    }

    #[test]
    fn parse_counter_list_acepta_pares_y_defaults() {
        assert_eq!(
            parse_counter_list("section 0 chapter 5", 0),
            vec![("section".into(), 0), ("chapter".into(), 5)]
        );
        // Default cuando no hay valor explícito.
        assert_eq!(
            parse_counter_list("h2", 1),
            vec![("h2".into(), 1)]
        );
        assert_eq!(parse_counter_list("none", 0), Vec::<(String, i32)>::new());
    }

    #[test]
    fn pseudo_element_extrae_del_selector() {
        let html = r##"<html><head><style>
            p::before { content: "PRE " }
            p::after { content: " POST" }
            p:before { content: "legacy" }
        </style></head><body><p>x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let before = eng.compute_pseudo(&p, PseudoElement::Before, None);
        let after = eng.compute_pseudo(&p, PseudoElement::After, None);
        // `:before` legacy también matchea Before pero llega después; el
        // último gana en empate de especificidad.
        assert_eq!(
            before.and_then(|s| s.content),
            Some(vec![ContentItem::Text("legacy".into())])
        );
        assert_eq!(
            after.and_then(|s| s.content),
            Some(vec![ContentItem::Text(" POST".into())])
        );
    }

    #[test]
    fn pseudo_element_sin_content_no_se_materializa() {
        // Una regla `::before` sin content → compute_pseudo devuelve None.
        let html = r##"<html><head><style>
            p::before { color: red }
        </style></head><body><p>x</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        assert!(eng.compute_pseudo(&p, PseudoElement::Before, None).is_none());
    }

    #[test]
    fn reglas_pseudo_no_pegan_al_elemento_real() {
        // `p::before { color: red }` NO debe afectar el color de `<p>`
        // — sólo de su `::before`.
        let html = r##"<html><head><style>
            p::before { content: "X"; color: red }
            p { color: blue }
        </style></head><body><p>texto</p></body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let p = dom.find("p").unwrap();
        let s = eng.compute(&p);
        assert_eq!(s.color, Color::rgb(0, 0, 255)); // blue, no red
    }

    #[test]
    fn parsea_z_index() {
        let html = r##"<html><head><style>
            .a { z-index: 5 }
            .b { z-index: -2 }
            .c { z-index: auto }
        </style></head><body>
            <div class="a"></div>
            <div class="b"></div>
            <div class="c"></div>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut divs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("div") {
                divs.push(n.clone());
            }
        });
        assert_eq!(divs.len(), 3);
        assert_eq!(eng.compute(&divs[0]).z_index, 5);
        assert_eq!(eng.compute(&divs[1]).z_index, -2);
        assert_eq!(eng.compute(&divs[2]).z_index, 0); // auto → 0
    }

    #[test]
    fn parsea_object_fit_y_llega_a_computed() {
        // Parser: keywords válidos (case-insensitive) e inválido → None.
        assert_eq!(parse_object_fit("cover"), Some(ObjectFit::Cover));
        assert_eq!(parse_object_fit("scale-down"), Some(ObjectFit::ScaleDown));
        assert_eq!(parse_object_fit("CONTAIN"), Some(ObjectFit::Contain));
        assert_eq!(parse_object_fit("fill"), Some(ObjectFit::Fill));
        assert_eq!(parse_object_fit("stretch"), None);

        let html = r##"<html><head><style>
            img.cov { object-fit: cover }
            img.plain { color: red }
        </style></head><body>
            <img class="cov" src="x.png">
            <img class="plain" src="y.png">
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut imgs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("img") {
                imgs.push(n.clone());
            }
        });
        assert_eq!(imgs.len(), 2);
        assert_eq!(eng.compute(&imgs[0]).object_fit, Some(ObjectFit::Cover));
        // Sin object-fit declarado → None (el chrome mantiene su encaje
        // por defecto, contain responsivo vía el compositor).
        assert_eq!(eng.compute(&imgs[1]).object_fit, None);
    }

    #[test]
    fn parsea_object_position_reusa_background_position() {
        let html = r##"<html><head><style>
            img.tr { object-position: right top }
            img.pct { object-position: 25% 75% }
            img.plain { color: red }
        </style></head><body>
            <img class="tr" src="a.png">
            <img class="pct" src="b.png">
            <img class="plain" src="c.png">
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut imgs = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            if crate::dom::element_name(n).as_deref() == Some("img") {
                imgs.push(n.clone());
            }
        });
        assert_eq!(imgs.len(), 3);
        // `right top` → x=100% (derecha), y=0% (arriba).
        assert_eq!(
            eng.compute(&imgs[0]).object_position,
            Some(BackgroundPosition { x: LengthVal::Pct(100.0), y: LengthVal::Pct(0.0) })
        );
        assert_eq!(
            eng.compute(&imgs[1]).object_position,
            Some(BackgroundPosition { x: LengthVal::Pct(25.0), y: LengthVal::Pct(75.0) })
        );
        // Sin declarar → None (el chrome centra).
        assert_eq!(eng.compute(&imgs[2]).object_position, None);
    }

    #[test]
    fn caret_color_fase_7_238() {
        // Parser puro.
        assert_eq!(parse_caret_color("auto"), None);
        assert_eq!(parse_caret_color("AUTO"), None);
        assert_eq!(parse_caret_color("currentColor"), None);
        assert_eq!(parse_caret_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_caret_color("zigzag"), None);

        // End-to-end: declarado, sin declarar, y herencia padre→hijo
        // (vía `compute_with_parent` — `compute()` no traversa).
        let html = r##"<html><head><style>
            body { caret-color: #00ff00 }
            input.a { caret-color: red }
            input.auto { caret-color: auto }
            input.plain {}
        </style></head><body>
          <input class="a"><input class="auto"><input class="plain">
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
        assert_eq!(body_cs.caret_color, Some(Color::rgb(0, 255, 0)));
        assert_eq!(inputs.len(), 3);
        assert_eq!(
            eng.compute_with_parent(&inputs[0], Some(&body_cs)).caret_color,
            Some(Color::rgb(255, 0, 0))
        );
        assert_eq!(eng.compute_with_parent(&inputs[1], Some(&body_cs)).caret_color, None);
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&inputs[2], Some(&body_cs)).caret_color,
            Some(Color::rgb(0, 255, 0))
        );
    }

    #[test]
    fn accent_color_fase_7_239() {
        assert_eq!(parse_auto_or_color("auto"), None);
        assert_eq!(parse_auto_or_color("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
        assert_eq!(parse_auto_or_color("zigzag"), None);

        let html = r##"<html><head><style>
            body { accent-color: #112233 }
            input.a { accent-color: blue }
            input.auto { accent-color: auto }
            input.plain {}
        </style></head><body>
          <input class="a"><input class="auto"><input class="plain">
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
        assert_eq!(body_cs.accent_color, Some(Color::rgb(0x11, 0x22, 0x33)));
        assert_eq!(
            eng.compute_with_parent(&inputs[0], Some(&body_cs)).accent_color,
            Some(Color::rgb(0, 0, 255))
        );
        assert_eq!(eng.compute_with_parent(&inputs[1], Some(&body_cs)).accent_color, None);
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&inputs[2], Some(&body_cs)).accent_color,
            Some(Color::rgb(0x11, 0x22, 0x33))
        );
    }

    #[test]
    fn cursor_fase_7_240() {
        // Parser puro: keywords reconocidos + fallback `auto` para
        // lo no soportado + tail-of-list (CSS `cursor: url(...), pointer`).
        assert_eq!(parse_cursor("pointer"), Some(Cursor::Pointer));
        assert_eq!(parse_cursor("POINTER"), Some(Cursor::Pointer));
        assert_eq!(parse_cursor("not-allowed"), Some(Cursor::NotAllowed));
        assert_eq!(parse_cursor("zoom-in"), Some(Cursor::ZoomIn));
        assert_eq!(parse_cursor("nesw-resize"), Some(Cursor::NeswResize));
        assert_eq!(parse_cursor("xyz"), Some(Cursor::Auto));
        // Lista CSS — tomamos el último fallback reconocido.
        assert_eq!(parse_cursor("url(a.png), pointer"), Some(Cursor::Pointer));
        assert_eq!(parse_cursor("url(a.png), nope"), Some(Cursor::Auto));

        // End-to-end: declarado, default UA y heredado.
        let html = r##"<html><head><style>
            body { cursor: text }
            a.btn { cursor: pointer }
            a.plain {}
        </style></head><body>
          <a class="btn">x</a><a class="plain">y</a><span>z</span>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut anchors = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("a") => anchors.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.cursor, Cursor::Text);
        assert_eq!(eng.compute_with_parent(&anchors[0], Some(&body_cs)).cursor, Cursor::Pointer);
        // `a.plain` sin regla de autor: la hoja UA `a { cursor: pointer }`
        // (Fase 7.1250) gana sobre la herencia de `body` — como en un browser
        // real. La herencia sólo aplica donde no hay declaración que matchee.
        assert_eq!(eng.compute_with_parent(&anchors[1], Some(&body_cs)).cursor, Cursor::Pointer);
        // El `<span>` (sin regla UA de cursor) sí hereda `text` de body.
        assert_eq!(eng.compute_with_parent(&spans[0], Some(&body_cs)).cursor, Cursor::Text);
    }

    #[test]
    fn text_overflow_fase_7_241() {
        assert_eq!(parse_text_overflow("clip"), Some(TextOverflow::Clip));
        assert_eq!(parse_text_overflow("ELLIPSIS"), Some(TextOverflow::Ellipsis));
        assert_eq!(parse_text_overflow("fade"), None);

        let html = r##"<html><head><style>
            body { text-overflow: ellipsis }
            p.a { text-overflow: clip }
            p.plain {}
        </style></head><body>
          <p class="a"></p><p class="plain"></p>
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
        assert_eq!(body_cs.text_overflow, TextOverflow::Ellipsis);
        // text-overflow NO hereda — el hijo sin declarar mantiene el default (Clip),
        // no toma el `ellipsis` del body.
        let p_a = eng.compute_with_parent(&ps[0], Some(&body_cs));
        assert_eq!(p_a.text_overflow, TextOverflow::Clip);
        let p_plain = eng.compute_with_parent(&ps[1], Some(&body_cs));
        assert_eq!(p_plain.text_overflow, TextOverflow::Clip);
    }

    #[test]
    fn scroll_behavior_fase_7_242() {
        assert_eq!(parse_scroll_behavior("auto"), Some(ScrollBehavior::Auto));
        assert_eq!(parse_scroll_behavior("SMOOTH"), Some(ScrollBehavior::Smooth));
        assert_eq!(parse_scroll_behavior("instant"), None);

        let html = r##"<html><head><style>
            body { scroll-behavior: smooth }
            div.a { scroll-behavior: auto }
            div.plain {}
        </style></head><body>
          <div class="a"></div><div class="plain"></div>
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
        assert_eq!(body_cs.scroll_behavior, ScrollBehavior::Smooth);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).scroll_behavior,
            ScrollBehavior::Auto
        );
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).scroll_behavior,
            ScrollBehavior::Smooth
        );
    }

    #[test]
    fn tab_size_fase_7_243() {
        assert_eq!(parse_tab_size("4"), Some(TabSize::Chars(4)));
        assert_eq!(parse_tab_size("0"), Some(TabSize::Chars(0)));
        assert_eq!(parse_tab_size("32px"), Some(TabSize::Px(32.0)));
        assert_eq!(parse_tab_size("-1"), None);
        assert_eq!(parse_tab_size("xx"), None);

        let html = r##"<html><head><style>
            body { tab-size: 4 }
            pre.a { tab-size: 16px }
            pre.plain {}
        </style></head><body>
          <pre class="a"></pre><pre class="plain"></pre>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut pres = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("pre") => pres.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.tab_size, TabSize::Chars(4));
        assert_eq!(
            eng.compute_with_parent(&pres[0], Some(&body_cs)).tab_size,
            TabSize::Px(16.0)
        );
        // Heredado de body.
        assert_eq!(
            eng.compute_with_parent(&pres[1], Some(&body_cs)).tab_size,
            TabSize::Chars(4)
        );
    }

    #[test]
    fn user_select_fase_7_244() {
        assert_eq!(parse_user_select("none"), Some(UserSelect::None));
        assert_eq!(parse_user_select("TEXT"), Some(UserSelect::Text));
        assert_eq!(parse_user_select("all"), Some(UserSelect::All));
        assert_eq!(parse_user_select("contain"), Some(UserSelect::Contain));
        assert_eq!(parse_user_select("auto"), Some(UserSelect::Auto));
        assert_eq!(parse_user_select("nada"), None);

        let html = r##"<html><head><style>
            body { user-select: text }
            div.lock { user-select: none }
            div.plain {}
        </style></head><body>
          <div class="lock"></div><div class="plain"></div>
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
        assert_eq!(body_cs.user_select, UserSelect::Text);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).user_select,
            UserSelect::None
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).user_select,
            UserSelect::Text
        );
    }

    #[test]
    fn overflow_wrap_fase_7_245() {
        assert_eq!(parse_overflow_wrap("normal"), Some(OverflowWrap::Normal));
        assert_eq!(parse_overflow_wrap("break-word"), Some(OverflowWrap::BreakWord));
        assert_eq!(parse_overflow_wrap("ANYWHERE"), Some(OverflowWrap::Anywhere));
        assert_eq!(parse_overflow_wrap("nope"), None);

        // `word-wrap` alias legacy.
        let html = r##"<html><head><style>
            body { word-wrap: break-word }
            p.b {}
            p.over { overflow-wrap: anywhere }
        </style></head><body>
          <p class="b"></p><p class="over"></p>
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
        assert_eq!(body_cs.overflow_wrap, OverflowWrap::BreakWord);
        // Heredado del body.
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).overflow_wrap,
            OverflowWrap::BreakWord
        );
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).overflow_wrap,
            OverflowWrap::Anywhere
        );
    }

    #[test]
    fn word_break_fase_7_246() {
        assert_eq!(parse_word_break("normal"), Some(WordBreak::Normal));
        assert_eq!(parse_word_break("break-all"), Some(WordBreak::BreakAll));
        assert_eq!(parse_word_break("keep-all"), Some(WordBreak::KeepAll));
        // `break-word` legacy → Normal por compat.
        assert_eq!(parse_word_break("break-word"), Some(WordBreak::Normal));
        assert_eq!(parse_word_break("nada"), None);

        let html = r##"<html><head><style>
            body { word-break: break-all }
            p.k { word-break: keep-all }
            p.plain {}
        </style></head><body>
          <p class="k"></p><p class="plain"></p>
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
        assert_eq!(body_cs.word_break, WordBreak::BreakAll);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).word_break,
            WordBreak::KeepAll
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).word_break,
            WordBreak::BreakAll
        );
    }

    #[test]
    fn hyphens_fase_7_247() {
        assert_eq!(parse_hyphens("none"), Some(Hyphens::None));
        assert_eq!(parse_hyphens("MANUAL"), Some(Hyphens::Manual));
        assert_eq!(parse_hyphens("auto"), Some(Hyphens::Auto));
        assert_eq!(parse_hyphens("x"), None);

        let html = r##"<html><head><style>
            body { -webkit-hyphens: auto }
            p.off { hyphens: none }
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
        assert_eq!(body_cs.hyphens, Hyphens::Auto);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).hyphens,
            Hyphens::None
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).hyphens,
            Hyphens::Auto
        );
    }

    #[test]
    fn resize_fase_7_248() {
        assert_eq!(parse_resize("none"), Some(Resize::None));
        assert_eq!(parse_resize("both"), Some(Resize::Both));
        assert_eq!(parse_resize("HORIZONTAL"), Some(Resize::Horizontal));
        assert_eq!(parse_resize("vertical"), Some(Resize::Vertical));
        assert_eq!(parse_resize("block"), Some(Resize::Block));
        assert_eq!(parse_resize("inline"), Some(Resize::Inline));
        assert_eq!(parse_resize("auto"), None);

        // `resize` NO se hereda (CSS UI 4).
        let html = r##"<html><head><style>
            body { resize: both }
            div.r { resize: vertical }
            div.plain {}
        </style></head><body>
          <div class="r"></div><div class="plain"></div>
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
        assert_eq!(body_cs.resize, Resize::Both);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).resize,
            Resize::Vertical
        );
        // NO se hereda → default `None`.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).resize,
            Resize::None
        );
    }

    #[test]
    fn writing_mode_fase_7_249() {
        assert_eq!(parse_writing_mode("horizontal-tb"), Some(WritingMode::HorizontalTb));
        assert_eq!(parse_writing_mode("VERTICAL-RL"), Some(WritingMode::VerticalRl));
        assert_eq!(parse_writing_mode("vertical-lr"), Some(WritingMode::VerticalLr));
        assert_eq!(parse_writing_mode("sideways-rl"), Some(WritingMode::SidewaysRl));
        assert_eq!(parse_writing_mode("sideways-lr"), Some(WritingMode::SidewaysLr));
        // Fase 7.910 — aliases legacy SVG 1.1 ahora SÍ se mapean.
        assert_eq!(parse_writing_mode("lr-tb"), Some(WritingMode::HorizontalTb));
        assert_eq!(parse_writing_mode("nope"), None);

        let html = r##"<html><head><style>
            body { writing-mode: vertical-rl }
            p.over { writing-mode: horizontal-tb }
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
        assert_eq!(body_cs.writing_mode, WritingMode::VerticalRl);
        assert_eq!(
            eng.compute_with_parent(&ps[0], Some(&body_cs)).writing_mode,
            WritingMode::HorizontalTb
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&ps[1], Some(&body_cs)).writing_mode,
            WritingMode::VerticalRl
        );
    }

    #[test]
    fn direction_fase_7_250() {
        assert_eq!(parse_direction("ltr"), Some(Direction::Ltr));
        assert_eq!(parse_direction("RTL"), Some(Direction::Rtl));
        assert_eq!(parse_direction("auto"), None);

        let html = r##"<html><head><style>
            body { direction: rtl }
            div.lr { direction: ltr }
            div.plain {}
        </style></head><body>
          <div class="lr"></div><div class="plain"></div>
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
        assert_eq!(body_cs.direction, Direction::Rtl);
        assert_eq!(
            eng.compute_with_parent(&divs[0], Some(&body_cs)).direction,
            Direction::Ltr
        );
        // Heredado.
        assert_eq!(
            eng.compute_with_parent(&divs[1], Some(&body_cs)).direction,
            Direction::Rtl
        );
    }

    #[test]
    fn unicode_bidi_fase_7_251() {
        assert_eq!(parse_unicode_bidi("normal"), Some(UnicodeBidi::Normal));
        assert_eq!(parse_unicode_bidi("embed"), Some(UnicodeBidi::Embed));
        assert_eq!(parse_unicode_bidi("ISOLATE"), Some(UnicodeBidi::Isolate));
        assert_eq!(parse_unicode_bidi("bidi-override"), Some(UnicodeBidi::BidiOverride));
        assert_eq!(parse_unicode_bidi("isolate-override"), Some(UnicodeBidi::IsolateOverride));
        assert_eq!(parse_unicode_bidi("plaintext"), Some(UnicodeBidi::Plaintext));
        assert_eq!(parse_unicode_bidi("xxx"), None);

        // `unicode-bidi` NO se hereda (CSS Writing Modes 3).
        let html = r##"<html><head><style>
            body { unicode-bidi: embed }
            span.b { unicode-bidi: isolate }
            span.plain {}
        </style></head><body>
          <span class="b"></span><span class="plain"></span>
        </body></html>"##;
        let dom = DomTree::parse(html);
        let eng = StyleEngine::from_dom(&dom);
        let mut bodies = Vec::new();
        let mut spans = Vec::new();
        crate::dom::walk(&dom.document(), &mut |n| {
            match crate::dom::element_name(n).as_deref() {
                Some("body") => bodies.push(n.clone()),
                Some("span") => spans.push(n.clone()),
                _ => {}
            }
        });
        let body_cs = eng.compute(&bodies[0]);
        assert_eq!(body_cs.unicode_bidi, UnicodeBidi::Embed);
        assert_eq!(
            eng.compute_with_parent(&spans[0], Some(&body_cs)).unicode_bidi,
            UnicodeBidi::Isolate
        );
        // NO se hereda → default Normal.
        assert_eq!(
            eng.compute_with_parent(&spans[1], Some(&body_cs)).unicode_bidi,
            UnicodeBidi::Normal
        );
    }

