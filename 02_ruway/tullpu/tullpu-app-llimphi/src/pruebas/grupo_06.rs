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
    fn rellenar_gradiente_respeta_bounds() {
        let buf = vec![0u8; 4 * 4 * 4];
        let out = rellenar_gradiente(
            &buf, 4, 4, 0.0, 0.0, 4.0, 0.0, [9, 9, 9, 255],
            Some((0, 0, 2, 2)),
        );
        // Dentro del rect: tocado (alfa > 0 cerca del ancla).
        assert!(out[(0 * 4 + 0) * 4 + 3] > 0);
        // Fuera del rect: intacto.
        assert_eq!(out[(2 * 4 + 2) * 4 + 3], 0);
        assert_eq!(out[(0 * 4 + 3) * 4 + 3], 0);
    }

    #[test]
    fn hotkey_d_emite_degradado() {
        let m = modelo_minimo();
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("d", Modifiers::default())),
            Some(Msg::CambiarHerramienta(Herramienta::Degradado))
        ));
    }

    #[test]
    fn degradado_drag_completo_rellena_y_snapshotea() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 8 * 8 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let mut lienzo = Lienzo::nuevo(8, 8);
        lienzo.apilar(cap);
        model.lienzo = lienzo;
        model.seleccionada = Some(id);
        model.color_picked = Some([0, 0, 255, 255]);
        model.herramienta = Herramienta::Degradado;
        aplicar_y_recomponer(&mut model);
        model.hist.reiniciar(model.lienzo.clone());
        let hash0 = model.lienzo.capa(id).unwrap().contenido;
        // Press en (0,0), arrastre +8 en X, soltar.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarDegradado { lx: 0.0, ly: 0.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarDegradado { dx: 8.0, dy: 0.0 },
            &Handle::for_test(),
        );
        model = <Tullpu as App>::update(
            model,
            Msg::FinalizarDegradado,
            &Handle::for_test(),
        );
        // Drag cerrado, capa cambiada, snapshot pusheado.
        assert!(model.gradiente_drag.is_none());
        assert_ne!(model.lienzo.capa(id).unwrap().contenido, hash0);
        assert_eq!(model.hist.len(), 2);
    }

    #[test]
    fn agregar_mascara_set_y_snapshotea() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(model.lienzo.capa(id).unwrap().mascara.is_none());
        model = <Tullpu as App>::update(model, Msg::AgregarMascara, &Handle::for_test());
        let mh = model.lienzo.capa(id).unwrap().mascara;
        assert!(mh.is_some(), "la capa debe quedar con máscara");
        // Máscara blanca = todo 255 (nada oculto), tamaño W·H.
        let buf = model.almacen.obtener(mh.unwrap()).unwrap();
        assert_eq!(buf.len(), (model.lienzo.width * model.lienzo.height) as usize);
        assert!(buf.iter().all(|&b| b == 255));
        assert_eq!(model.hist.len(), 2);
    }

    #[test]
    fn agregar_mascara_es_idempotente() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        model = <Tullpu as App>::update(model, Msg::AgregarMascara, &Handle::for_test());
        let mh1 = model.lienzo.capa(id).unwrap().mascara;
        let hist1 = model.hist.len();
        // Segunda vez: no-op (no pisa la máscara ni snapshotea).
        model = <Tullpu as App>::update(model, Msg::AgregarMascara, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().mascara, mh1);
        assert_eq!(model.hist.len(), hist1);
    }

    #[test]
    fn mascara_de_seleccion_visible_dentro_oculto_fuera() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });
        assert!(agregar_mascara_de_seleccion(&mut model));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        let w = model.lienzo.width as usize;
        // Dentro del rect (1,1)..(3,3): 255; fuera: 0.
        assert_eq!(buf[1 * w + 1], 255);
        assert_eq!(buf[2 * w + 2], 255);
        assert_eq!(buf[0], 0);
        assert_eq!(buf[3 * w + 3], 0);
    }

    #[test]
    fn invertir_mascara_intercambia_visible_oculto() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model)); // blanca (255)
        assert!(invertir_mascara(&mut model));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        assert!(buf.iter().all(|&b| b == 0), "invertir 255 → 0");
    }

    #[test]
    fn quitar_mascara_no_toca_pixeles() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let contenido0 = model.lienzo.capa(id).unwrap().contenido;
        assert!(agregar_mascara(&mut model));
        assert!(quitar_mascara(&mut model));
        assert!(model.lienzo.capa(id).unwrap().mascara.is_none());
        // El contenido raster no cambió (no destructivo).
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, contenido0);
        // Sin máscara, quitar de nuevo es no-op.
        assert!(!quitar_mascara(&mut model));
    }

    #[test]
    fn aplicar_mascara_hornea_alfa_y_la_quita() {
        let mut model = modelo_minimo();
        opacar_capa(&mut model);
        let id = model.seleccionada.unwrap();
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });
        assert!(agregar_mascara_de_seleccion(&mut model));
        assert!(aplicar_mascara(&mut model));
        // La máscara se consumió.
        assert!(model.lienzo.capa(id).unwrap().mascara.is_none());
        // El alfa del raster quedó horneado: 255 dentro, 0 fuera.
        let contenido = model.lienzo.capa(id).unwrap().contenido;
        let buf = model.almacen.obtener(contenido).unwrap();
        let w = model.lienzo.width as usize;
        assert_eq!(buf[(1 * w + 1) * 4 + 3], 255, "alfa visible adentro");
        assert_eq!(buf[(0 * w + 0) * 4 + 3], 0, "alfa oculto afuera");
    }

    #[test]
    fn aplicar_mascara_rechaza_derivada() {
        // Una capa derivada no debe poder hornear su máscara (su buffer
        // es cache regenerable). Construimos una derivada con máscara.
        let mut model = modelo_minimo();
        model = <Tullpu as App>::update(
            model,
            Msg::Agregar(OpLocal::Invertir),
            &Handle::for_test(),
        );
        // La derivada queda seleccionada; le ponemos una máscara directa.
        let id = model.seleccionada.unwrap();
        assert!(matches!(
            model.lienzo.capa(id).unwrap().origen,
            OrigenCapa::Derivada { .. }
        ));
        assert!(agregar_mascara(&mut model));
        assert!(!aplicar_mascara(&mut model), "derivada: aplicar es no-op");
        assert!(model.lienzo.capa(id).unwrap().mascara.is_some());
    }

    #[test]
    fn recortar_lienzo_mantiene_mascara_valida() {
        let mut model = modelo_minimo();
        opacar_capa(&mut model);
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model)); // 4×4 = 16 bytes
        recortar_lienzo_a(&mut model, 1, 1, 3, 3); // → 2×2
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        assert_eq!(buf.len(), 2 * 2, "máscara recortada al nuevo tamaño");
        // El render no debe fallar por tamaño de máscara.
        assert!(tullpu_render::componer(&model.lienzo, &model.almacen).is_ok());
    }

    // ---- Pintar sobre la máscara (fase 53) ----

    #[test]
    fn pintando_en_mascara_requiere_modo_y_mascara() {
        let mut model = modelo_minimo();
        // Sin modo ni máscara.
        assert!(!pintando_en_mascara(&model));
        // Modo activo pero sin máscara → sigue pintando contenido.
        model.editando_mascara = true;
        assert!(!pintando_en_mascara(&model));
        // Con máscara + modo → pinta la máscara.
        assert!(agregar_mascara(&mut model));
        assert!(pintando_en_mascara(&model));
        // Apagar el modo lo desactiva aunque haya máscara.
        model.editando_mascara = false;
        assert!(!pintando_en_mascara(&model));
    }

    #[test]
    fn pincel_en_modo_mascara_pinta_mascara_no_contenido() {
        let mut model = modelo_minimo();
        opacar_capa(&mut model);
        let id = model.seleccionada.unwrap();
        // Máscara blanca + invertir → todo oculto (0), para que pintar
        // (revelar=255) sea un cambio observable.
        assert!(agregar_mascara(&mut model));
        assert!(invertir_mascara(&mut model));
        let contenido_antes = model.lienzo.capa(id).unwrap().contenido;
        let mascara_antes = model.lienzo.capa(id).unwrap().mascara.unwrap();
        model.editando_mascara = true;
        // Pincel duro radio 0 en (1,1): un solo píxel a 255.
        assert!(pincel_punto_en_capa(&mut model, 1, 1, 0, false, 1.0, Simetria::Ninguna));
        // El contenido NO cambió; la máscara SÍ.
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, contenido_antes);
        let mascara_despues = model.lienzo.capa(id).unwrap().mascara.unwrap();
        assert_ne!(mascara_despues, mascara_antes);
        let buf = model.almacen.obtener(mascara_despues).unwrap();
        let w = model.lienzo.width as usize;
        assert_eq!(buf[1 * w + 1], 255, "el píxel pintado revela");
        assert_eq!(buf[0], 0, "el resto sigue oculto");
    }

    #[test]
    fn borrador_en_modo_mascara_oculta() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model)); // blanca (255 = visible)
        model.editando_mascara = true;
        // Borrador (borrar=true) → valor 0 (ocultar) en (2,2).
        assert!(pincel_punto_en_capa(&mut model, 2, 2, 0, true, 1.0, Simetria::Ninguna));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        let w = model.lienzo.width as usize;
        assert_eq!(buf[2 * w + 2], 0, "borrador oculta");
        assert_eq!(buf[0], 255, "el resto sigue visible");
    }

    #[test]
    fn balde_en_modo_mascara_rellena_a_255() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model));
        assert!(invertir_mascara(&mut model)); // todo 0
        model.editando_mascara = true;
        assert!(rellenar_flood_en_capa(&mut model, 0, 0));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        assert!(buf.iter().all(|&b| b == 255), "balde revela toda la región");
    }

    #[test]
    fn flood_fill_mascara_un_canal() {
        // 2×2 todo 0; flood desde (0,0) a 255 cubre el cuadro entero.
        let src = vec![0u8; 4];
        let out = flood_fill_mascara(&src, 2, 2, 0, 0, 255, 0, None).unwrap();
        assert_eq!(out, vec![255u8; 4]);
        // Sin cambio si ya está en el valor.
        assert!(flood_fill_mascara(&out, 2, 2, 0, 0, 255, 0, None).is_none());
    }

    #[test]
    fn estampar_disco_mascara_lerp_por_cobertura() {
        // 1 px, dureza dura: el centro va exacto a `valor`.
        let mut buf = vec![0u8; 9]; // 3×3
        estampar_disco_mascara(&mut buf, 3, 3, 1, 1, 0, 200, 1.0, None);
        assert_eq!(buf[1 * 3 + 1], 200);
        // Los vecinos no se tocan con radio 0.
        assert_eq!(buf[0], 0);
    }

    // === Fase 54: valor de gris arbitrario + thumb de máscara ===

    #[test]
    fn valor_mascara_default_es_255() {
        // El default calca fase 53 (pincel revela del todo).
        let model = modelo_minimo();
        assert_eq!(model.valor_mascara, 255);
    }

    #[test]
    fn bump_valor_mascara_clampa() {
        let mut model = modelo_minimo();
        model.valor_mascara = 128;
        model = <Tullpu as App>::update(model, Msg::BumpValorMascara(-200), &Handle::for_test());
        assert_eq!(model.valor_mascara, 0, "no baja de 0");
        model = <Tullpu as App>::update(model, Msg::BumpValorMascara(300), &Handle::for_test());
        assert_eq!(model.valor_mascara, 255, "no sube de 255");
        model = <Tullpu as App>::update(model, Msg::BumpValorMascara(-100), &Handle::for_test());
        assert_eq!(model.valor_mascara, 155, "delta intermedio");
    }

    #[test]
    fn pincel_mascara_escribe_valor_gris_arbitrario() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model)); // blanca (255)
        assert!(invertir_mascara(&mut model)); // todo 0, para ver el cambio
        model.editando_mascara = true;
        // Gris parcial: 100 (no 0/255).
        model.valor_mascara = 100;
        assert!(pincel_punto_en_capa(&mut model, 1, 1, 0, false, 1.0, Simetria::Ninguna));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        let w = model.lienzo.width as usize;
        assert_eq!(buf[1 * w + 1], 100, "el pincel escribe el gris elegido");
        assert_eq!(buf[0], 0, "el resto no se toca");
    }

    #[test]
    fn borrador_mascara_ignora_valor_y_oculta() {
        // El borrador siempre apunta a 0 aunque valor_mascara sea alto.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model)); // blanca (255)
        model.editando_mascara = true;
        model.valor_mascara = 200;
        assert!(pincel_punto_en_capa(&mut model, 2, 2, 0, true, 1.0, Simetria::Ninguna));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        let w = model.lienzo.width as usize;
        assert_eq!(buf[2 * w + 2], 0, "borrador oculta sin importar valor_mascara");
    }

    #[test]
    fn balde_mascara_rellena_con_valor_gris() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model));
        assert!(invertir_mascara(&mut model)); // todo 0
        model.editando_mascara = true;
        model.valor_mascara = 64;
        assert!(rellenar_flood_en_capa(&mut model, 0, 0));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        assert!(buf.iter().all(|&b| b == 64), "balde rellena al gris elegido");
    }

    #[test]
    fn thumbnail_de_mascara_expande_gris_opaco() {
        // Buffer de 1 canal del tamaño exacto del thumb (THUMB_LADO²): así
        // `thumbnail` no reescala y la expansión a (v,v,v,255) es exacta.
        let lado = THUMB_LADO;
        let mut alm = AlmacenEnMemoria::nuevo();
        let hash = alm.insertar(vec![128u8; (lado * lado) as usize]);
        let img = thumbnail_de_mascara(hash, lado, lado, &alm).expect("thumb");
        assert_eq!((img.image.width, img.image.height), (lado, lado));
        let data = img.image.data.data();
        // Cada byte de la máscara se expande a gris medio opaco.
        for px in data.chunks_exact(4) {
            assert_eq!(px, &[128, 128, 128, 255]);
        }
    }

    #[test]
    fn thumbnail_de_mascara_rechaza_tamano_incorrecto() {
        // Buffer que no mide w*h (4 bytes para 3×3 = 9) → None.
        let mut alm = AlmacenEnMemoria::nuevo();
        let hash = alm.insertar(vec![0u8; 4]);
        assert!(thumbnail_de_mascara(hash, 3, 3, &alm).is_none());
    }

    #[test]
    fn sincronizar_thumbs_cachea_la_mascara() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(agregar_mascara(&mut model));
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        sincronizar_thumbs(&mut model);
        assert!(model.thumbs_mascara.contains_key(&mh), "thumb de máscara cacheado");
        // Quitar la máscara y resync barre la entrada muerta.
        assert!(quitar_mascara(&mut model));
        sincronizar_thumbs(&mut model);
        assert!(!model.thumbs_mascara.contains_key(&mh), "entrada muerta barrida");
    }

    #[test]
    fn rotar_lienzo_mantiene_mascara_valida() {
        let mut model = modelo_minimo();
        // Lienzo no cuadrado para detectar trasposición incorrecta.
        model.lienzo.width = 4;
        model.lienzo.height = 2;
        let id = model.seleccionada.unwrap();
        let hash = model.almacen.insertar(vec![255u8; 4 * 2 * 4]);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        assert!(agregar_mascara(&mut model)); // 4×2 = 8 bytes
        assert!(rotar_lienzo(&mut model, true)); // → 2×4
        assert_eq!(model.lienzo.width, 2);
        assert_eq!(model.lienzo.height, 4);
        let mh = model.lienzo.capa(id).unwrap().mascara.unwrap();
        let buf = model.almacen.obtener(mh).unwrap();
        assert_eq!(buf.len(), 2 * 4, "máscara rotada conserva conteo");
        assert!(tullpu_render::componer(&model.lienzo, &model.almacen).is_ok());
    }
    // =====================================================================
    //  Fase A en la app: agrupar · clipping · capa de ajuste (vía update)
    // =====================================================================

    #[test]
    fn agrupar_envuelve_la_capa_y_selecciona_el_grupo() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        model = <Tullpu as App>::update(model, Msg::Agrupar(id), &Handle::for_test());
        // Hay un grupo nuevo y la capa cuelga de él.
        let gid = model.seleccionada.unwrap();
        assert_ne!(gid, id, "la selección pasa al grupo");
        let grupo = model.lienzo.capa(gid).unwrap();
        assert!(grupo.es_grupo());
        assert_eq!(model.lienzo.capa(id).unwrap().grupo, Some(gid));
        // El render sigue siendo válido.
        assert!(tullpu_render::componer(&model.lienzo, &model.almacen).is_ok());
    }

    #[test]
    fn toggle_clipping_alterna_el_flag() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert!(!model.lienzo.capa(id).unwrap().clipping);
        model = <Tullpu as App>::update(model, Msg::ToggleClipping(id), &Handle::for_test());
        assert!(model.lienzo.capa(id).unwrap().clipping, "ON tras 1er toggle");
        model = <Tullpu as App>::update(model, Msg::ToggleClipping(id), &Handle::for_test());
        assert!(!model.lienzo.capa(id).unwrap().clipping, "OFF tras 2do toggle");
    }

    #[test]
    fn agregar_ajuste_apila_capa_de_ajuste_y_compone() {
        let mut model = modelo_minimo();
        let n_antes = model.lienzo.capas.len();
        model = <Tullpu as App>::update(
            model,
            Msg::AgregarAjuste(OpLocal::Invertir),
            &Handle::for_test(),
        );
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        let id = model.seleccionada.unwrap();
        let aj = model.lienzo.capa(id).unwrap();
        assert!(aj.op_ajuste().is_some(), "es capa de ajuste");
        assert!(matches!(aj.op_ajuste().unwrap(), OpLocal::Invertir));
        assert!(tullpu_render::componer(&model.lienzo, &model.almacen).is_ok());
    }

    #[test]
    fn ajustar_parametro_de_capa_de_ajuste_muta_en_vivo() {
        // Una capa de ajuste Brillo editada por slider muta su op sin pasar
        // por el ciclo stale (los ajustes se recomponen en vivo).
        let mut model = modelo_minimo();
        let aj = Capa::ajuste("brillo", OpLocal::Brillo { delta: 0.0 });
        let id = aj.id;
        model.lienzo.apilar(aj);
        model.seleccionada = Some(id);
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarParametro {
                id,
                param: ParametroSlider::BrilloDelta,
                dv: 0.3,
            },
            &Handle::for_test(),
        );
        match model.lienzo.capa(id).unwrap().op_ajuste().unwrap() {
            OpLocal::Brillo { delta } => assert!((delta - 0.3).abs() < 1e-5, "delta={delta}"),
            otro => panic!("esperaba Brillo, vino {otro:?}"),
        }
        // La capa de ajuste NO queda en estado derivado/stale.
        assert!(matches!(
            model.lienzo.capa(id).unwrap().origen,
            OrigenCapa::Raster
        ));
    }

    // =====================================================================
    //  Fase B: varita mágica + selección como máscara (no rectangular)
    // =====================================================================

    #[test]
    fn varita_selecciona_region_por_color_y_arma_mascara() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // 4×4: mitad izquierda (x<2) roja opaca, derecha azul opaca.
        let (w, h) = (4u32, 4u32);
        let mut buf = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                if x < 2 {
                    buf.extend_from_slice(&[255, 0, 0, 255]);
                } else {
                    buf.extend_from_slice(&[0, 0, 255, 255]);
                }
            }
        }
        let hash = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        // Varita desde (0,0) = rojo: agarra sólo la mitad izquierda.
        assert!(seleccionar_por_color(&mut model, 0, 0));
        assert!(model.seleccion_mascara.is_some(), "se armó la máscara");
        let bbox = model.seleccion.unwrap();
        assert_eq!(
            (bbox.x0, bbox.y0, bbox.x1, bbox.y1),
            (0, 0, 2, 4),
            "bbox = mitad roja"
        );
    }

    #[test]
    fn limpiar_respeta_mascara_no_rectangular() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // Capa blanca opaca 4×4.
        let hash = model.almacen.insertar(vec![255u8; 4 * 4 * 4]);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        // Máscara: sólo el píxel (0,0) seleccionado.
        let mut mask = vec![0u8; 16];
        mask[0] = 255;
        let mh = model.almacen.insertar(mask);
        model.seleccion_mascara = Some(mh);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 1, y1: 1 });
        assert!(limpiar_seleccion_en_capa(&mut model));
        let nuevo = model.lienzo.capa(id).unwrap().contenido;
        let out = model.almacen.obtener(nuevo).unwrap();
        assert_eq!(out[3], 0, "px (0,0) limpiado por la máscara");
        assert_eq!(out[7], 255, "px (1,0) intacto (fuera de la máscara)");
    }

    #[test]
    fn marquee_limpia_la_mascara_de_la_varita() {
        // Hacer un marquee rectangular tras una selección de varita debe
        // descartar la máscara (degrada a rect).
        let mut model = modelo_minimo();
        let mh = model.almacen.insertar(vec![255u8; 16]);
        model.seleccion_mascara = Some(mh);
        model = <Tullpu as App>::update(
            model,
            Msg::SeleccionarTodo,
            &Handle::for_test(),
        );
        assert!(model.seleccion_mascara.is_none(), "select-all limpia la máscara");
    }

    #[test]
    fn lazo_rasteriza_poligono_a_mascara() {
        let mut model = modelo_minimo(); // lienzo 4×4
        // Triángulo que cubre la esquina superior-izquierda.
        let puntos = vec![(0, 0), (3, 0), (0, 3)];
        assert!(seleccionar_lazo(&mut model, &puntos));
        assert!(model.seleccion_mascara.is_some());
        // El bbox cubre el triángulo.
        let bbox = model.seleccion.unwrap();
        assert_eq!(bbox.x0, 0);
        assert_eq!(bbox.y0, 0);
        assert!(bbox.x1 >= 3 && bbox.y1 >= 3);
    }

    #[test]
    fn lazo_corto_es_no_op() {
        let mut model = modelo_minimo();
        assert!(!seleccionar_lazo(&mut model, &[(0, 0), (1, 1)]));
        assert!(model.seleccion_mascara.is_none());
    }

    #[test]
    fn invertir_seleccion_complementa_la_mascara() {
        let mut model = modelo_minimo(); // 4×4
        // Selección de un solo píxel (0,0).
        let mut mask = vec![0u8; 16];
        mask[0] = 255;
        let mh = model.almacen.insertar(mask);
        model.seleccion_mascara = Some(mh);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 1, y1: 1 });
        assert!(invertir_seleccion(&mut model));
        // Tras invertir, (0,0) NO está y el resto sí.
        let cov = cobertura_seleccion(&model).unwrap();
        assert_eq!(cov[0], 0, "(0,0) deseleccionado");
        assert_eq!(cov[1], 255, "(1,0) ahora seleccionado");
    }

    #[test]
    fn varita_con_shift_suma_a_la_seleccion_previa() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // Buffer 4×4: columna x<2 roja, x>=2 azul.
        let (w, h) = (4u32, 4u32);
        let mut buf = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                if x < 2 { buf.extend_from_slice(&[255, 0, 0, 255]); }
                else { buf.extend_from_slice(&[0, 0, 255, 255]); }
            }
        }
        let hash = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        // 1) selecciono el rojo (sin shift).
        model.shift_held = false;
        assert!(seleccionar_por_color(&mut model, 0, 0));
        // 2) shift + selecciono el azul ⇒ unión = todo.
        model.shift_held = true;
        assert!(seleccionar_por_color(&mut model, 3, 0));
        let cov = cobertura_seleccion(&model).unwrap();
        assert!(cov.iter().all(|&v| v == 255), "unión cubre todo el lienzo");
    }

    #[test]
    fn varita_con_alt_resta_de_la_seleccion_previa() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // Buffer 4×4: columna x<2 roja, x>=2 azul.
        let (w, h) = (4u32, 4u32);
        let mut buf = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                if x < 2 { buf.extend_from_slice(&[255, 0, 0, 255]); }
                else { buf.extend_from_slice(&[0, 0, 255, 255]); }
            }
        }
        let hash = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        // 1) selecciono rojo, 2) shift+azul ⇒ todo, 3) alt+azul ⇒ resta el azul.
        model.shift_held = false;
        model.alt_held = false;
        assert!(seleccionar_por_color(&mut model, 0, 0));
        model.shift_held = true;
        assert!(seleccionar_por_color(&mut model, 3, 0));
        model.shift_held = false;
        model.alt_held = true;
        assert!(seleccionar_por_color(&mut model, 3, 0));
        let cov = cobertura_seleccion(&model).unwrap();
        // Sólo el rojo (cols 0,1) queda; el azul (cols 2,3) restado.
        for y in 0..h as usize {
            assert_eq!(cov[y * 4], 255, "col 0 (rojo) sigue");
            assert_eq!(cov[y * 4 + 1], 255, "col 1 (rojo) sigue");
            assert_eq!(cov[y * 4 + 2], 0, "col 2 (azul) restada");
            assert_eq!(cov[y * 4 + 3], 0, "col 3 (azul) restada");
        }
        let bbox = model.seleccion.unwrap();
        assert_eq!((bbox.x0, bbox.x1), (0, 2), "bbox recogido a la mitad roja");
    }

    #[test]
    fn restar_toda_la_seleccion_la_vacia() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let hash = model.almacen.insertar(vec![255u8; 4 * 4 * 4]); // todo blanco
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        model.shift_held = false;
        model.alt_held = false;
        assert!(seleccionar_por_color(&mut model, 0, 0)); // todo el lienzo
        // Alt sobre el mismo color resta todo ⇒ selección vacía (devuelve false).
        model.alt_held = true;
        assert!(!seleccionar_por_color(&mut model, 0, 0));
        assert!(model.seleccion_mascara.is_none(), "resta total limpia la máscara");
        assert!(model.seleccion.is_none(), "y también el rect");
    }

    #[test]
    fn lazo_con_shift_alt_interseca() {
        let mut model = modelo_minimo(); // lienzo 4×4
        // 1) lazo cubre la banda superior (filas 0,1), todo el ancho.
        model.shift_held = false;
        model.alt_held = false;
        assert!(seleccionar_lazo(&mut model, &[(0, 0), (3, 0), (3, 2), (0, 2)]));
        // 2) shift+alt: lazo cubre la banda izquierda (cols 0..2), todo el alto.
        model.shift_held = true;
        model.alt_held = true;
        assert!(seleccionar_lazo(&mut model, &[(0, 0), (2, 0), (2, 4), (0, 4)]));
        let cov = cobertura_seleccion(&model).unwrap();
        let at = |x: usize, y: usize| cov[y * 4 + x];
        // Intersección = filas {0,1} ∩ cols {0,1,2}.
        assert_eq!(at(0, 0), 255, "esquina común seleccionada");
        assert_eq!(at(2, 1), 255, "interior común seleccionado");
        assert_eq!(at(3, 0), 0, "col 3 fuera de la banda izquierda");
        assert_eq!(at(0, 2), 0, "fila 2 fuera de la banda superior");
    }

    #[test]
    fn mover_con_mascara_irregular_conserva_la_forma() {
        // Lienzo 4×4. Máscara diagonal: sólo (0,0) y (1,1). Los píxeles NO
        // seleccionados dentro del bbox (1,0)/(0,1) deben quedarse quietos —
        // eso prueba que se levanta por máscara, no por rect.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let mut buf = vec![0u8; 4 * 4 * 4];
        let sel = [200u8, 100, 50, 255]; // color de los píxeles seleccionados
        let libre = [10u8, 20, 30, 255]; // color de los NO seleccionados del bbox
        let set = |b: &mut [u8], x: usize, y: usize, c: [u8; 4]| {
            let i = (y * 4 + x) * 4;
            b[i..i + 4].copy_from_slice(&c);
        };
        set(&mut buf, 0, 0, sel);
        set(&mut buf, 1, 1, sel);
        set(&mut buf, 1, 0, libre);
        set(&mut buf, 0, 1, libre);
        let hash = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        // Máscara diagonal (0,0)+(1,1); bbox = (0,0)..(2,2).
        let mut mask = vec![0u8; 16];
        mask[0] = 255;
        mask[1 * 4 + 1] = 255;
        let mh = model.almacen.insertar(mask);
        model.seleccion_mascara = Some(mh);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });

        assert!(mover_pixeles_seleccion(&mut model, 2, 2));
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        // Los seleccionados aterrizaron en la diagonal desplazada.
        assert_eq!(pix(2, 2), sel, "(0,0)→(2,2)");
        assert_eq!(pix(3, 3), sel, "(1,1)→(3,3)");
        // El origen de los seleccionados quedó vacío.
        assert_eq!(pix(0, 0), [0, 0, 0, 0], "(0,0) levantado");
        assert_eq!(pix(1, 1), [0, 0, 0, 0], "(1,1) levantado");
        // Los NO seleccionados del bbox NO se movieron (clave: no se levantó el rect).
        assert_eq!(pix(1, 0), libre, "(1,0) intacto — fuera de la máscara");
        assert_eq!(pix(0, 1), libre, "(0,1) intacto — fuera de la máscara");
        // La máscara siguió al contenido conservando su forma diagonal.
        assert!(model.seleccion_mascara.is_some(), "sigue siendo selección por máscara");
        let cov = cobertura_seleccion(&model).unwrap();
        assert_eq!(cov[2 * 4 + 2], 255, "(2,2) seleccionado");
        assert_eq!(cov[3 * 4 + 3], 255, "(3,3) seleccionado");
        assert_eq!(cov[2 * 4 + 3], 0, "(3,2) NO — la forma no degradó a rect");
        assert_eq!(cov[3 * 4 + 2], 0, "(2,3) NO — la forma no degradó a rect");
    }

    #[test]
    fn overlay_se_arma_con_la_varita_y_se_limpia_al_seleccionar_todo() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let buf = vec![255u8; 4 * 4 * 4];
        let hash = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        model.shift_held = false;
        assert!(seleccionar_por_color(&mut model, 0, 0));
        assert!(model.seleccion_overlay.is_some(), "varita arma overlay");
        // Select-all degrada a rect ⇒ overlay limpio.
        model = <Tullpu as App>::update(model, Msg::SeleccionarTodo, &Handle::for_test());
        assert!(model.seleccion_overlay.is_none(), "select-all limpia overlay");
    }

    #[test]
    fn voltear_capa_horizontal_espeja_columnas() {
        let mut model = modelo_minimo(); // 4×4
        let id = model.seleccionada.unwrap();
        // Fila 0: gradiente por x (x*10 en R), resto 0.
        let (w, h) = (4u32, 4u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for x in 0..w {
            let i = (x * 4) as usize;
            buf[i] = (x as u8) * 10;
            buf[i + 3] = 255;
        }
        let hash = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
        assert!(voltear_capa_activa(&mut model, true));
        let nuevo = model.lienzo.capa(id).unwrap().contenido;
        let out = model.almacen.obtener(nuevo).unwrap();
        // Tras voltear ↔: (0,0) toma el valor que estaba en (3,0) = 30.
        assert_eq!(out[0], 30);
        assert_eq!(out[3 * 4], 0, "(3,0) toma el de (0,0)=0");
    }

    #[test]
    fn agregar_capa_texto_crea_capa_de_texto_visible() {
        let mut model = modelo_minimo(); // 4×4 (chico, pero el rasterizado recorta)
        let n_antes = model.lienzo.capas.len();
        let id = agregar_capa_texto(&mut model, 0, 0);
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        let capa = model.lienzo.capa(id).unwrap();
        assert!(capa.params_texto().is_some(), "es capa de texto");
        assert_eq!(capa.params_texto().unwrap().texto, "Texto");
        assert!(capa.tiene_buffer(), "compone como píxeles");
        // El compositor la maneja sin error.
        assert!(tullpu_render::componer(&model.lienzo, &model.almacen).is_ok());
    }

    #[test]
    fn editar_params_texto_re_rasteriza_y_cambia_el_buffer() {
        // En un lienzo grande, cambiar el texto cambia el hash del contenido.
        let mut model = modelo_minimo();
        // Agrandar el lienzo a 64×32 para que entre texto.
        let buf = vec![0u8; 64 * 32 * 4];
        let hash = model.almacen.insertar(buf);
        let mut l = Lienzo::nuevo(64, 32);
        l.apilar(Capa::raster("base", hash));
        model.lienzo = l;
        model.seleccionada = model.lienzo.capas.first().map(|c| c.id);
        let id = agregar_capa_texto(&mut model, 1, 1);
        let antes = model.lienzo.capa(id).unwrap().contenido;
        editar_params_texto(&mut model, id, |p| p.texto = "Hola mundo".into());
        let despues = model.lienzo.capa(id).unwrap().contenido;
        assert_ne!(antes, despues, "re-rasterizar cambia el buffer");
        assert_eq!(model.lienzo.capa(id).unwrap().params_texto().unwrap().texto, "Hola mundo");
    }

    #[test]
    fn clonar_copia_pixeles_del_origen_al_destino() {
        // Lienzo 8×8, capa con un punto rojo opaco en (1,1); el resto vacío.
        let mut model = modelo_minimo();
        let (w, h) = (8u32, 8u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let i = ((1 * w + 1) * 4) as usize;
        buf[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        let hash = model.almacen.insertar(buf);
        let mut l = Lienzo::nuevo(w, h);
        l.apilar(Capa::raster("c", hash));
        model.lienzo = l;
        let id = model.lienzo.capas[0].id;
        model.seleccionada = Some(id);
        // Origen = (1,1); clonar en (5,5) con offset = origen - destino = (-4,-4).
        // radio 0 (1 px), dureza 1.0 (borde duro) → copia exacta del píxel.
        assert!(clonar_punto_en_capa(&mut model, 5, 5, -4, -4, 0, 1.0));
        let nuevo = model.lienzo.capa(id).unwrap().contenido;
        let out = model.almacen.obtener(nuevo).unwrap();
        let di = ((5 * w + 5) * 4) as usize;
        assert_eq!(&out[di..di + 4], &[255, 0, 0, 255], "clonó el rojo en (5,5)");
        // El origen sigue intacto.
        let oi = ((1 * w + 1) * 4) as usize;
        assert_eq!(&out[oi..oi + 4], &[255, 0, 0, 255]);
    }

    // === Fase D: transformación libre (Ctrl+T) ===

    /// Lienzo `n×n` con una capa raster cuyo contenido es un bloque opaco
    /// `[x0,x1) × [y0,y1)` de color `col`; el resto transparente.
    fn modelo_con_bloque(n: u32, x0: u32, y0: u32, x1: u32, y1: u32, col: [u8; 4]) -> (Model, Uuid) {
        let mut model = modelo_minimo();
        let mut buf = vec![0u8; (n * n * 4) as usize];
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * n + x) * 4) as usize;
                buf[i..i + 4].copy_from_slice(&col);
            }
        }
        let hash = model.almacen.insertar(buf);
        let mut l = Lienzo::nuevo(n, n);
        l.apilar(Capa::raster("c", hash));
        model.lienzo = l;
        let id = model.lienzo.capas[0].id;
        model.seleccionada = Some(id);
        (model, id)
    }

    fn alfa_en(model: &Model, id: Uuid, n: u32, x: u32, y: u32) -> u8 {
        let h = model.lienzo.capa(id).unwrap().contenido;
        let buf = model.almacen.obtener(h).unwrap();
        buf[((y * n + x) * 4 + 3) as usize]
    }

    #[test]
    fn ctrl_t_emite_iniciar_transform() {
        let m = modelo_minimo();
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("t", Modifiers { ctrl: true, ..Default::default() })),
            Some(Msg::IniciarTransform)
        ));
    }

    #[test]
    fn iniciar_transform_centra_el_pivote_en_el_bbox() {
        let (mut model, _id) = modelo_con_bloque(64, 16, 16, 48, 48, [200, 50, 50, 255]);
        assert!(iniciar_transform(&mut model));
        let t = model.transform.as_ref().expect("modo transformar activo");
        assert_eq!((t.piv_x, t.piv_y), (32.0, 32.0), "pivote = centro del bloque");
        assert_eq!((t.bx0, t.by0, t.bx1, t.by1), (16.0, 16.0, 48.0, 48.0));
    }

    #[test]
    fn iniciar_transform_rechaza_capa_derivada() {
        let mut model = modelo_minimo();
        let madre = model.seleccionada.unwrap();
        // Apilamos una capa derivada (ajuste) y la seleccionamos.
        let deriv = Capa::derivada(
            "inv",
            madre,
            tullpu_core::TransformacionPixel::Local(OpLocal::Invertir),
            [0u8; 32],
        );
        let id_deriv = deriv.id;
        model.lienzo.apilar(deriv);
        model.seleccionada = Some(id_deriv);
        assert!(matches!(
            model.lienzo.capa(id_deriv).unwrap().origen,
            OrigenCapa::Derivada { .. }
        ));
        assert!(!iniciar_transform(&mut model), "no transforma derivadas");
        assert!(model.transform.is_none());
    }

    #[test]
    fn transform_mover_desplaza_el_contenido() {
        // Bloque 16..48 sobre lienzo 64; sin zoom/pan ⇒ s=1, coords-imagen = locales.
        let (mut model, id) = modelo_con_bloque(64, 16, 16, 48, 48, [200, 50, 50, 255]);
        assert!(iniciar_transform(&mut model));
        // Press en el centro (32,32) ⇒ agarre Mover; arrastrar +8 en x.
        transform_press(&mut model, 32.0, 32.0, 64.0, 64.0);
        assert!(matches!(
            model.transform.as_ref().unwrap().agarre.unwrap().tipo,
            TipoAgarre::Mover { .. }
        ));
        transform_arrastrar(&mut model, 8.0, 0.0);
        assert!(confirmar_transform(&mut model));
        assert!(model.transform.is_none(), "modo cerrado al confirmar");
        // El bloque se corrió +8: x=52 ahora opaco, x=18 ahora vacío.
        assert_eq!(alfa_en(&model, id, 64, 52, 32), 255, "borde derecho corrido");
        assert_eq!(alfa_en(&model, id, 64, 18, 32), 0, "borde izquierdo liberado");
    }

    #[test]
    fn transform_escala_esquina_agranda_el_contenido() {
        let (mut model, id) = modelo_con_bloque(64, 16, 16, 48, 48, [200, 50, 50, 255]);
        assert!(iniciar_transform(&mut model));
        // Press en la esquina inferior-derecha (48,48) ⇒ agarre Escala.
        transform_press(&mut model, 48.0, 48.0, 64.0, 64.0);
        assert!(matches!(
            model.transform.as_ref().unwrap().agarre.unwrap().tipo,
            TipoAgarre::Escala { .. }
        ));
        // Llevar la esquina a (64,64): escala ×2 alrededor del centro (32,32),
        // el bloque (semi-ancho 16) pasa a semi-ancho 32 → cubre todo el lienzo.
        transform_arrastrar(&mut model, 16.0, 16.0);
        let t = model.transform.as_ref().unwrap();
        assert!((t.escala_x - 2.0).abs() < 1e-3 && (t.escala_y - 2.0).abs() < 1e-3);
        assert!(confirmar_transform(&mut model));
        // Una esquina del lienzo, antes transparente, ahora cae dentro del bloque.
        assert_eq!(alfa_en(&model, id, 64, 2, 2), 255, "×2 llena la esquina");
    }

    #[test]
    fn transform_cancelar_restaura_el_buffer_original() {
        let (mut model, id) = modelo_con_bloque(64, 16, 16, 48, 48, [200, 50, 50, 255]);
        let orig = model.lienzo.capa(id).unwrap().contenido;
        assert!(iniciar_transform(&mut model));
        transform_press(&mut model, 32.0, 32.0, 64.0, 64.0);
        transform_arrastrar(&mut model, 10.0, 6.0);
        // Preview ya cambió el contenido…
        assert_ne!(model.lienzo.capa(id).unwrap().contenido, orig);
        assert!(cancelar_transform(&mut model));
        // …pero cancelar lo restaura bit-a-bit y cierra el modo.
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, orig);
        assert!(model.transform.is_none());
    }

    // === Fase F: pincel corrector (healing) ===

    /// Lienzo 16×16 opaco en dos mitades: cols 0..8 gris `izq`, cols 8..16 gris
    /// `der`. Capa raster seleccionada. `(Model, id)`.
    fn modelo_dos_mitades(izq: u8, der: u8) -> (Model, Uuid) {
        let mut model = modelo_minimo();
        let n = 16u32;
        let mut buf = vec![0u8; (n * n * 4) as usize];
        for y in 0..n {
            for x in 0..n {
                let v = if x < 8 { izq } else { der };
                let i = ((y * n + x) * 4) as usize;
                buf[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let hash = model.almacen.insertar(buf);
        let mut l = Lienzo::nuevo(n, n);
        l.apilar(Capa::raster("c", hash));
        model.lienzo = l;
        let id = model.lienzo.capas[0].id;
        model.seleccionada = Some(id);
        (model, id)
    }

    fn canal_en(model: &Model, id: Uuid, n: u32, x: u32, y: u32) -> u8 {
        let h = model.lienzo.capa(id).unwrap().contenido;
        let buf = model.almacen.obtener(h).unwrap();
        buf[((y * n + x) * 4) as usize]
    }

    /// Inyecta un valor gris uniforme `v` en `(x,y)` del contenido de la capa.
    fn pintar_pixel(model: &mut Model, id: Uuid, n: u32, x: u32, y: u32, v: u8) {
        let h = model.lienzo.capa(id).unwrap().contenido;
        let mut buf = model.almacen.obtener(h).unwrap().to_vec();
        let i = ((y * n + x) * 4) as usize;
        buf[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        let h2 = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = h2;
    }

    #[test]
    fn sanar_borra_la_mancha_igualando_el_color_base() {
        // Destino gris 200 con una mancha oscura (40) en (12,8); origen gris
        // 100. Sanar muestreando el origen iguala el color base (≈200) y borra
        // la mancha; clonar la dejaría ≈100, aún visible sobre el 200.
        let (mut m_sanar, id) = modelo_dos_mitades(100, 200);
        pintar_pixel(&mut m_sanar, id, 16, 12, 8, 40);
        assert!(sanar_punto_en_capa(&mut m_sanar, 12, 8, -8, 0, 2, 1.0));
        let v_sanado = canal_en(&m_sanar, id, 16, 12, 8);
        assert!((185..=205).contains(&v_sanado), "el sanado funde la mancha en el fondo (≈200), fue {v_sanado}");

        let (mut m_clon, id2) = modelo_dos_mitades(100, 200);
        pintar_pixel(&mut m_clon, id2, 16, 12, 8, 40);
        assert!(clonar_punto_en_capa(&mut m_clon, 12, 8, -8, 0, 2, 1.0));
        let v_clon = canal_en(&m_clon, id2, 16, 12, 8);
        assert!(v_clon < 130, "clonar pega el origen crudo (≈100), aún visible, fue {v_clon}");
    }

    #[test]
    fn sanar_preserva_la_textura_del_origen() {
        // Origen 100 con un "detalle" brillante en (4,8)=200; destino 200.
        // El detalle (desviación +100 sobre su entorno) debe sobrevivir como
        // un punto más claro que su vecindario tras igualar el color base.
        let (mut model, id) = modelo_dos_mitades(100, 200);
        // Inyectamos el detalle en el origen.
        let h = model.lienzo.capa(id).unwrap().contenido;
        let mut buf = model.almacen.obtener(h).unwrap().to_vec();
        let det = ((8 * 16 + 4) * 4) as usize;
        buf[det..det + 4].copy_from_slice(&[200, 200, 200, 255]);
        let h2 = model.almacen.insertar(buf);
        model.lienzo.capa_mut(id).unwrap().contenido = h2;
        // Sanar en (12,8) muestreando (4,8): el píxel central toma el detalle.
        assert!(sanar_punto_en_capa(&mut model, 12, 8, -8, 0, 2, 1.0));
        let centro = canal_en(&model, id, 16, 12, 8); // tomó el detalle 200+delta
        let vecino = canal_en(&model, id, 16, 11, 8); // tomó el 100 base+delta ≈ dest
        assert!((180..=210).contains(&vecino), "vecino igualado al destino (≈200), fue {vecino}");
        assert!(centro > vecino + 30, "el detalle del origen sobrevive ({centro} vs {vecino})");
    }
