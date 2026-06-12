#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;
#[allow(unused_imports)] use std::collections::HashMap;
#[allow(unused_imports)] use std::path::{Path, PathBuf};
#[allow(unused_imports)] use llimphi_ui::llimphi_raster::peniko::{
        Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
    };
#[allow(unused_imports)] use llimphi_ui::{KeyState, Modifiers, PaintRect};
#[allow(unused_imports)] use tullpu_core::{
        Frescura, Historial, Lienzo, ModoFusion, OpLocal, OrigenCapa,
    };
#[allow(unused_imports)] use tullpu_render::{AlmacenEnMemoria, FormatoExport, FuenteBuffers};
#[allow(unused_imports)] use tullpu_paint::{cobertura_pincel, mezclar_src_over};
#[allow(unused_imports)] use uuid::Uuid;



    #[test]
    fn expandir_rect_contrae_y_colapsa_a_none() {
        let r = RectImagen { x0: 2, y0: 2, x1: 4, y1: 4 };
        // −1 por lado → (3,3)..(3,3) = área cero → None.
        assert!(expandir_rect(r, -1, 8, 8).is_none());
        // Un rect más grande contrae sin colapsar.
        let g = RectImagen { x0: 1, y0: 1, x1: 7, y1: 7 };
        let c = expandir_rect(g, -2, 8, 8).unwrap();
        assert_eq!(c, RectImagen { x0: 3, y0: 3, x1: 5, y1: 5 });
    }

    #[test]
    fn seleccionar_todo_cubre_el_lienzo_y_limpia_drags() {
        let mut model = modelo_minimo();
        model.seleccion_drag = Some(SeleccionDrag {
            ancla_ix: 0,
            ancla_iy: 0,
            cur_lx: 0.0,
            cur_ly: 0.0,
            rw: 4.0,
            rh: 4.0,
        });
        model = <Tullpu as App>::update(
            model,
            Msg::SeleccionarTodo,
            &Handle::for_test(),
        );
        assert_eq!(
            model.seleccion,
            Some(RectImagen { x0: 0, y0: 0, x1: 4, y1: 4 })
        );
        assert!(model.seleccion_drag.is_none());
    }

    #[test]
    fn hotkey_ctrl_a_emite_seleccionar_todo() {
        let m = modelo_minimo();
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("a", ctrl)),
            Some(Msg::SeleccionarTodo)
        ));
    }

    #[test]
    fn expandir_seleccion_dispatcha_y_no_toca_historial() {
        let mut model = modelo_minimo();
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::ExpandirSeleccion(1),
            &Handle::for_test(),
        );
        // La selección no vive en el DAG → el historial no cambia.
        assert_eq!(model.hist.len(), hist_antes);
        assert_eq!(
            model.seleccion,
            Some(RectImagen { x0: 0, y0: 0, x1: 4, y1: 4 })
        );
        // Contraer más de la cuenta colapsa a None.
        model = <Tullpu as App>::update(
            model,
            Msg::ExpandirSeleccion(-3),
            &Handle::for_test(),
        );
        assert!(model.seleccion.is_none());
        assert!(model.estado.contains("colapsada"));
    }

    #[test]
    fn expandir_seleccion_sin_seleccion_es_no_op() {
        let mut model = modelo_minimo();
        model.seleccion = None;
        model = <Tullpu as App>::update(
            model,
            Msg::ExpandirSeleccion(1),
            &Handle::for_test(),
        );
        assert!(model.seleccion.is_none());
        assert!(model.estado.contains("no hay selección"));
    }

    #[test]
    fn drag_to_move_sub_pixel_no_mueve_hasta_acumular_un_pixel() {
        let (mut model, id) = modelo_bloque_4x4();
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let hash0 = model.lienzo.capa(id).unwrap().contenido;
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion { lx: 1.0, ly: 1.0, rw: 4.0, rh: 4.0 },
            &Handle::for_test(),
        );
        // Medio píxel (0.4 < 0.5 redondea a 0) no debe mover nada.
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarSeleccion { dx: 0.4, dy: 0.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, hash0);
        // Acumular hasta pasar 0.5 sí mueve.
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarSeleccion { dx: 0.4, dy: 0.0 },
            &Handle::for_test(),
        );
        assert_ne!(model.lienzo.capa(id).unwrap().contenido, hash0);
    }

    #[test]
    fn confirmar_renombrar_con_nuevo_nombre_si_genera_snapshot() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let len_inicial = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("renombrado");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(model.hist.len(), len_inicial + 1);
        // Undo restaura el nombre original ("c").
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().nombre, "c");
    }

    #[test]
    fn flood_fill_rellena_la_region_contigua_y_respeta_el_borde_de_color() {
        let a = [255, 0, 0, 255];
        let b = [0, 0, 255, 255];
        let c = [0, 255, 0, 255];
        let buf = buffer_mitades(4, 4, a, b);
        // Semilla en la mitad izquierda (roja) → se vuelve verde; la
        // derecha (azul) queda intacta.
        let out = flood_fill(&buf, 4, 4, 0, 0, c, 0, None).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert_eq!(pix(0, 0), c);
        assert_eq!(pix(1, 3), c);
        // Mitad derecha sin tocar.
        assert_eq!(pix(2, 0), b);
        assert_eq!(pix(3, 3), b);
    }

    #[test]
    fn flood_fill_es_4_conexo_no_cruza_diagonal() {
        // Tablero donde (0,0) y (1,1) son rojos pero conectados sólo en
        // diagonal (vecinos ortogonales azules) → fill desde (0,0) no
        // alcanza (1,1).
        let r = [255, 0, 0, 255];
        let z = [0, 0, 255, 255];
        let c = [0, 255, 0, 255];
        // 2×2: (0,0)=r (1,0)=z (0,1)=z (1,1)=r
        let buf = vec![
            r[0], r[1], r[2], r[3], z[0], z[1], z[2], z[3], z[0], z[1], z[2],
            z[3], r[0], r[1], r[2], r[3],
        ];
        let out = flood_fill(&buf, 2, 2, 0, 0, c, 0, None).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 2 + x) * 4;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert_eq!(pix(0, 0), c); // semilla pintada
        assert_eq!(pix(1, 1), r); // diagonal NO alcanzada
    }

    #[test]
    fn flood_fill_confinado_a_bounds() {
        let a = [10, 10, 10, 255];
        let c = [99, 99, 99, 255];
        let buf = vec![a; 4 * 4].concat();
        // Sin bounds llenaría todo; con bounds (0,0,2,2) sólo el cuadrante.
        let out =
            flood_fill(&buf, 4, 4, 0, 0, c, 0, Some((0, 0, 2, 2))).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert_eq!(pix(0, 0), c);
        assert_eq!(pix(1, 1), c);
        // Fuera del rect, intacto.
        assert_eq!(pix(2, 2), a);
        assert_eq!(pix(0, 2), a);
    }

    #[test]
    fn flood_fill_semilla_fuera_de_bounds_es_none() {
        let buf = vec![[1u8, 2, 3, 4]; 4 * 4].concat();
        assert!(
            flood_fill(&buf, 4, 4, 3, 3, [9, 9, 9, 9], 0, Some((0, 0, 2, 2)))
                .is_none()
        );
    }

    #[test]
    fn flood_fill_mismo_color_es_none() {
        let c = [50, 60, 70, 255];
        let buf = vec![c; 4 * 4].concat();
        assert!(flood_fill(&buf, 4, 4, 0, 0, c, 0, None).is_none());
    }

    #[test]
    fn flood_fill_tolerancia_agrupa_colores_cercanos() {
        // Un píxel casi-igual (±4 por canal = 8 de suma) al resto.
        let base = [100, 100, 100, 255];
        let casi = [104, 100, 100, 255]; // suma |Δ| = 4
        let lejos = [200, 100, 100, 255];
        let c = [0, 0, 0, 255];
        // fila 1×3: base, casi, lejos
        let buf = vec![
            base[0], base[1], base[2], base[3], casi[0], casi[1], casi[2],
            casi[3], lejos[0], lejos[1], lejos[2], lejos[3],
        ];
        // tol=8 agrupa base+casi pero no lejos.
        let out = flood_fill(&buf, 3, 1, 0, 0, c, 8, None).unwrap();
        assert_eq!(out[0..4], c);
        assert_eq!(out[4..8], c);
        assert_eq!(out[8..12], lejos);
    }

    #[test]
    fn rellenar_flood_en_capa_pinta_raster_y_rechaza_derivada() {
        let mut model = modelo_minimo();
        let buf = buffer_mitades(4, 4, [255, 0, 0, 255], [0, 0, 255, 255]);
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        model.color_picked = Some([0, 255, 0, 255]);
        aplicar_y_recomponer(&mut model);
        // Flood desde (0,0): pinta la mitad izquierda.
        let ok = rellenar_flood_en_capa(&mut model, 0, 0);
        assert!(ok);
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        assert_eq!(&bp[0..4], &[0, 255, 0, 255]);

        // Derivada: rechazo.
        let capa = model.lienzo.capa_mut(id).unwrap();
        capa.origen = OrigenCapa::Derivada {
            madre: Uuid::new_v4(),
            op: TransformacionPixel::Local(OpLocal::Invertir),
            estado: Frescura::Fresca,
        };
        let ok2 = rellenar_flood_en_capa(&mut model, 2, 2);
        assert!(!ok2);
        assert!(model.estado.contains("derivada"));
    }

    #[test]
    fn hotkey_g_emite_cambiar_a_balde() {
        let m = modelo_minimo();
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("g", Modifiers::default())),
            Some(Msg::CambiarHerramienta(Herramienta::Balde))
        ));
    }

    // ---- Fase 45: pincel a mano alzada ----------------------------------

    #[test]
    fn estampar_disco_pinta_circulo_y_respeta_radio() {
        // Lienzo 5×5 transparente; disco radio 1 en el centro (2,2) →
        // cruz de 5 píxeles (centro + 4 ortogonales), esquinas no.
        let mut buf = vec![0u8; 5 * 5 * 4];
        let c = [255, 0, 0, 255];
        estampar_disco(&mut buf, 5, 5, 2, 2, 1, c, false, 1.0, None);
        let pix = |x: usize, y: usize| {
            let i = (y * 5 + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        assert_eq!(pix(2, 2), c); // centro
        assert_eq!(pix(1, 2), c); // izquierda
        assert_eq!(pix(3, 2), c); // derecha
        assert_eq!(pix(2, 1), c); // arriba
        assert_eq!(pix(2, 3), c); // abajo
        // Esquina del bounding box (dx=1,dy=1 → 2 > r²=1) NO pintada.
        assert_eq!(pix(1, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn estampar_disco_recorta_al_canvas_y_a_bounds() {
        let mut buf = vec![0u8; 4 * 4 * 4];
        let c = [9, 9, 9, 255];
        // Centro en la esquina (0,0), radio 2 → sólo entra el cuadrante
        // dentro del canvas.
        estampar_disco(&mut buf, 4, 4, 0, 0, 2, c, false, 1.0, None);
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        assert_eq!(pix(0, 0), c);
        assert_eq!(pix(1, 0), c);
        assert_eq!(pix(0, 1), c);
        // Con bounds (0,0,1,1) sólo el píxel (0,0) puede pintarse.
        let mut buf2 = vec![0u8; 4 * 4 * 4];
        estampar_disco(&mut buf2, 4, 4, 0, 0, 2, c, false, 1.0, Some((0, 0, 1, 1)));
        let pix2 = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [buf2[i], buf2[i + 1], buf2[i + 2], buf2[i + 3]]
        };
        assert_eq!(pix2(0, 0), c);
        assert_eq!(pix2(1, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn trazar_linea_pincel_es_continua_sin_huecos() {
        // Línea horizontal de (0,2) a (7,2) con radio 0 (un píxel por
        // paso) sobre lienzo 8×5 → toda la fila 2 pintada.
        let mut buf = vec![0u8; 8 * 5 * 4];
        let c = [1, 2, 3, 255];
        trazar_linea_pincel(&mut buf, 8, 5, 0, 2, 7, 2, 0, c, false, 1.0, None);
        for x in 0..8usize {
            let i = (2 * 8 + x) * 4;
            assert_eq!(&buf[i..i + 4], &c, "hueco en x={}", x);
        }
        // Fila vecina intacta.
        let i = (1 * 8 + 3) * 4;
        assert_eq!(&buf[i..i + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn pincel_punto_en_capa_pinta_raster_y_rechaza_derivada() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        model.color_picked = Some([255, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        // Disco radio 0 en (1,1).
        let ok = pincel_punto_en_capa(&mut model, 1, 1, 0, false, 1.0, Simetria::Ninguna);
        assert!(ok);
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let i = (1 * 4 + 1) * 4;
        assert_eq!(&bp[i..i + 4], &[255, 0, 0, 255]);

        // Derivada: rechazo.
        let capa = model.lienzo.capa_mut(id).unwrap();
        capa.origen = OrigenCapa::Derivada {
            madre: Uuid::new_v4(),
            op: TransformacionPixel::Local(OpLocal::Invertir),
            estado: Frescura::Fresca,
        };
        assert!(!pincel_punto_en_capa(&mut model, 2, 2, 0, false, 1.0, Simetria::Ninguna));
        assert!(model.estado.contains("derivada"));
    }

    #[test]
    fn trazo_completo_coalesce_a_un_undo_y_finalizar_corta_la_cadena() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 8 * 8 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(8, 8);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.color_picked = Some([0, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        model.hist.reiniciar(model.lienzo.clone());
        // Trazo: press en (1,1) + 3 moves → debe coalescer a 1 entrada.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 1.0, ly: 1.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        for k in 1..=3 {
            let _ = k;
            model = <Tullpu as App>::update(
                model,
                Msg::ContinuarTrazo { dx: 1.0, dy: 0.0 },
                &Handle::for_test(),
            );
        }
        assert_eq!(model.hist.len(), 2); // inicial + 1 trazo coalescido
        model = <Tullpu as App>::update(
            model,
            Msg::FinalizarTrazo,
            &Handle::for_test(),
        );
        assert!(model.pincel_drag.is_none());
        assert!(model.hist.ultima_etiqueta().is_none());
        // Un segundo trazo arranca entrada NUEVA (la cadena se cortó).
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 5.0, ly: 5.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.hist.len(), 3);
        // Un Undo deshace sólo el segundo trazo.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.hist.cursor(), 1);
    }

    #[test]
    fn hotkey_p_emite_cambiar_a_pincel() {
        let m = modelo_minimo();
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("p", Modifiers::default())),
            Some(Msg::CambiarHerramienta(Herramienta::Pincel))
        ));
    }

    // ---- Fase 46: pincel pro (radio ajustable + borrador + alpha) -------

    #[test]
    fn mezclar_src_over_opaco_pisa_transparente_no_toca_semi_compone() {
        // Opaco pisa.
        let mut d = [10, 20, 30, 255];
        mezclar_src_over(&mut d, [200, 100, 50, 255]);
        assert_eq!(d, [200, 100, 50, 255]);
        // Transparente no toca.
        let mut d = [10, 20, 30, 255];
        mezclar_src_over(&mut d, [9, 9, 9, 0]);
        assert_eq!(d, [10, 20, 30, 255]);
        // Blanco 50% sobre negro opaco → gris medio, alfa 255.
        let mut d = [0, 0, 0, 255];
        mezclar_src_over(&mut d, [255, 255, 255, 128]);
        assert_eq!(d[3], 255);
        assert!((d[0] as i32 - 128).abs() <= 2, "got {}", d[0]);
    }

    #[test]
    fn estampar_disco_borrar_pone_alfa_cero() {
        let mut buf = vec![255u8; 5 * 5 * 4]; // todo opaco blanco
        estampar_disco(&mut buf, 5, 5, 2, 2, 1, [0, 0, 0, 0], true, 1.0, None);
        let pix = |x: usize, y: usize| {
            let i = (y * 5 + x) * 4;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        // Centro borrado: alfa 0 (la goma sólo baja el alfa, el RGB
        // queda — es irrelevante bajo transparencia total).
        assert_eq!(pix(2, 2)[3], 0);
        assert_eq!(pix(1, 2)[3], 0);
        // Esquina fuera del disco intacta.
        assert_eq!(pix(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn estampar_disco_color_semitransparente_compone() {
        // Disco radio 0 (sólo el centro) con color 50% sobre fondo negro.
        let mut buf = vec![0u8, 0, 0, 255]; // 1 píxel negro opaco
        // Ajustar buffer a 1×1.
        estampar_disco(&mut buf, 1, 1, 0, 0, 0, [255, 255, 255, 128], false, 1.0, None);
        assert_eq!(buf[3], 255);
        assert!((buf[0] as i32 - 128).abs() <= 2);
    }

    #[test]
    fn bump_radio_pincel_clampa() {
        let mut model = modelo_minimo();
        model.radio_pincel = 0;
        model = <Tullpu as App>::update(
            model,
            Msg::BumpRadioPincel(-5),
            &Handle::for_test(),
        );
        assert_eq!(model.radio_pincel, 0); // no baja de 0
        model = <Tullpu as App>::update(
            model,
            Msg::BumpRadioPincel(100),
            &Handle::for_test(),
        );
        assert_eq!(model.radio_pincel, RADIO_PINCEL_MAX); // tope
    }

    #[test]
    fn hotkey_e_emite_borrador() {
        let m = modelo_minimo();
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("e", Modifiers::default())),
            Some(Msg::CambiarHerramienta(Herramienta::Borrador))
        ));
    }

    #[test]
    fn hotkey_brackets_contextual_radio_vs_opacidad() {
        let mut m = modelo_minimo();
        // Con herramienta Mover (no trazo) → opacidad.
        m.herramienta = Herramienta::Mover;
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("]", Modifiers::default())),
            Some(Msg::BumpOpacidad(_, _))
        ));
        // Con Pincel → radio.
        m.herramienta = Herramienta::Pincel;
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("]", Modifiers::default())),
            Some(Msg::BumpRadioPincel(1))
        ));
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("[", Modifiers::default())),
            Some(Msg::BumpRadioPincel(-1))
        ));
        // Borrador también es de trazo.
        m.herramienta = Herramienta::Borrador;
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("]", Modifiers::default())),
            Some(Msg::BumpRadioPincel(1))
        ));
    }

    #[test]
    fn borrador_via_trazo_borra_pixeles() {
        let mut model = modelo_minimo();
        let buf = vec![255u8; 4 * 4 * 4]; // todo opaco
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(4, 4);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.herramienta = Herramienta::Borrador;
        model.radio_pincel = 0;
        aplicar_y_recomponer(&mut model);
        // Trazo de borrador en (1,1).
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 1.0, ly: 1.0, rw: 4.0, rh: 4.0 },
            &Handle::for_test(),
        );
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let i = (1 * 4 + 1) * 4;
        assert_eq!(bp[i + 3], 0); // borrado: alfa 0
        // Vecino sin tocar.
        assert_eq!(&bp[0..4], &[255, 255, 255, 255]);
    }

    // ---- Fase 47: dureza / suavidad del pincel --------------------------

    #[test]
    fn cobertura_pincel_dura_es_binaria_y_suave_degrada() {
        // Dureza 1.0: 1.0 dentro del radio, 0 fuera.
        assert_eq!(cobertura_pincel(0.0, 4.0, 1.0), 1.0);
        assert_eq!(cobertura_pincel(4.0, 4.0, 1.0), 1.0);
        assert_eq!(cobertura_pincel(4.1, 4.0, 1.0), 0.0);
        // Radio 0 (1 px) → siempre 1.0 dentro.
        assert_eq!(cobertura_pincel(0.0, 0.0, 0.5), 1.0);
        // Dureza 0.0 sobre r=4: núcleo en d=0, cae lineal a 0 en d=4.
        assert_eq!(cobertura_pincel(0.0, 4.0, 0.0), 1.0);
        assert!((cobertura_pincel(2.0, 4.0, 0.0) - 0.5).abs() < 1e-6);
        assert_eq!(cobertura_pincel(4.0, 4.0, 0.0), 0.0);
        // Dureza 0.5: núcleo hasta d=2, luego cae a 0 en d=4.
        assert_eq!(cobertura_pincel(2.0, 4.0, 0.5), 1.0);
        assert!((cobertura_pincel(3.0, 4.0, 0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn estampar_disco_suave_deja_borde_translucido() {
        // Pincel suave (dureza 0) sobre fondo transparente: el centro
        // queda casi opaco, el borde casi transparente.
        let mut buf = vec![0u8; 9 * 9 * 4];
        let color = [255, 0, 0, 255];
        estampar_disco(&mut buf, 9, 9, 4, 4, 4, color, false, 0.0, None);
        let alfa = |x: usize, y: usize| buf[(y * 9 + x) * 4 + 3];
        // Centro: cobertura ~1 → alfa alto.
        assert!(alfa(4, 4) >= 250, "centro {}", alfa(4, 4));
        // A 2 px del centro: cobertura ~0.5 → alfa medio.
        let a = alfa(6, 4);
        assert!((a as i32 - 128).abs() <= 40, "medio {}", a);
        // Borde del disco (d≈4): cobertura ~0 → alfa bajo.
        assert!(alfa(8, 4) <= 10, "borde {}", alfa(8, 4));
    }

    #[test]
    fn bump_dureza_pincel_clampa() {
        let mut model = modelo_minimo();
        model.dureza_pincel = 0.5;
        model = <Tullpu as App>::update(
            model,
            Msg::BumpDurezaPincel(-1.0),
            &Handle::for_test(),
        );
        assert_eq!(model.dureza_pincel, 0.0);
        model = <Tullpu as App>::update(
            model,
            Msg::BumpDurezaPincel(5.0),
            &Handle::for_test(),
        );
        assert_eq!(model.dureza_pincel, 1.0);
    }

    #[test]
    fn hotkey_llaves_ajustan_dureza_solo_con_trazo() {
        let mut m = modelo_minimo();
        // Sin herramienta de trazo, `{` / `}` no hacen nada.
        m.herramienta = Herramienta::Mover;
        assert!(hotkey_a_msg(&m, &ev_char("}", Modifiers::default())).is_none());
        // Con pincel, `}` sube y `{` baja la dureza.
        m.herramienta = Herramienta::Pincel;
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("}", Modifiers::default())),
            Some(Msg::BumpDurezaPincel(d)) if d > 0.0
        ));
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("{", Modifiers::default())),
            Some(Msg::BumpDurezaPincel(d)) if d < 0.0
        ));
    }

    #[test]
    fn borrador_suave_baja_alfa_parcialmente() {
        // Goma suave (dureza 0) sobre opaco: el centro baja mucho el
        // alfa, el borde poco.
        let mut buf = vec![255u8; 9 * 9 * 4];
        estampar_disco(&mut buf, 9, 9, 4, 4, 4, [0, 0, 0, 0], true, 0.0, None);
        let alfa = |x: usize, y: usize| buf[(y * 9 + x) * 4 + 3];
        // Centro: cobertura ~1 → alfa ~0.
        assert!(alfa(4, 4) <= 5, "centro {}", alfa(4, 4));
        // Borde: cobertura ~0 → alfa casi intacto.
        assert!(alfa(8, 4) >= 245, "borde {}", alfa(8, 4));
    }

    // ---- Fase 48: trazo en línea recta con Shift ------------------------

    #[test]
    fn on_key_shift_sincroniza_shift_held() {
        let m = modelo_minimo();
        // Press de Shift → SetShift(true).
        let ev = KeyEvent {
            key: Key::Named(NamedKey::Shift),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        };
        assert!(matches!(
            <Tullpu as App>::on_key(&m, &ev),
            Some(Msg::SetShift(true))
        ));
        // Release → SetShift(false).
        let ev2 = KeyEvent {
            key: Key::Named(NamedKey::Shift),
            state: KeyState::Released,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        };
        assert!(matches!(
            <Tullpu as App>::on_key(&m, &ev2),
            Some(Msg::SetShift(false))
        ));
    }

    #[test]
    fn set_shift_actualiza_el_estado_vivo() {
        let mut model = modelo_minimo();
        assert!(!model.shift_held);
        model = <Tullpu as App>::update(model, Msg::SetShift(true), &Handle::for_test());
        assert!(model.shift_held);
        model = <Tullpu as App>::update(model, Msg::SetShift(false), &Handle::for_test());
        assert!(!model.shift_held);
    }

    #[test]
    fn shift_click_traza_linea_recta_desde_el_ultimo_punto() {
        // Lienzo 8×8 transparente, pincel radio 0 (1 px), negro opaco.
        let mut model = modelo_minimo();
        let buf = vec![0u8; 8 * 8 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(8, 8);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.herramienta = Herramienta::Pincel;
        model.radio_pincel = 0;
        model.color_picked = Some([0, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        // Primer click en (1,1) (sin shift) → punto, fija ultimo_pincel.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 1.0, ly: 1.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        model = <Tullpu as App>::update(model, Msg::FinalizarTrazo, &Handle::for_test());
        assert_eq!(model.ultimo_pincel, Some((1, 1)));
        // Shift + click en (6,1): debe pintar TODA la fila y=1 de x=1..=6.
        model = <Tullpu as App>::update(model, Msg::SetShift(true), &Handle::for_test());
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 6.0, ly: 1.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let alfa = |x: usize, y: usize| bp[(y * 8 + x) * 4 + 3];
        for x in 1..=6usize {
            assert_eq!(alfa(x, 1), 255, "hueco en ({},1)", x);
        }
        // Fuera de la línea, transparente.
        assert_eq!(alfa(3, 3), 0);
    }

    #[test]
    fn sin_shift_el_click_no_traza_linea() {
        // Mismo setup pero sin shift: el segundo click es un punto suelto,
        // no une con el anterior.
        let mut model = modelo_minimo();
        let buf = vec![0u8; 8 * 8 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(8, 8);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.herramienta = Herramienta::Pincel;
        model.radio_pincel = 0;
        model.color_picked = Some([0, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 1.0, ly: 1.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        model = <Tullpu as App>::update(model, Msg::FinalizarTrazo, &Handle::for_test());
        // shift_held sigue false; click en (6,1).
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 6.0, ly: 1.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let alfa = |x: usize, y: usize| bp[(y * 8 + x) * 4 + 3];
        // Los extremos pintados, el medio NO (no hubo línea).
        assert_eq!(alfa(1, 1), 255);
        assert_eq!(alfa(6, 1), 255);
        assert_eq!(alfa(3, 1), 0);
    }

    // ---- Fase 49: simetría de trazo (mirror painting) -------------------

    #[test]
    fn ejes_simetria_cuenta_correcta() {
        assert_eq!(ejes_simetria(Simetria::Ninguna).len(), 1);
        assert_eq!(ejes_simetria(Simetria::Vertical).len(), 2);
        assert_eq!(ejes_simetria(Simetria::Horizontal).len(), 2);
        assert_eq!(ejes_simetria(Simetria::Ambas).len(), 4);
    }

    #[test]
    fn aplicar_eje_refleja_sobre_el_centro() {
        // Lienzo 8×8. Punto (1,2).
        assert_eq!(aplicar_eje(1, 2, 8, 8, (false, false)), (1, 2));
        assert_eq!(aplicar_eje(1, 2, 8, 8, (true, false)), (6, 2)); // 8-1-1
        assert_eq!(aplicar_eje(1, 2, 8, 8, (false, true)), (1, 5)); // 8-1-2
        assert_eq!(aplicar_eje(1, 2, 8, 8, (true, true)), (6, 5));
    }

    #[test]
    fn simetria_siguiente_cicla() {
        assert_eq!(Simetria::Ninguna.siguiente(), Simetria::Vertical);
        assert_eq!(Simetria::Vertical.siguiente(), Simetria::Horizontal);
        assert_eq!(Simetria::Horizontal.siguiente(), Simetria::Ambas);
        assert_eq!(Simetria::Ambas.siguiente(), Simetria::Ninguna);
    }

    #[test]
    fn ciclar_simetria_hotkey_y_handler() {
        let m = modelo_minimo();
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("s", Modifiers::default())),
            Some(Msg::CiclarSimetria)
        ));
        let mut model = modelo_minimo();
        assert_eq!(model.simetria, Simetria::Ninguna);
        model = <Tullpu as App>::update(model, Msg::CiclarSimetria, &Handle::for_test());
        assert_eq!(model.simetria, Simetria::Vertical);
    }

    #[test]
    fn pincel_con_simetria_vertical_pinta_ambos_lados() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 8 * 8 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(8, 8);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.color_picked = Some([0, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        // Punto en (1,3) radio 0 con simetría vertical → también (6,3).
        let ok = pincel_punto_en_capa(
            &mut model, 1, 3, 0, false, 1.0, Simetria::Vertical,
        );
        assert!(ok);
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let alfa = |x: usize, y: usize| bp[(y * 8 + x) * 4 + 3];
        assert_eq!(alfa(1, 3), 255); // original
        assert_eq!(alfa(6, 3), 255); // espejo X (8-1-1)
        assert_eq!(alfa(1, 4), 0); // sin espejo Y
    }

    #[test]
    fn pincel_con_simetria_ambas_pinta_cuatro() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 8 * 8 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(8, 8);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.color_picked = Some([0, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        let ok = pincel_punto_en_capa(
            &mut model, 1, 2, 0, false, 1.0, Simetria::Ambas,
        );
        assert!(ok);
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let alfa = |x: usize, y: usize| bp[(y * 8 + x) * 4 + 3];
        // 4 cuadrantes: (1,2),(6,2),(1,5),(6,5).
        assert_eq!(alfa(1, 2), 255);
        assert_eq!(alfa(6, 2), 255);
        assert_eq!(alfa(1, 5), 255);
        assert_eq!(alfa(6, 5), 255);
    }

    // ---- Fase 50: herramienta degradé (linear gradient) -----------------

    #[test]
    fn rellenar_gradiente_horizontal_fade_de_opaco_a_transparente() {
        // Fondo transparente 5×1; eje horizontal de x=0 a x=4; color rojo
        // opaco. t crece de 0 a 1 → alfa de 255 a 0.
        let buf = vec![0u8; 5 * 1 * 4];
        let out = rellenar_gradiente(
            &buf, 5, 1, 0.0, 0.0, 4.0, 0.0, [255, 0, 0, 255], None,
        );
        let alfa = |x: usize| out[x * 4 + 3];
        // x=0 (centro 0.5, t≈0.125) alfa alto; x=4 (centro 4.5 → t clamp 1) alfa 0.
        assert!(alfa(0) > alfa(2), "{} vs {}", alfa(0), alfa(2));
        assert!(alfa(2) > alfa(4));
        assert_eq!(alfa(4), 0);
        // Componente roja presente donde hay alfa.
        assert_eq!(out[0], 255);
    }

    #[test]
    fn rellenar_gradiente_eje_cero_es_relleno_solido() {
        // Eje de longitud 0 → t=0 en todo → color pleno en todos lados.
        let buf = vec![0u8; 3 * 1 * 4];
        let out = rellenar_gradiente(
            &buf, 3, 1, 1.0, 0.0, 1.0, 0.0, [10, 20, 30, 255], None,
        );
        for x in 0..3 {
            assert_eq!(&out[x * 4..x * 4 + 4], &[10, 20, 30, 255]);
        }
    }