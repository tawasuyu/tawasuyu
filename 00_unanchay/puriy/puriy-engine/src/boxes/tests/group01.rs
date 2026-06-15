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

