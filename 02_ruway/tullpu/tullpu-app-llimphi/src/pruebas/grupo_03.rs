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
        let hist_antes = model.hist.len();
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
            model.hist.len(),
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
        let hist_antes = model.hist.len();
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
        assert_eq!(model.hist.len(), hist_antes + 1);
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
        let hist_antes = model.hist.len();
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
            model.hist.len(),
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
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(model, Msg::RecortarASeleccion, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        assert_eq!(model.hist.len(), hist_antes + 1);
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
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::LimpiarSeleccionEnCapa,
            &Handle::for_test(),
        );
        assert_eq!(model.hist.len(), hist_antes + 1);
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