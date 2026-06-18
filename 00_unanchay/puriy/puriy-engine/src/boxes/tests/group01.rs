//! Tests del box-tree (grupo 01, extraído de `boxes/mod.rs`, regla #1).
use crate::Engine;

    #[test]
    fn box_tree_no_vacio() {
        let html = "<html><body><h1>Hola</h1><p>Mundo</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert!(doc.box_tree.descendants_count() >= 3);
    }

    #[test]
    fn node_ids_son_unicos_y_no_cero() {
        let html = "<html><body><div><h1>Hola</h1><p>Mundo</p></div></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids = Vec::new();
        doc.box_tree.walk(|b| ids.push(b.node_id));
        assert!(ids.iter().all(|&id| id != 0), "ningún nodo queda en 0");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "los node_id son únicos");
        // DFS pre-orden arranca en 1 sobre la raíz (body).
        assert_eq!(doc.box_tree.root.node_id, 1);
    }

    #[test]
    fn display_none_excluye_head() {
        let html = "<html><head><title>t</title></head><body><p>x</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El árbol parte de body — head no debe haber aportado nada.
        let mut tags = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.tag {
                tags.push(t.clone());
            }
        });
        assert!(!tags.contains(&"title".to_string()));
        assert!(!tags.contains(&"head".to_string()));
    }

    #[test]
    fn ol_li_recibe_marker_decimal() {
        let html =
            "<html><body><ol><li>uno</li><li>dos</li><li>tres</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "2. ".into(), "3. ".into()]);
    }

    #[test]
    fn ul_li_recibe_marker_bullet() {
        let html = "<html><body><ul><li>a</li><li>b</li></ul></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('•') {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers.len(), 2);
    }

    #[test]
    fn li_marker_string_literal_fase_7_1216() {
        // `list-style-type: "<string>"` (CSS Lists 3): el marcador es el string
        // literal verbatim (antes se aproximaba a `•`).
        let html = "<html><head><style>\
            li { list-style-type: \"\u{2192} \"; }\
            </style></head><body><ul><li>a</li><li>b</li></ul></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('→') {
                    markers.push(t.clone());
                }
            }
        });
        // Dos <li> → dos marcadores "→ " (no el bullet por defecto).
        assert_eq!(markers, vec!["→ ".to_string(), "→ ".to_string()]);
    }

    #[test]
    fn counter_style_cyclic_marker_fase_7_1218() {
        // @counter-style cyclic con un símbolo: todos los <li> usan ese símbolo.
        let html = "<html><head><style>\
            @counter-style estrella { system: cyclic; symbols: \"\u{2605}\"; suffix: \" \"; }\
            ul { list-style-type: estrella; }\
            </style></head><body><ul><li>a</li><li>b</li></ul></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('★') {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["★ ".to_string(), "★ ".to_string()]);
    }

    #[test]
    fn counter_style_symbolic_y_fallback_fase_7_1218() {
        // system: symbolic con 2 símbolos repite: item3 → "**" (con prefix/suffix).
        let html = "<html><head><style>\
            @counter-style sym { system: symbolic; symbols: \"*\" \"\u{2020}\"; prefix: \"[\"; suffix: \"] \"; }\
            ol { list-style-type: sym; }\
            </style></head><body><ol><li>1</li><li>2</li><li>3</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('[') {
                    markers.push(t.clone());
                }
            }
        });
        // n=1 → "*", n=2 → "†", n=3 → "**" (símbolo[0] repetido 2 veces).
        assert_eq!(
            markers,
            vec!["[*] ".to_string(), "[†] ".to_string(), "[**] ".to_string()]
        );
    }

    #[test]
    fn counter_style_no_registrado_cae_a_decimal_fase_7_1218() {
        // list-style-type con un nombre custom sin @counter-style → decimal.
        let html = "<html><head><style>ol { list-style-type: inexistente; }</style></head>\
            <body><ol><li>x</li><li>y</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "2. ".to_string()]);
    }

    #[test]
    fn clip_path_inset_llega_al_box_node_fase_7_1219() {
        // `clip-path: inset(...)` se resuelve a insets px en el BoxNode (que el
        // chrome usa para recortar). Formas no rectangulares no se modelan.
        let html = "<html><body>\
            <div id=\"c\" style=\"clip-path: inset(10px 20px 30px 40px)\">x</div>\
            <div id=\"r\" style=\"clip-path: inset(5px round 8px)\">y</div>\
            <div id=\"n\">z</div>\
            <div id=\"circ\" style=\"clip-path: circle(50%)\">w</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let by_id = |id: &str| {
            let mut found = None;
            doc.box_tree.walk(|b| {
                if b.element_id.as_deref() == Some(id) {
                    found = Some(b.clip_inset);
                }
            });
            found.expect("box existe")
        };
        assert_eq!(by_id("c"), Some([10.0, 20.0, 30.0, 40.0]));
        // inset de un valor → los 4 lados iguales (el `round` no afecta el rect).
        assert_eq!(by_id("r"), Some([5.0, 5.0, 5.0, 5.0]));
        // sin clip-path → None.
        assert_eq!(by_id("n"), None);
        // forma no rectangular (circle) → None (no se modela como inset).
        assert_eq!(by_id("circ"), None);
    }

    #[test]
    fn clip_path_circle_ellipse_llega_al_box_node_fase_7_1220() {
        // `clip-path: circle()/ellipse()` se resuelve a un spec elíptico de 12
        // floats en el BoxNode: centro [cx_px, cx_pct, cy_px, cy_pct] + dos
        // radios [px, pct_w, pct_h, pct_diag]. El % se difiere al compositor.
        let html = "<html><body>\
            <div id=\"c1\" style=\"clip-path: circle(30px at 50% 50%)\">a</div>\
            <div id=\"c2\" style=\"clip-path: circle(40px at 10px 20px)\">b</div>\
            <div id=\"c3\" style=\"clip-path: circle(50%)\">f</div>\
            <div id=\"c4\" style=\"clip-path: circle()\">h</div>\
            <div id=\"e1\" style=\"clip-path: ellipse(20px 10px)\">c</div>\
            <div id=\"e2\" style=\"clip-path: ellipse(25% 40%)\">g</div>\
            <div id=\"e3\" style=\"clip-path: ellipse(farthest-side closest-side)\">i</div>\
            <div id=\"ins\" style=\"clip-path: inset(5px)\">d</div>\
            <div id=\"n\">e</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let by_id = |id: &str| {
            let mut found = None;
            doc.box_tree.walk(|b| {
                if b.element_id.as_deref() == Some(id) {
                    found = Some(b.clip_ellipse);
                }
            });
            found.expect("box existe")
        };
        // Layout del spec: [cx_px, cx_pct, cy_px, cy_pct, rx×5, ry×5] donde cada
        // radio es [px, pct_w, pct_h, pct_diag, side].
        // circle px, centro 50%/50% → radios px iguales, side 0.
        assert_eq!(
            by_id("c1"),
            Some([0.0, 50.0, 0.0, 50.0, 30.0, 0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 0.0, 0.0, 0.0])
        );
        // circle px, centro en px.
        assert_eq!(
            by_id("c2"),
            Some([10.0, 0.0, 20.0, 0.0, 40.0, 0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 0.0, 0.0, 0.0])
        );
        // circle(50%) → radio % sobre base DIAGONAL (ranura pct_diag), side 0.
        assert_eq!(
            by_id("c3"),
            Some([0.0, 50.0, 0.0, 50.0, 0.0, 0.0, 0.0, 50.0, 0.0, 0.0, 0.0, 0.0, 50.0, 0.0])
        );
        // circle() vacío → closest-side base circle (side=1) en ambos radios.
        assert_eq!(
            by_id("c4"),
            Some([0.0, 50.0, 0.0, 50.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0])
        );
        // ellipse px sin `at` → centro default 50%/50%, radios px distintos.
        assert_eq!(
            by_id("e1"),
            Some([0.0, 50.0, 0.0, 50.0, 20.0, 0.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0.0])
        );
        // ellipse % → rx% sobre ancho (pct_w), ry% sobre alto (pct_h).
        assert_eq!(
            by_id("e2"),
            Some([0.0, 50.0, 0.0, 50.0, 0.0, 25.0, 0.0, 0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 0.0])
        );
        // ellipse keywords → side base eje: rx farthest=4, ry closest=3.
        assert_eq!(
            by_id("e3"),
            Some([0.0, 50.0, 0.0, 50.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 3.0])
        );
        // inset() → es rectangular, no llena clip_ellipse.
        assert_eq!(by_id("ins"), None);
        // sin clip-path → None.
        assert_eq!(by_id("n"), None);
    }

    #[test]
    fn clip_path_polygon_llega_al_box_node_fase_7_1223() {
        // `clip-path: polygon(...)` → (evenodd, puntos) en el BoxNode; cada
        // punto [x_px, x_pct, y_px, y_pct], el % se difiere al compositor.
        let html = "<html><body>\
            <div id=\"tri\" style=\"clip-path: polygon(0 0, 100% 0, 50% 100%)\">a</div>\
            <div id=\"eo\" style=\"clip-path: polygon(evenodd, 0 0, 10px 20px)\">b</div>\
            <div id=\"circ\" style=\"clip-path: circle(50%)\">c</div>\
            <div id=\"n\">d</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let by_id = |id: &str| {
            let mut found = None;
            doc.box_tree.walk(|b| {
                if b.element_id.as_deref() == Some(id) {
                    found = Some(b.clip_polygon.clone());
                }
            });
            found.expect("box existe")
        };
        // triángulo: 0,0 (px) / 100%,0 / 50%,100%. nonzero.
        assert_eq!(
            by_id("tri"),
            Some((
                false,
                vec![
                    [0.0, 0.0, 0.0, 0.0],
                    [0.0, 100.0, 0.0, 0.0],
                    [0.0, 50.0, 0.0, 100.0],
                ]
            ))
        );
        // evenodd con coords px.
        assert_eq!(
            by_id("eo"),
            Some((true, vec![[0.0, 0.0, 0.0, 0.0], [10.0, 0.0, 20.0, 0.0]]))
        );
        // circle() no llena clip_polygon (es elíptico).
        assert_eq!(by_id("circ"), None);
        // sin clip-path → None.
        assert_eq!(by_id("n"), None);
    }

    #[test]
    fn clip_path_path_svg_llega_al_box_node_fase_7_1224() {
        // `clip-path: path(...)` → (evenodd, d) con el string SVG crudo.
        let html = "<html><body>\
            <div id=\"p\" style=\"clip-path: path('M0 0 L10 0 L10 10 Z')\">a</div>\
            <div id=\"pe\" style=\"clip-path: path(evenodd, 'M0 0 L5 5')\">b</div>\
            <div id=\"poly\" style=\"clip-path: polygon(0 0, 10px 10px)\">c</div>\
            <div id=\"n\">d</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let by_id = |id: &str| {
            let mut found = None;
            doc.box_tree.walk(|b| {
                if b.element_id.as_deref() == Some(id) {
                    found = Some(b.clip_path_svg.clone());
                }
            });
            found.expect("box existe")
        };
        assert_eq!(by_id("p"), Some((false, "M0 0 L10 0 L10 10 Z".to_string())));
        assert_eq!(by_id("pe"), Some((true, "M0 0 L5 5".to_string())));
        // polygon() no llena clip_path_svg.
        assert_eq!(by_id("poly"), None);
        // sin clip-path → None.
        assert_eq!(by_id("n"), None);
    }

    #[test]
    fn clip_path_geometry_box_llega_al_box_node_fase_7_1225() {
        // La caja de referencia → clip_ref_inset = insets del border-box:
        // padding-box = border; content-box = border+padding; border-box = None.
        let html = "<html><body>\
            <div id=\"pad\" style=\"border:5px solid red; padding:10px; clip-path: circle(50%) padding-box\">a</div>\
            <div id=\"con\" style=\"border:5px solid red; padding:10px; clip-path: circle(50%) content-box\">b</div>\
            <div id=\"bor\" style=\"border:5px solid red; padding:10px; clip-path: circle(50%) border-box\">c</div>\
            <div id=\"only\" style=\"border:3px solid red; padding:7px; clip-path: content-box\">d</div>\
            <div id=\"def\" style=\"border:5px solid red; clip-path: circle(50%)\">e</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let by_id = |id: &str| {
            let mut found = None;
            doc.box_tree.walk(|b| {
                if b.element_id.as_deref() == Some(id) {
                    found = Some(b.clip_ref_inset);
                }
            });
            found.expect("box existe")
        };
        // padding-box → insets = border (5 por lado).
        assert_eq!(by_id("pad"), Some([5.0, 5.0, 5.0, 5.0]));
        // content-box → insets = border + padding (15 por lado).
        assert_eq!(by_id("con"), Some([15.0, 15.0, 15.0, 15.0]));
        // border-box (explícito) → None (referencia = rect completo).
        assert_eq!(by_id("bor"), None);
        // caja sola sin forma → recorta a content-box (3+7=10 por lado).
        assert_eq!(by_id("only"), Some([10.0, 10.0, 10.0, 10.0]));
        // sin geometry-box → None.
        assert_eq!(by_id("def"), None);
    }

    #[test]
    fn filter_y_backdrop_filter_llegan_al_box_node_fase_7_1232() {
        // `filter`/`backdrop-filter` ya se parsean al ComputedStyle (Fases
        // 7.264/7.265); este test verifica que viajan hasta el BoxNode, que es
        // lo que el chrome (puriy-llimphi) lee para pintar la post-pasada GPU.
        use crate::style::FilterFn;
        let html = "<html><body>\
            <div id=\"f\" style=\"filter: blur(3px)\">x</div>\
            <div id=\"bf\" style=\"-webkit-backdrop-filter: blur(8px)\">y</div>\
            <div id=\"n\">z</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let by_id = |id: &str| {
            let mut found = None;
            doc.box_tree.walk(|b| {
                if b.element_id.as_deref() == Some(id) {
                    found = Some((b.filter.clone(), b.backdrop_filter.clone()));
                }
            });
            found.expect("box existe")
        };
        let (f, bf) = by_id("f");
        assert!(matches!(f.as_slice(), [FilterFn::Blur(v)] if (v - 3.0).abs() < 1e-3));
        assert!(bf.is_empty());
        let (f2, bf2) = by_id("bf");
        assert!(f2.is_empty());
        assert!(matches!(bf2.as_slice(), [FilterFn::Blur(v)] if (v - 8.0).abs() < 1e-3));
        // sin filter → ambos vacíos.
        let (f3, bf3) = by_id("n");
        assert!(f3.is_empty() && bf3.is_empty());
    }

    #[test]
    fn drop_shadow_llega_al_box_node_fase_7_1234() {
        // `filter: drop-shadow(...)` viaja al BoxNode.filter como FilterFn con su
        // BoxShadow (que puriy-llimphi mapea a una sombra Gaussiana). Fase 7.1234.
        use crate::style::FilterFn;
        let html = "<html><body>\
            <div id=\"d\" style=\"filter: drop-shadow(2px 4px 6px black)\">x</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                found = Some(b.filter.clone());
            }
        });
        let f = found.expect("box existe");
        assert!(matches!(
            f.as_slice(),
            [FilterFn::DropShadow(s)]
                if (s.offset_x - 2.0).abs() < 1e-3
                    && (s.offset_y - 4.0).abs() < 1e-3
                    && (s.blur_px - 6.0).abs() < 1e-3
        ));
    }

    #[test]
    fn unidades_viewport_resuelven_contra_el_viewport_real() {
        use crate::style::{LengthVal, Viewport};
        // `vw/vh/vmin/vmax` deben resolver contra el ancho/alto REAL de la
        // ventana, no contra DEFAULT_VIEWPORT (1280×800). Con viewport 800×600
        // y `style="…"` inline (que parsea `boxes::build`, no la hoja):
        //   50vw   = 50% de 800            = 400
        //   50vh   = 50% de 600            = 300
        //   50vmin = 50% de min(800,600)   = 300
        //   50vmax = 50% de max(800,600)   = 400
        let html = r#"<html><body>
            <div id="vw" style="width:50vw"></div>
            <div id="vh" style="width:50vh"></div>
            <div id="vmin" style="width:50vmin"></div>
            <div id="vmax" style="width:50vmax"></div>
        </body></html>"#;
        let vp = Viewport { width: 800.0, height: 600.0, dpr: 1.0 };
        let doc = Engine::new().with_viewport(vp).load_html("about:test", html);
        let mut widths = std::collections::HashMap::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = b.element_id.as_deref() {
                widths.insert(id.to_string(), b.width);
            }
        });
        assert_eq!(widths.get("vw"), Some(&LengthVal::Px(400.0)));
        assert_eq!(widths.get("vh"), Some(&LengthVal::Px(300.0)));
        assert_eq!(widths.get("vmin"), Some(&LengthVal::Px(300.0)));
        assert_eq!(widths.get("vmax"), Some(&LengthVal::Px(400.0)));
    }

    #[test]
    fn unidades_viewport_default_sin_viewport_real() {
        use crate::style::LengthVal;
        // Sin `with_viewport`, el Engine usa DEFAULT_VIEWPORT (1280×800):
        // 50vw = 640. Garantiza que el scope no contamina el path por defecto
        // (se restaura al dropear al volver de `load_html`).
        let html = r#"<html><body><div id="x" style="width:50vw"></div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let mut w = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                w = Some(b.width);
            }
        });
        assert_eq!(w, Some(LengthVal::Px(640.0)));
    }

    fn box_by_id(bt: &super::super::BoxTree, id: &str) -> Option<super::super::BoxNode> {
        let mut found = None;
        bt.walk(|b| {
            if found.is_none() && b.element_id.as_deref() == Some(id) {
                found = Some(b.clone());
            }
        });
        found
    }

    #[test]
    fn restyle_aplica_regla_de_clase_agregada() {
        // `.on` (no presente al cargar) + un selector descendiente `.on .child`.
        // Tras agregar la clase y recascadear, el fondo del box y el color del
        // hijo deben aparecer.
        let html = r#"<html><head><style>
            .on { background: red; }
            .on .child { color: blue; }
        </style></head><body>
            <div id="box"><p id="p" class="child">x</p></div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "box").unwrap().background, None);
        assert!(doc.box_tree.set_element_class_list("box", vec!["on".to_string()]));
        doc.box_tree.restyle();
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::super::Color::rgb(255, 0, 0))
        );
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn restyle_quitar_clase_revierte_estilo() {
        let html = r#"<html><head><style>
            #box { background: green; }
            #box.on { background: red; }
        </style></head><body><div id="box" class="on">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::super::Color::rgb(255, 0, 0))
        );
        doc.box_tree.set_element_class_list("box", vec![]);
        doc.box_tree.restyle();
        // Sin `.on`, gana la regla base `#box { background: green }`.
        assert_eq!(
            box_by_id(&doc.box_tree, "box").unwrap().background,
            Some(super::super::Color::rgb(0, 128, 0))
        );
    }

    #[test]
    fn restyle_combinador_hermano_afecta_posterior() {
        // Cambiar la clase de #t debe afectar a su HERMANO #pnl vía `+`.
        // Sólo posible recascadeando el árbol entero, no sólo el subárbol.
        let html = r#"<html><head><style>
            .open + .panel { background: red; }
        </style></head><body>
            <div id="t" class="tab"></div>
            <div id="pnl" class="panel">x</div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "pnl").unwrap().background, None);
        doc.box_tree
            .set_element_class_list("t", vec!["tab".into(), "open".into()]);
        doc.box_tree.restyle();
        assert_eq!(
            box_by_id(&doc.box_tree, "pnl").unwrap().background,
            Some(super::super::Color::rgb(255, 0, 0))
        );
    }

    #[test]
    fn restyle_toggle_display_none_oculta_y_muestra() {
        let html = r#"<html><head><style>
            .hidden { display: none; }
        </style></head><body><div id="box">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_ne!(box_by_id(&doc.box_tree, "box").unwrap().display, super::super::Display::None);
        doc.box_tree.set_element_class_list("box", vec!["hidden".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "box").unwrap().display, super::super::Display::None);
        doc.box_tree.set_element_class_list("box", vec![]);
        doc.box_tree.restyle();
        assert_ne!(box_by_id(&doc.box_tree, "box").unwrap().display, super::super::Display::None);
    }

    #[test]
    fn restyle_sin_cambios_es_idempotente() {
        let html = r#"<html><head><style>
            #box { background: red; color: green; padding: 5px; font-size: 20px; }
        </style></head><body><div id="box"><span id="s">hi</span></div></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        let before_box = box_by_id(&doc.box_tree, "box").unwrap();
        let before_s = box_by_id(&doc.box_tree, "s").unwrap();
        doc.box_tree.restyle();
        let after_box = box_by_id(&doc.box_tree, "box").unwrap();
        let after_s = box_by_id(&doc.box_tree, "s").unwrap();
        assert_eq!(before_box.background, after_box.background);
        assert_eq!(before_box.color, after_box.color);
        assert_eq!(before_box.display, after_box.display);
        assert_eq!(before_box.padding.top, after_box.padding.top);
        assert_eq!(before_box.font_size, after_box.font_size);
        // El span hereda color/font del padre, igual antes y después.
        assert_eq!(before_s.color, after_s.color);
        assert_eq!(before_s.font_size, after_s.font_size);
    }

    #[test]
    fn restyle_preserva_estilo_inline_seteado_por_js() {
        // `el.style.color='red'` (via set_element_style) debe sobrevivir a un
        // restyle posterior por classList: la cascada re-parsea el atributo
        // `style` y el inline gana sobre la regla `.on { color: blue }`.
        let html = r#"<html><head><style>.on { color: blue; }</style></head>
            <body><p id="p">x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        doc.box_tree.set_element_style("p", "color", "red");
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::super::Color::rgb(255, 0, 0));
        doc.box_tree.set_element_class_list("p", vec!["on".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "p").unwrap().color, super::super::Color::rgb(255, 0, 0));
    }

    #[test]
    fn build_retiene_display_none_de_autor_y_descarta_ua() {
        // Fase 7.185 — un elemento ocultado por CSS de autor se RETIENE en el
        // box tree (oculto, con su subárbol) para poder mostrarlo luego; el
        // ruido UA (`<script>`) se sigue descartando.
        let html = r#"<html><head><style>
            .modal { display: none; }
        </style></head><body>
            <div id="m" class="modal"><p id="inner">contenido</p></div>
            <script>var x = 1;</script>
            <span id="s">visible</span>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let m = box_by_id(&doc.box_tree, "m").expect("modal de autor retenido");
        assert_eq!(m.display, super::super::Display::None);
        assert!(box_by_id(&doc.box_tree, "inner").is_some(), "subárbol retenido");
        let mut script_text = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("var x") {
                    script_text = true;
                }
            }
        });
        assert!(!script_text, "el texto del <script> no debe filtrarse al box tree");
        assert!(box_by_id(&doc.box_tree, "s").is_some());
    }

    #[test]
    fn restyle_muestra_modal_oculto_al_cargar() {
        // El patrón clásico: modal arranca `display:none`, JS agrega `.open`
        // para mostrarlo. Posible porque retenemos el box oculto al cargar.
        let html = r#"<html><head><style>
            .modal { display: none; }
            .modal.open { display: block; }
        </style></head><body>
            <div id="m" class="modal">hola</div>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "m").unwrap().display, super::super::Display::None);
        doc.box_tree
            .set_element_class_list("m", vec!["modal".into(), "open".into()]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "m").unwrap().display, super::super::Display::Block);
    }

    #[test]
    fn pseudo_estado_checked_disabled_enabled() {
        let html = r#"<html><head><style>
            input:checked { background: red; }
            input:disabled { color: green; }
            input:enabled { color: blue; }
            input:required { background: yellow; }
        </style></head><body>
            <input id="a" type="checkbox" checked>
            <input id="b" type="checkbox">
            <input id="c" type="text" disabled>
            <input id="d" type="text" required>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::super::Color::rgb(255, 0, 0);
        let blue = super::super::Color::rgb(0, 0, 255);
        // a: checked → fondo rojo; enabled → color azul.
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().color, blue);
        // b: no checked → no rojo (conserva su fondo UA); enabled → azul.
        assert_ne!(box_by_id(&doc.box_tree, "b").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "b").unwrap().color, blue);
        // c: disabled → verde; NO enabled (no azul).
        assert_eq!(box_by_id(&doc.box_tree, "c").unwrap().color, super::super::Color::rgb(0, 128, 0));
        // d: required → fondo amarillo.
        assert_eq!(box_by_id(&doc.box_tree, "d").unwrap().background, Some(super::super::Color::rgb(255, 255, 0)));
    }

    #[test]
    fn pseudo_nth_of_type_y_only_of_type_y_nth_last() {
        let html = r#"<html><head><style>
            p:nth-of-type(2) { color: red; }
            li:nth-last-child(1) { color: green; }
            span:only-of-type { color: blue; }
        </style></head><body>
            <div><span id="sp">x</span><p id="p1">1</p><p id="p2">2</p></div>
            <ul><li id="l1">a</li><li id="l2">b</li></ul>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "p2").unwrap().color, super::super::Color::rgb(255, 0, 0));
        assert_ne!(box_by_id(&doc.box_tree, "p1").unwrap().color, super::super::Color::rgb(255, 0, 0));
        assert_eq!(box_by_id(&doc.box_tree, "l2").unwrap().color, super::super::Color::rgb(0, 128, 0));
        assert_ne!(box_by_id(&doc.box_tree, "l1").unwrap().color, super::super::Color::rgb(0, 128, 0));
        assert_eq!(box_by_id(&doc.box_tree, "sp").unwrap().color, super::super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn sync_checked_y_restyle_actualiza_pseudo_checked() {
        // Fase 7.187 — togglear un checkbox actualiza el atributo `checked` y
        // recascadea: `:checked` y `:checked + label` aplican en vivo.
        let html = r#"<html><head><style>
            input:checked { background: red; }
            input:checked + label { color: blue; }
        </style></head><body>
            <input id="cb" type="checkbox"><label id="lb">L</label>
        </body></html>"#;
        let mut doc = Engine::new().load_html("about:test", html);
        let red = super::super::Color::rgb(255, 0, 0);
        let blue = super::super::Color::rgb(0, 0, 255);
        assert_ne!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        // Marcar (el checkbox es el control índice 0).
        doc.box_tree.sync_checked_from(&[true]);
        doc.box_tree.restyle();
        assert_eq!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        assert_eq!(box_by_id(&doc.box_tree, "lb").unwrap().color, blue);
        // Desmarcar revierte ambos.
        doc.box_tree.sync_checked_from(&[false]);
        doc.box_tree.restyle();
        assert_ne!(box_by_id(&doc.box_tree, "cb").unwrap().background, Some(red));
        assert_ne!(box_by_id(&doc.box_tree, "lb").unwrap().color, blue);
    }

    #[test]
    fn pseudo_is_y_where_matchean_lista() {
        let html = r#"<html><head><style>
            :is(h1, h2) { color: red; }
            .box :where(.a, .b) { background: green; }
            #x:is(.on, .off) { color: blue; }
        </style></head><body>
            <h2 id="h">t</h2>
            <div class="box"><span id="s" class="b">x</span></div>
            <p id="x" class="on">p</p>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "h").unwrap().color, super::super::Color::rgb(255, 0, 0));
        assert_eq!(
            box_by_id(&doc.box_tree, "s").unwrap().background,
            Some(super::super::Color::rgb(0, 128, 0))
        );
        assert_eq!(box_by_id(&doc.box_tree, "x").unwrap().color, super::super::Color::rgb(0, 0, 255));
    }

    #[test]
    fn pseudo_where_no_aporta_especificidad() {
        // `:where(#hero)` tiene especificidad 0 → lo vence el selector de tag
        // `p` (que llega después y tiene especificidad 1). Si `:where` aportara
        // los 100 del `#id`, ganaría el rojo.
        let html = r#"<html><head><style>
            :where(#hero) { color: red; }
            p { color: green; }
        </style></head><body><p id="hero">x</p></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "hero").unwrap().color, super::super::Color::rgb(0, 128, 0));
    }

    #[test]
    fn shorthand_inset_y_flex_flow() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { position: absolute; inset: 10px 20px; }
            #b { display: flex; flex-flow: column wrap; }
        </style></head><body>
            <div id="a">x</div><div id="b">y</div>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // `inset: 10px 20px` → top/bottom=10, right/left=20.
        assert_eq!(a.inset_top, LengthVal::Px(10.0));
        assert_eq!(a.inset_right, LengthVal::Px(20.0));
        assert_eq!(a.inset_bottom, LengthVal::Px(10.0));
        assert_eq!(a.inset_left, LengthVal::Px(20.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.flex_direction, super::super::FlexDirection::Column);
        assert_eq!(b.flex_wrap, super::super::FlexWrap::Wrap);
    }

    #[test]
    fn pseudo_not_con_lista() {
        // CSS4: `:not(.a, .b)` no matchea si el elemento tiene .a O .b.
        let html = r#"<html><head><style>
            li:not(.skip, .hidden) { color: red; }
        </style></head><body><ul>
            <li id="n1">uno</li>
            <li id="n2" class="skip">dos</li>
            <li id="n3" class="hidden">tres</li>
        </ul></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::super::Color::rgb(255, 0, 0);
        assert_eq!(box_by_id(&doc.box_tree, "n1").unwrap().color, red); // sin clases → rojo
        assert_ne!(box_by_id(&doc.box_tree, "n2").unwrap().color, red); // .skip → excluido
        assert_ne!(box_by_id(&doc.box_tree, "n3").unwrap().color, red); // .hidden → excluido
    }

    #[test]
    fn propiedades_logicas_de_caja() {
        let html = r#"<html><head><style>
            #a { margin-inline: 10px 20px; padding-block: 5px; }
            #b { margin-inline-start: 8px; padding-block-end: 12px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // margin-inline: 10 20 → left=10 (start), right=20 (end), LTR.
        assert_eq!(a.margin.left, 10.0);
        assert_eq!(a.margin.right, 20.0);
        // padding-block: 5 → top=bottom=5.
        assert_eq!(a.padding.top, 5.0);
        assert_eq!(a.padding.bottom, 5.0);
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.margin.left, 8.0); // inline-start = left (LTR)
        assert_eq!(b.padding.bottom, 12.0); // block-end = bottom
    }

    #[test]
    fn inset_logico_inline_y_block() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { position: absolute; inset-inline: 10px 20px; inset-block: 5px; }
            #b { position: absolute; inset-inline-start: 8px; inset-block-end: 12px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // inset-inline: 10 20 → left=10 (start), right=20 (end), LTR.
        assert_eq!(a.inset_left, LengthVal::Px(10.0));
        assert_eq!(a.inset_right, LengthVal::Px(20.0));
        // inset-block: 5 → top=bottom=5.
        assert_eq!(a.inset_top, LengthVal::Px(5.0));
        assert_eq!(a.inset_bottom, LengthVal::Px(5.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.inset_left, LengthVal::Px(8.0)); // inline-start = left (LTR)
        assert_eq!(b.inset_bottom, LengthVal::Px(12.0)); // block-end = bottom
    }

    #[test]
    fn height_explicito_se_propaga_al_box() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { height: 200px; }
            #b { height: 50%; }
            #c { width: 100px; }
        </style></head><body>
            <div id="a">x</div><div id="b">y</div><div id="c">z</div>
        </body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        assert_eq!(box_by_id(&doc.box_tree, "a").unwrap().height, LengthVal::Px(200.0));
        assert_eq!(box_by_id(&doc.box_tree, "b").unwrap().height, LengthVal::Pct(50.0));
        // Sin `height` declarado → Auto (lo dimensiona el contenido).
        assert_eq!(box_by_id(&doc.box_tree, "c").unwrap().height, LengthVal::Auto);
    }

    #[test]
    fn tamanos_logicos_inline_block() {
        use crate::style::LengthVal;
        let html = r#"<html><head><style>
            #a { inline-size: 120px; block-size: 80px; }
            #b { min-inline-size: 10px; max-block-size: 200px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // inline-size → width, block-size → height (LTR/horizontal).
        assert_eq!(a.width, LengthVal::Px(120.0));
        assert_eq!(a.height, LengthVal::Px(80.0));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        assert_eq!(b.min_width, LengthVal::Px(10.0));
        assert_eq!(b.max_height, LengthVal::Px(200.0));
    }

    #[test]
    fn border_logico_inline_y_block() {
        let html = r#"<html><head><style>
            #a { border-inline: 3px solid red; }
            #b { border-block-start: 5px solid blue; border-inline-end-width: 7px; }
        </style></head><body><div id="a">x</div><div id="b">y</div></body></html>"#;
        let doc = Engine::new().load_html("about:test", html);
        let red = super::super::Color::rgb(255, 0, 0);
        let blue = super::super::Color::rgb(0, 0, 255);
        let a = box_by_id(&doc.box_tree, "a").unwrap();
        // border-inline: 3px solid red → left y right (LTR), no top/bottom.
        assert_eq!(a.border_widths.left, 3.0);
        assert_eq!(a.border_widths.right, 3.0);
        assert_eq!(a.border_widths.top, 0.0);
        assert_eq!(a.border_colors.left, Some(red));
        assert_eq!(a.border_colors.right, Some(red));
        let b = box_by_id(&doc.box_tree, "b").unwrap();
        // border-block-start = top.
        assert_eq!(b.border_widths.top, 5.0);
        assert_eq!(b.border_colors.top, Some(blue));
        // border-inline-end-width = right-width.
        assert_eq!(b.border_widths.right, 7.0);
    }

    #[test]
    fn list_style_none_suprime_marker() {
        let html = r#"<html><head><style>
            ul { list-style-type: none }
        </style></head><body><ul><li>uno</li><li>dos</li></ul></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut has_bullet = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains('•') {
                    has_bullet = true;
                }
            }
        });
        assert!(!has_bullet, "no debería haber marker con list-style-type:none");
    }

    #[test]
    fn ol_start_corre_el_contador() {
        let html =
            "<html><body><ol start=\"5\"><li>x</li><li>y</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["5. ".to_string(), "6. ".into()]);
    }

    #[test]
    fn li_value_resetea_el_contador() {
        let html = "<html><body><ol><li>x</li><li value=\"10\">y</li><li>z</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "10. ".into(), "11. ".into()]);
    }

    #[test]
    fn lower_roman_y_lower_alpha_aplican() {
        let html = r#"<html><head><style>
            .roman { list-style-type: lower-roman }
            .alpha { list-style-type: upper-alpha }
        </style></head><body>
          <ol class="roman"><li>a</li><li>b</li><li>c</li></ol>
          <ol class="alpha"><li>a</li><li>b</li></ol>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        // ol.roman → i. ii. iii.   ol.alpha → A. B.
        assert_eq!(
            markers,
            vec![
                "i. ".to_string(),
                "ii. ".into(),
                "iii. ".into(),
                "A. ".into(),
                "B. ".into(),
            ]
        );
    }

    #[test]
    fn to_alpha_y_to_roman_son_correctos() {
        use super::super::{to_alpha, to_roman};
        assert_eq!(to_alpha(1, false), "a");
        assert_eq!(to_alpha(26, false), "z");
        assert_eq!(to_alpha(27, false), "aa");
        assert_eq!(to_alpha(28, false), "ab");
        assert_eq!(to_alpha(52, true), "AZ");
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(9, true), "IX");
        assert_eq!(to_roman(1994, false), "mcmxciv");
        assert_eq!(to_roman(3999, true), "MMMCMXCIX");
        // Fuera de rango → decimal fallback.
        assert_eq!(to_roman(4000, false), "4000");
        assert_eq!(to_roman(0, true), "0");
    }

    #[test]
    fn estilo_inline_aplica_color() {
        let html = r#"<html><body><p style="color: #ff0000">x</p></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_red = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::super::Color::rgb(255, 0, 0) {
                found_red = true;
            }
        });
        assert!(found_red, "no se encontró <p> con color rojo");
    }

    #[test]
    fn letter_spacing_hereda_al_leaf_fase_7_1252() {
        // letter-spacing/word-spacing heredan: la hoja de texto del build
        // estático toma el valor del contenedor (px resueltos).
        let html = "<html><head><style>\
            .ancho { letter-spacing: 3px; word-spacing: 5px; }\
            </style></head><body>\
            <div class=\"ancho\">texto espaciado</div></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ls = None;
        let mut ws = None;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("espaciado") {
                    ls = Some(b.letter_spacing);
                    ws = Some(b.word_spacing);
                }
            }
        });
        assert_eq!(ls, Some(3.0), "letter-spacing hereda al leaf");
        assert_eq!(ws, Some(5.0), "word-spacing hereda al leaf");
    }

    #[test]
    fn text_overflow_ellipsis_se_propaga_al_leaf_fase_7_1251() {
        use crate::style::TextOverflow;
        // `text-overflow: ellipsis` vive en el contenedor, pero el glifo está en
        // la hoja de texto. Con `overflow: hidden` la intención se propaga al
        // leaf; sin recorte (`overflow: visible`) NO aplica y queda en `Clip`.
        let html = "<html><head><style>\
            .corta { overflow: hidden; text-overflow: ellipsis; }\
            .libre { text-overflow: ellipsis; }\
            </style></head><body>\
            <div class=\"corta\">recortame porque soy larguisimo</div>\
            <div class=\"libre\">a mi no me recortes</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut recortada: Option<TextOverflow> = None;
        let mut libre: Option<TextOverflow> = None;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("recortame") {
                    recortada = Some(b.text_overflow);
                } else if t.contains("no me recortes") {
                    libre = Some(b.text_overflow);
                }
            }
        });
        assert_eq!(recortada, Some(TextOverflow::Ellipsis), "con overflow:hidden propaga");
        assert_eq!(libre, Some(TextOverflow::Clip), "sin recorte no aplica");
    }

    #[test]
    fn white_space_hereda_al_leaf_fase_7_1253() {
        use crate::WhiteSpace;
        // `white-space` HEREDA (CSS): la hoja de texto del build estático toma
        // el valor del contenedor. El wire lee `NoWrap`/`Pre` para shapear en
        // una sola línea; `Normal` (default) envuelve.
        let html = "<html><head><style>\
            .nw { white-space: nowrap; }\
            .pre { white-space: pre; }\
            </style></head><body>\
            <div class=\"nw\">no me envuelvas aunque sea larguisimo</div>\
            <div class=\"pre\">preformateado</div>\
            <div>texto normal que envuelve</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut nw: Option<WhiteSpace> = None;
        let mut pre: Option<WhiteSpace> = None;
        let mut normal: Option<WhiteSpace> = None;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("no me envuelvas") {
                    nw = Some(b.white_space);
                } else if t.contains("preformateado") {
                    pre = Some(b.white_space);
                } else if t.contains("texto normal") {
                    normal = Some(b.white_space);
                }
            }
        });
        assert_eq!(nw, Some(WhiteSpace::NoWrap), "nowrap hereda al leaf");
        assert_eq!(pre, Some(WhiteSpace::Pre), "pre hereda al leaf");
        assert_eq!(normal, Some(WhiteSpace::Normal), "default es Normal (envuelve)");
    }

    #[test]
    fn overflow_wrap_y_word_break_heredan_al_leaf_fase_7_1254() {
        use crate::style::{OverflowWrap, WordBreak};
        // `overflow-wrap` y `word-break` HEREDAN (CSS): la hoja de texto toma el
        // valor del contenedor. El wire lee `BreakWord`/`Anywhere` (overflow-wrap)
        // o `BreakAll` (word-break) para habilitar partir dentro de la palabra;
        // `Normal` (default) deja desbordar.
        let html = "<html><head><style>\
            .bw { overflow-wrap: break-word; }\
            .any { overflow-wrap: anywhere; }\
            .ba { word-break: break-all; }\
            </style></head><body>\
            <div class=\"bw\">palabra_break_word</div>\
            <div class=\"any\">palabra_anywhere</div>\
            <div class=\"ba\">palabra_break_all</div>\
            <div>palabra_normal</div>\
            </body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut bw: Option<OverflowWrap> = None;
        let mut any: Option<OverflowWrap> = None;
        let mut ba: Option<WordBreak> = None;
        let mut normal_ow: Option<OverflowWrap> = None;
        let mut normal_wb: Option<WordBreak> = None;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains("break_word") {
                    bw = Some(b.overflow_wrap);
                } else if t.contains("anywhere") {
                    any = Some(b.overflow_wrap);
                } else if t.contains("break_all") {
                    ba = Some(b.word_break);
                } else if t.contains("normal") {
                    normal_ow = Some(b.overflow_wrap);
                    normal_wb = Some(b.word_break);
                }
            }
        });
        assert_eq!(bw, Some(OverflowWrap::BreakWord), "break-word hereda al leaf");
        assert_eq!(any, Some(OverflowWrap::Anywhere), "anywhere hereda al leaf");
        assert_eq!(ba, Some(WordBreak::BreakAll), "break-all hereda al leaf");
        assert_eq!(normal_ow, Some(OverflowWrap::Normal), "default overflow-wrap = Normal");
        assert_eq!(normal_wb, Some(WordBreak::Normal), "default word-break = Normal");
    }

