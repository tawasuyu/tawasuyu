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
    fn blend_anterior_es_inverso_de_siguiente() {
        // Probar 5 modos elegidos a lo largo del ciclo para confirmar
        // que las dos funciones realmente son inversas — protege contra
        // que alguien agregue un modo al `siguiente` y se olvide del otro
        // (ahora son derivados del mismo `CICLO_BLEND` así que es
        // imposible, pero el test guarda la invariante).
        for &m in [
            ModoFusion::Normal,
            ModoFusion::Multiplicar,
            ModoFusion::LuzSuave,
            ModoFusion::HslColor,
            ModoFusion::Disolver,
        ]
        .iter()
        {
            assert_eq!(blend_anterior(siguiente_blend(m)), m);
            assert_eq!(siguiente_blend(blend_anterior(m)), m);
        }
        // El ciclo debe rotar exactamente con la cantidad de variantes:
        // aplicar `siguiente` CICLO_BLEND.len() veces es la identidad.
        let mut x = ModoFusion::Normal;
        for _ in 0..CICLO_BLEND.len() {
            x = siguiente_blend(x);
        }
        assert_eq!(x, ModoFusion::Normal);
    }

    #[test]
    fn hotkey_delete_elimina_capa_seleccionada() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Delete, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::Eliminar(x)) if x == id));
    }

    #[test]
    fn hotkey_ctrl_d_duplica() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let mods = Modifiers { ctrl: true, ..Default::default() };
        let msg = hotkey_a_msg(&m, &ev_char("d", mods));
        assert!(matches!(msg, Some(Msg::Duplicar(x)) if x == id));
    }

    #[test]
    fn hotkey_v_toggle_visible() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_char("v", Modifiers::default()));
        assert!(matches!(msg, Some(Msg::ToggleVisible(x)) if x == id));
    }

    #[test]
    fn hotkey_b_y_shift_b_son_inversos_de_dispatch() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let fwd = hotkey_a_msg(&m, &ev_char("b", Modifiers::default()));
        assert!(matches!(fwd, Some(Msg::CiclarBlend(x)) if x == id));
        let bwd = hotkey_a_msg(
            &m,
            &ev_char("b", Modifiers { shift: true, ..Default::default() }),
        );
        assert!(matches!(bwd, Some(Msg::CiclarBlendInverso(x)) if x == id));
    }

    #[test]
    fn hotkey_brackets_bump_opacidad_signo_correcto() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let baja = hotkey_a_msg(&m, &ev_char("[", Modifiers::default()));
        let sube = hotkey_a_msg(&m, &ev_char("]", Modifiers::default()));
        match baja {
            Some(Msg::BumpOpacidad(x, d)) if x == id && (d + 0.1).abs() < 1e-6 => {}
            other => panic!("[ no dió −0.1: {other:?}", other = other.is_some()),
        }
        match sube {
            Some(Msg::BumpOpacidad(x, d)) if x == id && (d - 0.1).abs() < 1e-6 => {}
            other => panic!("] no dió +0.1: {other:?}", other = other.is_some()),
        }
    }

    #[test]
    fn hotkey_ctrl_s_y_ctrl_shift_s_exportan_distinto_formato() {
        let m = modelo_minimo();
        let png = hotkey_a_msg(
            &m,
            &ev_char("s", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(png, Some(Msg::Exportar(FormatoExport::Png))));
        let webp = hotkey_a_msg(
            &m,
            &ev_char(
                "s",
                Modifiers { ctrl: true, shift: true, ..Default::default() },
            ),
        );
        assert!(matches!(webp, Some(Msg::Exportar(FormatoExport::Webp))));
    }

    #[test]
    fn hotkey_sin_seleccion_no_dispara_msg_de_capa() {
        let mut m = modelo_minimo();
        m.seleccionada = None;
        // Sin selección, Delete/V/B/[]/Ctrl+D no producen nada.
        for ev in [
            ev_named(NamedKey::Delete, Modifiers::default()),
            ev_char("v", Modifiers::default()),
            ev_char("b", Modifiers::default()),
            ev_char("[", Modifiers::default()),
            ev_char("]", Modifiers::default()),
            ev_char("d", Modifiers { ctrl: true, ..Default::default() }),
        ] {
            assert!(hotkey_a_msg(&m, &ev).is_none());
        }
        // Pero Ctrl+S sí — exporta el lienzo entero, no depende de capa.
        let png = hotkey_a_msg(
            &m,
            &ev_char("s", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(png, Some(Msg::Exportar(FormatoExport::Png))));
    }

    #[test]
    fn hotkey_f2_inicia_renombrado() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::F2, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::IniciarRenombrar(x)) if x == id));
    }

    #[test]
    fn renombrar_precarga_nombre_y_lo_actualiza_en_confirmar() {
        // Simulo el flujo entero del update sin la UI: IniciarRenombrar
        // crea el TextInputState con el nombre actual; las teclas lo
        // editan; Confirmar lo escribe a la capa.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // Renombrar a "fondo nuevo" — la app pone el nombre actual y el
        // user va al final y tipea. Acá lo simplifico: set_text directo.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        assert!(model.renombrando.is_some());
        // El input arranca con el nombre actual de la capa.
        let (_, input) = model.renombrando.as_ref().unwrap();
        assert_eq!(input.text(), "c");
        // Edito directamente vía set_text — equivale a borrar + tipear.
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("fondo nuevo");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert!(model.renombrando.is_none());
        assert_eq!(
            model.lienzo.capas.iter().find(|c| c.id == id).unwrap().nombre,
            "fondo nuevo"
        );
    }

    #[test]
    fn cancelar_renombrado_no_cambia_el_nombre() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let nombre_original = model
            .lienzo
            .capas
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .nombre
            .clone();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("intento descartado");
        }
        model = <Tullpu as App>::update(model, Msg::CancelarRenombrar, &Handle::for_test());
        assert!(model.renombrando.is_none());
        assert_eq!(
            model.lienzo.capas.iter().find(|c| c.id == id).unwrap().nombre,
            nombre_original
        );
    }

    #[test]
    fn confirmar_renombrado_vacio_no_pisa_el_nombre() {
        // Un input vacío al confirmar no es un nombre válido (rompería
        // la UX — la fila quedaría sin etiqueta). El update lo descarta y
        // mantiene el nombre original.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("   ");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(
            model.lienzo.capas.iter().find(|c| c.id == id).unwrap().nombre,
            "c"
        );
    }

    #[test]
    fn ajustar_dims_iguales_devuelve_sin_tocar() {
        let src = vec![1, 2, 3, 4, 5, 6, 7, 8]; // 1×2 px Rgba8
        let copia = src.clone();
        let out = ajustar_a_lienzo(src, 1, 2, 1, 2).expect("dims iguales");
        assert_eq!(out, copia);
    }

    #[test]
    fn ajustar_contain_pad_transparente_centrado() {
        // 100×50 (2:1) → 200×200: cabe perfecto a 200×100, padding vertical
        // de 50 px arriba y abajo. Verifico que las esquinas son
        // transparentes y que la franja del medio tiene color.
        let mut src = Vec::with_capacity(100 * 50 * 4);
        for _ in 0..(100 * 50) {
            src.extend_from_slice(&[200, 100, 50, 255]); // naranja opaco
        }
        let out = ajustar_a_lienzo(src, 100, 50, 200, 200).expect("ajuste ok");
        assert_eq!(out.len(), 200 * 200 * 4);

        // Esquina superior izquierda: en el pad → transparente.
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        // Píxel (100, 100) ≈ centro → opaco con color cercano al naranja.
        let i = (100 * 200 + 100) * 4;
        assert_eq!(out[i + 3], 255, "centro opaco");
        assert!(out[i] > 100, "rojo presente: {}", out[i]);
        // Esquina inferior derecha: en el pad → transparente.
        let j = (199 * 200 + 199) * 4;
        assert_eq!(&out[j..j + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn ajustar_dst_cero_devuelve_none() {
        let src = vec![0u8; 4];
        assert!(ajustar_a_lienzo(src, 1, 1, 0, 1).is_none());
    }

    #[test]
    fn es_imagen_soportada_filtra_extensiones() {
        assert!(es_imagen_soportada(Path::new("foo.png")));
        assert!(es_imagen_soportada(Path::new("foo.PNG")));
        assert!(es_imagen_soportada(Path::new("foo.jpg")));
        assert!(es_imagen_soportada(Path::new("foo.jpeg")));
        assert!(!es_imagen_soportada(Path::new("foo.psd")));
        assert!(!es_imagen_soportada(Path::new("foo.txt")));
        assert!(!es_imagen_soportada(Path::new("foo")));
    }

    // ---- Fase 23: undo/redo --------------------------------------------------

    #[test]
    fn hotkey_ctrl_z_y_variantes_redo_emiten_msg_correcto() {
        let m = modelo_minimo();
        // Ctrl+Z = undo.
        let undo = hotkey_a_msg(
            &m,
            &ev_char("z", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(undo, Some(Msg::Undo)));
        // Ctrl+Shift+Z = redo.
        let redo_shift = hotkey_a_msg(
            &m,
            &ev_char(
                "z",
                Modifiers { ctrl: true, shift: true, ..Default::default() },
            ),
        );
        assert!(matches!(redo_shift, Some(Msg::Redo)));
        // Ctrl+Y = redo (alias).
        let redo_y = hotkey_a_msg(
            &m,
            &ev_char("y", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(redo_y, Some(Msg::Redo)));
    }

    #[test]
    fn undo_sin_historial_anota_estado_pero_no_panickea() {
        // Modelo recién armado: historial tiene 1 sola entrada (la inicial).
        // Un Undo no debería hacer nada y el estado debe reflejarlo.
        let mut model = modelo_minimo();
        let lienzo_antes = model.lienzo.clone();
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo, lienzo_antes, "lienzo intacto");
        assert!(model.estado.contains("nada que deshacer"));
        assert_eq!(model.hist.cursor(), 0);
    }

    #[test]
    fn undo_revierte_toggle_visible() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let visible_original = model.lienzo.capa(id).unwrap().visible;
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, !visible_original);
        assert_eq!(model.hist.len(), 2);
        // Undo: volvemos al estado anterior.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, visible_original);
        // Redo: re-aplicamos el toggle.
        model = <Tullpu as App>::update(model, Msg::Redo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, !visible_original);
    }

    #[test]
    fn nueva_mutacion_tras_undo_trunca_la_rama_de_redo() {
        // Mutación 1, mutación 2, undo (vuelvo a 1), mutación 3: la rama 2
        // queda descartada y un redo posterior debe ser no-op.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // M1: invertir visibilidad
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        // M2: invertirla de nuevo (la deja igual al estado original).
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        assert_eq!(model.hist.len(), 3);
        assert_eq!(model.hist.cursor(), 2);
        // Undo: vuelvo a M1.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.hist.cursor(), 1);
        // M3: ciclar blend → debe truncar M2.
        model = <Tullpu as App>::update(model, Msg::CiclarBlend(id), &Handle::for_test());
        assert_eq!(model.hist.len(), 3, "M2 fue truncada");
        assert_eq!(model.hist.cursor(), 2);
        // Redo ahora no tiene a dónde ir.
        let snapshot = model.lienzo.clone();
        model = <Tullpu as App>::update(model, Msg::Redo, &Handle::for_test());
        assert_eq!(model.lienzo, snapshot);
        assert!(model.estado.contains("nada que rehacer"));
    }

    #[test]
    fn bump_opacidad_coalesce_drag_en_una_sola_entrada() {
        // Simulo un drag del slider: 50 BumpOpacidad consecutivas sobre la
        // misma capa. El historial debe crecer en 1 sola entrada (la final).
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let len_inicial = model.hist.len();
        for _ in 0..50 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id, -0.01),
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.hist.len(),
            len_inicial + 1,
            "el drag entero debe coalesce a 1 snapshot"
        );
        // Un solo undo revierte el drag completo.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        let cap = model.lienzo.capa(id).unwrap();
        assert!(
            (cap.opacidad - 1.0).abs() < 1e-6,
            "opacidad volvió a 1.0, no quedó atrapada a medio camino: {}",
            cap.opacidad
        );
    }

    #[test]
    fn coalesce_no_cruza_entre_capas_distintas() {
        // Drag de opacidad sobre capa A y luego sobre capa B no deben
        // colapsar en la misma entrada — son operaciones independientes.
        let mut model = modelo_minimo();
        let id_a = model.seleccionada.unwrap();
        // Agrego una segunda capa raster para tener dos targets distintos.
        let mut almacen2 = std::mem::replace(&mut model.almacen, AlmacenEnMemoria::nuevo());
        let h_b = almacen2.insertar(vec![1u8; 4 * 4 * 4]);
        model.almacen = almacen2;
        let cap_b = Capa::raster("b", h_b);
        let id_b = cap_b.id;
        model.lienzo.apilar(cap_b);
        // Forzamos un snapshot manual del estado post-agregado (simulando
        // que la capa B vino vía Agregar/Eliminar — para este test ad-hoc
        // basta con pushear directo).
        pushear_snapshot(&mut model, None);
        let base = model.hist.len();

        // Drag sobre A
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id_a, -0.05),
                &Handle::for_test(),
            );
        }
        assert_eq!(model.hist.len(), base + 1);

        // Drag sobre B (capa distinta → no coalesce con el de A)
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id_b, -0.05),
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.hist.len(),
            base + 2,
            "drag en B agrega entrada propia"
        );
    }

    #[test]
    fn historial_capado_descarta_entradas_viejas() {
        // Fuerzo HIST_CAP+5 snapshots no-coalescables (sin etiqueta).
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        for _ in 0..(HIST_CAP + 5) {
            model = <Tullpu as App>::update(
                model,
                Msg::ToggleVisible(id),
                &Handle::for_test(),
            );
        }
        assert_eq!(model.hist.len(), HIST_CAP);
        assert_eq!(model.hist.cursor(), HIST_CAP - 1);
    }

    #[test]
    fn undo_de_eliminar_resucita_la_capa() {
        // Una capa eliminada debe volver al hacer Ctrl+Z. La selección se
        // reajusta a la capa restaurada (única en el lienzo).
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert_eq!(model.lienzo.capas.len(), 1);
        model = <Tullpu as App>::update(model, Msg::Eliminar(id), &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 0);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.lienzo.capas[0].id, id);
        // Ajusta_seleccion_tras_restaurar la reasigna ya que tras Eliminar
        // la habíamos blanqueado.
        assert_eq!(model.seleccionada, Some(id));
    }

    #[test]
    fn confirmar_renombrar_vacio_no_genera_snapshot() {
        // El path "input vacío" no muta el nombre → no debe ensuciar el
        // historial con una entrada idéntica.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let len_inicial = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("   "); // whitespace only — descartado por update
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(model.hist.len(), len_inicial);
    }

    // ---- Fase 24: zoom y pan --------------------------------------------------

    #[test]
    fn transform_lienzo_fit_centra_imagen_en_zoom_1() {
        // Imagen 100×100 en un rect 200×200 → s_fit=2, dw=200, off=0,0.
        let (s, off_x, off_y) = transform_lienzo(100, 100, 200.0, 200.0, 1.0, 0.0, 0.0)
            .expect("ok");
        assert!((s - 2.0).abs() < 1e-9);
        assert!(off_x.abs() < 1e-9);
        assert!(off_y.abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_aspect_distinto_pad_simétrico() {
        // Imagen 100×50 (2:1) en rect 200×200: s_fit=min(2, 4)=2, dw=200,
        // dh=100 → off_y = (200-100)/2 = 50, off_x = 0.
        let (s, off_x, off_y) = transform_lienzo(100, 50, 200.0, 200.0, 1.0, 0.0, 0.0)
            .expect("ok");
        assert!((s - 2.0).abs() < 1e-9);
        assert!(off_x.abs() < 1e-9);
        assert!((off_y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_factor_zoom_2_duplica_y_descentra() {
        // Imagen 100×100 fit en 200×200 con zoom=2: s=4, dw=400, off=-100,-100.
        // (la imagen "se sale" del rect — el clip se encarga en paint).
        let (s, off_x, off_y) = transform_lienzo(100, 100, 200.0, 200.0, 2.0, 0.0, 0.0)
            .expect("ok");
        assert!((s - 4.0).abs() < 1e-9);
        assert!((off_x + 100.0).abs() < 1e-9);
        assert!((off_y + 100.0).abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_pan_solo_traslada() {
        // Cualquier pan se suma directo al offset sin afectar la escala.
        let (s_a, ax, ay) = transform_lienzo(100, 100, 200.0, 200.0, 1.5, 0.0, 0.0).unwrap();
        let (s_b, bx, by) = transform_lienzo(100, 100, 200.0, 200.0, 1.5, 17.0, -23.0).unwrap();
        assert!((s_a - s_b).abs() < 1e-9);
        assert!((bx - ax - 17.0).abs() < 1e-9);
        assert!((by - ay + 23.0).abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_dims_cero_devuelve_none() {
        assert!(transform_lienzo(0, 10, 100.0, 100.0, 1.0, 0.0, 0.0).is_none());
        assert!(transform_lienzo(10, 10, 0.0, 100.0, 1.0, 0.0, 0.0).is_none());
        assert!(transform_lienzo(10, 10, 100.0, -1.0, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn zoom_a_cursor_mantiene_el_pixel_bajo_el_cursor_fijo() {
        // Imagen 100×100, rect 200×200, zoom_old=1 → s=2, top-left=(0,0).
        // Cursor en (50, 60) → píxel-imagen (25, 30).
        // Zoom a 2: s_new=4, queremos top-left tal que (25,30) caiga en (50,60):
        // tx_new = 50 - 25*4 = -50, ty_new = 60 - 30*4 = -60.
        // dw=400, dh=400 → centered_off = (200-400)/2 = -100.
        // pan = tx_new - centered_off = -50 - (-100) = 50.
        let rect = PaintRect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let (pan_x, pan_y) =
            pan_para_zoom_a_cursor(100, 100, rect, 50.0, 60.0, 1.0, 2.0, 0.0, 0.0);
        assert!((pan_x - 50.0).abs() < 1e-3, "pan_x = {}", pan_x);
        // píxel-imagen y=30 → tx_new=60-30*4=-60, centered_off_y=-100,
        // pan_y = -60 - (-100) = 40.
        assert!((pan_y - 40.0).abs() < 1e-3, "pan_y = {}", pan_y);

        // Verificación cruzada: aplico el transform y reviso que (50,60)
        // corresponde a (25, 30) en coords-imagen al zoom 2.
        let (s_new, off_x, off_y) =
            transform_lienzo(100, 100, rect.w, rect.h, 2.0, pan_x, pan_y).unwrap();
        let tx = rect.x as f64 + off_x;
        let ty = rect.y as f64 + off_y;
        let ix = (50.0 - tx) / s_new;
        let iy = (60.0 - ty) / s_new;
        assert!((ix - 25.0).abs() < 1e-3, "ix = {}", ix);
        assert!((iy - 30.0).abs() < 1e-3, "iy = {}", iy);
    }

    #[test]
    fn dentro_de_rect_es_inclusive_en_bordes() {
        let r = PaintRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        assert!(dentro_de_rect(r, 10.0, 20.0));
        assert!(dentro_de_rect(r, 110.0, 70.0));
        assert!(dentro_de_rect(r, 60.0, 45.0));
        assert!(!dentro_de_rect(r, 9.99, 50.0));
        assert!(!dentro_de_rect(r, 60.0, 70.01));
        assert!(!dentro_de_rect(r, 110.01, 45.0));
    }

    #[test]
    fn msg_zoom_aplica_clamp_min_max() {
        // factor_zoom inicial = 1.0. Mult = 0.0001 → clamp a ZOOM_MIN.
        let mut model = modelo_minimo();
        model = <Tullpu as App>::update(
            model,
            Msg::Zoom { mult: 0.0001, ancla: None },
            &Handle::for_test(),
        );
        assert!((model.factor_zoom - ZOOM_MIN).abs() < 1e-6);
        // Y al revés: mult grande → ZOOM_MAX.
        model = <Tullpu as App>::update(
            model,
            Msg::Zoom { mult: 1e6, ancla: None },
            &Handle::for_test(),
        );
        assert!((model.factor_zoom - ZOOM_MAX).abs() < 1e-6);
    }

    #[test]
    fn msg_pan_acumula_offsets() {
        let mut model = modelo_minimo();
        model = <Tullpu as App>::update(model, Msg::Pan(10.0, -5.0), &Handle::for_test());
        model = <Tullpu as App>::update(model, Msg::Pan(3.0, 7.0), &Handle::for_test());
        assert!((model.pan_x - 13.0).abs() < 1e-6);
        assert!((model.pan_y - 2.0).abs() < 1e-6);
    }

    #[test]
    fn msg_reset_vista_restaura_zoom_y_pan_default() {
        let mut model = modelo_minimo();
        model.factor_zoom = 3.5;
        model.pan_x = 42.0;
        model.pan_y = -17.0;
        model = <Tullpu as App>::update(model, Msg::ResetVista, &Handle::for_test());
        assert!((model.factor_zoom - 1.0).abs() < 1e-6);
        assert_eq!(model.pan_x, 0.0);
        assert_eq!(model.pan_y, 0.0);
    }

    #[test]
    fn hotkey_cero_emite_reset_vista() {
        let model = modelo_minimo();
        let msg = hotkey_a_msg(&model, &ev_char("0", Modifiers::default()));
        assert!(matches!(msg, Some(Msg::ResetVista)));
        // Con Ctrl no — el 0 estándar es sin modificador.
        let ctrl0 = hotkey_a_msg(
            &model,
            &ev_char("0", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(ctrl0, None));
    }

    #[test]
    fn recoger_color_pixel_central_a_zoom_1() {
        // Imagen 4×4 en rect 200×200 → s_fit = 50; cada píxel ocupa 50 px.
        // Click en (75, 25) cae en x=1, y=0 → R=60, G=0, B=17, α=255.
        let buf = buffer_patron_4x4();
        let col =
            recoger_color_en(&buf, 4, 4, 75.0, 25.0, 200.0, 200.0, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(col, [60, 0, 17, 255], "(1, 0) esperado");
        // Click en (125, 175) cae en x=2, y=3 → R=120, G=180, B=17, α=255.
        let col2 =
            recoger_color_en(&buf, 4, 4, 125.0, 175.0, 200.0, 200.0, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(col2, [120, 180, 17, 255], "(2, 3) esperado");
    }

    #[test]
    fn recoger_color_fuera_de_imagen_devuelve_none() {
        // Imagen 4×4 en rect 200×100 fit-contain → s=25, dw=100, off_x=50
        // (la imagen está centrada con bandas a izquierda y derecha). Un
        // click en x=10 cae en la banda transparente → fuera de la imagen.
        let buf = buffer_patron_4x4();
        assert!(recoger_color_en(&buf, 4, 4, 10.0, 50.0, 200.0, 100.0, 1.0, 0.0, 0.0).is_none());
        // También fuera por arriba.
        assert!(recoger_color_en(&buf, 4, 4, 100.0, -5.0, 200.0, 100.0, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn recoger_color_respeta_zoom_y_pan() {
        // Mismo buffer 4×4 en rect 200×200. A zoom 2 + pan (0,0) la imagen
        // queda 400×400 centrada en el rect → top-left en (-100, -100).
        // Cada píxel ocupa 100 px. Click en (0, 0) (esquina del rect) cae
        // en píxel (1, 1) — verifico R=60, G=60.
        let buf = buffer_patron_4x4();
        let col = recoger_color_en(&buf, 4, 4, 0.0, 0.0, 200.0, 200.0, 2.0, 0.0, 0.0).unwrap();
        assert_eq!(col, [60, 60, 17, 255], "esquina superior con zoom 2");
    }

    #[test]
    fn recoger_color_buffer_corto_devuelve_none() {
        // Buffer prometido como 4×4 pero sólo trae 2 píxeles → indexar más
        // allá no debe panickear: devolvemos None.
        let buf = vec![10, 20, 30, 255, 40, 50, 60, 255];
        // Click que apuntaría al píxel (3, 3) — fuera del buffer real.
        assert!(recoger_color_en(&buf, 4, 4, 175.0, 175.0, 200.0, 200.0, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn msg_recoger_color_actualiza_color_picked() {
        // Un Model mínimo con una imagen 4×4 conocida; despachamos
        // RecogerColor y verificamos que `color_picked` queda con el RGBA
        // del píxel correcto.
        let mut model = modelo_minimo();
        // Reemplazamos la imagen del modelo por una con buffer conocido.
        let buf = buffer_patron_4x4();
        let blob = Blob::from(buf);
        model.imagen = Some(Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 4, height: 4 }));
        // Click en píxel (2, 3) sobre rect 200×200 a zoom 1 → R=120, G=180.
        model = <Tullpu as App>::update(
            model,
            Msg::RecogerColor { lx: 125.0, ly: 175.0, rw: 200.0, rh: 200.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.color_picked, Some([120, 180, 17, 255]));
        assert!(model.estado.contains("#78B411"), "estado = {}", model.estado);
    }

    #[test]
    fn msg_recoger_color_fuera_no_pisa_color_anterior() {
        let mut model = modelo_minimo();
        let buf = buffer_patron_4x4();
        let blob = Blob::from(buf);
        model.imagen = Some(Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: 4, height: 4 }));
        model.color_picked = Some([1, 2, 3, 4]);
        // Click fuera del área de imagen (banda del pad).
        model = <Tullpu as App>::update(
            model,
            Msg::RecogerColor { lx: 5.0, ly: 50.0, rw: 200.0, rh: 100.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.color_picked, Some([1, 2, 3, 4]), "color anterior intacto");
        assert!(model.estado.contains("fuera"));
    }

    #[test]
    fn msg_cambiar_herramienta_actualiza_modo() {
        let mut model = modelo_minimo();
        assert_eq!(model.herramienta, Herramienta::Mover);
        model = <Tullpu as App>::update(
            model,
            Msg::CambiarHerramienta(Herramienta::Cuentagotas),
            &Handle::for_test(),
        );
        assert_eq!(model.herramienta, Herramienta::Cuentagotas);
        model = <Tullpu as App>::update(
            model,
            Msg::CambiarHerramienta(Herramienta::Mover),
            &Handle::for_test(),
        );
        assert_eq!(model.herramienta, Herramienta::Mover);
    }

    #[test]
    fn hotkey_m_e_i_emiten_cambio_de_herramienta() {
        let model = modelo_minimo();
        let mover = hotkey_a_msg(&model, &ev_char("m", Modifiers::default()));
        assert!(matches!(
            mover,
            Some(Msg::CambiarHerramienta(Herramienta::Mover))
        ));
        let cuenta = hotkey_a_msg(&model, &ev_char("i", Modifiers::default()));
        assert!(matches!(
            cuenta,
            Some(Msg::CambiarHerramienta(Herramienta::Cuentagotas))
        ));
        // Con Ctrl o Alt no deben disparar — son hotkeys de tecla suelta.
        assert!(hotkey_a_msg(
            &model,
            &ev_char("m", Modifiers { ctrl: true, ..Default::default() })
        )
        .is_none());
        assert!(hotkey_a_msg(
            &model,
            &ev_char("i", Modifiers { alt: true, ..Default::default() })
        )
        .is_none());
    }

    // ---- Fase 26: capa de relleno sólido ------------------------------------

    #[test]
    fn buffer_relleno_tiene_tamano_y_patron_correctos() {
        let buf = buffer_relleno(3, 2, [10, 20, 30, 40]);
        // 3×2 píxeles × 4 bytes/px = 24 bytes.
        assert_eq!(buf.len(), 24);
        // Cada cuádruple es el RGBA pedido — sin gaps.
        for cuadruple in buf.chunks_exact(4) {
            assert_eq!(cuadruple, &[10, 20, 30, 40]);
        }
    }