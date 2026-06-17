#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;
#[allow(unused_imports)] use llimphi_raster::kurbo::{Cap, Join};
#[allow(unused_imports)] use llimphi_raster::peniko::{Brush, Extend};



    #[test]
    fn apply_dataset_mutation_actualiza_box_tree() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><div id="x">y</div></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "x".into(),
            tag_name: "div".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval("document.getElementById('x').dataset.role = 'main'")
            .expect("e");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().expect("bt");
        let mut found = false;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("x") {
                if b.dataset().iter().any(|(k, v)| *k == "role" && *v == "main") {
                    found = true;
                }
            }
        });
        assert!(found, "data-role debería ser 'main' en el BoxTree");
    }

    // ============= Fase 7.12 — appendChild/removeChild =============

    #[test]
    fn apply_append_child_inserta_box_node_sintetico() {
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(r#"<body><ul id="list"></ul></body>"#));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "list".into(),
            tag_name: "ul".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(), attributes: Vec::new(), dfs_index: 0,
        }])
        .expect("e");
        rt.eval(
            "var li = document.createElement('li'); \
             li.textContent = 'hola'; \
             document.getElementById('list').appendChild(li);",
        )
        .expect("e");
        apply_dom_mutations(t);
        // El <ul id=list> ahora tiene un hijo extra que es <li>.
        let bt = t.box_tree.as_ref().expect("bt");
        let mut li_count = 0;
        let mut text_found = false;
        bt.walk(|b| {
            if b.tag.as_deref() == Some("li") {
                li_count += 1;
                if let Some(c) = b.children.first() {
                    if c.text.as_deref() == Some("hola") {
                        text_found = true;
                    }
                }
            }
        });
        assert_eq!(li_count, 1);
        assert!(text_found, "el <li> debe tener un text leaf 'hola'");
    }

    #[test]
    fn classlist_add_recascadea_y_aplica_regla() {
        // Fase 7.184 — `el.classList.add('on')` publica la mutación 'classList';
        // el chrome actualiza la clase y re-corre la cascada → el `.on` aplica.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<html><head><style>.on { background: red; }</style></head>
               <body><div id="box">x</div></body></html>"#,
        ));
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "box".into(),
            tag_name: "div".into(),
            text_content: "x".into(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: 0,
        }])
        .expect("set_elements");
        // Antes del toggle: sin background.
        let bg0 = {
            let bt = t.box_tree.as_ref().unwrap();
            let mut bg = None;
            bt.walk(|b| {
                if b.element_id.as_deref() == Some("box") {
                    bg = b.background;
                }
            });
            bg
        };
        assert_eq!(bg0, None);
        rt.eval("document.getElementById('box').classList.add('on');")
            .expect("eval");
        apply_dom_mutations(t);
        let bt = t.box_tree.as_ref().unwrap();
        let mut bg = None;
        bt.walk(|b| {
            if b.element_id.as_deref() == Some("box") {
                bg = b.background;
            }
        });
        assert_eq!(bg, Some(puriy_engine::Color::rgb(255, 0, 0)));
    }

    // ---- Fase 7.196 — Canvas 2D al render ----
    #[test]
    fn canvas_frame_deserializa_y_helpers() {
        let json = r##"[{"id":"c","width":100,"height":50,"cmds":[["fillRect",1,2,3,4,"#ff0000",{"ga":1}]]}]"##;
        let frames: Vec<CanvasFrame> = serde_json::from_str(json).expect("parse");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].id, "c");
        assert_eq!(frames[0].width, 100.0);
        assert_eq!(frames[0].cmds[0][0].as_str(), Some("fillRect"));
        // Helpers puros.
        assert_eq!(canvas_font_px(Some("16px sans-serif")), 16.0);
        assert_eq!(canvas_font_px(Some("bold 24.5px Arial")), 24.5);
        assert_eq!(canvas_font_px(None), 10.0);
        let c = canvas_color(Some(&serde_json::Value::String("#ff0000".into())), 0.5);
        assert_eq!(c.to_rgba8().to_u8_array(), [255, 0, 0, 127]);
    }

    #[test]
    fn paint_canvas_cmds_encodea_primitivas() {
        // fillRect + un path con fill: la escena vello queda no-vacía. No
        // necesita GPU (Scene es CPU-side). Smoke del intérprete.
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let cmds: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r##"[
                ["fillRect", 1, 2, 3, 4, "#ff0000", {"ga": 1}],
                ["beginPath"],
                ["moveTo", 0, 0],
                ["lineTo", 10, 10],
                ["arc", 20, 20, 5, 0, 6.28],
                ["fill", {"f": "#00ff00", "ga": 1}]
            ]"##).unwrap();
        assert!(scene.encoding().is_empty(), "escena arranca vacía");
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 50.0, &Default::default());
        assert!(!scene.encoding().is_empty(), "tras pintar debería haber segmentos");
    }

    #[test]
    fn canvas_dibuja_y_refresca_frames_end_to_end() {
        // Pipeline: box tree con <canvas>, snapshot con width/height, script
        // que pide contexto y dibuja → apply_dom_mutations refresca los frames.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="120" height="80"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "c".into(),
            tag_name: "canvas".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("width".into(), "120".into()), ("height".into(), "80".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval("var ctx = document.getElementById('c').getContext('2d'); ctx.fillStyle = '#123456'; ctx.fillRect(10, 10, 40, 30);")
            .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame del canvas");
        assert_eq!(frame.width, 120.0);
        assert_eq!(frame.height, 80.0);
        assert!(
            frame.cmds.iter().any(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect")),
            "el frame debería incluir el fillRect dibujado: {:?}",
            frame.cmds
        );
    }

    #[test]
    fn canvas_brush_gradiente_y_degradacion() {
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        // String → Brush sólido.
        let s = serde_json::Value::String("#ff0000".into());
        assert!(matches!(canvas_brush(Some(&s), 1.0, &imgs), Brush::Solid(_)));
        // CanvasGradient linear con 2 stops → Brush::Gradient(Linear).
        let lin: serde_json::Value = serde_json::from_str(
            r##"{"_kind":"linear","_coords":[0,0,100,0],"_stops":[[0,"#ff0000"],[1,"#0000ff"]]}"##,
        )
        .unwrap();
        match canvas_brush(Some(&lin), 1.0, &imgs) {
            Brush::Gradient(g) => {
                assert!(matches!(g.kind, GradientKind::Linear { .. }));
                assert_eq!(g.stops.0.len(), 2);
            }
            _ => panic!("debería ser gradiente"),
        }
        // Radial.
        let rad: serde_json::Value = serde_json::from_str(
            r##"{"_kind":"radial","_coords":[10,10,0,10,10,50],"_stops":[[0,"#fff"],[1,"#000"]]}"##,
        )
        .unwrap();
        assert!(matches!(
            canvas_brush(Some(&rad), 1.0, &imgs),
            Brush::Gradient(g) if matches!(g.kind, GradientKind::Radial { .. })
        ));
        // Gradiente con un solo stop (inválido) → degrada a sólido (último stop).
        let bad: serde_json::Value =
            serde_json::from_str(r##"{"_kind":"linear","_coords":[0,0,1,0],"_stops":[[0,"#0f0"]]}"##)
                .unwrap();
        assert!(matches!(canvas_brush(Some(&bad), 1.0, &imgs), Brush::Solid(_)));
        // globalAlpha multiplica el alpha de cada stop del gradiente.
        match canvas_brush(Some(&lin), 0.5, &imgs) {
            Brush::Gradient(g) => {
                let a = g.stops.0[0].color.components[3];
                assert!((a - 0.5).abs() < 0.02, "alpha ~0.5, got {a}");
            }
            _ => panic!("gradiente"),
        }
        // Patrón (createPattern): con la imagen decodificada → Brush::Image;
        // sin imagen en el mapa → degrada a sólido.
        let pat: serde_json::Value =
            serde_json::from_str(r##"{"_pattern":true,"src":"u","rep":"repeat"}"##).unwrap();
        assert!(matches!(canvas_brush(Some(&pat), 1.0, &imgs), Brush::Solid(_)));
        let mut con_img = imgs.clone();
        con_img.insert(
            "u".into(),
            PenikoImage::new(ImageData { data: Blob::from(vec![255u8, 0, 0, 255]), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 1, height: 1 }),
        );
        match canvas_brush(Some(&pat), 0.5, &con_img) {
            Brush::Image(im) => {
                assert!(matches!(im.sampler.x_extend, Extend::Repeat));
                assert!(matches!(im.sampler.y_extend, Extend::Repeat));
                assert!((im.sampler.alpha - 0.5).abs() < 0.001, "alpha ~0.5, got {}", im.sampler.alpha);
            }
            _ => panic!("debería ser patrón de imagen"),
        }
        // repeat-x → Repeat en x, Pad en y.
        let pat_x: serde_json::Value =
            serde_json::from_str(r##"{"_pattern":true,"src":"u","rep":"repeat-x"}"##).unwrap();
        match canvas_brush(Some(&pat_x), 1.0, &con_img) {
            Brush::Image(im) => {
                assert!(matches!(im.sampler.x_extend, Extend::Repeat));
                assert!(matches!(im.sampler.y_extend, Extend::Pad));
            }
            _ => panic!("patrón repeat-x"),
        }
    }

    #[test]
    fn canvas_stroke_dash_cap_join() {
        // setLineDash con patrón impar se duplica; cap/join se mapean.
        let st: serde_json::Value = serde_json::from_str(
            r##"{"lc":"round","lj":"bevel","ld":[5,3,2],"ldo":1.0}"##,
        )
        .unwrap();
        let stroke = canvas_stroke(Some(&st), 2.0);
        assert_eq!(stroke.width, 2.0);
        assert!(matches!(stroke.start_cap, Cap::Round));
        assert!(matches!(stroke.join, Join::Bevel));
        // 3 segmentos impares → duplicados a 6.
        assert_eq!(stroke.dash_pattern.len(), 6);
        assert_eq!(stroke.dash_offset, 1.0);
        // Sin dash declarado → sin patrón.
        let plain: serde_json::Value = serde_json::from_str(r##"{"lw":1}"##).unwrap();
        assert!(canvas_stroke(Some(&plain), 1.0).dash_pattern.is_empty());
    }

    #[test]
    fn paint_canvas_cmds_gradiente_clip_dash_balancea() {
        // Gradiente real + clip dentro de save/restore + stroke punteado:
        // la escena queda no-vacía y los push_layer del clip se balancean
        // (no debe panicar ni dejar layers colgando).
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 50.0 };
        let cmds: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r##"[
                ["save"],
                ["beginPath"],
                ["rect", 0, 0, 50, 50],
                ["clip"],
                ["fillRect", 0, 0, 100, 50,
                    {"_kind":"linear","_coords":[0,0,100,0],"_stops":[[0,"#ff0000"],[1,"#0000ff"]]},
                    {"ga": 1}],
                ["restore"],
                ["beginPath"],
                ["moveTo", 0, 0],
                ["lineTo", 100, 50],
                ["stroke", {"s": "#000000", "lw": 2, "ld": [4, 4], "ldo": 0}]
            ]"##,
        )
        .unwrap();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 50.0, &Default::default());
        assert!(!scene.encoding().is_empty(), "debería haber dibujo");
    }

    #[test]
    fn canvas_gradiente_y_dash_llegan_al_frame_end_to_end() {
        // El JS construye un gradiente + setLineDash y el snapshot debe llevar
        // el objeto CanvasGradient y el array `ld`.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "c".into(),
            tag_name: "canvas".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("width".into(), "100".into()), ("height".into(), "100".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx = document.getElementById('c').getContext('2d');\
             var g = ctx.createLinearGradient(0,0,100,0);\
             g.addColorStop(0,'#ff0000'); g.addColorStop(1,'#0000ff');\
             ctx.fillStyle = g; ctx.fillRect(0,0,100,100);\
             ctx.setLineDash([6,4]); ctx.strokeStyle='#000';\
             ctx.beginPath(); ctx.moveTo(0,0); ctx.lineTo(100,100); ctx.stroke();",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        // El fillRect lleva el objeto gradiente en el arg 5.
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect");
        assert_eq!(fr[5].get("_kind").and_then(|v| v.as_str()), Some("linear"));
        assert_eq!(fr[5].get("_stops").and_then(|v| v.as_array()).map(|a| a.len()), Some(2));
        // El stroke lleva el snapshot con `ld`.
        let stk = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("stroke"))
            .expect("stroke");
        let ld = stk[1].get("ld").and_then(|v| v.as_array()).expect("ld");
        assert_eq!(ld.len(), 2);
    }

    #[test]
    fn drawimage_de_img_dom_se_decodifica_end_to_end() {
        // <canvas> + <img src=data:…> → ctx.drawImage(img) registra el src y
        // refresh_canvas_frames (→ decode_canvas_images) lo decodifica.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let mk = |id: &str, tag: &str, attrs: Vec<(String, String)>| puriy_js::ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: attrs,
            dfs_index: 0,
        };
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            mk("c", "canvas", vec![("width".into(), "100".into()), ("height".into(), "100".into())]),
            mk("i", "img", vec![("src".into(), png_1x1.into())]),
        ])
        .expect("set_elements");
        rt.eval(
            "var ctx = document.getElementById('c').getContext('2d');\
             var im = document.getElementById('i');\
             ctx.drawImage(im, 5, 5);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let di = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("drawImage"))
            .expect("drawImage en el frame");
        assert_eq!(di.get(1).and_then(|v| v.as_str()), Some(png_1x1));
        let img = t.canvas_images.get(png_1x1).expect("decodificada").as_ref();
        assert_eq!(img.map(|i| (i.image.width, i.image.height)), Some((1, 1)));
    }

    #[test]
    fn createpattern_de_img_dom_se_decodifica_end_to_end() {
        // <canvas> + <img> → ctx.createPattern(img,'repeat') usado como
        // fillStyle: el snapshot del fillRect lleva el descriptor {_pattern,src}
        // y decode_canvas_images (vía refresh) decodifica ese src. Fase 7.198.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let mk = |id: &str, tag: &str, attrs: Vec<(String, String)>| puriy_js::ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: attrs,
            dfs_index: 0,
        };
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[
            mk("c", "canvas", vec![("width".into(), "100".into()), ("height".into(), "100".into())]),
            mk("i", "img", vec![("src".into(), png_1x1.into())]),
        ])
        .expect("set_elements");
        rt.eval(
            "var ctx = document.getElementById('c').getContext('2d');\
             var im = document.getElementById('i');\
             var pat = ctx.createPattern(im, 'repeat');\
             ctx.fillStyle = pat;\
             ctx.fillRect(0, 0, 50, 50);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        // El fillRect lleva el descriptor de patrón en el arg 5.
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect en el frame");
        assert_eq!(fr[5].get("_pattern").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(fr[5].get("src").and_then(|v| v.as_str()), Some(png_1x1));
        assert_eq!(fr[5].get("rep").and_then(|v| v.as_str()), Some("repeat"));
        // decode_canvas_images recogió el src del patrón y lo decodificó.
        let img = t.canvas_images.get(png_1x1).expect("decodificada").as_ref();
        assert_eq!(img.map(|i| (i.image.width, i.image.height)), Some((1, 1)));
        // El painter pinta el patrón (escena no-vacía).
        let mut images: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        images.insert(png_1x1.into(), img.unwrap().clone());
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        paint_canvas_cmds(&mut scene, &mut ts, rect, &frame.cmds, 100.0, 100.0, &images);
        assert!(!scene.encoding().is_empty(), "el patrón debería pintar");
    }

    #[test]
    fn background_image_size_position_repeat_pinta_y_tilea() {
        // Fase 7.204 — paint_background_image resuelve size/position/repeat.
        let img = PenikoImage::new(ImageData { data: llimphi_raster::peniko::Blob::from(vec![255u8; 2 * 2 * 4]), format: llimphi_raster::peniko::ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 2, height: 2 });
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let sz = BackgroundSize::Explicit { x: LengthVal::Px(60.0), y: LengthVal::Px(60.0) };
        let pos = BackgroundPosition { x: LengthVal::Px(0.0), y: LengthVal::Px(0.0) };

        // no-repeat con tile 60×60 sobre 100×100 → un solo draw de imagen.
        let mut once = llimphi_raster::vello::Scene::new();
        paint_background_image(&mut once, rect, rect, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::NoRepeat);
        assert!(!once.encoding().is_empty(), "un background-image debería pintar");

        // repeat con el mismo tile → 2×2 = 4 tiles → más draw_tags.
        let mut tiled = llimphi_raster::vello::Scene::new();
        paint_background_image(&mut tiled, rect, rect, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::Repeat);
        assert!(
            tiled.encoding().draw_tags.len() > once.encoding().draw_tags.len(),
            "repeat debería encodar más tiles ({} vs {})",
            tiled.encoding().draw_tags.len(),
            once.encoding().draw_tags.len()
        );

        // rect de ancho 0 → no pinta nada (early-return).
        let mut empty = llimphi_raster::vello::Scene::new();
        let zero = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 0.0, h: 50.0 };
        paint_background_image(
            &mut empty, zero, zero, 0.0, &img, 2.0, 2.0,
            BackgroundSize::Auto,
            BackgroundPosition { x: LengthVal::Pct(0.0), y: LengthVal::Pct(0.0) },
            BackgroundRepeat::Repeat,
        );
        assert!(empty.encoding().is_empty(), "rect de ancho 0 no debería pintar");
    }

    #[test]
    fn background_clip_recorta_a_caja_mas_chica() {
        // Fase 7.207 — `background-clip`: con un clip box más chico que el
        // origin box, el tiling cubre el área de posicionamiento pero el
        // recorte limita el pintado. Verificamos que ambas rutas pintan y que
        // un clip box degenerado (ancho 0) no deja salir nada.
        let img = PenikoImage::new(ImageData { data: llimphi_raster::peniko::Blob::from(vec![255u8; 2 * 2 * 4]), format: llimphi_raster::peniko::ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 2, height: 2 });
        let area = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let sz = BackgroundSize::Explicit { x: LengthVal::Px(20.0), y: LengthVal::Px(20.0) };
        let pos = BackgroundPosition { x: LengthVal::Px(0.0), y: LengthVal::Px(0.0) };

        // clip box = padding-box (inset 10px) → sigue pintando los tiles.
        let clip = llimphi_ui::PaintRect { x: 10.0, y: 10.0, w: 80.0, h: 80.0 };
        let mut s = llimphi_raster::vello::Scene::new();
        paint_background_image(&mut s, area, clip, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::Repeat);
        assert!(!s.encoding().is_empty(), "clip padding-box debería pintar tiles");

        // El origen del tiling es `area` (no `clip`): con un área mayor hay más
        // tiles que recortando el área misma al clip chico.
        let mut s_small_area = llimphi_raster::vello::Scene::new();
        paint_background_image(
            &mut s_small_area, clip, clip, 0.0, &img, 2.0, 2.0, sz, pos, BackgroundRepeat::Repeat,
        );
        assert!(
            s.encoding().draw_tags.len() >= s_small_area.encoding().draw_tags.len(),
            "tilear sobre el origin box (100×100) no debería dar menos tiles que sobre 80×80"
        );
    }

    #[test]
    fn background_clip_text_rellena_glifos_con_gradiente() {
        // Fase 7.208 — el camino real de `background-clip: text`: shaping del
        // texto + draw_layout_brush_xf con un Brush::Gradient. Verifica que
        // pinta (encoding no vacío) y que el gradiente añade más draws que el
        // mismo texto en color sólido.
        use puriy_engine::style::{GradientGeometry, GradientStop, LinearGradient};
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        // Forzamos la DejaVu embebida (registrada en `Typesetter::new`) para
        // que el texto Latin shapee también en el sandbox sin fuentes de
        // sistema; en una máquina real el font-family normal funciona igual.
        let layout = ts.layout(
            "Hola",
            48.0,
            None,
            llimphi_ui::llimphi_text::Alignment::Start,
            1.2,
            false,
            Some("DejaVu Sans"),
            400.0,
            false,
            false,
        );
        let local = llimphi_ui::PaintRect {
            x: 0.0,
            y: 0.0,
            w: (layout.width()).max(1.0),
            h: 60.0,
        };
        let grad = LinearGradient {
            geometry: GradientGeometry::Linear { angle_deg: 90.0 },
            stops: vec![
                GradientStop { color: puriy_engine::Color::rgb(255, 0, 0), pos: None },
                GradientStop { color: puriy_engine::Color::rgb(0, 0, 255), pos: None },
            ],
            repeating: false,
        };
        let brush = llimphi_raster::peniko::Brush::Gradient(
            build_linear_gradient_brush(&grad, local, 1.0).expect("gradiente de 2 stops"),
        );
        let xf = llimphi_raster::kurbo::Affine::translate((10.0, 10.0));
        let mut scene = llimphi_raster::vello::Scene::new();
        llimphi_ui::llimphi_text::draw_layout_brush_xf(&mut scene, &layout, &brush, xf);
        // Los glifos se encodan en `draw_tags` + `glyph_runs` (las siluetas se
        // resuelven después, así que `path_tags`/`is_empty()` no sirven acá).
        assert!(
            !scene.encoding().draw_tags.is_empty(),
            "los glifos con gradiente deberían encodar un draw"
        );
        assert!(
            !scene.encoding().resources.glyph_runs.is_empty(),
            "debería haber al menos un glyph run shapeado (DejaVu)"
        );
    }

    #[test]
    fn object_fit_scale_por_modo() {
        use puriy_engine::ObjectFit;
        // Imagen 100×50 (2:1) en caja 200×200, zoom 1.
        let (iw, ih, rw, rh, z) = (100.0, 50.0, 200.0, 200.0, 1.0);
        // Fill: estira por eje independiente.
        assert_eq!(object_fit_scale(ObjectFit::Fill, rw, rh, iw, ih, z), (2.0, 4.0));
        // Contain: min de las dos (2.0) → cabe sin recortar.
        assert_eq!(object_fit_scale(ObjectFit::Contain, rw, rh, iw, ih, z), (2.0, 2.0));
        // Cover: max de las dos (4.0) → cubre, recorta horizontal.
        assert_eq!(object_fit_scale(ObjectFit::Cover, rw, rh, iw, ih, z), (4.0, 4.0));
        // None: tamaño natural × zoom.
        assert_eq!(object_fit_scale(ObjectFit::None, rw, rh, iw, ih, z), (1.0, 1.0));
        // ScaleDown: min(contain=2, natural=1) = 1 (la imagen es chica → no agranda).
        assert_eq!(object_fit_scale(ObjectFit::ScaleDown, rw, rh, iw, ih, z), (1.0, 1.0));
        // ScaleDown con imagen grande (300×300) en caja 100×100: contain=1/3 < 1 → encoge.
        let (sx, sy) = object_fit_scale(ObjectFit::ScaleDown, 100.0, 100.0, 300.0, 300.0, 1.0);
        assert!((sx - 1.0 / 3.0).abs() < 1e-9 && (sy - 1.0 / 3.0).abs() < 1e-9);
        // Imagen degenerada → escala neutra (no divide por cero).
        assert_eq!(object_fit_scale(ObjectFit::Cover, rw, rh, 0.0, ih, z), (1.0, 1.0));
    }

    #[test]
    fn paint_extra_bg_layers_pinta_imagen_y_gradiente() {
        // Fase 7.206 — las capas extra (debajo de la capa 0) se pintan: una
        // imagen vía paint_background_image y un gradiente lineal vía fill.
        use puriy_engine::style::{GradientGeometry, GradientStop, LinearGradient};
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };

        // Sin capas → no pinta nada.
        let mut none = llimphi_raster::vello::Scene::new();
        paint_extra_bg_layers(&mut none, rect, 0.0, &[], &[], 1.0);
        assert!(none.encoding().is_empty(), "sin capas no debería pintar");

        // Una capa de gradiente → un fill.
        let grad = LinearGradient {
            geometry: GradientGeometry::Linear { angle_deg: 180.0 },
            stops: vec![
                GradientStop { color: puriy_engine::Color::rgb(255, 0, 0), pos: None },
                GradientStop { color: puriy_engine::Color::rgb(0, 0, 255), pos: None },
            ],
            repeating: false,
        };
        let mut g = llimphi_raster::vello::Scene::new();
        paint_extra_bg_layers(&mut g, rect, 0.0, &[PreparedBgLayer::Gradient(grad.clone())], &[None], 1.0);
        assert!(!g.encoding().is_empty(), "una capa de gradiente debería pintar");

        // La misma capa con `background-blend-mode: multiply` abre una
        // push_layer de blend → más draw tags que sin blend. Fase 7.1236.
        let mut gb = llimphi_raster::vello::Scene::new();
        let mult = Some(llimphi_raster::peniko::BlendMode::from(llimphi_raster::peniko::Mix::Multiply));
        paint_extra_bg_layers(&mut gb, rect, 0.0, &[PreparedBgLayer::Gradient(grad.clone())], &[mult], 1.0);
        assert!(
            gb.encoding().draw_tags.len() > g.encoding().draw_tags.len(),
            "el blend debería agregar la capa (push_layer): {} vs {}",
            gb.encoding().draw_tags.len(),
            g.encoding().draw_tags.len()
        );

        // Imagen + gradiente → más draws que el gradiente solo.
        let img = PenikoImage::new(ImageData { data: llimphi_raster::peniko::Blob::from(vec![255u8; 2 * 2 * 4]), format: llimphi_raster::peniko::ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 2, height: 2 });
        let layers = vec![
            PreparedBgLayer::Image {
                img,
                iw: 2.0,
                ih: 2.0,
                size: BackgroundSize::Explicit { x: LengthVal::Px(50.0), y: LengthVal::Px(50.0) },
                position: BackgroundPosition { x: LengthVal::Px(0.0), y: LengthVal::Px(0.0) },
                repeat: BackgroundRepeat::NoRepeat,
            },
            PreparedBgLayer::Gradient(grad),
        ];
        let mut both = llimphi_raster::vello::Scene::new();
        paint_extra_bg_layers(&mut both, rect, 0.0, &layers, &[None, None], 1.0);
        assert!(
            both.encoding().draw_tags.len() > g.encoding().draw_tags.len(),
            "dos capas deberían encodar más draws que una ({} vs {})",
            both.encoding().draw_tags.len(),
            g.encoding().draw_tags.len()
        );
    }

    #[test]
    fn canvas_shadow_lee_estado() {
        // Sin campo `sc` → None.
        let plain: serde_json::Value = serde_json::from_str(r#"{"ga":1.0}"#).unwrap();
        assert!(canvas_shadow(Some(&plain), 1.0).is_none());
        // Color totalmente transparente → None (aunque haya blur/offset).
        let transp: serde_json::Value =
            serde_json::from_str(r#"{"sc":"rgba(0,0,0,0)","sb":5,"sox":2,"soy":2}"#).unwrap();
        assert!(canvas_shadow(Some(&transp), 1.0).is_none());
        // Blur 0 + ambos offsets 0 → inactiva.
        let inactive: serde_json::Value =
            serde_json::from_str(r##"{"sc":"#000","sb":0,"sox":0,"soy":0}"##).unwrap();
        assert!(canvas_shadow(Some(&inactive), 1.0).is_none());
        // Activa: blur 4, offset (3,5); ga 0.5 reduce el alpha del color.
        let active: serde_json::Value =
            serde_json::from_str(r#"{"sc":"rgba(0,0,0,1)","sb":4,"sox":3,"soy":5}"#).unwrap();
        let (col, blur, ox, oy) = canvas_shadow(Some(&active), 0.5).expect("sombra activa");
        assert_eq!((blur, ox, oy), (4.0, 3.0, 5.0));
        assert!((col.components[3] - 0.5).abs() < 0.02, "alpha ~0.5, got {}", col.components[3]);
    }

    #[test]
    fn paint_canvas_cmds_sombra_agrega_draw() {
        // Un fillRect con sombra encoda MÁS draw objects que sin sombra (la
        // sombra blureada es un draw extra vía draw_blurred_rounded_rect).
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let sin: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0}]]"##).unwrap();
        let con: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0,"sc":"rgba(0,0,0,1)","sb":6,"sox":4,"soy":4}]]"##,
        )
        .unwrap();
        let mut s1 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s1, &mut ts, rect, &sin, 100.0, 100.0, &imgs);
        let mut s2 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s2, &mut ts, rect, &con, 100.0, 100.0, &imgs);
        assert!(
            s2.encoding().draw_tags.len() > s1.encoding().draw_tags.len(),
            "la sombra debería agregar un draw object: {} vs {}",
            s2.encoding().draw_tags.len(),
            s1.encoding().draw_tags.len()
        );
    }

    #[test]
    fn sombra_llega_al_frame_end_to_end() {
        // ctx.shadow* + fillRect → el snapshot del fillRect lleva sc/sb/sox/soy
        // y canvas_shadow lo resuelve. Fase 7.199.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "c".into(),
            tag_name: "canvas".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("width".into(), "100".into()), ("height".into(), "100".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx=document.getElementById('c').getContext('2d');\
             ctx.shadowColor='rgba(0,0,0,0.7)'; ctx.shadowBlur=8;\
             ctx.shadowOffsetX=4; ctx.shadowOffsetY=4;\
             ctx.fillStyle='#3366ff'; ctx.fillRect(20,20,40,40);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect");
        assert_eq!(fr[6].get("sc").and_then(|v| v.as_str()), Some("rgba(0,0,0,0.7)"));
        assert_eq!(fr[6].get("sb").and_then(|v| v.as_f64()), Some(8.0));
        assert_eq!(fr[6].get("sox").and_then(|v| v.as_f64()), Some(4.0));
        assert!(canvas_shadow(Some(&fr[6]), 1.0).is_some(), "la sombra debería resolverse");
    }

    #[test]
    fn canvas_composite_mapea_modos() {
        // source-over (default) y desconocidos → None (sin capa de blend).
        let so: serde_json::Value = serde_json::from_str(r#"{"gco":"source-over"}"#).unwrap();
        assert!(canvas_composite(Some(&so)).is_none());
        let raro: serde_json::Value = serde_json::from_str(r#"{"gco":"qwerty"}"#).unwrap();
        assert!(canvas_composite(Some(&raro)).is_none());
        assert!(canvas_composite(Some(&serde_json::json!({"ga": 1.0}))).is_none());
        // Modo de mezcla → Mix (compose SrcOver).
        use llimphi_raster::peniko::{Compose, Mix};
        let mul: serde_json::Value = serde_json::from_str(r#"{"gco":"multiply"}"#).unwrap();
        let bm = canvas_composite(Some(&mul)).expect("multiply mapea");
        assert_eq!((bm.mix, bm.compose), (Mix::Multiply, Compose::SrcOver));
        // Porter-Duff → Compose (mix Normal).
        let lighter: serde_json::Value = serde_json::from_str(r#"{"gco":"lighter"}"#).unwrap();
        let bm = canvas_composite(Some(&lighter)).expect("lighter mapea");
        assert_eq!((bm.mix, bm.compose), (Mix::Normal, Compose::Plus));
        let dout: serde_json::Value =
            serde_json::from_str(r#"{"gco":"destination-out"}"#).unwrap();
        assert_eq!(canvas_composite(Some(&dout)).unwrap().compose, Compose::DestOut);
    }

    #[test]
    fn paint_canvas_cmds_composite_agrega_layer() {
        // Un fillRect con globalCompositeOperation != source-over encoda MÁS
        // draw objects (el push_layer/pop_layer de blend agrega tags de clip).
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let sin: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0}]]"##).unwrap();
        let con: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r##"[["fillRect",10,10,40,40,"#ff0000",{"ga":1.0,"gco":"lighter"}]]"##,
        )
        .unwrap();
        let mut s1 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s1, &mut ts, rect, &sin, 100.0, 100.0, &imgs);
        let mut s2 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s2, &mut ts, rect, &con, 100.0, 100.0, &imgs);
        assert!(
            s2.encoding().draw_tags.len() > s1.encoding().draw_tags.len(),
            "la capa de blend debería agregar draw objects: {} vs {}",
            s2.encoding().draw_tags.len(),
            s1.encoding().draw_tags.len()
        );
    }

    #[test]
    fn gco_llega_al_frame_end_to_end() {
        // ctx.globalCompositeOperation + fillRect → el snapshot lleva `gco` y
        // canvas_composite lo resuelve. Fase 7.200.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="100" height="100"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "c".into(),
            tag_name: "canvas".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("width".into(), "100".into()), ("height".into(), "100".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx=document.getElementById('c').getContext('2d');\
             ctx.globalCompositeOperation='multiply';\
             ctx.fillStyle='#3366ff'; ctx.fillRect(20,20,40,40);",
        )
        .expect("draw");
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let fr = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("fillRect"))
            .expect("fillRect");
        assert_eq!(fr[6].get("gco").and_then(|v| v.as_str()), Some("multiply"));
        assert!(canvas_composite(Some(&fr[6])).is_some(), "el composite debería resolverse");
    }

    #[test]
    fn paint_canvas_cmds_drawimage_dibuja() {
        // Una imagen 2×2 en el mapa + un drawImage que la coloca → la escena
        // queda no-vacía. Cubre las 3 aridades (2/4/8 números).
        let mut scene = llimphi_raster::vello::Scene::new();
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let img = PenikoImage::new(ImageData { data: Blob::from(vec![255u8; 16]), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 2, height: 2 });
        let mut images = std::collections::HashMap::new();
        images.insert("u".to_string(), img);
        for cmds_src in [
            r#"[["drawImage","u",10,10]]"#,                 // 3-arg
            r#"[["drawImage","u",10,10,40,40]]"#,           // 5-arg
            r#"[["drawImage","u",0,0,2,2,10,10,40,40]]"#,   // 9-arg (sub-rect)
        ] {
            let mut s = llimphi_raster::vello::Scene::new();
            let cmds: Vec<Vec<serde_json::Value>> = serde_json::from_str(cmds_src).unwrap();
            paint_canvas_cmds(&mut s, &mut ts, rect, &cmds, 100.0, 100.0, &images);
            assert!(!s.encoding().is_empty(), "drawImage debería pintar: {cmds_src}");
        }
        // Un src ausente del mapa → no-op (no panic, escena vacía).
        let cmds: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["drawImage","falta",0,0]]"#).unwrap();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 100.0, &images);
        assert!(scene.encoding().is_empty(), "src ausente no pinta");
    }

    #[test]
    fn drawimage_con_snapshot_aplica_composite_y_alpha() {
        // Fase 7.201 — un drawImage con snapshot de composite/alpha sigue
        // dibujando (las coords se parsean con filter_map, descartando el
        // snapshot del final) y la capa de blend agrega draw objects.
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let img = PenikoImage::new(ImageData { data: Blob::from(vec![255u8; 16]), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 2, height: 2 });
        let mut images = std::collections::HashMap::new();
        images.insert("u".to_string(), img);
        // Sin snapshot (compat hacia atrás): dibuja.
        let plano: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["drawImage","u",10,10,40,40,{"ga":1.0}]]"#).unwrap();
        let mut s_plano = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s_plano, &mut ts, rect, &plano, 100.0, 100.0, &images);
        assert!(!s_plano.encoding().is_empty(), "drawImage con snapshot debería pintar");
        // Con composite 'lighter' → capa de blend extra.
        let comp: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["drawImage","u",10,10,40,40,{"ga":1.0,"gco":"lighter"}]]"#)
                .unwrap();
        let mut s_comp = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s_comp, &mut ts, rect, &comp, 100.0, 100.0, &images);
        assert!(
            s_comp.encoding().draw_tags.len() > s_plano.encoding().draw_tags.len(),
            "el composite debería agregar draw objects: {} vs {}",
            s_comp.encoding().draw_tags.len(),
            s_plano.encoding().draw_tags.len()
        );
        // Las coords (8 números, sub-rect) + snapshot siguen mapeando bien.
        let sub: Vec<Vec<serde_json::Value>> = serde_json::from_str(
            r#"[["drawImage","u",0,0,2,2,10,10,40,40,{"ga":0.5,"sc":"rgba(0,0,0,1)","sb":6,"sox":3,"soy":3}]]"#,
        )
        .unwrap();
        let mut s_sub = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s_sub, &mut ts, rect, &sub, 100.0, 100.0, &images);
        assert!(!s_sub.encoding().is_empty(), "sub-rect con alpha+sombra debería pintar");
    }

    #[test]
    fn drawimage_a_getimagedata_pipeline_end_to_end() {
        // Fase 7.203 — flujo COMPLETO por run_scripts_on_tab: el chrome inyecta
        // los píxeles del <img> antes del script, así un drawImage+getImageData
        // (pipeline de filtros) lee la imagen real. El PNG 1×1 es rojo opaco.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut t = TabState::new("about:test".into());
        t.url = "about:test".into();
        t.has_canvas = true;
        t.box_tree = Some(parse(&format!(
            r#"<body><canvas id="c" width="4" height="4"></canvas><img id="i" src="{png_1x1}"></body>"#
        )));
        let scripts = vec![puriy_engine::ScriptInfo {
            src: None,
            inline: Some(
                "var ctx=document.getElementById('c').getContext('2d');\
                 var im=document.getElementById('i');\
                 ctx.drawImage(im,0,0);\
                 var g=ctx.getImageData(0,0,1,1);\
                 globalThis.__r = g.data[0]+','+g.data[1]+','+g.data[2]+','+g.data[3];"
                    .into(),
            ),
            type_attr: None,
            is_module: false,
            defer: false,
            async_: false,
        }];
        run_scripts_on_tab(&mut t, &scripts, 0, None);
        assert_eq!(t.js_summary.errors, 0, "el script no debería errar");
        let r = t.js.as_mut().unwrap().eval("__r").expect("r");
        // rojo opaco leído del framebuffer JS tras drawImage.
        assert_eq!(r, puriy_js::JsValue::String("255,0,0,255".into()));
    }

    #[test]
    fn collect_dom_image_pixels_decodifica_imgs() {
        // Fase 7.203 — el chrome recolecta los píxeles de los <img> de la
        // página (cuando hay canvas) para inyectarlos al runtime.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.has_canvas = true;
        t.box_tree = Some(parse(&format!(
            r#"<body><canvas id="c" width="10" height="10"></canvas><img id="i" src="{png_1x1}"></body>"#
        )));
        let px = collect_dom_image_pixels(t);
        assert_eq!(px.len(), 1, "debería recolectar 1 img");
        assert_eq!(px[0].0, png_1x1);
        assert_eq!((px[0].1, px[0].2), (1, 1));
        assert_eq!(px[0].3.len(), 4, "rgba de 1×1 = 4 bytes");
        // Sin canvas → vacío (gate de costo).
        t.has_canvas = false;
        assert!(collect_dom_image_pixels(t).is_empty());
    }

    #[test]
    fn paint_canvas_cmds_putimagedata_dibuja() {
        // Fase 7.202 — un comando putImageData con base64 RGBA válido pinta.
        // "/wAA/w==" = 1 pixel rojo opaco (FF 00 00 FF).
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let cmds: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["putImageData",3,4,1,1,"/wAA/w=="]]"#).unwrap();
        let mut scene = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &cmds, 100.0, 100.0, &imgs);
        assert!(!scene.encoding().is_empty(), "putImageData debería pintar");
        // base64 inválido / dims en cero → no-op (no panic).
        let mala: Vec<Vec<serde_json::Value>> =
            serde_json::from_str(r#"[["putImageData",0,0,0,0,"@@@"]]"#).unwrap();
        let mut s2 = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut s2, &mut ts, rect, &mala, 100.0, 100.0, &imgs);
        assert!(s2.encoding().is_empty(), "putImageData inválido no pinta");
    }

    #[test]
    fn putimagedata_llega_al_frame_end_to_end() {
        // ctx.putImageData por el runtime JS real → el frame lleva el comando
        // con dx/dy/w/h/base64, y el painter lo dibuja. Fase 7.202.
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        t.box_tree = Some(parse(
            r#"<body><canvas id="c" width="20" height="20"></canvas></body>"#,
        ));
        t.has_canvas = true;
        let rt = t.js.as_mut().expect("rt");
        rt.set_elements(&[puriy_js::ElementSnapshot {
            id: "c".into(),
            tag_name: "canvas".into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: vec![("width".into(), "20".into()), ("height".into(), "20".into())],
            dfs_index: 0,
        }])
        .expect("set_elements");
        rt.eval(
            "var ctx=document.getElementById('c').getContext('2d');\
             var id=ctx.createImageData(2,2);\
             for(var i=0;i<id.data.length;i+=4){id.data[i]=10;id.data[i+1]=20;id.data[i+2]=30;id.data[i+3]=255;}\
             ctx.putImageData(id,1,1);\
             var back=ctx.getImageData(1,1,1,1);",
        )
        .expect("draw");
        // getImageData round-trip dentro del runtime.
        assert_eq!(rt.eval("back.data[0]").expect("e"), puriy_js::JsValue::Number(10.0));
        assert_eq!(rt.eval("back.data[2]").expect("e"), puriy_js::JsValue::Number(30.0));
        apply_dom_mutations(t);
        let frame = t.canvas_frames.get("c").expect("frame");
        let put = frame
            .cmds
            .iter()
            .find(|c| c.first().and_then(|v| v.as_str()) == Some("putImageData"))
            .expect("putImageData");
        assert_eq!(put.get(3).and_then(|v| v.as_u64()), Some(2)); // w
        assert_eq!(put.get(4).and_then(|v| v.as_u64()), Some(2)); // h
        assert!(put.get(5).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()), "base64 presente");
        // El painter lo dibuja.
        let mut ts = llimphi_ui::llimphi_text::Typesetter::new();
        let rect = llimphi_ui::PaintRect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        let imgs: std::collections::HashMap<String, PenikoImage> =
            std::collections::HashMap::new();
        let mut scene = llimphi_raster::vello::Scene::new();
        paint_canvas_cmds(&mut scene, &mut ts, rect, &frame.cmds, 20.0, 20.0, &imgs);
        assert!(!scene.encoding().is_empty(), "el frame con putImageData debería pintar");
    }

    #[test]
    fn decode_canvas_images_resuelve_data_url() {
        // decode_canvas_images decodifica el src de un drawImage (data: PNG 1×1)
        // y lo deja en t.canvas_images.
        let png_1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
        let mut m = model_con_script("/* boot */");
        let t = &mut m.tabs[0];
        t.url = "about:test".into();
        let cmds_json = format!(r#"[["drawImage","{png_1x1}",0,0]]"#);
        t.canvas_frames.insert(
            "c".into(),
            CanvasFrame {
                id: "c".into(),
                width: 100.0,
                height: 100.0,
                cmds: serde_json::from_str(&cmds_json).unwrap(),
            },
        );
        decode_canvas_images(t);
        let got = t.canvas_images.get(png_1x1).expect("entrada decodificada");
        let img = got.as_ref().expect("la imagen 1×1 decodifica");
        assert_eq!((img.image.width, img.image.height), (1, 1));
        // Segunda llamada no re-decodifica (idempotente: la clave ya existe).
        decode_canvas_images(t);
        assert_eq!(t.canvas_images.len(), 1);
    }

    #[test]
    fn checkbox_glyph_color_aplica_accent_solo_marcado_fase_7_1238() {
        // `accent-color` tinta el glifo MARCADO (☑ / ●) de checkbox/radio.
        let neutral = llimphi_raster::peniko::Color::from_rgb8(40, 40, 50);
        let accent = puriy_engine::Color::rgb(0x11, 0x22, 0x33);
        let accent_painted = llimphi_raster::peniko::Color::from_rgba8(0x11, 0x22, 0x33, 0xff);

        // Marcado + accent seteado → el accent.
        assert_eq!(checkbox_glyph_color(Some(accent), true), accent_painted);
        // Marcado pero accent `auto` (None) → gris neutro.
        assert_eq!(checkbox_glyph_color(None, true), neutral);
        // Desmarcado, aunque haya accent → gris neutro (sólo el fill se tinta).
        assert_eq!(checkbox_glyph_color(Some(accent), false), neutral);
        // Desmarcado + auto → neutro.
        assert_eq!(checkbox_glyph_color(None, false), neutral);
        // El alpha del accent se respeta (rgba con a<255).
        let translucido = puriy_engine::Color { r: 10, g: 20, b: 30, a: 128 };
        assert_eq!(
            checkbox_glyph_color(Some(translucido), true),
            llimphi_raster::peniko::Color::from_rgba8(10, 20, 30, 128)
        );
    }

    #[test]
    fn image_rendering_mapea_calidad_de_muestreo_fase_7_1239() {
        // `image-rendering` fija la calidad de muestreo de la ImageBrush.
        use llimphi_raster::peniko::ImageQuality;
        use puriy_engine::ImageRendering as IR;
        // Mapeo CSS → peniko: auto deja el default (None), pixelated/crisp-edges
        // → Low (nearest), smooth → High (bilineal).
        assert_eq!(image_quality_for(IR::Auto), None);
        assert_eq!(image_quality_for(IR::Smooth), Some(ImageQuality::High));
        assert_eq!(image_quality_for(IR::CrispEdges), Some(ImageQuality::Low));
        assert_eq!(image_quality_for(IR::Pixelated), Some(ImageQuality::Low));
        // with_image_rendering fija sampler.quality; `auto` conserva el default
        // Medium de la brush.
        let mk = || {
            PenikoImage::new(ImageData {
                data: Blob::from(vec![255u8; 4]),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: 1,
                height: 1,
            })
        };
        assert_eq!(with_image_rendering(mk(), IR::Auto).sampler.quality, ImageQuality::Medium);
        assert_eq!(with_image_rendering(mk(), IR::Pixelated).sampler.quality, ImageQuality::Low);
        assert_eq!(with_image_rendering(mk(), IR::CrispEdges).sampler.quality, ImageQuality::Low);
        assert_eq!(with_image_rendering(mk(), IR::Smooth).sampler.quality, ImageQuality::High);
    }