//! Tests del box-tree (grupo 03, extraído de `boxes/mod.rs`, regla #1).
use super::super::Display;
use crate::Engine;

    #[test]
    fn margin_collapse_padre_bloqueado_por_padding() {
        // Si el body tiene padding-top, el margin del primer hijo NO
        // colapsa contra el body — el padding es la "barrera".
        let html = r##"<html><body style="margin: 8px; padding: 10px 0 0 0">
            <div style="margin: 20px 0 0 0">x</div>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert_eq!(doc.box_tree.root.margin.top, 8.0);
        let first_div = &doc.box_tree.root.children[0];
        assert_eq!(first_div.margin.top, 20.0);
    }

    #[test]
    fn margin_collapsing_max_entre_block_siblings() {
        // `<h2 style="margin: 0 0 20px 0">` seguido de `<p style="margin: 10px 0 0 0">`:
        // gap esperado es max(20, 10) = 20. El margin_bottom del h2
        // queda intacto (20), el margin_top del p baja a 0.
        let html = r##"<html><body>
            <h2 style="margin: 0 0 20px 0">Heading</h2>
            <p style="margin: 10px 0 0 0">Para</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut h2_margin_bottom: Option<f32> = None;
        let mut p_margin_top: Option<f32> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("h2") {
                h2_margin_bottom = Some(b.margin.bottom);
            }
            if b.tag.as_deref() == Some("p") {
                p_margin_top = Some(b.margin.top);
            }
        });
        assert_eq!(h2_margin_bottom, Some(20.0));
        // 10 - min(20, 10) = 10 - 10 = 0. Gap total = 20 + 0 = 20 = max.
        assert_eq!(p_margin_top, Some(0.0));
    }

    #[test]
    fn margin_collapsing_no_aplica_a_inline() {
        // Block + inline no colapsan — el inline vive en otro flow.
        let html = r##"<html><body>
            <p style="margin: 0 0 10px 0">Para</p>
            <span style="margin: 5px 0 0 0">inline</span>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_margin_top: Option<f32> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_margin_top = Some(b.margin.top);
            }
        });
        // No tocado.
        assert_eq!(span_margin_top, Some(5.0));
    }

    #[test]
    fn prefetch_no_crashea_sin_imagenes() {
        // Sanity: páginas sin imágenes no deben fallar el prefetch.
        let html = "<html><body><p>solo texto</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Si llegó acá sin panic, OK.
        assert!(doc.box_tree.descendants_count() > 0);
    }

    #[test]
    fn prefetch_skip_de_urls_no_http() {
        // URLs `about:`/`file:`/`data:` no deben encolarse al pool —
        // sería un round-trip al timeout para nada. El test pone una
        // base `about:test` con `<img src="...">` que resuelve a
        // about:... y verifica que la carga termina rápido (sin
        // esperar timeouts de red).
        let html = r##"<html><body><img src="x.png"></body></html>"##;
        let eng = Engine::new();
        let t0 = std::time::Instant::now();
        let _ = eng.load_html("about:test", html);
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 500,
            "load_html con base about: y un <img> debería ser instantáneo, fue {elapsed:?}"
        );
    }

    #[test]
    fn img_data_url_se_decodifica_inline() {
        // `<img src="data:image/png;base64,...">` con un PNG 1×1 (un pixel rojo).
        // `resolve_href` bloquea data: (no navegable), pero como fuente de
        // imagen `fetch_image_src` lo decodifica sin tocar la red.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let html = format!(r##"<html><body><img src="{png_1x1}"></body></html>"##);
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut img_dims: Option<(u32, u32)> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("img") {
                if let Some(img) = &b.image {
                    img_dims = Some((img.width, img.height));
                }
            }
        });
        assert_eq!(img_dims, Some((1, 1)), "el PNG data: debería decodificar a 1×1");
    }

    #[test]
    fn mask_image_url_se_decodifica_inline_fase_7_1226() {
        // `mask-image: url(data:...)` decodifica con la misma cache/decoder que
        // `<img>`/`background-image`; el box lleva la imagen-máscara lista para
        // que el compositor la aplique como luminancia sobre el subárbol. Los
        // nodos sin `mask-image` quedan en `None`.
        //
        // Se usa la forma `data:` percent-encoded (sin `;base64`) a propósito:
        // el `;` de `;base64` rompería el splitter naive de declaraciones CSS
        // (`split(';')`) — limitación pre-existente del parser. Estos son los
        // bytes crudos del mismo PNG 1×1 que usa `img_data_url_se_decodifica`.
        let png_1x1 = "data:image/png,%89PNG%0D%0A%1A%0A%00%00%00%0DIHDR%00%00%00%01%00%00%00%01%08%06%00%00%00%1F%15%C4%89%00%00%00%0DIDATx%9Cc%F8%CF%C0%F0%1F%00%05%00%01%FF%89%99%3D%1D%00%00%00%00IEND%AEB%60%82";
        let html = format!(
            r##"<html><body><div style="mask-image: url({png_1x1})">x</div><p>y</p></body></html>"##
        );
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        use crate::style::{BackgroundRepeat, BackgroundSize, MaskMode};
        let mut dims_con_mask: Vec<(u32, u32)> = Vec::new();
        let mut encaje_con_mask: Vec<(BackgroundSize, BackgroundRepeat, MaskMode)> = Vec::new();
        let mut cajas_con_mask: Vec<(Option<[f32; 4]>, Option<[f32; 4]>)> = Vec::new();
        let mut hay_sin_mask = false;
        doc.box_tree.walk(|b| match &b.mask_image {
            Some(spec) => {
                dims_con_mask.push((spec.image.width, spec.image.height));
                encaje_con_mask.push((spec.size, spec.repeat, spec.mode));
                cajas_con_mask.push((spec.clip_inset, spec.origin_inset));
            }
            None => hay_sin_mask = true,
        });
        assert_eq!(
            dims_con_mask,
            vec![(1, 1)],
            "sólo el div con mask-image lleva la máscara decodificada (1×1)"
        );
        // El encaje + modo viajan con la imagen — sin mask-size/repeat/mode
        // declarados, los defaults CSS (auto / repeat / match-source) llegan al
        // box (Fase 7.1227 encaje, 7.1228 modo). `match-source` lo resuelve el
        // wire a alpha (raster).
        assert_eq!(
            encaje_con_mask,
            vec![(
                BackgroundSize::Auto,
                BackgroundRepeat::Repeat,
                MaskMode::MatchSource
            )],
            "el box lleva el encaje y modo por defecto (auto / repeat / match-source)"
        );
        // mask-clip/mask-origin default = border-box → sin insets (Fase 7.1230).
        assert_eq!(
            cajas_con_mask,
            vec![(None, None)],
            "mask-clip/origin por defecto (border-box) no insetean"
        );
        assert!(hay_sin_mask, "los nodos sin mask-image quedan en None");
    }

    #[test]
    fn mask_image_capas_multiples_se_decodifican_fase_7_1231() {
        // `mask-image: url(a), url(b)` → capa 0 en `spec.image` + 1 extra en
        // `spec.extra` con el operador mask-composite compartido (default add).
        // Se usan dos data: percent-encoded (sin `;`) del mismo PNG 1×1; la
        // coma interna de cada `data:image/png,...` la protege el `url(...)`.
        let png = "data:image/png,%89PNG%0D%0A%1A%0A%00%00%00%0DIHDR%00%00%00%01%00%00%00%01%08%06%00%00%00%1F%15%C4%89%00%00%00%0DIDATx%9Cc%F8%CF%C0%F0%1F%00%05%00%01%FF%89%99%3D%1D%00%00%00%00IEND%AEB%60%82";
        let html = format!(
            r##"<html><body><div style="mask-image: url({png}), url({png})">x</div></body></html>"##
        );
        let eng = Engine::new();
        let doc = eng.load_html("about:test", &html);
        let mut found = None;
        doc.box_tree.walk(|b| {
            if let Some(spec) = &b.mask_image {
                found = Some((
                    (spec.image.width, spec.image.height),
                    spec.extra.len(),
                    spec.extra.first().map(|(im, comp)| ((im.width, im.height), *comp)),
                ));
            }
        });
        assert_eq!(
            found,
            Some(((1, 1), 1, Some(((1, 1), crate::style::MaskComposite::Add)))),
            "capa 0 (1×1) + 1 extra (1×1) con composite default add"
        );
    }

    #[test]
    fn canvas_genera_box_con_tamano_intrinseco() {
        // `<canvas>` ya no es display:none (Fase 7.196): produce un box con
        // `canvas: Some((w, h))` tomado de los atributos, default 300×150.
        let html = r##"<html><body>
            <canvas id="c1" width="200" height="120"></canvas>
            <canvas id="c2"></canvas>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Vec<(String, Option<(f32, f32)>)> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("canvas") {
                found.push((b.element_id.clone().unwrap_or_default(), b.canvas));
            }
        });
        assert_eq!(
            found,
            vec![
                ("c1".to_string(), Some((200.0, 120.0))),
                ("c2".to_string(), Some((300.0, 150.0))),
            ],
            "canvas con atributos toma su tamaño; sin atributos cae a 300×150"
        );
    }

    #[test]
    fn counter_numera_h2_sequencialmente() {
        // Patrón clásico: body resetea el contador a 0, cada h2::before
        // lo incrementa y muestra el valor — h2 numerados 1, 2, 3.
        let html = r##"<html><head><style>
            body { counter-reset: sec }
            h2::before { counter-increment: sec; content: counter(sec) ". " }
        </style></head><body>
            <h2>Intro</h2>
            <h2>Cuerpo</h2>
            <h2>Cierre</h2>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Recolectamos el primer text leaf de cada h2 (el ::before).
        let mut h2_prefixes: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("h2") {
                if let Some(first) = b.children.first() {
                    if let Some(t) = &first.text {
                        h2_prefixes.push(t.clone());
                    }
                }
            }
        });
        assert_eq!(h2_prefixes, vec!["1. ", "2. ", "3. "]);
    }

    #[test]
    fn attr_en_content_lee_del_padre_del_pseudo() {
        // `<a data-tag="X">` con `a::after { content: " [" attr(data-tag) "]" }`
        // debe inyectar " [X]" después del texto del link.
        let html = r##"<html><head><style>
            a::after { content: " [" attr(data-tag) "]" }
        </style></head><body>
            <a href="#" data-tag="ALPHA">link</a>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut a_children: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") && a_children.is_empty() {
                a_children = b
                    .children
                    .iter()
                    .filter_map(|c| c.text.clone())
                    .collect();
            }
        });
        assert_eq!(a_children, vec!["link".to_string(), " [ALPHA]".to_string()]);
    }

    #[test]
    fn before_y_after_se_inyectan_como_children() {
        let html = r##"<html><head><style>
            .badge::before { content: "▸ " }
            .badge::after  { content: " !" }
        </style></head><body>
            <p class="badge">Hola</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El `<p>` tiene 3 hijos: el ::before, el text leaf "Hola", el ::after.
        let mut p_children: Option<Vec<String>> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && p_children.is_none() {
                p_children = Some(
                    b.children
                        .iter()
                        .filter_map(|c| c.text.clone())
                        .collect(),
                );
            }
        });
        let texts = p_children.expect("debería encontrar <p>");
        assert_eq!(texts, vec!["▸ ".to_string(), "Hola".to_string(), " !".to_string()]);
    }

    #[test]
    fn find_y_of_match_devuelve_y_creciente_por_match() {
        let html = r##"<html><body>
            <p>alfa</p><p>beta</p><p>alfa beta</p><p>alfa</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let bt = &doc.box_tree;
        let y1 = bt.find_y_of_match("alfa", 1).expect("match 1");
        let y2 = bt.find_y_of_match("alfa", 2).expect("match 2");
        let y3 = bt.find_y_of_match("alfa", 3).expect("match 3");
        assert!(y2 > y1, "match 2 debe quedar más abajo que match 1");
        assert!(y3 > y2);
        // Sin match para el 4to.
        assert!(bt.find_y_of_match("alfa", 4).is_none());
        // Query vacía o nth=0 devuelven None.
        assert!(bt.find_y_of_match("", 1).is_none());
        assert!(bt.find_y_of_match("alfa", 0).is_none());
    }

    #[test]
    fn input_autofocus_se_marca_solo_para_inputs_con_attr() {
        let html = r##"<html><body>
            <form>
                <input type="text" name="a">
                <input type="text" name="b" autofocus>
                <input type="text" name="c" autofocus>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut flags: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.input_kind.is_some() {
                flags.push(b.input_autofocus);
            }
        });
        assert_eq!(flags, vec![false, true, true]);
    }

    #[test]
    fn element_id_se_extrae_del_attr() {
        let html = r##"<html><body>
            <h2 id="intro">Intro</h2>
            <p id="">vacío no cuenta</p>
            <p>sin id</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = &b.element_id {
                ids.push(id.clone());
            }
        });
        assert_eq!(ids, vec!["intro".to_string()]);
    }

    #[test]
    fn ws_solo_inline_no_se_dropea_si_padre_es_inline_flow() {
        // <p>foo<span> </span>bar</p> — el espacio dentro de span sí debe
        // quedar porque separa "foo" de "bar".
        let html = "<html><body><p>foo<span> </span>bar</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_space = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                for c in &b.children {
                    if c.text.as_deref().map(|s| s.contains(' ')).unwrap_or(false) {
                        found_space = true;
                    }
                }
            }
        });
        assert!(found_space, "el espacio dentro de <span> debería preservarse");
    }

    #[test]
    fn set_element_text_content_reemplaza_hoja() {
        let html = r#"<html><body><h1 id="hero">Hola</h1></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("hero", "Adiós");
        assert!(ok);
        // Verificar que la hoja de texto se actualizó.
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("Adiós") {
                found = true;
            }
        });
        assert!(found, "no se encontró 'Adiós' en el árbol post-mutación");
    }

    #[test]
    fn set_element_text_content_no_encuentra_id_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("fantasma", "x");
        assert!(!ok);
    }

    #[test]
    fn set_element_style_color_actualiza_text_leaves() {
        let html = r#"<html><body><h1 id="h">hola</h1></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_style("h", "color", "red");
        assert!(ok);
        // El leaf de texto debe haber heredado el color rojo.
        let mut color_changed = false;
        doc.box_tree.walk(|b| {
            if b.text.as_deref() == Some("hola") {
                if b.color.r == 255 && b.color.g == 0 && b.color.b == 0 {
                    color_changed = true;
                }
            }
        });
        assert!(color_changed);
    }

    #[test]
    fn set_element_style_background_hex() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_style("d", "background", "#abc"));
        let mut bg_set = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                if let Some(c) = b.background {
                    if c.r == 0xaa && c.g == 0xbb && c.b == 0xcc {
                        bg_set = true;
                    }
                }
            }
        });
        assert!(bg_set);
    }

    #[test]
    fn set_element_style_display_none_oculta() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_style("d", "display", "none"));
        let mut hidden = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("d") {
                if matches!(b.display, Display::None) {
                    hidden = true;
                }
            }
        });
        assert!(hidden);
    }

    #[test]
    fn set_element_style_prop_desconocida_devuelve_false() {
        let html = r#"<html><body><div id="d">x</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_style("d", "transform", "rotate(45deg)"));
    }

    #[test]
    fn set_element_style_id_inexistente_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_style("fantasma", "color", "red"));
    }

    // ============= Fase 7.16 — attributes genéricos =============

    #[test]
    fn box_node_attributes_contiene_todos_los_attrs_html() {
        let html = r#"<html><body><a id="x" href="https://tawasuyu.net" aria-current="page" data-track="hero" rel="noopener">x</a></body></html>"#;
        let doc = Engine::new().load_html("about:t", html);
        let mut found: Option<Vec<(String, String)>> = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                found = Some(b.attributes.clone());
            }
        });
        let attrs = found.expect("a#x existe");
        // Todos los attrs aparecen, lowercase names, values literales.
        assert!(attrs.iter().any(|(k, v)| k == "href" && v == "https://tawasuyu.net"));
        assert!(attrs.iter().any(|(k, v)| k == "aria-current" && v == "page"));
        assert!(attrs.iter().any(|(k, v)| k == "data-track" && v == "hero"));
        assert!(attrs.iter().any(|(k, v)| k == "rel" && v == "noopener"));
        // El attr id también aparece — no se filtra (el getAttribute('id')
        // resuelve por la rama especial del JS, pero el campo se mantiene
        // uniforme para evitar ramas adicionales en el chrome).
        assert!(attrs.iter().any(|(k, v)| k == "id" && v == "x"));
    }

    #[test]
    fn box_node_dataset_filter_view_devuelve_solo_data_attrs() {
        let html = r##"<html><body><div id="x" data-foo="1" aria-label="hi" data-bar-baz="2" href="#">y</div></body></html>"##;
        let doc = Engine::new().load_html("about:t", html);
        let mut found: Option<Vec<(String, String)>> = None;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                found = Some(b.dataset().into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
            }
        });
        let ds = found.expect("div#x existe");
        assert_eq!(ds.len(), 2);
        assert!(ds.iter().any(|(k, v)| k == "foo" && v == "1"));
        assert!(ds.iter().any(|(k, v)| k == "bar-baz" && v == "2"));
    }

    #[test]
    fn set_element_attribute_agrega_attr_nuevo() {
        let html = r#"<html><body><div id="x">y</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_attribute("x", "aria-current", "step"));
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "aria-current" && v == "step")
            {
                found = true;
            }
        });
        assert!(found);
    }

    #[test]
    fn set_element_attribute_reemplaza_attr_existente() {
        let html = r#"<html><body><a id="x" href="/old">y</a></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.set_element_attribute("x", "href", "/nuevo"));
        let mut count_href = 0;
        let mut val = String::new();
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                for (k, v) in &b.attributes {
                    if k == "href" {
                        count_href += 1;
                        val = v.clone();
                    }
                }
            }
        });
        assert_eq!(count_href, 1, "href no debe duplicarse al reemplazar");
        assert_eq!(val, "/nuevo");
    }

    #[test]
    fn remove_element_attribute_quita_la_key() {
        let html = r#"<html><body><a id="x" href="/x" aria-label="hi">y</a></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(doc.box_tree.remove_element_attribute("x", "aria-label"));
        let mut still = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, _)| k == "aria-label")
            {
                still = true;
            }
        });
        assert!(!still);
    }

    #[test]
    fn set_element_dataset_wrapper_usa_set_element_attribute() {
        let html = r#"<html><body><div id="x">y</div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        // El wrapper de Fase 7.11 ahora delega a set_element_attribute
        // con el prefijo data-; verificamos que ambos vean el mismo store.
        assert!(doc.box_tree.set_element_dataset("x", "role", "main"));
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x")
                && b.attributes.iter().any(|(k, v)| k == "data-role" && v == "main")
            {
                found = true;
            }
        });
        assert!(found, "set_element_dataset debe poblar attributes con data-<key>");
    }

    #[test]
    fn set_element_attribute_id_inexistente_devuelve_false() {
        let html = r#"<html><body><p>x</p></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        assert!(!doc.box_tree.set_element_attribute("fantasma", "href", "/"));
    }

    #[test]
    fn set_element_text_content_reemplaza_primer_leaf_no_los_demas() {
        let html = r#"<html><body><div id="d"><span>uno</span><span>dos</span></div></body></html>"#;
        let mut doc = Engine::new().load_html("about:t", html);
        let ok = doc.box_tree.set_element_text_content("d", "X");
        assert!(ok);
        let mut texts = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if !t.trim().is_empty() {
                    texts.push(t.clone());
                }
            }
        });
        // El primer text leaf "uno" pasa a "X"; "dos" sigue intacto.
        assert!(texts.contains(&"X".to_string()), "texts: {texts:?}");
        assert!(texts.contains(&"dos".to_string()), "texts: {texts:?}");
        assert!(!texts.contains(&"uno".to_string()), "texts: {texts:?}");
    }

    #[test]
    fn box_tree_resuelve_animation_contra_keyframes() {
        // `animation: fade …` + `@keyframes fade` debe poblar BoxNode.animation
        // (Tier B: wiring del runtime de tween rescatado de engine).
        let html = r##"<html><head><style>
            @keyframes fade { from { opacity: 0 } to { opacity: 1 } }
            #target { animation: fade 2s linear }
        </style></head><body><div id="target">hola</div></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("target") {
                let inst = b.animation.as_ref().expect("div animado sin AnimationInstance");
                assert_eq!(inst.binding.name, "fade");
                // A mitad de los 2s (linear) la opacity interpolada ≈ 0.5.
                let p = crate::anim::animation_progress(&inst.binding, 1.0).unwrap();
                let ov = crate::anim::sample_keyframes(&inst.keyframes, p);
                let op = ov.opacity.expect("keyframes fade interpola opacity");
                assert!((op - 0.5).abs() < 0.05, "opacity a mitad: {op}");
                found = true;
            }
        });
        assert!(found, "no se encontró #target en el box tree");
    }

    #[test]
    fn box_tree_animation_none_sin_keyframes_match() {
        // `animation: <name>` sin `@keyframes <name>` → animation: None.
        let html = r##"<html><head><style>
            #x { animation: noexiste 1s }
        </style></head><body><div id="x">a</div></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut checked = false;
        doc.box_tree.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                assert!(b.animation.is_none(), "no debería resolver sin @keyframes");
                checked = true;
            }
        });
        assert!(checked);
    }

    #[test]
    fn background_blend_mode_llega_al_box_fase_7_1236() {
        // `background-blend-mode: multiply, screen` (lista paralela a las capas)
        // computa y aterriza en `BoxNode.background_blend_mode`, listo para que
        // el wire abra una capa de blend por capa de background. Los nodos sin
        // la propiedad quedan con la lista vacía (default = `normal`).
        use crate::style::BlendMode;
        let html = r##"<html><body>
            <div id="x" style="background-blend-mode: multiply, screen">x</div>
            <p id="y">y</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut x_modes: Option<Vec<BlendMode>> = None;
        let mut y_modes: Option<Vec<BlendMode>> = None;
        doc.box_tree.walk(|b| match b.element_id.as_deref() {
            Some("x") => x_modes = Some(b.background_blend_mode.clone()),
            Some("y") => y_modes = Some(b.background_blend_mode.clone()),
            _ => {}
        });
        assert_eq!(
            x_modes,
            Some(vec![BlendMode::Multiply, BlendMode::Screen]),
            "los dos modos llegan al box en orden de lista"
        );
        assert_eq!(
            y_modes,
            Some(vec![]),
            "sin la propiedad, la lista queda vacía (default normal)"
        );
    }
