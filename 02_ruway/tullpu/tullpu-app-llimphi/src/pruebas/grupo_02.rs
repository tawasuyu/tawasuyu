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
    fn agregar_capa_relleno_default_cuando_no_hay_color_picked() {
        // Sin color leído, debe usar RELLENO_DEFAULT (gris medio).
        let mut model = modelo_minimo();
        assert!(model.color_picked.is_none());
        let n_antes = model.lienzo.capas.len();
        agregar_capa_relleno(&mut model);
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        let nueva = model.lienzo.capas.last().unwrap();
        assert!(
            nueva.nombre.starts_with("relleno #")
                && nueva.nombre.contains("808080"),
            "nombre {} debe llevar el hex del default",
            nueva.nombre
        );
    }

    #[test]
    fn agregar_capa_relleno_usa_color_picked_si_existe() {
        let mut model = modelo_minimo();
        model.color_picked = Some([200, 100, 50, 255]);
        agregar_capa_relleno(&mut model);
        let nueva = model.lienzo.capas.last().unwrap();
        assert!(
            nueva.nombre.contains("C86432"),
            "nombre {} debe llevar el hex del picked",
            nueva.nombre
        );
    }

    #[test]
    fn agregar_capa_relleno_dos_veces_mismo_color_comparte_hash() {
        // Content-addressing: dos rellenos del mismo color al mismo lienzo
        // producen el mismo Hash y comparten el slot del almacén — no
        // duplican RAM. Las capas tienen Uuid distinto pero contenido = ptr
        // al mismo buffer.
        let mut model = modelo_minimo();
        model.color_picked = Some([42, 42, 42, 255]);
        agregar_capa_relleno(&mut model);
        let h1 = match model.lienzo.capas.last().unwrap().origen {
            tullpu_core::OrigenCapa::Raster => model
                .lienzo
                .capas
                .last()
                .unwrap()
                .contenido,
            _ => panic!("esperaba raster"),
        };
        agregar_capa_relleno(&mut model);
        let h2 = match model.lienzo.capas.last().unwrap().origen {
            tullpu_core::OrigenCapa::Raster => model
                .lienzo
                .capas
                .last()
                .unwrap()
                .contenido,
            _ => panic!("esperaba raster"),
        };
        assert_eq!(h1, h2, "mismo color → mismo hash (dedup)");
        // Pero los Uuid son distintos: son capas independientes.
        let n = model.lienzo.capas.len();
        assert_ne!(
            model.lienzo.capas[n - 1].id,
            model.lienzo.capas[n - 2].id
        );
    }

    #[test]
    fn agregar_capa_relleno_se_inserta_encima_de_la_seleccionada() {
        // Si hay selección, la capa nueva queda en idx_sel + 1.
        let mut model = modelo_minimo();
        let sel = model.seleccionada.unwrap();
        let idx_sel = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.id == sel)
            .unwrap();
        // Agrego una capa B "vieja" para tener una vecina arriba de sel.
        let hash_b = model.almacen.insertar(vec![9u8; 4 * 4 * 4]);
        let cap_b = Capa::raster("vieja", hash_b);
        model.lienzo.apilar(cap_b);
        // Ahora reapunto la selección a sel y agrego el relleno: debe
        // quedar entre sel y "vieja", no al tope.
        model.seleccionada = Some(sel);
        agregar_capa_relleno(&mut model);
        let nueva_idx = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.nombre.starts_with("relleno"))
            .unwrap();
        assert_eq!(nueva_idx, idx_sel + 1, "encima de la seleccionada");
        // Y "vieja" pasó a estar arriba del relleno (idx mayor).
        let vieja_idx = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.nombre == "vieja")
            .unwrap();
        assert!(vieja_idx > nueva_idx);
    }

    #[test]
    fn msg_agregar_relleno_dispatcha_y_snapshotea() {
        // El flujo entero por el update: el historial crece, el lienzo
        // tiene una capa más, y un Undo lo deshace.
        let mut model = modelo_minimo();
        let n_antes = model.lienzo.capas.len();
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(model, Msg::AgregarRelleno, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        assert_eq!(model.hist.len(), hist_antes + 1);
        // Undo lo revierte.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), n_antes);
    }

    #[test]
    fn combinar_capa_en_fondo_es_no_op_con_mensaje() {
        // La capa en idx 0 no tiene nada debajo: la merge es un no-op
        // semántico. El lienzo no cambia y el estado avisa.
        let (mut model, id_b, _) = modelo_dos_capas([10, 20, 30, 255], [200, 100, 50, 255]);
        let lienzo_antes = model.lienzo.clone();
        let ok = combinar_capa_abajo(&mut model, id_b);
        assert!(!ok, "no debe reportar éxito");
        assert_eq!(model.lienzo, lienzo_antes, "lienzo intacto");
        assert!(model.estado.contains("no hay capa debajo"));
    }

    #[test]
    fn combinar_capas_normales_aplana_a_la_de_arriba_opaca() {
        // Dos rasters opacos con blend Normal y opacidad 1.0: el composite
        // es exactamente la capa de arriba (la de abajo queda totalmente
        // cubierta). La merge debe producir un buffer de ese color.
        let (mut model, _id_b, id_a) =
            modelo_dos_capas([10, 20, 30, 255], [200, 100, 50, 255]);
        assert_eq!(model.lienzo.capas.len(), 2);
        let ok = combinar_capa_abajo(&mut model, id_a);
        assert!(ok);
        assert_eq!(model.lienzo.capas.len(), 1);
        let nueva = &model.lienzo.capas[0];
        let buf = model.almacen.obtener(nueva.contenido).unwrap();
        // 2×2 píxeles, todos el color de arriba.
        assert_eq!(buf.len(), 16);
        for px in buf.chunks_exact(4) {
            assert_eq!(px, &[200, 100, 50, 255]);
        }
        // El nombre conserva la genealogía con el separador ⊕.
        assert!(nueva.nombre.contains("⊕"), "nombre = {}", nueva.nombre);
        // Selección apuntó a la merged.
        assert_eq!(model.seleccionada, Some(nueva.id));
    }

    #[test]
    fn combinar_capa_con_opacidad_media_mezcla_50_50() {
        // Arriba semitransparente (α=128) sobre fondo opaco: el resultado
        // debe ser aprox. promedio. Tolerancia ±2 por el rounding del
        // compositor (premultiplicación + división).
        let (mut model, _id_b, id_a) =
            modelo_dos_capas([0, 0, 0, 255], [255, 255, 255, 255]);
        // Bajamos la opacidad de la capa de arriba a 0.5.
        let idx_a = model.lienzo.capas.iter().position(|c| c.id == id_a).unwrap();
        model.lienzo.capas[idx_a].opacidad = 0.5;
        let ok = combinar_capa_abajo(&mut model, id_a);
        assert!(ok);
        let nueva = &model.lienzo.capas[0];
        let buf = model.almacen.obtener(nueva.contenido).unwrap();
        for px in buf.chunks_exact(4) {
            for c in 0..3 {
                assert!(
                    (px[c] as i32 - 128).abs() <= 4,
                    "canal {} = {} no está cerca de 128",
                    c,
                    px[c]
                );
            }
            assert_eq!(px[3], 255);
        }
        // Crítico: la merged tiene opacidad 1.0 y blend Normal, no
        // heredando el 0.5 — el 0.5 ya quedó horneado en los píxeles.
        assert!((nueva.opacidad - 1.0).abs() < 1e-6);
        assert_eq!(nueva.blend, ModoFusion::Normal);
    }

    #[test]
    fn combinar_dos_mismos_pares_comparten_hash() {
        // Content-addressing: mergear el mismo par dos veces produce el
        // mismo hash en el almacén (la pintura es función de las capas).
        let (mut m1, _, id_a1) =
            modelo_dos_capas([12, 34, 56, 255], [78, 90, 12, 255]);
        let (mut m2, _, id_a2) =
            modelo_dos_capas([12, 34, 56, 255], [78, 90, 12, 255]);
        combinar_capa_abajo(&mut m1, id_a1);
        combinar_capa_abajo(&mut m2, id_a2);
        let h1 = m1.lienzo.capas[0].contenido;
        let h2 = m2.lienzo.capas[0].contenido;
        assert_eq!(h1, h2, "mismo composite ⇒ mismo hash");
    }

    #[test]
    fn msg_combinar_dispatcha_y_undo_restaura() {
        // El flujo completo por update: tras Combinar hay 1 capa, tras
        // Undo vuelven las 2 originales.
        let (mut model, id_b, id_a) =
            modelo_dos_capas([0, 0, 0, 255], [255, 255, 255, 255]);
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(model, Msg::Combinar(id_a), &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.hist.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 2);
        // Los Uuid originales vuelven (el historial guarda el Lienzo
        // entero, no rastrea hashes de buffers).
        let ids_post: Vec<Uuid> = model.lienzo.capas.iter().map(|c| c.id).collect();
        assert!(ids_post.contains(&id_b));
        assert!(ids_post.contains(&id_a));
    }

    #[test]
    fn hotkey_ctrl_e_emite_combinar() {
        let (model, _, id_a) = modelo_dos_capas([0; 4], [0; 4]);
        let mods = Modifiers { ctrl: true, ..Default::default() };
        let msg = hotkey_a_msg(&model, &ev_char("e", mods));
        assert!(matches!(msg, Some(Msg::Combinar(x)) if x == id_a));
        // Sin Ctrl, la `e` suelta cambia a la herramienta borrador
        // (Fase 46) — antes era no-op.
        let msg2 = hotkey_a_msg(&model, &ev_char("e", Modifiers::default()));
        assert!(matches!(
            msg2,
            Some(Msg::CambiarHerramienta(Herramienta::Borrador))
        ));
    }

    #[test]
    fn aplanar_con_cero_visibles_es_no_op() {
        let (mut model, ids) = modelo_n_capas(&[[10, 20, 30, 255]]);
        // Oculto la única capa que hay.
        let idx = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.id == ids[0])
            .unwrap();
        model.lienzo.capas[idx].visible = false;
        let lienzo_antes = model.lienzo.clone();
        let ok = aplanar_capas_visibles(&mut model);
        assert!(!ok);
        assert_eq!(model.lienzo, lienzo_antes);
        assert!(model.estado.contains("nada visible"));
    }

    #[test]
    fn aplanar_con_una_sola_visible_es_no_op() {
        let (mut model, _) = modelo_n_capas(&[[10, 20, 30, 255]]);
        let lienzo_antes = model.lienzo.clone();
        let ok = aplanar_capas_visibles(&mut model);
        assert!(!ok);
        assert_eq!(model.lienzo, lienzo_antes);
        assert!(model.estado.contains("una sola"));
    }

    #[test]
    fn aplanar_dos_visibles_da_una_capa_con_composite() {
        // Dos Normal/opacas: el composite es el color de arriba.
        let (mut model, _) =
            modelo_n_capas(&[[10, 20, 30, 255], [200, 100, 50, 255]]);
        let ok = aplanar_capas_visibles(&mut model);
        assert!(ok);
        assert_eq!(model.lienzo.capas.len(), 1);
        let buf = model.almacen.obtener(model.lienzo.capas[0].contenido).unwrap();
        for px in buf.chunks_exact(4) {
            assert_eq!(px, &[200, 100, 50, 255]);
        }
        // La merged hereda defaults Normal/1.0/visible.
        assert!((model.lienzo.capas[0].opacidad - 1.0).abs() < 1e-6);
        assert_eq!(model.lienzo.capas[0].blend, ModoFusion::Normal);
    }

    #[test]
    fn aplanar_preserva_hidden_intercalado_en_su_posicion_topologica() {
        // Lienzo de 4 capas en orden fondo→tope:
        //   c0 (v)  bg
        //   c1 (h)  hidA — entre dos visibles
        //   c2 (v)  fg
        //   c3 (h)  hidB — encima de la última visible
        // Esperado tras aplanar: [hidA, merged, hidB] (3 capas).
        let (mut model, ids) = modelo_n_capas(&[
            [10, 0, 0, 255],
            [0, 20, 0, 255],
            [0, 0, 30, 255],
            [40, 40, 40, 255],
        ]);
        // Marco c1 y c3 como hidden.
        for &id in &[ids[1], ids[3]] {
            let idx = model
                .lienzo
                .capas
                .iter()
                .position(|c| c.id == id)
                .unwrap();
            model.lienzo.capas[idx].visible = false;
        }
        let ok = aplanar_capas_visibles(&mut model);
        assert!(ok);
        // 4 originales − 2 visibles + 1 merged = 3 capas.
        assert_eq!(model.lienzo.capas.len(), 3);
        // Orden esperado: hidA (idx 0), merged (idx 1), hidB (idx 2).
        assert_eq!(model.lienzo.capas[0].id, ids[1]);
        assert!(model.lienzo.capas[1].nombre.starts_with("aplanado"));
        assert_eq!(model.lienzo.capas[2].id, ids[3]);
        // hidA y hidB siguen siendo invisibles.
        assert!(!model.lienzo.capas[0].visible);
        assert!(!model.lienzo.capas[2].visible);
    }

    #[test]
    fn aplanar_no_visible_arriba_de_todo_inserta_al_tope() {
        // Caso degenerado: una hidden ARRIBA de la última visible.
        // [v0 v, v1 v, hid v0 → no, hidden h]
        // Tras aplanar: [merged, hidden] (merged va donde estaba v1,
        // que era max_visible=1; hidden de 0..=1 = 0, así que insert_idx=0
        // — merged va a idx 0, hidden queda atrás al final).
        let (mut model, ids) = modelo_n_capas(&[
            [10, 0, 0, 255],
            [0, 20, 0, 255],
            [40, 40, 40, 255],
        ]);
        let idx_hidden = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.id == ids[2])
            .unwrap();
        model.lienzo.capas[idx_hidden].visible = false;
        let ok = aplanar_capas_visibles(&mut model);
        assert!(ok);
        assert_eq!(model.lienzo.capas.len(), 2);
        // merged va a 0 (no había hidden por debajo del top visible).
        assert!(model.lienzo.capas[0].nombre.starts_with("aplanado"));
        // La hidden queda en idx 1 (arriba).
        assert_eq!(model.lienzo.capas[1].id, ids[2]);
    }

    #[test]
    fn msg_aplanar_dispatcha_y_undo_restaura() {
        let (mut model, ids) =
            modelo_n_capas(&[[1, 2, 3, 255], [4, 5, 6, 255], [7, 8, 9, 255]]);
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(model, Msg::AplanarVisibles, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.hist.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 3);
        let ids_post: Vec<Uuid> = model.lienzo.capas.iter().map(|c| c.id).collect();
        for id in ids {
            assert!(ids_post.contains(&id));
        }
    }

    #[test]
    fn hotkey_ctrl_shift_e_emite_aplanar() {
        let m = modelo_minimo();
        let mods = Modifiers { ctrl: true, shift: true, ..Default::default() };
        let msg = hotkey_a_msg(&m, &ev_char("e", mods));
        assert!(matches!(msg, Some(Msg::AplanarVisibles)));
        // Ctrl+E (sin shift) sigue siendo Combinar(id), no AplanarVisibles.
        let solo_ctrl = hotkey_a_msg(
            &m,
            &ev_char("e", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(solo_ctrl, Some(Msg::Combinar(_))));
    }

    #[test]
    fn rotar_buffer_90_cw_mueve_top_left_a_top_right() {
        // src 2×3 con 6 colores distintos. Verifico el mapeo:
        //   src        dst (3×2)
        //   A B        E C A
        //   C D   →    F D B
        //   E F
        let src = vec![
            // row 0:           A             B
            10, 0, 0, 255,   20, 0, 0, 255,
            // row 1:           C             D
            30, 0, 0, 255,   40, 0, 0, 255,
            // row 2:           E             F
            50, 0, 0, 255,   60, 0, 0, 255,
        ];
        let out = rotar_buffer_90_cw(&src, 2, 3);
        // dst dims son 3×2.
        assert_eq!(out.len(), 24);
        // A en (0,0) → (2,0) en dst
        assert_eq!(px_at(&out, 3, 2, 0)[0], 10);
        // B en (1,0) → (2,1) en dst
        assert_eq!(px_at(&out, 3, 2, 1)[0], 20);
        // C en (0,1) → (1,0) en dst
        assert_eq!(px_at(&out, 3, 1, 0)[0], 30);
        // E en (0,2) → (0,0) en dst (top-left de dst era bottom-left de src)
        assert_eq!(px_at(&out, 3, 0, 0)[0], 50);
        // F en (1,2) → (0,1) en dst
        assert_eq!(px_at(&out, 3, 0, 1)[0], 60);
    }

    #[test]
    fn rotar_buffer_90_ccw_es_inversa_de_cw() {
        // Aplicar CW y luego CCW debe devolver el buffer original
        // bit-a-bit. Garantía para que "rotar a un lado y volver" no
        // pierda nada.
        let src = vec![
            // 4×3 con un patrón distinguible.
            1, 2, 3, 255,    4, 5, 6, 255,    7, 8, 9, 255,   10, 11, 12, 255,
            13, 14, 15, 255, 16, 17, 18, 255, 19, 20, 21, 255, 22, 23, 24, 255,
            25, 26, 27, 255, 28, 29, 30, 255, 31, 32, 33, 255, 34, 35, 36, 255,
        ];
        let cw = rotar_buffer_90_cw(&src, 4, 3);
        // cw quedó con dims 3×4. Aplicar CCW debe revertir a 4×3 idéntico.
        let regreso = rotar_buffer_90_ccw(&cw, 3, 4);
        assert_eq!(regreso, src);
    }

    #[test]
    fn rotar_buffer_90_cw_dos_veces_es_rotacion_180() {
        // CW + CW debe equivaler a espejar h + espejar v (rotación 180°).
        // Calculo ambos y comparo.
        let src = vec![
            10, 0, 0, 255,   20, 0, 0, 255,
            30, 0, 0, 255,   40, 0, 0, 255,
            50, 0, 0, 255,   60, 0, 0, 255,
        ];
        let dos_cw = {
            let una = rotar_buffer_90_cw(&src, 2, 3);
            // una es 3×2. CW de nuevo da 2×3.
            rotar_buffer_90_cw(&una, 3, 2)
        };
        // Construyo el espejado 180° vía buffer_relleno + manual:
        // src reversed-byte-wise (en grupos de 4) da el 180°.
        let mut esperado = vec![0u8; src.len()];
        for i in 0..(src.len() / 4) {
            let i_src = i * 4;
            let i_dst = ((src.len() / 4) - 1 - i) * 4;
            esperado[i_dst..i_dst + 4].copy_from_slice(&src[i_src..i_src + 4]);
        }
        assert_eq!(dos_cw, esperado);
    }

    #[test]
    fn rotar_lienzo_cw_intercambia_dimensiones() {
        let (mut model, _) =
            modelo_n_capas(&[[10, 20, 30, 255], [200, 100, 50, 255]]);
        // El lienzo era 2×2 después de modelo_n_capas. Tras rotar +90°
        // sigue siendo 2×2 (cuadrado), así que para verificar el swap
        // armo un lienzo 2×3 explícitamente.
        model.lienzo = Lienzo::nuevo(2, 3);
        // Cargo una capa raster cualquiera; lo que importa es la dim.
        let buf = buffer_relleno(2, 3, [100, 100, 100, 255]);
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("c", h);
        let id = cap.id;
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        let ok = rotar_lienzo(&mut model, true);
        assert!(ok);
        assert_eq!(model.lienzo.width, 3);
        assert_eq!(model.lienzo.height, 2);
    }

    #[test]
    fn rotar_lienzo_ccw_es_inversa_de_cw() {
        // CW seguido de CCW debe restaurar dims (y los buffers de las
        // capas son content-addressed: cada rotación inserta un nuevo
        // hash, pero el FINAL coincide con el hash original).
        let mut model = modelo_minimo();
        // Empiezo con 2×3.
        model.lienzo = Lienzo::nuevo(2, 3);
        let buf = vec![
            10, 0, 0, 255,   20, 0, 0, 255,
            30, 0, 0, 255,   40, 0, 0, 255,
            50, 0, 0, 255,   60, 0, 0, 255,
        ];
        let h_inicial = model.almacen.insertar(buf);
        let cap = Capa::raster("c", h_inicial);
        let id = cap.id;
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        // CW: dims 2×3 → 3×2.
        rotar_lienzo(&mut model, true);
        assert_eq!((model.lienzo.width, model.lienzo.height), (3, 2));
        // CCW: vuelve a 2×3.
        rotar_lienzo(&mut model, false);
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 3));
        // El hash final debe igualar el inicial (content-addressing).
        let h_final = model.lienzo.capa(id).unwrap().contenido;
        assert_eq!(h_final, h_inicial);
    }

    #[test]
    fn rotar_lienzo_sin_capas_es_no_op() {
        let mut model = modelo_minimo();
        model.lienzo.capas.clear();
        let ok = rotar_lienzo(&mut model, true);
        assert!(!ok);
        assert!(model.estado.contains("nada que rotar"));
    }

    #[test]
    fn msg_rotar_lienzo_dispatcha_y_undo_restaura_dims() {
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(2, 3);
        let buf = buffer_relleno(2, 3, [50, 50, 50, 255]);
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("c", h);
        model.lienzo.apilar(cap);
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::RotarLienzo { cw: true },
            &Handle::for_test(),
        );
        assert_eq!((model.lienzo.width, model.lienzo.height), (3, 2));
        assert_eq!(model.hist.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 3));
    }

    // ---- Fase 31: auto-trim del lienzo --------------------------------------

    #[test]
    fn bbox_devuelve_none_si_todo_transparente() {
        // Buffer 3×3 todo a alfa=0.
        let buf = vec![0u8; 3 * 3 * 4];
        assert_eq!(bbox_no_transparente(&buf, 3, 3), None);
    }

    #[test]
    fn bbox_un_solo_pixel_devuelve_rect_de_un_pixel() {
        // 3×3, todo transparente excepto el píxel central (1, 1).
        let mut buf = vec![0u8; 3 * 3 * 4];
        let i = ((1 * 3 + 1) * 4) as usize;
        buf[i] = 100;
        buf[i + 1] = 200;
        buf[i + 2] = 50;
        buf[i + 3] = 255;
        let bb = bbox_no_transparente(&buf, 3, 3).unwrap();
        // Half-open: el píxel (1,1) da (1, 1, 2, 2).
        assert_eq!(bb, (1, 1, 2, 2));
    }

    #[test]
    fn bbox_full_alpha_cubre_el_lienzo_entero() {
        // Buffer 2×3 todo opaco.
        let buf = buffer_relleno(2, 3, [10, 20, 30, 255]);
        assert_eq!(bbox_no_transparente(&buf, 2, 3), Some((0, 0, 2, 3)));
    }

    #[test]
    fn bbox_ignora_pixeles_alfa_cero_aun_con_rgb_no_cero() {
        // Photoshop/PSD a veces deja "pixel data" con alfa=0 — no son
        // tinta visible. El bbox debe ignorarlos.
        let mut buf = vec![0u8; 4 * 4 * 4];
        // Toda la columna izquierda: RGB no-cero pero alfa=0.
        for y in 0..4 {
            let i = (y * 4 * 4) as usize;
            buf[i] = 200;
            buf[i + 3] = 0;
        }
        // Píxel (3, 2) opaco con alfa=255.
        let j = ((2 * 4 + 3) * 4) as usize;
        buf[j + 3] = 255;
        let bb = bbox_no_transparente(&buf, 4, 4).unwrap();
        assert_eq!(bb, (3, 2, 4, 3));
    }

    #[test]
    fn recortar_buffer_extrae_subrect_correcto() {
        // 4×3 con un patrón de gradiente lineal en R.
        let mut buf = Vec::with_capacity(4 * 3 * 4);
        for i in 0..(4 * 3) {
            buf.extend_from_slice(&[i as u8 * 20, 0, 0, 255]);
        }
        // Recorto el rect (1, 1, 4, 3) → 3×2 píxeles.
        // Esperado: filas 1 y 2, columnas 1, 2, 3 del src.
        let out = recortar_buffer(&buf, 4, 1, 1, 4, 3);
        assert_eq!(out.len(), 3 * 2 * 4);
        // Píxel (0, 0) del out = píxel (1, 1) del src = idx 5 lineal.
        // R = 5 * 20 = 100.
        assert_eq!(out[0], 100);
        // Píxel (2, 1) del out = píxel (3, 2) del src = idx 11. R = 220.
        let i = (1 * 3 + 2) * 4;
        assert_eq!(out[i], 220);
    }

    #[test]
    fn autotrim_no_op_si_lienzo_todo_opaco() {
        // El bbox cubre todo el lienzo → no-op + estado.
        let (mut model, _) = modelo_n_capas(&[[10, 20, 30, 255]]);
        // `modelo_n_capas` no recompone — forzamos para que `model.imagen`
        // exista y `recortar_lienzo_a_visible` no caiga en la rama de
        // "no hay composite".
        aplicar_y_recomponer(&mut model);
        let dims_antes = (model.lienzo.width, model.lienzo.height);
        let ok = recortar_lienzo_a_visible(&mut model);
        assert!(!ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), dims_antes);
        assert!(model.estado.contains("ya está justo"));
    }

    #[test]
    fn autotrim_no_op_si_lienzo_todo_transparente() {
        // Capa única con alfa=0 → bbox = None → no-op con mensaje.
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(3, 3);
        let buf = buffer_relleno(3, 3, [100, 100, 100, 0]); // RGB pero alfa 0
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("trasparent", h);
        model.lienzo.apilar(cap);
        // Forzamos recompose para llenar model.imagen.
        aplicar_y_recomponer(&mut model);
        let ok = recortar_lienzo_a_visible(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("vacío"));
    }

    #[test]
    fn autotrim_recorta_lienzo_a_la_region_opaca() {
        // Lienzo 4×4 todo transparente excepto un rect interior 2×2
        // (filas 1-2, cols 1-2). Tras autotrim el lienzo debería
        // reducirse a 2×2.
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(4, 4);
        // Buffer 4×4 con sólo (1..3, 1..3) opaco rojo.
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 1..3 {
            for x in 1..3 {
                let i = (y * 4 + x) * 4;
                buf[i] = 200;
                buf[i + 3] = 255;
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("isla", h);
        let id = cap.id;
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        let ok = recortar_lienzo_a_visible(&mut model);
        assert!(ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        // El buffer recortado de la capa: todos los 4 píxeles son la isla
        // roja opaca.
        let nueva_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(nueva_h).unwrap();
        assert_eq!(buf_post.len(), 2 * 2 * 4);
        for px in buf_post.chunks_exact(4) {
            assert_eq!(px, &[200, 0, 0, 255]);
        }
    }

    #[test]
    fn msg_autotrim_dispatcha_y_undo_restaura() {
        // El flujo entero por update: autotrim baja dims, Undo las
        // restaura.
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(4, 4);
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 1..3 {
            for x in 1..3 {
                let i = (y * 4 + x) * 4;
                buf[i + 3] = 255;
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("isla", h);
        model.lienzo.apilar(cap);
        aplicar_y_recomponer(&mut model);
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(model, Msg::AutotrimLienzo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        assert_eq!(model.hist.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (4, 4));
    }

    #[test]
    fn ajustar_brillo_suma_delta_y_marca_stale() {
        let (mut model, _madre, deriv) =
            modelo_con_derivada(OpLocal::Brillo { delta: 0.1 });
        let cambio = ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::BrilloDelta,
            0.2,
        );
        assert!(cambio);
        // Delta queda en 0.3 con clamp [-1, 1].
        let capa = model.lienzo.capa(deriv).unwrap();
        match &capa.origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Brillo { delta }),
                estado,
                ..
            } => {
                assert!((delta - 0.3).abs() < 1e-6);
                assert_eq!(*estado, Frescura::Stale);
            }
            other => panic!("esperaba Brillo derivada: {:?}", other),
        }
    }

    #[test]
    fn curva_press_lejano_inserta_punto_y_engancha() {
        let (mut model, _madre, deriv) =
            modelo_con_derivada(OpLocal::curvas_identidad());
        // Canvas 100×100; click al centro (0.5, 0.5) — lejos de ambos
        // extremos (dist ≈ 0.707 > umbral) ⇒ inserta un 3er punto.
        let ok = curva_press(&mut model, deriv, 50.0, 50.0, 100.0, 100.0);
        assert!(ok);
        let pts = puntos_de(&model, deriv);
        assert_eq!(pts.len(), 3, "debió insertar un punto medio");
        assert!((pts[1].0 - 0.5).abs() < 1e-3 && (pts[1].1 - 0.5).abs() < 1e-3);
        // Drag enganchado al punto recién insertado (idx 1).
        assert_eq!(model.curva_arrastrando.map(|d| d.idx), Some(1));
    }

    #[test]
    fn curva_press_cercano_engancha_sin_insertar() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::curvas_identidad());
        // Click cerca del extremo negro (0,0) → mapea a pantalla (0,100).
        let ok = curva_press(&mut model, deriv, 2.0, 98.0, 100.0, 100.0);
        assert!(ok);
        let pts = puntos_de(&model, deriv);
        assert_eq!(pts.len(), 2, "no debió insertar — enganchó el extremo");
        assert_eq!(model.curva_arrastrando.map(|d| d.idx), Some(0));
    }

    #[test]
    fn curva_arrastrar_mueve_el_punto_activo() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::curvas_identidad());
        curva_press(&mut model, deriv, 50.0, 50.0, 100.0, 100.0);
        // dy=-20 px sobre rh=100 ⇒ +0.2 en y-curva (sube la salida).
        let ok = curva_arrastrar(&mut model, deriv, 0.0, -20.0);
        assert!(ok);
        let pts = puntos_de(&model, deriv);
        assert!((pts[1].1 - 0.7).abs() < 1e-3, "y={}", pts[1].1);
    }

    #[test]
    fn curva_arrastrar_extremo_fija_x_en_cero() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::curvas_identidad());
        curva_press(&mut model, deriv, 2.0, 98.0, 100.0, 100.0); // engancha idx 0
        // Intentar moverlo en x (+30 px) no debe sacarlo de x=0.
        curva_arrastrar(&mut model, deriv, 30.0, 0.0);
        let pts = puntos_de(&model, deriv);
        assert_eq!(pts[0].0, 0.0, "el extremo negro mantiene x=0");
    }

    #[test]
    fn curva_arrastrar_sin_drag_es_noop() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::curvas_identidad());
        // Sin press previo no hay `curva_arrastrando` ⇒ no-op.
        assert!(!curva_arrastrar(&mut model, deriv, 10.0, 10.0));
        assert_eq!(puntos_de(&model, deriv).len(), 2);
    }

    #[test]
    fn curva_reset_vuelve_a_identidad() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::curvas_identidad());
        curva_press(&mut model, deriv, 50.0, 50.0, 100.0, 100.0);
        curva_arrastrar(&mut model, deriv, 0.0, -30.0);
        assert_eq!(puntos_de(&model, deriv).len(), 3);
        let ok = curva_reset(&mut model, deriv);
        assert!(ok);
        let pts = puntos_de(&model, deriv);
        assert_eq!(pts, vec![(0.0, 0.0), (1.0, 1.0)]);
    }

    #[test]
    fn curva_press_sobre_capa_no_curva_es_noop() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::Brillo { delta: 0.0 });
        assert!(!curva_press(&mut model, deriv, 50.0, 50.0, 100.0, 100.0));
        assert!(model.curva_arrastrando.is_none());
    }

    #[test]
    fn ajustar_brillo_clamp_a_min_1_max_1() {
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::Brillo { delta: 0.0 });
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::BrilloDelta,
            10.0,
        );
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Brillo { delta }),
                ..
            } => assert!((delta - 1.0).abs() < 1e-6),
            _ => unreachable!(),
        }
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::BrilloDelta,
            -10.0,
        );
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Brillo { delta }),
                ..
            } => assert!((delta - (-1.0)).abs() < 1e-6),
            _ => unreachable!(),
        }
    }

    #[test]
    fn ajustar_param_no_concuerda_con_op_es_no_op() {
        // ParametroSlider::BrilloDelta sobre una capa con OpLocal::Saturacion
        // no debe mutar nada.
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::Saturacion { factor: 1.0 });
        let cambio = ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::BrilloDelta,
            0.5,
        );
        assert!(!cambio);
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Saturacion { factor }),
                ..
            } => assert!((factor - 1.0).abs() < 1e-6, "saturación intacta"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn ajustar_param_sobre_raster_es_no_op() {
        // No hay derivada acá — el modelo_minimo es raster pura.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let cambio = ajustar_parametro_derivada(
            &mut model,
            id,
            ParametroSlider::BrilloDelta,
            0.5,
        );
        assert!(!cambio);
    }