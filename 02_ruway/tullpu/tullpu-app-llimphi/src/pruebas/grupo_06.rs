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
