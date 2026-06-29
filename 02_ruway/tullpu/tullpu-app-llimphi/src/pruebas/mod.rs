//! Tests de la app tullpu — extraídos de `main.rs` para respetar la regla de tamaño del repo (main.rs era ~4600 LOC, ~3800 de ellas tests). El módulo es hermano de la raíz del crate, así `use super::*` resuelve igual que cuando estaba inline.

// Tests partidos en grupos por tamaño (Regla 1). Sin reordenar lógica.
mod grupo_01;
mod grupo_02;
mod grupo_03;
mod grupo_04;
mod grupo_05;
mod grupo_06;

    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use llimphi_ui::llimphi_raster::peniko::{
        Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
    };
    use llimphi_ui::{KeyState, Modifiers, PaintRect};

    use tullpu_core::{
        Frescura, Historial, Lienzo, ModoFusion, OpLocal, OrigenCapa,
    };
    use tullpu_render::{AlmacenEnMemoria, FormatoExport, FuenteBuffers};
    // Kernel puro usado sólo por tests (no re-exportado por `ops` porque el
    // build normal no lo invoca directamente — sólo desde dentro del crate).
    use tullpu_paint::{cobertura_pincel, mezclar_src_over};
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
        let hist = Historial::nuevo(lienzo.clone(), HIST_CAP);
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
            hist,
            factor_zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            herramienta: Herramienta::Mover,
            color_picked: None,
            histograma: None,
            seleccion: None,
            seleccion_mascara: None,
            seleccion_overlay: None,
            seleccion_drag: None,
            mover_drag: None,
            pincel_drag: None,
            radio_pincel: RADIO_PINCEL,
            dureza_pincel: DUREZA_PINCEL,
            shift_held: false,
            alt_held: false,
            clon_ancla: None,
            clon_offset: None,
            ultimo_pincel: None,
            simetria: Simetria::Ninguna,
            gradiente_drag: None,
            lazo_drag: None,
            editando_texto: None,
            portapapeles: None,
            editando_mascara: false,
            valor_mascara: 255,
            thumbs_mascara: HashMap::new(),
            curva_arrastrando: None,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: llimphi_motion::Tween::idle(1.0),
            context_menu: None,
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: llimphi_motion::Tween::idle(1.0),
            clipboard: llimphi_clipboard::SystemClipboard::new(),
            toasts: Vec::new(),
            next_toast: 0,
        }
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
        model.hist.reiniciar(model.lienzo.clone());
        (model, id_b, id_a)
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
        model.hist.reiniciar(model.lienzo.clone());
        (model, ids)
    }

    // ---- Fase 30: rotar lienzo 90° -----------------------------------------

    fn px_at(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
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
        model.hist.reiniciar(model.lienzo.clone());
        (model, id_madre, id_deriv)
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