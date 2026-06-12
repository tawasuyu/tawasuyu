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
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::RellenarSeleccionEnCapa,
            &Handle::for_test(),
        );
        assert_eq!(model.hist.len(), hist_antes + 1);
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
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::DuplicarSeleccionACapa,
            &Handle::for_test(),
        );
        assert_eq!(model.hist.len(), hist_antes + 1);
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
        let hist_antes = model.hist.len();
        let n_capas = model.lienzo.capas.len();
        let ok = copiar_seleccion(&mut model);
        assert!(ok);
        // No tocó historial ni capas.
        assert_eq!(model.hist.len(), hist_antes);
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
        model.hist.reiniciar(model.lienzo.clone());
        let hist_antes = model.hist.len();
        model = <Tullpu as App>::update(
            model,
            Msg::PegarPortapapeles,
            &Handle::for_test(),
        );
        assert_eq!(model.hist.len(), hist_antes + 1);
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
        model.hist.reiniciar(model.lienzo.clone());
        // Tres nudges seguidos coalescen a una sola entrada.
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::MoverSeleccion { dx: 1, dy: 0 },
                &Handle::for_test(),
            );
        }
        assert_eq!(model.hist.len(), 2); // inicial + 1 coalescida
        // Un solo Undo restaura el hash original de la capa.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().contenido, hash_inicial);
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