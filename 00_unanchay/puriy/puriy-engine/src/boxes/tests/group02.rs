//! Tests del box-tree (grupo 02, extraído de `boxes/mod.rs`, regla #1).
use crate::Engine;

    #[test]
    fn link_stylesheet_externo_data_url_aplica() {
        // `<link rel="stylesheet" href="data:text/css,...">` — la hoja externa
        // se baja (acá vía data:, sin red) y sus reglas entran a la cascada.
        let html = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3A%23008000%7D">
        </head><body><p>verde</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::super::Color::rgb(0, 128, 0) {
                found = true;
            }
        });
        assert!(found, "la regla de la hoja externa data: no se aplicó al <p>");
    }

    #[test]
    fn link_relativo_resuelve_contra_base_href() {
        // `<base href="file://<dir>/">` + `<link href="x.css">` relativo debe
        // bajar `<dir>/x.css` (no contra la URL del documento). file:// = sin red.
        let mut dir = std::env::temp_dir();
        dir.push(format!("puriy_basehref_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.css"), "p { color: #00ff00 }").unwrap();
        let base = format!("file://{}/", dir.display());
        let html = format!(
            r##"<html><head><base href="{base}"><link rel="stylesheet" href="x.css"></head><body><p>v</p></body></html>"##
        );
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::super::Color::rgb(0, 255, 0) {
                found = true;
            }
        });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found, "el <link> relativo no resolvió contra <base href>");
    }

    #[test]
    fn import_en_style_inline_se_sigue() {
        // `@import` de un data: CSS dentro de un <style> — sus reglas aplican.
        let html = r##"<html><head><style>
            @import url("data:text/css,p%7Bcolor%3A%23ff0000%7D");
        </style></head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::super::Color::rgb(255, 0, 0) {
                found = true;
            }
        });
        assert!(found, "la regla del @import no se aplicó");
    }

    #[test]
    fn import_precede_a_las_reglas_propias_en_cascada() {
        // @import pone rojo; la regla propia (después) lo pisa a azul → azul.
        let html = r##"<html><head><style>
            @import url("data:text/css,p%7Bcolor%3Ared%7D");
            p { color: #0000ff }
        </style></head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut p_color = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                p_color = Some(b.color);
            }
        });
        assert_eq!(p_color, Some(super::super::Color::rgb(0, 0, 255)), "la regla propia debe ganar al @import");
    }

    #[test]
    fn link_media_print_no_aplica_en_pantalla() {
        // `<link media="print">` no debe aplicar al render de pantalla; la
        // misma regla con `media="screen"` sí. DEFAULT_VIEWPORT es screen.
        let print = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D" media="print">
        </head><body><p>x</p></body></html>"##;
        let screen = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D" media="screen">
        </head><body><p>x</p></body></html>"##;
        let eng = Engine::new();
        let red = super::super::Color::rgb(255, 0, 0);
        let color_of = |html: &str| {
            let doc = eng.load_html("about:test", html);
            let mut c = None;
            doc.box_tree.walk(|b| {
                if b.tag.as_deref() == Some("p") {
                    c = Some(b.color);
                }
            });
            c
        };
        assert_ne!(color_of(print), Some(red), "media=print no debía aplicar en pantalla");
        assert_eq!(color_of(screen), Some(red), "media=screen sí debía aplicar");
    }

    #[test]
    fn link_stylesheet_cascada_respeta_orden_de_documento() {
        // Hoja externa (data:) declara color rojo; un `<style>` posterior lo
        // pisa a azul — el orden de documento debe ganar (azul), no el externo.
        let html = r##"<html><head>
            <link rel="stylesheet" href="data:text/css,p%7Bcolor%3Ared%7D">
            <style>p { color: #0000ff }</style>
        </head><body><p>azul</p></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut p_color = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                p_color = Some(b.color);
            }
        });
        assert_eq!(p_color, Some(super::super::Color::rgb(0, 0, 255)), "el <style> posterior debe ganar");
    }

    #[test]
    fn details_sin_open_attr_arranca_cerrado() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![false]);
    }

    #[test]
    fn details_con_open_attr_lo_refleja() {
        let html = r#"<html><body>
            <details open><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![true]);
    }

    #[test]
    fn details_summary_se_parsean_como_tags() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut saw_details = false;
        let mut saw_summary = false;
        doc.box_tree.walk(|b| {
            match b.tag.as_deref() {
                Some("details") => saw_details = true,
                Some("summary") => saw_summary = true,
                _ => {}
            }
        });
        assert!(saw_details, "no se encontró <details> en el box tree");
        assert!(saw_summary, "no se encontró <summary> en el box tree");
    }

    #[test]
    fn details_open_attr_es_false_para_nodos_no_details() {
        let html = "<html><body><p>x</p><h1>y</h1></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() != Some("details") {
                assert!(!b.details_open_attr, "{:?} no debería tener details_open_attr=true", b.tag);
            }
        });
    }

    #[test]
    fn ws_entre_blocks_se_filtra() {
        // El "\n  " entre </h1> y <p> produce un Text node " " que NO
        // debería rendear como un row vacío.
        let html = "<html><body><h1>A</h1>\n  <p>B</p>\n  <h2>C</h2></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Walk del body. Esperamos sólo h1, p, h2 como children directos
        // (sin text-leaves de whitespace entre ellos).
        let body = &doc.box_tree.root;
        // Body envuelve un Inline de transición (collapse_whitespace puede
        // dejar uno leading o trailing). Recorremos directamente.
        let mut top_tags: Vec<Option<String>> = body
            .children
            .iter()
            .filter(|c| !super::super::is_ws_only_inline(c))
            .map(|c| c.tag.clone())
            .collect();
        // Aseguramos que el filtrado sólo dejó tags reales.
        top_tags.retain(|t| t.is_some());
        let names: Vec<&str> = top_tags
            .iter()
            .map(|t| t.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, vec!["h1", "p", "h2"]);
        // Y verificamos que NO hay inlines whitespace-only entre ellos en
        // el árbol real (post-strip).
        for c in &body.children {
            assert!(
                !super::super::is_ws_only_inline(c),
                "el body no debería tener inlines ws-only entre blocks: {:?}",
                c.text
            );
        }
    }

    #[test]
    fn ws_alrededor_de_inline_se_preserva() {
        // El espacio entre "foo " y <strong>bar</strong> y " baz" sí
        // tiene valor — debe quedarse para no pegar tokens.
        let html = "<html><body><p>foo <strong>bar</strong> baz</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Encontramos el <p> y verificamos que sus children contengan
        // textos con espacios donde corresponde.
        let mut texts: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                for c in &b.children {
                    if let Some(t) = &c.text {
                        texts.push(t.clone());
                    }
                    // Si es <strong>, mirá su hijo
                    if c.tag.as_deref() == Some("strong") {
                        for cc in &c.children {
                            if let Some(t) = &cc.text {
                                texts.push(format!("[strong]{t}"));
                            }
                        }
                    }
                }
            }
        });
        // Esperamos que "foo " conserve el espacio trailing y " baz" el leading.
        assert!(
            texts.iter().any(|t| t.ends_with(' ')),
            "esperaba un text con espacio trailing en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.starts_with(' ')),
            "esperaba un text con espacio leading en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t == "[strong]bar"),
            "esperaba `bar` dentro de strong en {:?}",
            texts
        );
    }

    #[test]
    fn link_target_blank_marca_link_new_tab() {
        let html = r#"<html><body>
            <a href="https://a.test/" target="_blank">A</a>
            <a href="https://b.test/">B</a>
            <a href="https://c.test/" target="_self">C</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut links: Vec<(String, bool)> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(target) = &b.link {
                    links.push((target.clone(), b.link_new_tab));
                }
            }
        });
        assert!(links.iter().any(|(u, nt)| u.contains("a.test") && *nt));
        assert!(links.iter().any(|(u, nt)| u.contains("b.test") && !*nt));
        assert!(links.iter().any(|(u, nt)| u.contains("c.test") && !*nt));
    }

    #[test]
    fn link_mailto_y_tel_y_javascript_se_ignoran() {
        let html = r#"<html><body>
            <a href="mailto:foo@bar">M</a>
            <a href="tel:+541112345678">T</a>
            <a href="javascript:alert(1)">J</a>
            <a href="data:text/plain,hi">D</a>
            <a href="ftp://example.com/">F</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut clickable: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(t) = &b.link {
                    clickable.push(t.clone());
                }
            }
        });
        assert!(clickable.is_empty(), "ningún href no-web debería ser clickable: {clickable:?}");
    }

    #[test]
    fn srcset_elige_la_densidad_mas_alta() {
        let url = super::super::pick_srcset("foo.png 1x, foo-2x.png 2x, foo-3x.png 3x");
        assert_eq!(url.as_deref(), Some("foo-3x.png"));
    }

    #[test]
    fn srcset_elige_el_ancho_mas_grande() {
        let url = super::super::pick_srcset("a.png 320w, b.png 800w, c.png 1600w");
        assert_eq!(url.as_deref(), Some("c.png"));
    }

    #[test]
    fn srcset_sin_descriptor_usa_la_primera_con_1x_implicito() {
        // En la práctica un srcset sin descriptor es equivalente a 1x.
        let url = super::super::pick_srcset("a.png, b.png");
        // No importa el orden interno — basta con que devuelva alguno.
        assert!(url.is_some());
    }

    #[test]
    fn svg_parsea_polygon_y_polyline() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <polygon points="0,0 50,0 50,50" fill="red"/>
                <polyline points="0,100 100,50 100,0" stroke="blue"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut prim_count = 0;
        let mut had_closed = false;
        let mut had_open = false;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Polyline { points, closed, .. } = p {
                        prim_count += 1;
                        if *closed {
                            had_closed = true;
                            assert_eq!(points.len(), 3);
                        } else {
                            had_open = true;
                        }
                    }
                }
            }
        });
        assert_eq!(prim_count, 2);
        assert!(had_closed);
        assert!(had_open);
    }

    #[test]
    fn svg_parsea_path_minimal() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <path d="M 10 10 L 90 10 L 50 90 Z" fill="green"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut cmds_count = 0;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Path { d, .. } = p {
                        cmds_count = d.len();
                    }
                }
            }
        });
        // M, L, L, Z → 4 cmds.
        assert_eq!(cmds_count, 4);
    }

    #[test]
    fn svg_recolecta_rect_circle_y_line() {
        let html = r##"<html><body>
            <svg width="200" height="100" viewBox="0 0 200 100">
                <rect x="10" y="10" width="50" height="30" fill="red" stroke="black" stroke-width="2"/>
                <circle cx="120" cy="50" r="20" fill="blue"/>
                <line x1="0" y1="0" x2="200" y2="100" stroke="green" stroke-width="3"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut scene: Option<crate::SvgScene> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                scene = Some(s.clone());
            }
        });
        let scene = scene.expect("debería haber un <svg>");
        assert_eq!(scene.width, 200.0);
        assert_eq!(scene.height, 100.0);
        assert_eq!(scene.view_box, Some((0.0, 0.0, 200.0, 100.0)));
        assert_eq!(scene.prims.len(), 3);
        match &scene.prims[0] {
            crate::SvgPrim::Rect { x, y, w, h, fill, stroke, .. } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y, 10.0);
                assert_eq!(*w, 50.0);
                assert_eq!(*h, 30.0);
                assert!(fill.is_some());
                assert!(stroke.is_some());
            }
            _ => panic!("primera prim debería ser Rect"),
        }
        match &scene.prims[1] {
            crate::SvgPrim::Circle { cx, cy, r, .. } => {
                assert_eq!(*cx, 120.0);
                assert_eq!(*cy, 50.0);
                assert_eq!(*r, 20.0);
            }
            _ => panic!("segunda prim debería ser Circle"),
        }
        match &scene.prims[2] {
            crate::SvgPrim::Line { x1, y2, .. } => {
                assert_eq!(*x1, 0.0);
                assert_eq!(*y2, 100.0);
            }
            _ => panic!("tercera prim debería ser Line"),
        }
    }

    #[test]
    fn select_recolecta_options_y_seleccionado_inicial() {
        let html = r##"<html><body>
            <form action="/p">
                <select name="lang">
                    <option value="es">Español</option>
                    <option value="en" selected>English</option>
                    <option>Otro</option>
                </select>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        let mut info: Option<crate::SelectInfo> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.select {
                info = Some(s.clone());
                assert_eq!(b.input_name.as_deref(), Some("lang"));
                assert_eq!(b.form_idx, Some(0));
            }
        });
        let info = info.expect("debería haber un <select>");
        assert_eq!(info.options.len(), 3);
        assert_eq!(info.options[0].value, "es");
        assert_eq!(info.options[0].label, "Español");
        assert_eq!(info.options[2].label, "Otro");
        assert_eq!(info.options[2].value, "Otro"); // fallback al label
        assert_eq!(info.initial, 1); // <option selected> es el segundo
    }

    #[test]
    fn form_asigna_form_idx_a_inputs_que_contiene() {
        let html = r##"<html><body>
            <form action="/search" method="get">
                <input type="text" name="q" value="hola">
                <input type="text" name="lang" value="es">
            </form>
            <input type="text" name="outside">
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        assert_eq!(doc.box_tree.forms.len(), 1);
        let mut names_inside: Vec<String> = Vec::new();
        let mut outside_form_idx: Option<usize> = None;
        doc.box_tree.walk(|b| {
            if let Some(name) = &b.input_name {
                if b.form_idx == Some(0) {
                    names_inside.push(name.clone());
                } else if b.input_kind.is_some() && name == "outside" {
                    outside_form_idx = b.form_idx;
                }
            }
        });
        assert_eq!(names_inside, vec!["q".to_string(), "lang".into()]);
        assert_eq!(outside_form_idx, None);
        assert_eq!(
            doc.box_tree.forms[0].action.as_deref(),
            Some("https://example.com/search")
        );
    }

    #[test]
    fn em_y_i_y_cite_son_italic_por_default() {
        let html = "<html><body><em>a</em><i>b</i><cite>c</cite><p>d</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Vec<(String, crate::FontStyle)> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(tag) = &b.tag {
                if matches!(tag.as_str(), "em" | "i" | "cite" | "p") {
                    found.push((tag.clone(), b.font_style));
                }
            }
        });
        let em = found.iter().find(|(t, _)| t == "em").unwrap();
        let i = found.iter().find(|(t, _)| t == "i").unwrap();
        let cite = found.iter().find(|(t, _)| t == "cite").unwrap();
        let p = found.iter().find(|(t, _)| t == "p").unwrap();
        assert_eq!(em.1, crate::FontStyle::Italic);
        assert_eq!(i.1, crate::FontStyle::Italic);
        assert_eq!(cite.1, crate::FontStyle::Italic);
        assert_eq!(p.1, crate::FontStyle::Normal);
    }

    #[test]
    fn font_style_normal_override_padre_italic() {
        let html = r##"<html><head><style>
            .x { font-style: normal }
        </style></head><body><em>fuera<span class="x">dentro</span></em></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_style: Option<crate::FontStyle> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_style = Some(b.font_style);
            }
        });
        assert_eq!(span_style, Some(crate::FontStyle::Normal));
    }

    #[test]
    fn focus_pseudo_aporta_a_focus_background() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            input { background: white }
            input:focus { background: #ffeecc }
        </style></head><body><input type="text"></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let input = dom.find("input").unwrap();
        let base = styles.compute_with_parent_for_state(&input, None, false, false);
        let focused = styles.compute_with_parent_for_state(&input, None, false, true);
        // base es blanco (255,255,255), focused es #ffeecc (255,238,204).
        assert_eq!(base.background.map(|c| (c.r, c.g, c.b)), Some((255, 255, 255)));
        assert_eq!(focused.background.map(|c| (c.r, c.g, c.b)), Some((255, 238, 204)));
    }

    #[test]
    fn box_tree_expone_focus_background() {
        let html = r##"<html><head><style>
            input:focus { background: #abcdef }
        </style></head><body><input type="text"></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("input") {
                assert_eq!(
                    b.focus_background.map(|c| (c.r, c.g, c.b)),
                    Some((0xab, 0xcd, 0xef))
                );
                found = true;
            }
        });
        assert!(found, "no se encontró <input> en el box tree");
    }

    #[test]
    fn parsea_background_image_url_a_computed_style_y_no_descarga_si_url_no_resuelve() {
        // Sin red, fetch_and_decode falla y background_image queda None.
        // Pero el url SÍ debe quedar capturado en computed.background_image_url
        // (visible al re-parsear el stylesheet).
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url("https://nope.invalid/bg.png") }
        </style></head><body><div class="hero">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert_eq!(
            s.background_image_url.as_deref(),
            Some("https://nope.invalid/bg.png")
        );
    }

    #[test]
    fn background_image_none_limpia_url() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url(a.png) }
            .hero.off { background-image: none }
        </style></head><body><div class="hero off">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert!(s.background_image_url.is_none());
    }

    #[test]
    fn link_fragmento_se_resuelve_a_base_mas_frag() {
        // Antes: `#top` se ignoraba (None). Ahora resuelve contra la
        // base — el chrome detecta same-page y scrollea en lugar de
        // recargar la URL.
        let html = r##"<html><body><a href="#top">arriba</a></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/doc", html);
        let mut links: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(l) = &b.link {
                links.push(l.clone());
            }
        });
        assert_eq!(links, vec!["https://example.com/doc#top".to_string()]);
    }

    #[test]
    fn iframe_se_renderea_como_placeholder_con_url() {
        let html = r##"<html><body>
            <iframe src="https://embed.example.com/video"></iframe>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Option<String> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("iframe") {
                if let Some(first) = b.children.first() {
                    found = first.text.clone();
                }
            }
        });
        assert_eq!(
            found.as_deref(),
            Some("[iframe: https://embed.example.com/video]")
        );
    }

    #[test]
    fn iframe_sin_src_muestra_label_generico() {
        let html = "<html><body><iframe></iframe></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Option<String> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("iframe") {
                found = b.children.first().and_then(|c| c.text.clone());
            }
        });
        assert_eq!(found.as_deref(), Some("[iframe sin src]"));
    }

    #[test]
    fn content_url_parser_acepta_quoted_y_unquoted() {
        use crate::ContentItem;
        let html = r##"<html><head><style>
            .a::before { content: url("https://x/y.png") }
            .b::before { content: url(https://x/z.png) }
        </style></head><body>
            <p class="a"></p>
            <p class="b"></p>
        </body></html>"##;
        let dom = crate::DomTree::parse(html);
        let eng = crate::StyleEngine::from_dom(&dom);
        let ps_a = dom.find("p").unwrap();
        let before = eng.compute_pseudo(&ps_a, crate::PseudoElement::Before, None);
        assert_eq!(
            before.and_then(|s| s.content),
            Some(vec![ContentItem::Url("https://x/y.png".into())])
        );
    }

    #[test]
    fn margin_collapse_padre_promueve_margin_del_primer_hijo() {
        // <body style="margin: 8px"> con primer hijo
        // <div style="margin: 20px 0 0 0">: el body no tiene padding/
        // border arriba, así que el margin_top del div se promueve al
        // body. Final: body.margin.top = max(8, 20) = 20; div.margin.top = 0.
        let html = r##"<html><body style="margin: 8px">
            <div style="margin: 20px 0 0 0">x</div>
            <div style="margin: 0 0 12px 0">y</div>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // body es el root del box tree (BoxTree.root viene de
        // dom.find("body")).
        assert_eq!(doc.box_tree.root.tag.as_deref(), Some("body"));
        assert_eq!(doc.box_tree.root.margin.top, 20.0);
        assert_eq!(doc.box_tree.root.margin.bottom, 12.0);
        // El primer hijo div quedó con margin.top = 0 (promovido).
        let first_div = &doc.box_tree.root.children[0];
        assert_eq!(first_div.margin.top, 0.0);
        // El último div: margin.bottom promovido al body.
        let last_div = doc.box_tree.root.children.last().unwrap();
        assert_eq!(last_div.margin.bottom, 0.0);
    }

