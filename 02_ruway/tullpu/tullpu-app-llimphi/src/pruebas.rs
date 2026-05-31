//! Tests de la app tullpu — extraídos de `main.rs` para respetar la regla de tamaño del repo (main.rs era ~4600 LOC, ~3800 de ellas tests). El módulo es hermano de la raíz del crate, así `use super::*` resuelve igual que cuando estaba inline.

    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
    use llimphi_ui::{KeyState, Modifiers, PaintRect};

    use tullpu_core::{
        Frescura, Lienzo, ModoFusion, OpLocal, OrigenCapa,
    };
    use tullpu_render::{AlmacenEnMemoria, FormatoExport, FuenteBuffers};
    use uuid::Uuid;

    fn ev_char(s: &str, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key: Key::Character(s.into()),
            state: KeyState::Pressed,
            text: Some(s.to_string()),
            modifiers: mods,
            repeat: false,
        }
    }
    fn ev_named(k: NamedKey, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key: Key::Named(k),
            state: KeyState::Pressed,
            text: None,
            modifiers: mods,
            repeat: false,
        }
    }
    fn modelo_minimo() -> Model {
        // Lienzo 4×4 con una capa raster, picker cerrado.
        let mut almacen = AlmacenEnMemoria::nuevo();
        let hash = almacen.insertar(vec![0u8; 4 * 4 * 4]);
        let mut lienzo = Lienzo::nuevo(4, 4);
        let cap = Capa::raster("c", hash);
        let id = cap.id;
        lienzo.apilar(cap);
        let historial = vec![lienzo.clone()];
        Model {
            lienzo,
            almacen,
            seleccionada: Some(id),
            imagen: None,
            estado: "test".into(),
            proveedor: Box::new(pixel_verbo_mock::ProveedorMock::nuevo()),
            proveedor_etiqueta: "test".into(),
            thumbs: HashMap::new(),
            raiz: PathBuf::from("/"),
            imagenes_disponibles: Vec::new(),
            picker: None,
            renombrando: None,
            historial,
            cursor_historial: 0,
            ultima_etiqueta_snapshot: None,
            factor_zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            herramienta: Herramienta::Mover,
            color_picked: None,
            histograma: None,
            seleccion: None,
            seleccion_drag: None,
            mover_drag: None,
            pincel_drag: None,
            radio_pincel: RADIO_PINCEL,
            dureza_pincel: DUREZA_PINCEL,
            shift_held: false,
            ultimo_pincel: None,
            simetria: Simetria::Ninguna,
            gradiente_drag: None,
            portapapeles: None,
            editando_mascara: false,
            valor_mascara: 255,
            thumbs_mascara: HashMap::new(),
            curva_arrastrando: None,
            menu_open: None,
            context_menu: None,
            edit_menu: None,
            clipboard: llimphi_clipboard::SystemClipboard::new(),
        }
    }

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
        assert_eq!(model.cursor_historial, 0);
    }

    #[test]
    fn undo_revierte_toggle_visible() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let visible_original = model.lienzo.capa(id).unwrap().visible;
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, !visible_original);
        assert_eq!(model.historial.len(), 2);
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
        assert_eq!(model.historial.len(), 3);
        assert_eq!(model.cursor_historial, 2);
        // Undo: vuelvo a M1.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.cursor_historial, 1);
        // M3: ciclar blend → debe truncar M2.
        model = <Tullpu as App>::update(model, Msg::CiclarBlend(id), &Handle::for_test());
        assert_eq!(model.historial.len(), 3, "M2 fue truncada");
        assert_eq!(model.cursor_historial, 2);
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
        let len_inicial = model.historial.len();
        for _ in 0..50 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id, -0.01),
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.historial.len(),
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
        let base = model.historial.len();

        // Drag sobre A
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id_a, -0.05),
                &Handle::for_test(),
            );
        }
        assert_eq!(model.historial.len(), base + 1);

        // Drag sobre B (capa distinta → no coalesce con el de A)
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id_b, -0.05),
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.historial.len(),
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
        assert_eq!(model.historial.len(), HIST_CAP);
        assert_eq!(model.cursor_historial, HIST_CAP - 1);
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
        let len_inicial = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("   "); // whitespace only — descartado por update
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(model.historial.len(), len_inicial);
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

    // ---- Fase 25: herramientas + cuentagotas ---------------------------------

    /// Construye un buffer Rgba8 4×4 con un patrón conocido: cada píxel
    /// codifica su posición en (R, G), con B fijo y α opaco. Útil para
    /// verificar que el sampler aterriza en la celda correcta.
    fn buffer_patron_4x4() -> Vec<u8> {
        let mut v = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                v.extend_from_slice(&[x * 60, y * 60, 17, 255]);
            }
        }
        v
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
        model.imagen = Some(Image::new(blob, ImageFormat::Rgba8, 4, 4));
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
        model.imagen = Some(Image::new(blob, ImageFormat::Rgba8, 4, 4));
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
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::AgregarRelleno, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        assert_eq!(model.historial.len(), hist_antes + 1);
        // Undo lo revierte.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), n_antes);
    }

    // ---- Fase 27: combinar capa hacia abajo (merge down) -------------------

    /// Construye un modelo con dos capas raster opacas de colores planos:
    /// debajo `rgba_bajo` ocupando todo el lienzo 2×2; encima `rgba_alto`
    /// también ocupando todo. Devuelve (model, id_abajo, id_arriba).
    fn modelo_dos_capas(rgba_bajo: [u8; 4], rgba_alto: [u8; 4]) -> (Model, Uuid, Uuid) {
        let mut model = modelo_minimo();
        // El minimo trae una capa de 4×4 todo en cero — la usamos como
        // "abajo". Reemplazamos su contenido por el del color pedido.
        model.lienzo = Lienzo::nuevo(2, 2);
        let buf_b = buffer_relleno(2, 2, rgba_bajo);
        let h_b = model.almacen.insertar(buf_b);
        let cap_b = Capa::raster("base", h_b);
        let id_b = cap_b.id;
        model.lienzo.apilar(cap_b);
        let buf_a = buffer_relleno(2, 2, rgba_alto);
        let h_a = model.almacen.insertar(buf_a);
        let cap_a = Capa::raster("sobre", h_a);
        let id_a = cap_a.id;
        model.lienzo.apilar(cap_a);
        model.seleccionada = Some(id_a);
        // Reseteamos el historial para que este sea el estado base.
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        (model, id_b, id_a)
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
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::Combinar(id_a), &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.historial.len(), hist_antes + 1);
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

    // ---- Fase 28: aplanar visibles (merge visible) ---------------------------

    /// Helper que mete N capas raster opacas de colores distintos al
    /// modelo mínimo. Devuelve los Uuid en orden de inserción
    /// (capas[0] = primera retornada).
    fn modelo_n_capas(colores: &[[u8; 4]]) -> (Model, Vec<Uuid>) {
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(2, 2);
        let mut ids = Vec::new();
        for (i, &c) in colores.iter().enumerate() {
            let buf = buffer_relleno(2, 2, c);
            let h = model.almacen.insertar(buf);
            let cap = Capa::raster(format!("c{}", i), h);
            ids.push(cap.id);
            model.lienzo.apilar(cap);
        }
        model.seleccionada = ids.first().copied();
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        (model, ids)
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
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::AplanarVisibles, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.historial.len(), hist_antes + 1);
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

    // ---- Fase 30: rotar lienzo 90° -----------------------------------------

    fn px_at(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
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
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::RotarLienzo { cw: true },
            &Handle::for_test(),
        );
        assert_eq!((model.lienzo.width, model.lienzo.height), (3, 2));
        assert_eq!(model.historial.len(), hist_antes + 1);
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
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::AutotrimLienzo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        assert_eq!(model.historial.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (4, 4));
    }

    // ---- Fase 32: editar parámetros de derivada con sliders ---------------

    /// Construye un modelo con 1 raster fondo + 1 derivada de la op
    /// pasada. Devuelve (model, id_fondo, id_derivada).
    fn modelo_con_derivada(op: OpLocal) -> (Model, Uuid, Uuid) {
        let mut model = modelo_minimo();
        // La capa que viene en modelo_minimo es la madre.
        let id_madre = model.seleccionada.unwrap();
        let derivada = Capa::derivada(
            "deriv",
            id_madre,
            TransformacionPixel::Local(op),
            [0u8; 32],
        );
        let id_deriv = derivada.id;
        model.lienzo.apilar(derivada);
        model.seleccionada = Some(id_deriv);
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        (model, id_madre, id_deriv)
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

    /// Extrae los puntos de control de una capa derivada `Curvas`.
    fn puntos_de(model: &Model, id: Uuid) -> Vec<(f32, f32)> {
        match &model.lienzo.capa(id).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Curvas { puntos }),
                ..
            } => puntos.clone(),
            other => panic!("esperaba Curvas: {:?}", other),
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

    #[test]
    fn ajustar_param_propaga_stale_al_cono() {
        // Construyo: raster → derivada Brillo → derivada Saturacion
        // (hija de la Brillo). Ajustar el Brillo debe marcar la
        // Saturacion también Stale.
        let (mut model, _madre, brillo) =
            modelo_con_derivada(OpLocal::Brillo { delta: 0.1 });
        let sat = Capa::derivada(
            "sat",
            brillo,
            TransformacionPixel::Local(OpLocal::Saturacion { factor: 1.0 }),
            [0u8; 32],
        );
        let id_sat = sat.id;
        model.lienzo.apilar(sat);
        // Marco sat como Fresca para verificar que propagar realmente la
        // toca.
        if let OrigenCapa::Derivada { estado, .. } =
            &mut model.lienzo.capa_mut(id_sat).unwrap().origen
        {
            *estado = Frescura::Fresca;
        }
        ajustar_parametro_derivada(
            &mut model,
            brillo,
            ParametroSlider::BrilloDelta,
            0.1,
        );
        assert!(model.lienzo.capa(id_sat).unwrap().esta_stale());
    }

    #[test]
    fn msg_ajustar_parametro_coalesce_drag_en_un_solo_snapshot() {
        // 30 events seguidos sobre el mismo slider deben colapsar en 1
        // entrada nueva de historial (mismo coalesce-key).
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::Brillo { delta: 0.0 });
        let hist_antes = model.historial.len();
        for _ in 0..30 {
            model = <Tullpu as App>::update(
                model,
                Msg::AjustarParametro {
                    id: deriv,
                    param: ParametroSlider::BrilloDelta,
                    dv: 0.01,
                },
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.historial.len(),
            hist_antes + 1,
            "drag de slider coalesce a 1 entrada"
        );
        // Un solo Undo revierte el drag entero.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Brillo { delta }),
                ..
            } => assert!(delta.abs() < 1e-6, "vuelve a 0, no quedó a medio"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn sliders_parametros_devuelve_none_para_op_no_parametrizable() {
        let theme = llimphi_theme::Theme::dark();
        // Invertir no tiene parámetros → no aparece sección.
        let (model, _, _) = modelo_con_derivada(OpLocal::Invertir);
        assert!(sliders_parametros_capa(&theme, &model).is_none());
        // EspejarHorizontal idem.
        let (model, _, _) = modelo_con_derivada(OpLocal::EspejarHorizontal);
        assert!(sliders_parametros_capa(&theme, &model).is_none());
    }

    #[test]
    fn sliders_parametros_devuelve_some_para_ops_parametrizables() {
        let theme = llimphi_theme::Theme::dark();
        for op in [
            OpLocal::Brillo { delta: 0.0 },
            OpLocal::Contraste { factor: 1.0 },
            OpLocal::Saturacion { factor: 1.0 },
            OpLocal::Tonalidad { grados: 0.0 },
            OpLocal::Blur { radio: 0.0 },
            OpLocal::Opacidad { factor: 1.0 },
        ] {
            let (model, _, _) = modelo_con_derivada(op);
            assert!(
                sliders_parametros_capa(&theme, &model).is_some(),
                "esperaba sección para op parametrizable"
            );
        }
    }

    // ---- Fase 33: Niveles con 3 sliders ---------------------------------

    #[test]
    fn ajustar_niveles_min_max_gamma_mutan_campo_correcto() {
        let (mut model, _, deriv) = modelo_con_derivada(OpLocal::Niveles {
            entrada_min: 0.1,
            entrada_max: 0.9,
            gamma: 1.0,
        });
        // Bump del min en +0.05 → 0.15.
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesEntradaMin,
            0.05,
        );
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op:
                    TransformacionPixel::Local(OpLocal::Niveles {
                        entrada_min,
                        entrada_max,
                        gamma,
                    }),
                ..
            } => {
                assert!((entrada_min - 0.15).abs() < 1e-6);
                // Otros campos intactos.
                assert!((entrada_max - 0.9).abs() < 1e-6);
                assert!((gamma - 1.0).abs() < 1e-6);
            }
            _ => unreachable!(),
        }
        // Ahora bump del max en -0.1 → 0.8.
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesEntradaMax,
            -0.1,
        );
        // Y del gamma en +0.5 → 1.5.
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesGamma,
            0.5,
        );
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op:
                    TransformacionPixel::Local(OpLocal::Niveles {
                        entrada_min,
                        entrada_max,
                        gamma,
                    }),
                ..
            } => {
                assert!((entrada_min - 0.15).abs() < 1e-6);
                assert!((entrada_max - 0.8).abs() < 1e-6);
                assert!((gamma - 1.5).abs() < 1e-6);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn ajustar_niveles_clamps_por_param() {
        let (mut model, _, deriv) = modelo_con_derivada(OpLocal::Niveles {
            entrada_min: 0.5,
            entrada_max: 0.5,
            gamma: 1.0,
        });
        // Empujar entrada_min muy arriba → clamp a 1.0.
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesEntradaMin,
            10.0,
        );
        // Empujar entrada_max muy abajo → clamp a 0.0 (sí, cruzando min:
        // permitido por design, ver doc del helper).
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesEntradaMax,
            -10.0,
        );
        // Gamma a 0.05 → clamp a 0.1 (min). 100 → clamp a 4.0.
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesGamma,
            -100.0,
        );
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op:
                    TransformacionPixel::Local(OpLocal::Niveles {
                        entrada_min,
                        entrada_max,
                        gamma,
                    }),
                ..
            } => {
                assert!((entrada_min - 1.0).abs() < 1e-6);
                assert!(entrada_max.abs() < 1e-6);
                assert!((gamma - 0.1).abs() < 1e-6);
            }
            _ => unreachable!(),
        }
        // Ahora gamma para arriba.
        ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesGamma,
            100.0,
        );
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Niveles { gamma, .. }),
                ..
            } => assert!((gamma - 4.0).abs() < 1e-6),
            _ => unreachable!(),
        }
    }

    #[test]
    fn ajustar_niveles_param_min_sobre_brillo_es_no_op() {
        // Defensivo: NivelesEntradaMin sobre una capa con OpLocal::Brillo
        // no debe mutar nada.
        let (mut model, _, deriv) =
            modelo_con_derivada(OpLocal::Brillo { delta: 0.3 });
        let cambio = ajustar_parametro_derivada(
            &mut model,
            deriv,
            ParametroSlider::NivelesEntradaMin,
            0.1,
        );
        assert!(!cambio);
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Brillo { delta }),
                ..
            } => assert!((delta - 0.3).abs() < 1e-6, "brillo intacto"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn sliders_parametros_para_niveles_devuelve_tres_rows() {
        let theme = llimphi_theme::Theme::dark();
        let (model, _, _) = modelo_con_derivada(OpLocal::Niveles {
            entrada_min: 0.1,
            entrada_max: 0.9,
            gamma: 1.2,
        });
        let rows = sliders_parametros_capa(&theme, &model)
            .expect("Niveles ya debe tener sliders");
        assert_eq!(rows.len(), 3, "tres sliders: gamma + max + min");
    }

    #[test]
    fn niveles_drag_de_gamma_coalesce_en_un_solo_snapshot() {
        let (mut model, _, deriv) = modelo_con_derivada(OpLocal::Niveles {
            entrada_min: 0.0,
            entrada_max: 1.0,
            gamma: 1.0,
        });
        let hist_antes = model.historial.len();
        // 20 deltas pequeños — todos misma capa, mismo param → 1 snapshot.
        for _ in 0..20 {
            model = <Tullpu as App>::update(
                model,
                Msg::AjustarParametro {
                    id: deriv,
                    param: ParametroSlider::NivelesGamma,
                    dv: 0.05,
                },
                &Handle::for_test(),
            );
        }
        assert_eq!(model.historial.len(), hist_antes + 1);
        // Un Undo revierte el drag completo de gamma.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        match &model.lienzo.capa(deriv).unwrap().origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Niveles { gamma, .. }),
                ..
            } => assert!((gamma - 1.0).abs() < 1e-6, "gamma vuelve a 1.0"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn niveles_drag_de_min_no_coalesce_con_drag_de_max() {
        // Dos params distintos sobre la misma capa Niveles → 2 entradas
        // separadas en historial (la clave_coalesce difiere).
        let (mut model, _, deriv) = modelo_con_derivada(OpLocal::Niveles {
            entrada_min: 0.0,
            entrada_max: 1.0,
            gamma: 1.0,
        });
        let hist_antes = model.historial.len();
        for _ in 0..5 {
            model = <Tullpu as App>::update(
                model,
                Msg::AjustarParametro {
                    id: deriv,
                    param: ParametroSlider::NivelesEntradaMin,
                    dv: 0.05,
                },
                &Handle::for_test(),
            );
        }
        for _ in 0..5 {
            model = <Tullpu as App>::update(
                model,
                Msg::AjustarParametro {
                    id: deriv,
                    param: ParametroSlider::NivelesEntradaMax,
                    dv: -0.05,
                },
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.historial.len(),
            hist_antes + 2,
            "min y max coalesce por separado"
        );
    }

    // ---- Fase 34: histograma RGB --------------------------------------------

    #[test]
    fn histograma_de_buffer_uniforme_concentra_todo_en_un_bin() {
        // 16 píxeles todos (50, 100, 200, 255). El histograma debe
        // mostrar 16 en R[50], G[100], B[200] y 0 en todo lo demás.
        let buf = buffer_relleno(4, 4, [50, 100, 200, 255]);
        let h = histograma_rgb(&buf);
        for v in 0..256 {
            let esperado_r = if v == 50 { 16 } else { 0 };
            let esperado_g = if v == 100 { 16 } else { 0 };
            let esperado_b = if v == 200 { 16 } else { 0 };
            assert_eq!(h[0][v], esperado_r, "R[{v}]");
            assert_eq!(h[1][v], esperado_g, "G[{v}]");
            assert_eq!(h[2][v], esperado_b, "B[{v}]");
        }
    }

    #[test]
    fn histograma_ignora_alfa() {
        // Mismo valor RGB en dos píxeles, uno con alfa=0 y otro con
        // alfa=255. El histograma cuenta los dos: alfa no afecta el
        // conteo.
        let buf = vec![
            50, 100, 200, 0,   // alfa cero
            50, 100, 200, 255, // alfa máximo
        ];
        let h = histograma_rgb(&buf);
        assert_eq!(h[0][50], 2);
        assert_eq!(h[1][100], 2);
        assert_eq!(h[2][200], 2);
    }

    #[test]
    fn histograma_gradiente_distribuye_bins() {
        // 256 píxeles con R variando 0..255 linealmente, G y B fijos.
        // El histograma de R debe tener exactamente 1 en cada bin.
        let mut buf = Vec::with_capacity(256 * 4);
        for v in 0..256u32 {
            buf.extend_from_slice(&[v as u8, 99, 33, 255]);
        }
        let h = histograma_rgb(&buf);
        for v in 0..256 {
            assert_eq!(h[0][v], 1, "R[{v}]");
        }
        // G[99] = 256 (todos los píxeles), G en otros bins = 0.
        assert_eq!(h[1][99], 256);
        for v in 0..256 {
            if v != 99 {
                assert_eq!(h[1][v], 0);
            }
        }
        assert_eq!(h[2][33], 256);
    }

    #[test]
    fn histograma_buffer_vacio_es_todo_cero() {
        let h = histograma_rgb(&[]);
        for canal in 0..3 {
            for v in 0..256 {
                assert_eq!(h[canal][v], 0);
            }
        }
    }

    #[test]
    fn aplicar_y_recomponer_actualiza_histograma() {
        // El cache se refresca dentro de aplicar_y_recomponer.
        let mut model = modelo_minimo();
        assert!(model.histograma.is_none());
        aplicar_y_recomponer(&mut model);
        let h = model.histograma.expect("histograma debe existir");
        // El modelo mínimo tiene un buffer 4×4 a ceros → todos los
        // canales tienen 16 píxeles en el bin 0.
        assert_eq!(h[0][0], 16);
        assert_eq!(h[1][0], 16);
        assert_eq!(h[2][0], 16);
    }

    #[test]
    fn agregar_capa_recompone_y_actualiza_histograma() {
        // Después de agregar un relleno con color picked, el histograma
        // del composite debe reflejar el color predominante.
        let mut model = modelo_minimo();
        model.color_picked = Some([200, 100, 50, 255]);
        agregar_capa_relleno(&mut model);
        // El composite = relleno opaco encima de la capa base → la base
        // queda cubierta. Todos los píxeles del composite son
        // (200, 100, 50).
        let h = model.histograma.unwrap();
        let total: u32 = h[0].iter().sum();
        // 4×4 = 16 píxeles.
        assert_eq!(total, 16);
        assert_eq!(h[0][200], 16);
        assert_eq!(h[1][100], 16);
        assert_eq!(h[2][50], 16);
    }

    // ---- Fase 35: selección rectangular (marquee) ----------------------------

    #[test]
    fn local_a_imagen_zoom_1_centra_la_conversion() {
        // Lienzo 100×100 que cabe en rect 200×200 a zoom 1 → s=2,
        // off_x=off_y=0. Click en (50, 50) local → (25, 25) image.
        let (ix, iy) =
            local_a_imagen(50.0, 50.0, 200.0, 200.0, 100, 100, 1.0, 0.0, 0.0).unwrap();
        assert!((ix - 25.0).abs() < 1e-3);
        assert!((iy - 25.0).abs() < 1e-3);
    }

    #[test]
    fn local_a_imagen_dims_cero_devuelve_none() {
        assert!(local_a_imagen(0.0, 0.0, 0.0, 100.0, 100, 100, 1.0, 0.0, 0.0).is_none());
        assert!(local_a_imagen(0.0, 0.0, 100.0, 100.0, 0, 100, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn rect_imagen_desde_drag_normaliza_ancla_y_cursor_invertidos() {
        // Drag de abajo-derecha hacia arriba-izquierda: ancla > cur en
        // ambos ejes. El rect debe quedar con x0<x1, y0<y1.
        // ancla en (8, 8) image; cursor local convertido a (2, 2) image.
        // En un lienzo 10×10 con rect 20×20 a zoom 1 → s=2, off=0.
        // cur_lx=4, cur_ly=4 → (2, 2) image.
        let drag = SeleccionDrag {
            ancla_ix: 8,
            ancla_iy: 8,
            cur_lx: 4.0,
            cur_ly: 4.0,
            rw: 20.0,
            rh: 20.0,
        };
        let rect = rect_imagen_desde_drag(&drag, 10, 10, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(rect, RectImagen { x0: 2, y0: 2, x1: 8, y1: 8 });
    }

    #[test]
    fn rect_imagen_desde_drag_clampea_al_lienzo() {
        // Drag que sale por arriba/derecha: el rect debe clampear a
        // [0, w] × [0, h], no leakear coords negativas o > dims.
        let drag = SeleccionDrag {
            ancla_ix: -5,
            ancla_iy: -3,
            cur_lx: 200.0, // muy a la derecha
            cur_ly: 200.0, // muy abajo
            rw: 20.0,
            rh: 20.0,
        };
        // Lienzo 10×10 en rect 20×20 zoom 1 → cur local (200, 200) →
        // image (100, 100) muy fuera del lienzo.
        let rect = rect_imagen_desde_drag(&drag, 10, 10, 1.0, 0.0, 0.0).unwrap();
        // Clamped: x0=0, y0=0, x1=10, y1=10.
        assert_eq!(rect, RectImagen { x0: 0, y0: 0, x1: 10, y1: 10 });
    }

    #[test]
    fn rect_imagen_desde_drag_area_cero_devuelve_none() {
        // Click sin drag — ancla y cursor en mismo punto → rect
        // degenerado.
        let drag = SeleccionDrag {
            ancla_ix: 5,
            ancla_iy: 5,
            cur_lx: 10.0,
            cur_ly: 10.0,
            rw: 20.0,
            rh: 20.0,
        };
        // Lienzo 10×10, rect 20×20 zoom 1 → cur local (10, 10) →
        // image (5, 5) = ancla. Rect degenerado.
        assert!(rect_imagen_desde_drag(&drag, 10, 10, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn iniciar_seleccion_setea_ancla_y_limpia_seleccion_previa() {
        let mut model = modelo_minimo();
        // Simulamos que ya había una selección de un drag anterior.
        model.seleccion = Some(RectImagen {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 2,
        });
        // El lienzo de modelo_minimo es 4×4. imagen tiene que existir
        // para que iniciar entre — un aplicar_y_recomponer la crea.
        aplicar_y_recomponer(&mut model);
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion {
                lx: 8.0,
                ly: 8.0,
                rw: 16.0,
                rh: 16.0,
            },
            &Handle::for_test(),
        );
        // Press dispara: limpia seleccion previa + abre el drag.
        assert!(model.seleccion.is_none());
        let drag = model.seleccion_drag.expect("debe haber drag");
        // Lienzo 4×4 en rect 16×16 → s = 4. Local (8, 8) → image (2, 2).
        assert_eq!(drag.ancla_ix, 2);
        assert_eq!(drag.ancla_iy, 2);
    }

    #[test]
    fn ajustar_seleccion_acumula_y_construye_rect() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        // Init en (0, 0) local → image (0, 0).
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion {
                lx: 0.0,
                ly: 0.0,
                rw: 16.0,
                rh: 16.0,
            },
            &Handle::for_test(),
        );
        // Move acumulado a (16, 16) local → image (4, 4) (esquina).
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarSeleccion { dx: 16.0, dy: 16.0 },
            &Handle::for_test(),
        );
        let rect = model.seleccion.expect("rect post-move");
        // Lienzo 4×4 → rect entero.
        assert_eq!(
            rect,
            RectImagen { x0: 0, y0: 0, x1: 4, y1: 4 }
        );
    }

    #[test]
    fn finalizar_seleccion_limpia_el_drag_y_mantiene_el_rect() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion {
                lx: 0.0,
                ly: 0.0,
                rw: 16.0,
                rh: 16.0,
            },
            &Handle::for_test(),
        );
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarSeleccion { dx: 12.0, dy: 8.0 },
            &Handle::for_test(),
        );
        let rect_pre_end = model.seleccion;
        model = <Tullpu as App>::update(model, Msg::FinalizarSeleccion, &Handle::for_test());
        // Drag vacío; rect intacto.
        assert!(model.seleccion_drag.is_none());
        assert_eq!(model.seleccion, rect_pre_end);
    }

    #[test]
    fn limpiar_seleccion_borra_todo() {
        let mut model = modelo_minimo();
        model.seleccion = Some(RectImagen {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 2,
        });
        model.seleccion_drag = Some(SeleccionDrag {
            ancla_ix: 0,
            ancla_iy: 0,
            cur_lx: 0.0,
            cur_ly: 0.0,
            rw: 100.0,
            rh: 100.0,
        });
        model = <Tullpu as App>::update(model, Msg::LimpiarSeleccion, &Handle::for_test());
        assert!(model.seleccion.is_none());
        assert!(model.seleccion_drag.is_none());
    }

    #[test]
    fn hotkey_r_emite_cambio_a_marco() {
        let m = modelo_minimo();
        let msg = hotkey_a_msg(&m, &ev_char("r", Modifiers::default()));
        assert!(matches!(
            msg,
            Some(Msg::CambiarHerramienta(Herramienta::Marco))
        ));
    }

    #[test]
    fn hotkey_esc_limpia_si_hay_seleccion_o_drag() {
        let mut m = modelo_minimo();
        // Sin selección → Esc no emite nada (deja que otros modales lo
        // consuman si corresponde).
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Escape, Modifiers::default()));
        assert!(msg.is_none());
        // Con selección → Esc emite LimpiarSeleccion.
        m.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 1, y1: 1 });
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Escape, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::LimpiarSeleccion)));
    }

    // ---- Fase 36: recortar a selección -----------------------------------

    #[test]
    fn recortar_a_seleccion_sin_seleccion_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let dims_antes = (model.lienzo.width, model.lienzo.height);
        let ok = recortar_lienzo_a_seleccion(&mut model);
        assert!(!ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), dims_antes);
        assert!(model.estado.contains("no hay selección"));
    }

    #[test]
    fn recortar_a_seleccion_cubriendo_todo_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        // Selección de todo el lienzo 4×4.
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 4, y1: 4 });
        let ok = recortar_lienzo_a_seleccion(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("cubre todo"));
    }

    #[test]
    fn recortar_a_seleccion_fuera_del_lienzo_es_no_op() {
        // Selección con coords más allá del lienzo (4×4 acá).
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 10, y0: 10, x1: 12, y1: 12 });
        let ok = recortar_lienzo_a_seleccion(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("fuera"));
    }

    #[test]
    fn recortar_a_seleccion_aplica_rect_y_limpia_seleccion() {
        // Lienzo 4×4 con un patrón distinguible, recortar a (1, 1, 3, 3)
        // = subrect 2×2 central.
        let mut model = modelo_minimo();
        // Reemplazamos la capa base por una con buffer conocido para
        // poder verificar pixel-perfect post-crop.
        let mut buf = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                buf.extend_from_slice(&[x * 20, y * 20, 100, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("patron", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });

        let ok = recortar_lienzo_a_seleccion(&mut model);
        assert!(ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        // Selección debe limpiar — sus coords ya no son del nuevo
        // coord-space.
        assert!(model.seleccion.is_none());
        // Verificación de píxeles: el píxel (0,0) post-crop debe ser
        // el píxel (1,1) original = (20, 20, 100, 255).
        let nueva_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(nueva_h).unwrap();
        assert_eq!(buf_post[0..4], [20, 20, 100, 255]);
        // Píxel (1, 1) post-crop = (2, 2) original = (40, 40, 100, 255).
        let i = (1 * 2 + 1) * 4;
        assert_eq!(buf_post[i..i + 4], [40, 40, 100, 255]);
    }

    #[test]
    fn recortar_a_seleccion_clampea_si_rect_sobresale_parcialmente() {
        // Selección (2, 2, 10, 10) con lienzo 4×4 → intersección
        // (2, 2, 4, 4) = 2×2 esquina inferior derecha.
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 2, y0: 2, x1: 10, y1: 10 });
        let ok = recortar_lienzo_a_seleccion(&mut model);
        assert!(ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
    }

    #[test]
    fn msg_recortar_a_seleccion_dispatcha_y_undo_restaura() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::RecortarASeleccion, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        assert_eq!(model.historial.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (4, 4));
    }

    // ---- Fase 37: limpiar selección a transparente ----------------------

    #[test]
    fn limpiar_rect_en_buffer_pone_alfa_cero_dentro_y_deja_el_resto_intacto() {
        // Buffer 3×2 con un patrón distinguible (R = x*10, G = y*10, B = 100,
        // A = 200). Limpiamos el rect (1, 0, 3, 2): los 4 píxeles de las
        // 2 columnas derechas.
        let mut buf = Vec::with_capacity(3 * 2 * 4);
        for y in 0..2u8 {
            for x in 0..3u8 {
                buf.extend_from_slice(&[x * 10, y * 10, 100, 200]);
            }
        }
        let out = limpiar_rect_en_buffer(&buf, 3, 1, 0, 3, 2);
        assert_eq!(out.len(), buf.len());
        // Píxel (0, 0) intacto.
        assert_eq!(out[0..4], [0, 0, 100, 200]);
        // Píxel (1, 0) limpio.
        assert_eq!(out[4..8], [0, 0, 0, 0]);
        // Píxel (2, 0) limpio.
        assert_eq!(out[8..12], [0, 0, 0, 0]);
        // Píxel (0, 1) intacto.
        assert_eq!(out[12..16], [0, 10, 100, 200]);
        // Píxel (1, 1) limpio.
        assert_eq!(out[16..20], [0, 0, 0, 0]);
        // Píxel (2, 1) limpio.
        assert_eq!(out[20..24], [0, 0, 0, 0]);
    }

    #[test]
    fn limpiar_rect_vacio_es_identidad() {
        // Rect half-open con x1 = x0 = 0 → ningún píxel debería tocarse.
        // Lo invocamos para verificar que el loop interno no panickea
        // cuando el rango es vacío (la validación de área va aguas
        // arriba, pero el helper debe sobrevivir un rect degenerado).
        let buf = vec![7u8; 4 * 4 * 4];
        let out = limpiar_rect_en_buffer(&buf, 4, 1, 1, 1, 1);
        assert_eq!(out, buf);
    }

    #[test]
    fn limpiar_seleccion_sin_seleccion_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("no hay selección"));
    }

    #[test]
    fn limpiar_seleccion_sin_capa_seleccionada_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        model.seleccionada = None;
        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("no hay capa"));
    }

    #[test]
    fn limpiar_seleccion_fuera_del_lienzo_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 10, y0: 10, x1: 12, y1: 12 });
        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("fuera"));
    }

    #[test]
    fn limpiar_seleccion_sobre_derivada_es_no_op() {
        // La capa seleccionada arranca raster — la convertimos en
        // derivada manualmente para verificar el rechazo.
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let id = model.seleccionada.unwrap();
        let capa = model.lienzo.capa_mut(id).unwrap();
        let madre_id = Uuid::new_v4();
        capa.origen = OrigenCapa::Derivada {
            madre: madre_id,
            op: TransformacionPixel::Local(OpLocal::Invertir),
            estado: Frescura::Fresca,
        };
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("derivada"));
    }

    #[test]
    fn limpiar_seleccion_pone_alfa_cero_pixel_perfect_y_mantiene_seleccion() {
        // Lienzo 4×4 con patrón distinguible; selección (1, 1, 3, 3) =
        // 2×2 central. Tras la op, los 4 píxeles centrales deben quedar
        // todo cero y los 12 del borde intactos.
        let mut model = modelo_minimo();
        let mut buf = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                buf.extend_from_slice(&[x * 20, y * 20, 100, 255]);
            }
        }
        let h = model.almacen.insertar(buf.clone());
        let cap = Capa::raster("patron", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        let sel = RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 };
        model.seleccion = Some(sel);

        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(ok);
        // La selección se mantiene (workflow Photoshop: marquee + Del +
        // pintar adentro sin re-armar el rect).
        assert_eq!(model.seleccion, Some(sel));
        // Dims del lienzo intactas.
        assert_eq!((model.lienzo.width, model.lienzo.height), (4, 4));
        // Píxel-perfect: los 4 centrales son [0,0,0,0]; el píxel (0,0)
        // de borde queda con su valor original.
        let nueva_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(nueva_h).unwrap();
        let pix = |x: u32, y: u32| {
            let i = (y as usize * 4 + x as usize) * 4;
            [buf_post[i], buf_post[i + 1], buf_post[i + 2], buf_post[i + 3]]
        };
        assert_eq!(pix(1, 1), [0, 0, 0, 0]);
        assert_eq!(pix(2, 1), [0, 0, 0, 0]);
        assert_eq!(pix(1, 2), [0, 0, 0, 0]);
        assert_eq!(pix(2, 2), [0, 0, 0, 0]);
        // Borde superior izquierdo intacto.
        assert_eq!(pix(0, 0), [0, 0, 100, 255]);
        // Borde inferior derecho intacto.
        assert_eq!(pix(3, 3), [60, 60, 100, 255]);
    }

    #[test]
    fn limpiar_seleccion_clampea_si_rect_sobresale_parcialmente() {
        // Selección (2, 2, 10, 10) con lienzo 4×4 → intersección
        // (2, 2, 4, 4) = 2×2 esquina inferior derecha. Necesitamos un
        // buffer no-cero para que la limpieza efectivamente cambie el
        // hash y la función devuelva true.
        let mut model = modelo_minimo();
        let buf = vec![200u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("opaca", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 2, y0: 2, x1: 10, y1: 10 });
        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(ok);
        // El cuadrante inferior derecho tiene que estar todo cero, el
        // resto intacto (RGBA 200).
        let new_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(new_h).unwrap();
        for y in 2..4 {
            for x in 2..4 {
                let i = (y * 4 + x) * 4;
                assert_eq!(&buf_post[i..i + 4], &[0, 0, 0, 0]);
            }
        }
        // Borde superior izquierdo intacto.
        assert_eq!(&buf_post[0..4], &[200, 200, 200, 200]);
        // Borde fila 1, col 1 (fuera del rect 2..4 × 2..4) intacto.
        let i = (1 * 4 + 1) * 4;
        assert_eq!(&buf_post[i..i + 4], &[200, 200, 200, 200]);
    }

    #[test]
    fn limpiar_seleccion_sobre_rect_ya_transparente_es_no_op() {
        // Capa enteramente transparente — limpiar cualquier subrect
        // produce el mismo hash que el original, no hay cambio efectivo.
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("vacia", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = limpiar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("ya transparente"));
    }

    #[test]
    fn msg_limpiar_seleccion_dispatcha_y_undo_restaura() {
        let mut model = modelo_minimo();
        // Buffer con valores no-cero para que la limpieza efectivamente
        // cambie el hash.
        let buf = vec![123u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let hash_inicial = h;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::LimpiarSeleccionEnCapa,
            &Handle::for_test(),
        );
        assert_eq!(model.historial.len(), hist_antes + 1);
        // El hash de la capa cambió.
        assert_ne!(model.lienzo.capa(id).unwrap().contenido, hash_inicial);
        // Undo restaura el hash original.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, hash_inicial);
    }

    #[test]
    fn hotkey_delete_con_seleccion_emite_limpiar_y_sin_seleccion_emite_eliminar() {
        let mut m = modelo_minimo();
        // Sin selección → Eliminar(id).
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Delete, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::Eliminar(x)) if x == id));
        // Con selección → LimpiarSeleccionEnCapa.
        m.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Delete, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::LimpiarSeleccionEnCapa)));
        // Backspace mismo comportamiento.
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Backspace, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::LimpiarSeleccionEnCapa)));
    }

    // ---- Fase 38: rellenar selección con el color activo ----------------

    #[test]
    fn etiqueta_color_activo_hex_o_gris() {
        assert_eq!(etiqueta_color_activo(None), "gris");
        assert_eq!(
            etiqueta_color_activo(Some([0x12, 0xAB, 0xFF, 0xFF])),
            "#12ABFF"
        );
        // Ignora el alfa.
        assert_eq!(etiqueta_color_activo(Some([10, 20, 30, 0])), "#0A141E");
    }

    #[test]
    fn rellenar_rect_en_buffer_pone_color_dentro_y_deja_el_resto_intacto() {
        // Buffer 3×2 con patrón distinguible; rellenamos rect (1,0,3,2)
        // con magenta opaco. Las 2 columnas derechas cambian, la izq no.
        let mut buf = Vec::with_capacity(3 * 2 * 4);
        for y in 0..2u8 {
            for x in 0..3u8 {
                buf.extend_from_slice(&[x * 10, y * 10, 100, 200]);
            }
        }
        let magenta = [255, 0, 255, 255];
        let out = rellenar_rect_en_buffer(&buf, 3, 1, 0, 3, 2, magenta);
        assert_eq!(out.len(), buf.len());
        // Píxel (0,0) intacto.
        assert_eq!(out[0..4], [0, 0, 100, 200]);
        // Píxeles (1,0) y (2,0) magenta.
        assert_eq!(out[4..8], magenta);
        assert_eq!(out[8..12], magenta);
        // Píxel (0,1) intacto.
        assert_eq!(out[12..16], [0, 10, 100, 200]);
        // Píxeles (1,1) y (2,1) magenta.
        assert_eq!(out[16..20], magenta);
        assert_eq!(out[20..24], magenta);
    }

    #[test]
    fn rellenar_rect_vacio_es_identidad() {
        let buf = vec![7u8; 4 * 4 * 4];
        let out = rellenar_rect_en_buffer(&buf, 4, 2, 2, 2, 2, [1, 2, 3, 4]);
        assert_eq!(out, buf);
    }

    #[test]
    fn rellenar_seleccion_sin_seleccion_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let ok = rellenar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("no hay selección"));
    }

    #[test]
    fn rellenar_seleccion_sobre_derivada_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let id = model.seleccionada.unwrap();
        let capa = model.lienzo.capa_mut(id).unwrap();
        capa.origen = OrigenCapa::Derivada {
            madre: Uuid::new_v4(),
            op: TransformacionPixel::Local(OpLocal::Invertir),
            estado: Frescura::Fresca,
        };
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = rellenar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("derivada"));
    }

    #[test]
    fn rellenar_seleccion_usa_color_picked_pixel_perfect_y_mantiene_seleccion() {
        // Lienzo 4×4 todo transparente; color leído = naranja opaco;
        // selección 2×2 central. Los 4 centrales deben quedar naranjas,
        // los 12 de borde transparentes.
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("vacia", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        let naranja = [255, 140, 0, 255];
        model.color_picked = Some(naranja);
        aplicar_y_recomponer(&mut model);
        let sel = RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 };
        model.seleccion = Some(sel);

        let ok = rellenar_seleccion_en_capa(&mut model);
        assert!(ok);
        // La selección se mantiene.
        assert_eq!(model.seleccion, Some(sel));
        let nueva_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(nueva_h).unwrap();
        let pix = |x: u32, y: u32| {
            let i = (y as usize * 4 + x as usize) * 4;
            [buf_post[i], buf_post[i + 1], buf_post[i + 2], buf_post[i + 3]]
        };
        assert_eq!(pix(1, 1), naranja);
        assert_eq!(pix(2, 2), naranja);
        // Borde sigue transparente.
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
        assert_eq!(pix(3, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn rellenar_seleccion_sin_color_picked_usa_gris_default() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("vacia", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        model.color_picked = None; // → RELLENO_DEFAULT
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = rellenar_seleccion_en_capa(&mut model);
        assert!(ok);
        let nueva_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(nueva_h).unwrap();
        assert_eq!(&buf_post[0..4], &RELLENO_DEFAULT);
    }

    #[test]
    fn rellenar_seleccion_con_mismo_color_es_no_op() {
        // Capa ya pintada del color activo → el rellenado no cambia el
        // hash, no-op con estado descriptivo.
        let mut model = modelo_minimo();
        let gris = RELLENO_DEFAULT;
        let buf: Vec<u8> = std::iter::repeat(gris).take(4 * 4).flatten().collect();
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("gris", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        model.color_picked = Some(gris);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = rellenar_seleccion_en_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("ya tenía ese color"));
    }

    #[test]
    fn hotkey_shift_delete_con_seleccion_emite_rellenar() {
        let mut m = modelo_minimo();
        let shift = Modifiers { shift: true, ..Default::default() };
        // Sin selección, Shift+Del cae a Eliminar (la arm de fill exige
        // selección).
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Delete, shift));
        assert!(matches!(msg, Some(Msg::Eliminar(x)) if x == id));
        // Con selección → RellenarSeleccionEnCapa (Shift) vs.
        // LimpiarSeleccionEnCapa (sin Shift).
        m.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Delete, shift));
        assert!(matches!(msg, Some(Msg::RellenarSeleccionEnCapa)));
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Backspace, shift));
        assert!(matches!(msg, Some(Msg::RellenarSeleccionEnCapa)));
        let msg =
            hotkey_a_msg(&m, &ev_named(NamedKey::Delete, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::LimpiarSeleccionEnCapa)));
    }

    #[test]
    fn msg_rellenar_seleccion_dispatcha_y_undo_restaura() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let hash_inicial = h;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        model.color_picked = Some([255, 0, 0, 255]);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::RellenarSeleccionEnCapa,
            &Handle::for_test(),
        );
        assert_eq!(model.historial.len(), hist_antes + 1);
        assert_ne!(model.lienzo.capa(id).unwrap().contenido, hash_inicial);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, hash_inicial);
    }

    // ---- Fase 39: duplicar selección a capa nueva (Ctrl+J) --------------

    #[test]
    fn extraer_rect_a_buffer_conserva_dentro_y_borra_afuera() {
        // Buffer 3×2 opaco con patrón; extraemos rect (1,0,3,2).
        let mut buf = Vec::with_capacity(3 * 2 * 4);
        for y in 0..2u8 {
            for x in 0..3u8 {
                buf.extend_from_slice(&[x * 10, y * 10, 100, 255]);
            }
        }
        let (out, hubo) = extraer_rect_a_buffer(&buf, 3, 2, 1, 0, 3, 2);
        assert!(hubo);
        assert_eq!(out.len(), buf.len());
        // Afuera del rect (col 0) → transparente.
        assert_eq!(out[0..4], [0, 0, 0, 0]);
        assert_eq!(out[12..16], [0, 0, 0, 0]);
        // Dentro del rect → copiado de src.
        assert_eq!(out[4..8], [10, 0, 100, 255]);
        assert_eq!(out[8..12], [20, 0, 100, 255]);
        assert_eq!(out[16..20], [10, 10, 100, 255]);
        assert_eq!(out[20..24], [20, 10, 100, 255]);
    }

    #[test]
    fn extraer_rect_a_buffer_rect_transparente_reporta_sin_contenido() {
        let buf = vec![0u8; 4 * 4 * 4];
        let (out, hubo) = extraer_rect_a_buffer(&buf, 4, 4, 0, 0, 2, 2);
        assert!(!hubo);
        assert_eq!(out, vec![0u8; 4 * 4 * 4]);
    }

    #[test]
    fn extraer_rect_es_complemento_de_limpiar_rect() {
        // extraer(rect) + limpiar(rect) deben reconstruir el original
        // (partición disjunta de píxeles: cada byte está en uno u otro).
        let mut buf = Vec::with_capacity(4 * 4 * 4);
        for i in 0..(4 * 4) {
            buf.extend_from_slice(&[i as u8, 0, 0, 255]);
        }
        let (extraido, _) = extraer_rect_a_buffer(&buf, 4, 4, 1, 1, 3, 3);
        let resto = limpiar_rect_en_buffer(&buf, 4, 1, 1, 3, 3);
        for i in 0..buf.len() {
            assert_eq!(
                extraido[i].wrapping_add(resto[i]),
                buf[i],
                "byte {} no reconstruye",
                i
            );
        }
    }

    #[test]
    fn duplicar_seleccion_sin_seleccion_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let n_antes = model.lienzo.capas.len();
        let ok = duplicar_seleccion_a_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("no hay selección"));
        assert_eq!(model.lienzo.capas.len(), n_antes);
    }

    #[test]
    fn duplicar_seleccion_transparente_es_no_op() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("vacia", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let n_antes = model.lienzo.capas.len();
        let ok = duplicar_seleccion_a_capa(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("nada que copiar"));
        assert_eq!(model.lienzo.capas.len(), n_antes);
    }

    #[test]
    fn duplicar_seleccion_crea_capa_encima_con_solo_el_rect() {
        let mut model = modelo_minimo();
        let mut buf = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                buf.extend_from_slice(&[x * 20, y * 20, 100, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("madre", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });

        let ok = duplicar_seleccion_a_capa(&mut model);
        assert!(ok);
        // Hay una capa nueva, seleccionada, justo encima de la madre.
        assert_eq!(model.lienzo.capas.len(), 2);
        let idx_madre =
            model.lienzo.capas.iter().position(|c| c.id == id).unwrap();
        assert_eq!(idx_madre, 0);
        let nueva_id = model.seleccionada.unwrap();
        assert_ne!(nueva_id, id);
        assert_eq!(model.lienzo.capas[1].id, nueva_id);
        // La madre no se tocó.
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, h);
        // La nueva tiene sólo el rect 2×2 central; el resto transparente.
        let nh = model.lienzo.capa(nueva_id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: u32, y: u32| {
            let i = (y as usize * 4 + x as usize) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        assert_eq!(pix(1, 1), [20, 20, 100, 255]);
        assert_eq!(pix(2, 2), [40, 40, 100, 255]);
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
        assert_eq!(pix(3, 3), [0, 0, 0, 0]);
        // La selección se mantiene.
        assert_eq!(
            model.seleccion,
            Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 })
        );
    }

    #[test]
    fn hotkey_ctrl_j_emite_duplicar_seleccion_solo_con_seleccion() {
        let mut m = modelo_minimo();
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        // Sin selección → None (la capa entera ya se duplica con Ctrl+D).
        let msg = hotkey_a_msg(&m, &ev_char("j", ctrl));
        assert!(msg.is_none());
        // Con selección → DuplicarSeleccionACapa.
        m.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let msg = hotkey_a_msg(&m, &ev_char("j", ctrl));
        assert!(matches!(msg, Some(Msg::DuplicarSeleccionACapa)));
    }

    #[test]
    fn msg_duplicar_seleccion_dispatcha_snapshotea_y_undo_quita_la_capa() {
        let mut model = modelo_minimo();
        let buf = vec![200u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::DuplicarSeleccionACapa,
            &Handle::for_test(),
        );
        assert_eq!(model.historial.len(), hist_antes + 1);
        assert_eq!(model.lienzo.capas.len(), 2);
        // Undo vuelve a una sola capa.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.lienzo.capas[0].id, id);
    }

    // ---- Fase 40: portapapeles interno (copiar/cortar/pegar) ------------

    #[test]
    fn recortar_subbuffer_tight_y_reporta_contenido() {
        // Buffer 4×2; recortamos el rect (1,0,3,2) → tight 2×2.
        let mut buf = Vec::with_capacity(4 * 2 * 4);
        for y in 0..2u8 {
            for x in 0..4u8 {
                buf.extend_from_slice(&[x * 10, y * 10, 50, 255]);
            }
        }
        let (sub, hubo) = recortar_subbuffer(&buf, 4, 1, 0, 3, 2);
        assert!(hubo);
        // 2×2×4 = 16 bytes, NO el tamaño del origen.
        assert_eq!(sub.len(), 2 * 2 * 4);
        // (1,0),(2,0),(1,1),(2,1) en orden row-major.
        assert_eq!(sub[0..4], [10, 0, 50, 255]);
        assert_eq!(sub[4..8], [20, 0, 50, 255]);
        assert_eq!(sub[8..12], [10, 10, 50, 255]);
        assert_eq!(sub[12..16], [20, 10, 50, 255]);
    }

    #[test]
    fn recortar_subbuffer_transparente_reporta_sin_contenido() {
        let buf = vec![0u8; 4 * 4 * 4];
        let (sub, hubo) = recortar_subbuffer(&buf, 4, 0, 0, 2, 2);
        assert!(!hubo);
        assert_eq!(sub, vec![0u8; 2 * 2 * 4]);
    }

    #[test]
    fn componer_clip_en_canvas_ubica_en_offset() {
        // Clip 2×2 sólido rojo; lienzo 4×4; offset (1,1).
        let rojo = [255, 0, 0, 255];
        let clip: Vec<u8> = std::iter::repeat(rojo).take(4).flatten().collect();
        let out = componer_clip_en_canvas(&clip, 2, 2, 4, 4, 1, 1);
        assert_eq!(out.len(), 4 * 4 * 4);
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        // El clip cae en (1,1)..(3,3).
        assert_eq!(pix(1, 1), rojo);
        assert_eq!(pix(2, 2), rojo);
        // Afuera transparente.
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
        assert_eq!(pix(3, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn componer_clip_recorta_lo_que_sobresale() {
        // Clip 3×3 con offset (2,2) sobre lienzo 4×4 → sólo entra el
        // cuadrante (2,2)..(4,4) = 2×2.
        let v = [9, 9, 9, 255];
        let clip: Vec<u8> = std::iter::repeat(v).take(9).flatten().collect();
        let out = componer_clip_en_canvas(&clip, 3, 3, 4, 4, 2, 2);
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        assert_eq!(pix(2, 2), v);
        assert_eq!(pix(3, 3), v);
        // (1,1) fuera del clip.
        assert_eq!(pix(1, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn copiar_seleccion_llena_portapapeles_sin_snapshot() {
        let mut model = modelo_minimo();
        let mut buf = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                buf.extend_from_slice(&[x * 20, y * 20, 100, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("madre", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });
        let hist_antes = model.historial.len();
        let n_capas = model.lienzo.capas.len();
        let ok = copiar_seleccion(&mut model);
        assert!(ok);
        // No tocó historial ni capas.
        assert_eq!(model.historial.len(), hist_antes);
        assert_eq!(model.lienzo.capas.len(), n_capas);
        let clip = model.portapapeles.unwrap();
        assert_eq!((clip.w, clip.h), (2, 2));
        assert_eq!((clip.ox, clip.oy), (1, 1));
        // El clip tight tiene el píxel (1,1) del origen.
        let datos = model.almacen.obtener(clip.datos).unwrap();
        assert_eq!(&datos[0..4], &[20, 20, 100, 255]);
    }

    #[test]
    fn copiar_seleccion_transparente_no_llena_portapapeles() {
        let mut model = modelo_minimo();
        let buf = vec![0u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("vacia", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = copiar_seleccion(&mut model);
        assert!(!ok);
        assert!(model.portapapeles.is_none());
        assert!(model.estado.contains("nada que copiar"));
    }

    #[test]
    fn cortar_seleccion_copia_y_borra_en_raster() {
        let mut model = modelo_minimo();
        let buf = vec![200u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let borro = cortar_seleccion(&mut model);
        assert!(borro);
        // Portapapeles lleno.
        assert!(model.portapapeles.is_some());
        // El rect quedó transparente en la capa.
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        assert_eq!(&bp[0..4], &[0, 0, 0, 0]);
        // Fuera del rect sigue opaco.
        let i = (2 * 4 + 2) * 4;
        assert_eq!(&bp[i..i + 4], &[200, 200, 200, 200]);
    }

    #[test]
    fn cortar_sobre_derivada_copia_pero_no_borra() {
        let mut model = modelo_minimo();
        let buf = vec![200u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let mut cap = Capa::raster("der", h);
        cap.origen = OrigenCapa::Derivada {
            madre: Uuid::new_v4(),
            op: TransformacionPixel::Local(OpLocal::Invertir),
            estado: Frescura::Fresca,
        };
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let borro = cortar_seleccion(&mut model);
        assert!(!borro); // no snapshot
        assert!(model.portapapeles.is_some()); // pero sí copió
        assert!(model.estado.contains("no se borró"));
        // La capa derivada quedó intacta.
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, h);
    }

    #[test]
    fn pegar_vacio_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let n = model.lienzo.capas.len();
        let ok = pegar_portapapeles(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("vacío"));
        assert_eq!(model.lienzo.capas.len(), n);
    }

    #[test]
    fn copiar_y_pegar_crea_capa_con_el_clip_en_su_origen() {
        let mut model = modelo_minimo();
        let mut buf = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                buf.extend_from_slice(&[x * 20, y * 20, 100, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("madre", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 1, y0: 1, x1: 3, y1: 3 });
        assert!(copiar_seleccion(&mut model));
        let ok = pegar_portapapeles(&mut model);
        assert!(ok);
        // Capa nueva encima, seleccionada.
        assert_eq!(model.lienzo.capas.len(), 2);
        let nueva_id = model.seleccionada.unwrap();
        assert_ne!(nueva_id, id);
        assert_eq!(model.lienzo.capas[1].id, nueva_id);
        // El clip 2×2 cae de nuevo en (1,1) (origen preservado).
        let nh = model.lienzo.capa(nueva_id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        assert_eq!(pix(1, 1), [20, 20, 100, 255]);
        assert_eq!(pix(2, 2), [40, 40, 100, 255]);
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn pegar_clampea_origen_tras_un_crop_que_lo_dejo_fuera() {
        // Copiamos un clip 2×2 desde origen (4,4) en un lienzo 8×8, luego
        // simulamos que el lienzo se achicó a 4×4 — el origen (4,4) cae
        // fuera; pegar debe clampear a (2,2) para que entre.
        let mut model = modelo_minimo();
        // Clip directo en el portapapeles (sin pasar por copiar).
        let clip_buf = vec![123u8; 2 * 2 * 4];
        let datos = model.almacen.insertar(clip_buf);
        model.portapapeles = Some(PortaPixeles {
            w: 2,
            h: 2,
            datos,
            ox: 4,
            oy: 4,
        });
        // Lienzo vigente 4×4.
        let base = model.almacen.insertar(vec![0u8; 4 * 4 * 4]);
        let cap = Capa::raster("base", base);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.lienzo.width = 4;
        model.lienzo.height = 4;
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        let ok = pegar_portapapeles(&mut model);
        assert!(ok);
        // Origen clampeado a (4-2, 4-2) = (2,2): el clip ocupa (2,2)..(4,4).
        let nueva_id = model.seleccionada.unwrap();
        let nh = model.lienzo.capa(nueva_id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        assert_eq!(pix(2, 2), [123, 123, 123, 123]);
        assert_eq!(pix(3, 3), [123, 123, 123, 123]);
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn hotkeys_portapapeles_gateados_por_seleccion_y_clip() {
        let mut m = modelo_minimo();
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        // Sin selección: Ctrl+C/X no emiten copy/cut (caen a otra cosa o None).
        assert!(!matches!(
            hotkey_a_msg(&m, &ev_char("c", ctrl)),
            Some(Msg::CopiarSeleccion)
        ));
        assert!(!matches!(
            hotkey_a_msg(&m, &ev_char("x", ctrl)),
            Some(Msg::CortarSeleccion)
        ));
        // Con selección: Ctrl+C → copiar, Ctrl+X → cortar.
        m.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("c", ctrl)),
            Some(Msg::CopiarSeleccion)
        ));
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("x", ctrl)),
            Some(Msg::CortarSeleccion)
        ));
        // Ctrl+V sin clip → no pega.
        assert!(!matches!(
            hotkey_a_msg(&m, &ev_char("v", ctrl)),
            Some(Msg::PegarPortapapeles)
        ));
        // Con clip → pega (no requiere selección).
        m.seleccion = None;
        m.portapapeles = Some(PortaPixeles {
            w: 1,
            h: 1,
            datos: m.almacen.insertar(vec![1, 2, 3, 4]),
            ox: 0,
            oy: 0,
        });
        assert!(matches!(
            hotkey_a_msg(&m, &ev_char("v", ctrl)),
            Some(Msg::PegarPortapapeles)
        ));
    }

    #[test]
    fn msg_pegar_dispatcha_snapshotea_y_undo_quita_la_capa() {
        let mut model = modelo_minimo();
        let buf = vec![200u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        model = <Tullpu as App>::update(
            model,
            Msg::CopiarSeleccion,
            &Handle::for_test(),
        );
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::PegarPortapapeles,
            &Handle::for_test(),
        );
        assert_eq!(model.historial.len(), hist_antes + 1);
        assert_eq!(model.lienzo.capas.len(), 2);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
    }

    // ---- Fase 41: mover los píxeles de la selección (nudge) -------------

    #[test]
    fn paso_nudge_es_10_con_shift_y_1_sin_el() {
        assert_eq!(paso_nudge(false), 1);
        assert_eq!(paso_nudge(true), 10);
    }

    #[test]
    fn blit_alpha_sobre_opaco_pisa_y_transparente_no_borra() {
        // dst 3×1 rojo opaco; clip 2×1 con [verde opaco, transparente].
        let dst = vec![255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255];
        let clip = vec![0, 255, 0, 255, 9, 9, 9, 0];
        let out = blit_alpha_sobre(&dst, 3, 1, &clip, 2, 1, 1, 0);
        // (0): intacto rojo.
        assert_eq!(out[0..4], [255, 0, 0, 255]);
        // (1): verde opaco pisó.
        assert_eq!(out[4..8], [0, 255, 0, 255]);
        // (2): clip transparente → dst rojo intacto (no borró).
        assert_eq!(out[8..12], [255, 0, 0, 255]);
    }

    #[test]
    fn blit_alpha_sobre_semitransparente_compone() {
        // dst negro opaco; clip blanco a 50% alfa → gris ~ medio.
        let dst = vec![0, 0, 0, 255];
        let clip = vec![255, 255, 255, 128];
        let out = blit_alpha_sobre(&dst, 1, 1, &clip, 1, 1, 0, 0);
        // alfa salida = 128 + 255*(127)/255 = 128 + 127 = 255.
        assert_eq!(out[3], 255);
        // color ~ (255*128 + 0) / 255 ≈ 128.
        assert!((out[0] as i32 - 128).abs() <= 2, "got {}", out[0]);
    }

    #[test]
    fn blit_alpha_recorta_offset_negativo_y_fuera() {
        // clip 2×2 opaco con offset (-1,-1) sobre dst 2×2 → sólo el
        // píxel (0,0) del dst recibe el píxel (1,1) del clip.
        let dst = vec![0u8; 2 * 2 * 4];
        let clip: Vec<u8> = (0..4u8).flat_map(|i| [i, i, i, 255]).collect();
        let out = blit_alpha_sobre(&dst, 2, 2, &clip, 2, 2, -1, -1);
        // clip (1,1) = índice 3 → valor 3.
        assert_eq!(out[0..4], [3, 3, 3, 255]);
        // El resto del dst sin tocar.
        assert_eq!(out[4..8], [0, 0, 0, 0]);
    }

    #[test]
    fn mover_seleccion_sin_seleccion_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let ok = mover_pixeles_seleccion(&mut model, 1, 0);
        assert!(!ok);
        assert!(model.estado.contains("no hay selección"));
    }

    #[test]
    fn mover_seleccion_sobre_derivada_es_no_op() {
        let mut model = modelo_minimo();
        aplicar_y_recomponer(&mut model);
        let id = model.seleccionada.unwrap();
        let capa = model.lienzo.capa_mut(id).unwrap();
        capa.origen = OrigenCapa::Derivada {
            madre: Uuid::new_v4(),
            op: TransformacionPixel::Local(OpLocal::Invertir),
            estado: Frescura::Fresca,
        };
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = mover_pixeles_seleccion(&mut model, 1, 0);
        assert!(!ok);
        assert!(model.estado.contains("derivada"));
    }

    #[test]
    fn mover_seleccion_traslada_contenido_y_deja_transparencia() {
        // Lienzo 4×4 transparente; un bloque 2×2 opaco en la esquina
        // (0,0). Selección sobre ese bloque; lo movemos +2,+2.
        let mut model = modelo_minimo();
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 0..2 {
            for x in 0..2 {
                let i = (y * 4 + x) * 4;
                buf[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("bloque", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });

        let ok = mover_pixeles_seleccion(&mut model, 2, 2);
        assert!(ok);
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        // El origen (0,0) quedó transparente (se levantó el contenido).
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
        assert_eq!(pix(1, 1), [0, 0, 0, 0]);
        // El bloque aterrizó en (2,2)..(4,4).
        assert_eq!(pix(2, 2), [200, 100, 50, 255]);
        assert_eq!(pix(3, 3), [200, 100, 50, 255]);
        // La selección siguió al contenido.
        assert_eq!(
            model.seleccion,
            Some(RectImagen { x0: 2, y0: 2, x1: 4, y1: 4 })
        );
    }

    #[test]
    fn mover_seleccion_parcialmente_fuera_recorta_y_seleccion_clampea() {
        // Bloque 2×2 en (2,2); movemos +2,+2 en lienzo 4×4 → la mitad
        // se va del lienzo, sólo entra el píxel que cae en (4-?)...
        // en realidad (2,2)->(4,4) cae TODO fuera salvo nada; usamos
        // +1,+1 para que entre parcialmente.
        let mut model = modelo_minimo();
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 2..4 {
            for x in 2..4 {
                let i = (y * 4 + x) * 4;
                buf[i..i + 4].copy_from_slice(&[10, 20, 30, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("b", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 2, y0: 2, x1: 4, y1: 4 });

        let ok = mover_pixeles_seleccion(&mut model, 1, 1);
        assert!(ok);
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        // El origen se limpió.
        assert_eq!(pix(2, 2), [0, 0, 0, 0]);
        // Sólo el píxel que cae en (3,3) sobrevive (el resto se fue del
        // lienzo).
        assert_eq!(pix(3, 3), [10, 20, 30, 255]);
        // La selección se clampeó a (3,3)..(4,4).
        assert_eq!(
            model.seleccion,
            Some(RectImagen { x0: 3, y0: 3, x1: 4, y1: 4 })
        );
    }

    #[test]
    fn mover_seleccion_delta_cero_es_no_op() {
        let mut model = modelo_minimo();
        let buf = vec![200u8; 4 * 4 * 4];
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("b", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        let ok = mover_pixeles_seleccion(&mut model, 0, 0);
        assert!(!ok);
        assert!(model.estado.contains("sin efecto"));
    }

    #[test]
    fn hotkey_flechas_emiten_mover_con_paso_y_signo_correctos() {
        let mut m = modelo_minimo();
        // Sin selección, las flechas no emiten MoverSeleccion.
        let s = hotkey_a_msg(&m, &ev_named(NamedKey::ArrowLeft, Modifiers::default()));
        assert!(!matches!(s, Some(Msg::MoverSeleccion { .. })));
        // Con selección: cada flecha con su signo, Shift = 10.
        m.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        assert!(matches!(
            hotkey_a_msg(&m, &ev_named(NamedKey::ArrowLeft, Modifiers::default())),
            Some(Msg::MoverSeleccion { dx: -1, dy: 0 })
        ));
        assert!(matches!(
            hotkey_a_msg(&m, &ev_named(NamedKey::ArrowRight, Modifiers::default())),
            Some(Msg::MoverSeleccion { dx: 1, dy: 0 })
        ));
        assert!(matches!(
            hotkey_a_msg(&m, &ev_named(NamedKey::ArrowUp, Modifiers::default())),
            Some(Msg::MoverSeleccion { dx: 0, dy: -1 })
        ));
        let shift = Modifiers { shift: true, ..Default::default() };
        assert!(matches!(
            hotkey_a_msg(&m, &ev_named(NamedKey::ArrowDown, shift)),
            Some(Msg::MoverSeleccion { dx: 0, dy: 10 })
        ));
    }

    #[test]
    fn msg_mover_seleccion_coalesce_y_undo_restaura() {
        let mut model = modelo_minimo();
        let mut buf = vec![0u8; 4 * 4 * 4];
        buf[0..4].copy_from_slice(&[200, 100, 50, 255]);
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("base", h);
        let id = cap.id;
        let hash_inicial = h;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 1, y1: 1 });
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        // Tres nudges seguidos coalescen a una sola entrada.
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::MoverSeleccion { dx: 1, dy: 0 },
                &Handle::for_test(),
            );
        }
        assert_eq!(model.historial.len(), 2); // inicial + 1 coalescida
        // Un solo Undo restaura el hash original de la capa.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, hash_inicial);
    }

    // ---- Fase 42: arrastrar el contenido de la selección (drag-to-move) -

    /// Modelo 4×4 con un bloque 2×2 opaco en la esquina (0,0) y una capa
    /// raster seleccionada, listo para tests de drag. Con `rw=rh=4`,
    /// `zoom=1`, `pan=0` la conversión local→imagen es la identidad.
    fn modelo_bloque_4x4() -> (Model, Uuid) {
        let mut model = modelo_minimo();
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 0..2 {
            for x in 0..2 {
                let i = (y * 4 + x) * 4;
                buf[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("bloque", h);
        let id = cap.id;
        model.lienzo.capas.clear();
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        (model, id)
    }

    #[test]
    fn iniciar_seleccion_dentro_arranca_mover_drag_sin_limpiar() {
        let (mut model, _) = modelo_bloque_4x4();
        let sel = RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 };
        model.seleccion = Some(sel);
        // Press local (1,1) con rw=rh=4 → imagen (1,1), dentro del rect.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion { lx: 1.0, ly: 1.0, rw: 4.0, rh: 4.0 },
            &Handle::for_test(),
        );
        assert!(model.mover_drag.is_some());
        assert!(model.seleccion_drag.is_none());
        // La selección NO se limpió (sigue ahí para mover su contenido).
        assert_eq!(model.seleccion, Some(sel));
    }

    #[test]
    fn iniciar_seleccion_fuera_arranca_marquee_y_limpia() {
        let (mut model, _) = modelo_bloque_4x4();
        model.seleccion = Some(RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 });
        // Press local (3,3) → imagen (3,3), fuera del rect.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion { lx: 3.0, ly: 3.0, rw: 4.0, rh: 4.0 },
            &Handle::for_test(),
        );
        assert!(model.mover_drag.is_none());
        assert!(model.seleccion_drag.is_some());
        // El press fuera limpia la selección previa.
        assert!(model.seleccion.is_none());
    }

    #[test]
    fn drag_to_move_traslada_el_contenido_y_la_seleccion() {
        let (mut model, id) = modelo_bloque_4x4();
        let sel = RectImagen { x0: 0, y0: 0, x1: 2, y1: 2 };
        model.seleccion = Some(sel);
        // Press dentro (1,1).
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarSeleccion { lx: 1.0, ly: 1.0, rw: 4.0, rh: 4.0 },
            &Handle::for_test(),
        );
        // Arrastrar +2 en X (s=1 → 2 px-imagen).
        model = <Tullpu as App>::update(
            model,
            Msg::AjustarSeleccion { dx: 2.0, dy: 0.0 },
            &Handle::for_test(),
        );
        // El contenido se movió a x=2..4; el origen quedó transparente.
        let nh = model.lienzo.capa(id).unwrap().contenido;
        let bp = model.almacen.obtener(nh).unwrap();
        let pix = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [bp[i], bp[i + 1], bp[i + 2], bp[i + 3]]
        };
        assert_eq!(pix(0, 0), [0, 0, 0, 0]);
        assert_eq!(pix(2, 0), [200, 100, 50, 255]);
        assert_eq!(pix(3, 1), [200, 100, 50, 255]);
        // La selección siguió al contenido.
        assert_eq!(
            model.seleccion,
            Some(RectImagen { x0: 2, y0: 0, x1: 4, y1: 2 })
        );
        // Finalizar limpia el drag.
        model = <Tullpu as App>::update(
            model,
            Msg::FinalizarSeleccion,
            &Handle::for_test(),
        );
        assert!(model.mover_drag.is_none());
    }

    // ---- Fase 43: gestión de la selección (todo + expandir/contraer) ----

    #[test]
    fn expandir_rect_crece_y_clampea_al_lienzo() {
        let r = RectImagen { x0: 2, y0: 2, x1: 4, y1: 4 };
        // +1 por lado → (1,1)..(5,5).
        let e = expandir_rect(r, 1, 8, 8).unwrap();
        assert_eq!(e, RectImagen { x0: 1, y0: 1, x1: 5, y1: 5 });
        // +10 satura a los bordes del lienzo 8×8.
        let e = expandir_rect(r, 10, 8, 8).unwrap();
        assert_eq!(e, RectImagen { x0: 0, y0: 0, x1: 8, y1: 8 });
    }

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
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::ExpandirSeleccion(1),
            &Handle::for_test(),
        );
        // La selección no vive en el DAG → el historial no cambia.
        assert_eq!(model.historial.len(), hist_antes);
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
        let len_inicial = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("renombrado");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(model.historial.len(), len_inicial + 1);
        // Undo restaura el nombre original ("c").
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().nombre, "c");
    }

    // ---- Fase 44: balde de pintura (flood fill) -------------------------

    /// Construye un buffer `w×h` con dos colores: mitad izquierda `a`,
    /// mitad derecha `b` (corte en `x < w/2`).
    fn buffer_mitades(w: u32, h: u32, a: [u8; 4], b: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let _ = y;
                v.extend_from_slice(if x < w / 2 { &a } else { &b });
            }
        }
        v
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
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
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
        assert_eq!(model.historial.len(), 2); // inicial + 1 trazo coalescido
        model = <Tullpu as App>::update(
            model,
            Msg::FinalizarTrazo,
            &Handle::for_test(),
        );
        assert!(model.pincel_drag.is_none());
        assert!(model.ultima_etiqueta_snapshot.is_none());
        // Un segundo trazo arranca entrada NUEVA (la cadena se cortó).
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarTrazo { lx: 5.0, ly: 5.0, rw: 8.0, rh: 8.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.historial.len(), 3);
        // Un Undo deshace sólo el segundo trazo.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.cursor_historial, 1);
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
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
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
        assert_eq!(model.historial.len(), 2);
    }

    // ---- Máscaras de capa (fase 52) ----

    /// Reemplaza el buffer de la capa seleccionada por uno opaco blanco
    /// (alfa 255 en todo), para que el efecto de una máscara sobre el
    /// alfa sea observable en el composite.
    fn opacar_capa(model: &mut Model) {
        let id = model.seleccionada.unwrap();
        let n = (model.lienzo.width * model.lienzo.height) as usize;
        let hash = model.almacen.insertar(vec![255u8; n * 4]);
        model.lienzo.capa_mut(id).unwrap().contenido = hash;
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
        assert_eq!(model.historial.len(), 2);
    }

    #[test]
    fn agregar_mascara_es_idempotente() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        model = <Tullpu as App>::update(model, Msg::AgregarMascara, &Handle::for_test());
        let mh1 = model.lienzo.capa(id).unwrap().mascara;
        let hist1 = model.historial.len();
        // Segunda vez: no-op (no pisa la máscara ni snapshotea).
        model = <Tullpu as App>::update(model, Msg::AgregarMascara, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().mascara, mh1);
        assert_eq!(model.historial.len(), hist1);
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
        assert_eq!((img.width, img.height), (lado, lado));
        let data = img.data.data();
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
