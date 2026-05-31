//! `tullpu` — app de escritorio Llimphi: lienzo central + panel de capas +
//! paleta de operaciones (locales e IA). MVP del editor de imágenes por
//! capas IA-able.
//!
//! Layout:
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────┐
//! │ header: dimensiones · proveedor IA · estado               │
//! ├──────────────┬─────────────────────────────┬──────────────┤
//! │ capas        │                             │ locales      │
//! │  • fondo     │        LIENZO compuesto     │  + Invertir  │
//! │  • inversión │        (peniko::Image)      │  + Brillo+   │
//! │  • brillo    │                             │  …           │
//! │              │                             │ IA           │
//! │ [+ raster]   │                             │  + Restyle   │
//! │              │                             │  + Segmentar │
//! │              │                             │  + Inpaint   │
//! │              │                             │  + Generar   │
//! └──────────────┴─────────────────────────────┴──────────────┘
//! ```
//!
//! Cada panel de capa es un botón clicable que la selecciona; el panel
//! derecho aplica una op nueva como capa derivada de la seleccionada.
//! Las ops IA se delegan al [`pixel_verbo_core::Proveedor`] que la app
//! resuelve al arranque: si encuentra el daemon `pixel-verbo-daemon` en
//! `$XDG_RUNTIME_DIR/pixel-verbo.sock` lo usa; si no, cae al `ProveedorMock`
//! en proceso — así el botón "Generar" igual funciona sin daemon corriendo.
//! Cada cambio dispara `regenerar_stale_con_ia` + `componer` sincrónicamente.
//!
//! ## Hotkeys
//!
//! Actúan sobre la capa seleccionada (excepto los de export/picker que son
//! globales). Si el picker está abierto las teclas van al filtro, no acá.
//!
//! - `Ctrl+P`         — abre fuzzy file picker para agregar capa
//! - `Delete` / `Backspace` — con selección activa, limpia los píxeles
//!   del rect (alfa=0) en la capa raster; sin selección, elimina la
//!   capa entera
//! - `Shift+Delete` / `Shift+Backspace` — con selección activa, rellena
//!   el rect con el color activo (cuentagotas o gris default)
//! - `Ctrl+D`         — duplicar capa
//! - `Ctrl+J`         — con selección activa, duplica el rect a una
//!   capa raster nueva (layer via copy)
//! - `Ctrl+C` / `Ctrl+X` — copiar / cortar el rect de la selección al
//!   portapapeles interno
//! - `Ctrl+V`         — pegar el portapapeles como capa nueva
//! - `Ctrl+A`         — seleccionar todo el lienzo
//! - `g`              — herramienta balde (flood fill); click rellena la
//!   región contigua con el color activo (acotado a la selección)
//! - `p`              — herramienta pincel; drag pinta un trazo a mano
//!   alzada con el color activo (acotado a la selección)
//! - `e`              — herramienta borrador (goma): drag borra (alfa=0)
//! - `[` / `]`        — con pincel/borrador, ∓1 al radio; si no, opacidad
//! - `{` / `}`        — con pincel/borrador, ∓10% a la dureza (borde)
//! - `Shift`+click (pincel/borrador) — traza una línea recta desde el
//!   último punto pintado hasta el click
//! - `s`              — cicla la simetría del trazo (✕/↔/↕/✛)
//! - `d`              — herramienta degradé; drag rellena un degradado
//!   lineal del color activo a transparente (acotado a la selección)
//! - `←` `↑` `↓` `→`  — con selección activa, mueve sus píxeles 1 px
//!   (10 px con `Shift`) dentro de la capa raster
//!
//! Con la herramienta Marco, arrastrar DESDE ADENTRO de una selección
//! existente mueve su contenido (drag-to-move); arrastrar desde afuera
//! arma un marquee nuevo.
//! - `F2`             — renombrar capa in-situ (Enter confirma · Esc cancela)
//! - `V`              — toggle visibilidad
//! - `B` / `Shift+B`  — ciclar blend forward / reverse
//! - `Ctrl+Z` / `Ctrl+Shift+Z` (o `Ctrl+Y`) — undo / redo
//! - `Ctrl+S` / `Ctrl+Shift+S` — exportar PNG / WebP

#![forbid(unsafe_code)]

mod blend;
mod carga;
mod compose;
mod historial;
mod hotkeys;
mod model;
mod ops;
mod view;
mod viewport;

use std::path::PathBuf;

use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_module_file_picker::{self as picker, PickerMsg, PickerPalette};
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, FlexDirection, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::{
    App, Handle, Key, KeyEvent, Modifiers, NamedKey, View, WheelDelta,
};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_edit_menu::{self as editmenu, EditFlags};
use llimphi_widget_menubar::{
    menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_text_input::TextInputState;

use tullpu_core::{Capa, TransformacionPixel};
use tullpu_ops::transformacion_ia;

use crate::blend::*;
use crate::carga::*;
use crate::compose::*;
use crate::historial::*;
use crate::hotkeys::*;
use crate::model::*;
use crate::ops::*;
use crate::view::*;
use crate::viewport::*;

// =============================================================================
//  App
// =============================================================================

struct Tullpu;

impl App for Tullpu {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "tullpu · editor de imágenes por capas"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        inicializar()
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Seleccionar(id) => {
                model.seleccionada = Some(id);
            }
            Msg::ToggleVisible(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.visible = !c.visible;
                }
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::BumpOpacidad(id, delta) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.opacidad = (c.opacidad + delta).clamp(0.0, 1.0);
                }
                aplicar_y_recomponer(&mut model);
                // Coalesce: un drag continuo del slider sobre la misma capa
                // colapsa a una sola entrada de historial.
                pushear_snapshot(&mut model, Some((id, "opacidad")));
            }
            Msg::CiclarBlend(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.blend = siguiente_blend(c.blend);
                }
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::CiclarBlendInverso(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.blend = blend_anterior(c.blend);
                }
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::MoverArriba(id) => {
                // Reordenar no toca dependencias por Uuid, así que basta
                // recomponer — `regenerar_stale_con_ia` corre igual y es
                // barato si nada está stale.
                if model.lienzo.mover_arriba(id) {
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::MoverAbajo(id) => {
                if model.lienzo.mover_abajo(id) {
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Duplicar(id) => {
                if let Some(nuevo) = model.lienzo.duplicar(id) {
                    model.seleccionada = Some(nuevo);
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Eliminar(id) => {
                model.lienzo.capas.retain(|c| c.id != id);
                if model.seleccionada == Some(id) {
                    model.seleccionada = model.lienzo.capas.last().map(|c| c.id);
                }
                // Las capas derivadas que quedaron huérfanas se marcan stale
                // — su regeneración fallará silenciosamente (BufferFaltante).
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::Agregar(op) => {
                if let Some(madre_id) = model.seleccionada {
                    // El contenido_cache inicial lo dejamos en ceros — el
                    // orquestador lo rellena en la siguiente regeneración.
                    let nueva = Capa::derivada(
                        format!("{}", op_etiqueta(&op)),
                        madre_id,
                        TransformacionPixel::Local(op),
                        [0u8; 32],
                    );
                    let nuevo_id = nueva.id;
                    model.lienzo.apilar(nueva);
                    model.seleccionada = Some(nuevo_id);
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AgregarIa(op) => {
                if let Some(madre_id) = model.seleccionada {
                    let modelo = model.proveedor.model_id().name.clone();
                    let nombre = format!("ia:{}", op.etiqueta());
                    let trans = transformacion_ia(modelo, &op);
                    let nueva = Capa::derivada(nombre, madre_id, trans, [0u8; 32]);
                    let nuevo_id = nueva.id;
                    model.lienzo.apilar(nueva);
                    model.seleccionada = Some(nuevo_id);
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Recargar => {
                aplicar_y_recomponer(&mut model);
            }
            Msg::Picker(pm) => {
                model = aplicar_picker(model, pm);
            }
            Msg::IniciarRenombrar(id) => {
                // Pre-cargar el text-input con el nombre actual para que
                // editar sea "tocar el final" en vez de "borrar todo y
                // tipear de nuevo".
                if let Some(c) = model.lienzo.capas.iter().find(|c| c.id == id) {
                    let mut input = TextInputState::new();
                    input.set_text(c.nombre.clone());
                    model.renombrando = Some((id, input));
                    model.seleccionada = Some(id);
                    model.estado = "renombrando · Enter confirma · Esc cancela".into();
                }
            }
            Msg::TeclaRenombrar(ev) => {
                if let Some((_, input)) = model.renombrando.as_mut() {
                    input.apply_key(&ev);
                }
            }
            Msg::ConfirmarRenombrar => {
                if let Some((id, input)) = model.renombrando.take() {
                    let nuevo = input.text();
                    let mut cambio = false;
                    if !nuevo.trim().is_empty() {
                        if let Some(c) = model.lienzo.capa_mut(id) {
                            if c.nombre != nuevo {
                                c.nombre = nuevo;
                                cambio = true;
                            }
                        }
                    }
                    if cambio {
                        pushear_snapshot(&mut model, None);
                    }
                    model.estado = "listo".into();
                }
            }
            Msg::CancelarRenombrar => {
                model.renombrando = None;
                model.estado = "listo".into();
            }
            Msg::FileDrop(path) => {
                // Drag&drop OS-level: reusamos exactamente el mismo path
                // que el picker. Si la extensión no está en el catálogo
                // soportado (PNG/JPEG), `agregar_capa_desde_archivo` falla
                // al decodificar y deja el lienzo intacto con un estado
                // descriptivo — no preflight check para mantener una sola
                // rama de error.
                if agregar_capa_desde_archivo(&mut model, &path) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Undo => {
                if aplicar_undo(&mut model) {
                    ajustar_seleccion_tras_restaurar(&mut model);
                    aplicar_y_recomponer(&mut model);
                    model.estado = format!(
                        "↶ undo · {}/{}",
                        model.cursor_historial + 1,
                        model.historial.len()
                    );
                } else {
                    model.estado = "↶ nada que deshacer".into();
                }
            }
            Msg::Redo => {
                if aplicar_redo(&mut model) {
                    ajustar_seleccion_tras_restaurar(&mut model);
                    aplicar_y_recomponer(&mut model);
                    model.estado = format!(
                        "↷ redo · {}/{}",
                        model.cursor_historial + 1,
                        model.historial.len()
                    );
                } else {
                    model.estado = "↷ nada que rehacer".into();
                }
            }
            Msg::Zoom { mult, ancla } => {
                let zoom_anterior = model.factor_zoom;
                let zoom_nuevo = (zoom_anterior * mult).clamp(ZOOM_MIN, ZOOM_MAX);
                // Si el cursor está sobre el lienzo (ancla = Some), ajustamos
                // pan para que el píxel bajo el cursor quede fijo
                // (zoom-a-cursor) — la sensación natural de un image editor.
                // Sin ancla, dejamos pan tal cual: el centro de la imagen
                // mostrada permanece fijo (consecuencia de la ecuación de
                // offset).
                if let (Some((rect, cx, cy)), Some(img)) = (ancla, model.imagen.as_ref()) {
                    let (pan_x_nuevo, pan_y_nuevo) = pan_para_zoom_a_cursor(
                        img.width,
                        img.height,
                        rect,
                        cx,
                        cy,
                        zoom_anterior,
                        zoom_nuevo,
                        model.pan_x,
                        model.pan_y,
                    );
                    model.pan_x = pan_x_nuevo;
                    model.pan_y = pan_y_nuevo;
                }
                model.factor_zoom = zoom_nuevo;
            }
            Msg::Pan(dx, dy) => {
                model.pan_x += dx;
                model.pan_y += dy;
            }
            Msg::ResetVista => {
                model.factor_zoom = 1.0;
                model.pan_x = 0.0;
                model.pan_y = 0.0;
                model.estado = "vista reseteada".into();
            }
            Msg::CambiarHerramienta(h) => {
                model.herramienta = h;
                model.estado = format!("herramienta · {}", h.etiqueta());
            }
            Msg::AgregarRelleno => {
                if agregar_capa_relleno(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Combinar(id) => {
                if combinar_capa_abajo(&mut model, id) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AplanarVisibles => {
                if aplanar_capas_visibles(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::RotarLienzo { cw } => {
                if rotar_lienzo(&mut model, cw) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AutotrimLienzo => {
                if recortar_lienzo_a_visible(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AjustarParametro { id, param, dv } => {
                if ajustar_parametro_derivada(&mut model, id, param, dv) {
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, Some((id, param.clave_coalesce())));
                }
            }
            Msg::CurvaPress { id, lx, ly, rw, rh } => {
                // El press recompone en vivo (muestra el punto recién
                // enganchado/insertado); el snapshot lo difiere al
                // `CurvaSoltar` para que todo el gesto sea 1 sola entrada
                // de historial.
                curva_press(&mut model, id, lx, ly, rw, rh);
            }
            Msg::CurvaArrastrar { id, dx, dy } => {
                curva_arrastrar(&mut model, id, dx, dy);
            }
            Msg::CurvaSoltar { id } => {
                model.curva_arrastrando = None;
                pushear_snapshot(&mut model, Some((id, "curva")));
            }
            Msg::CurvaReset { id } => {
                if curva_reset(&mut model, id) {
                    pushear_snapshot(&mut model, Some((id, "curva-reset")));
                }
            }
            Msg::IniciarSeleccion { lx, ly, rw, rh } => {
                // Capturamos el ancla en coords-imagen y empezamos el
                // drag. Si la conversión local→imagen falla (lienzo
                // degenerado), descartamos el press.
                if let (Some(img), Some((ix, iy))) = (
                    model.imagen.as_ref(),
                    local_a_imagen(
                        lx,
                        ly,
                        rw,
                        rh,
                        // image_w / image_h: usamos las del lienzo,
                        // no del peniko::Image (en general coinciden,
                        // pero el lienzo es la fuente de verdad).
                        model.lienzo.width,
                        model.lienzo.height,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    ),
                ) {
                    let _ = img;
                    let pix_x = ix.floor() as i32;
                    let pix_y = iy.floor() as i32;
                    // Si el press cae DENTRO de la selección vigente,
                    // arrastramos su contenido (drag-to-move) en vez de
                    // construir un marquee nuevo (Photoshop standard).
                    let dentro = model.seleccion.is_some_and(|r| {
                        pix_x >= r.x0 as i32
                            && pix_x < r.x1 as i32
                            && pix_y >= r.y0 as i32
                            && pix_y < r.y1 as i32
                    });
                    if dentro {
                        model.mover_drag = Some(MoverDrag {
                            press_lx: lx,
                            press_ly: ly,
                            cur_lx: lx,
                            cur_ly: ly,
                            rw,
                            rh,
                            aplicado_ix: 0,
                            aplicado_iy: 0,
                        });
                    } else {
                        model.seleccion_drag = Some(SeleccionDrag {
                            ancla_ix: pix_x,
                            ancla_iy: pix_y,
                            cur_lx: lx,
                            cur_ly: ly,
                            rw,
                            rh,
                        });
                        // Press fuera limpia la selección previa — vamos
                        // a construir una nueva sobre la marcha.
                        model.seleccion = None;
                    }
                }
            }
            Msg::AjustarSeleccion { dx, dy } => {
                if let Some(drag) = model.seleccion_drag.as_mut() {
                    drag.cur_lx += dx;
                    drag.cur_ly += dy;
                    let drag = *drag;
                    model.seleccion = rect_imagen_desde_drag(
                        &drag,
                        model.lienzo.width,
                        model.lienzo.height,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    );
                } else if let Some(md) = model.mover_drag.as_mut() {
                    // Drag-to-move: acumular el desplazamiento local,
                    // convertir a coords-imagen vía la escala del fit, y
                    // mover el contenido por el paso entero que todavía
                    // falte aplicar (el resto sub-píxel queda en cur-press).
                    md.cur_lx += dx;
                    md.cur_ly += dy;
                    let md = *md;
                    if let Some((s, _, _)) = transform_lienzo(
                        model.lienzo.width,
                        model.lienzo.height,
                        md.rw,
                        md.rh,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    ) {
                        if s > 0.0 {
                            let total_ix = (((md.cur_lx - md.press_lx) as f64)
                                / s)
                                .round() as i32;
                            let total_iy = (((md.cur_ly - md.press_ly) as f64)
                                / s)
                                .round() as i32;
                            let paso_x = total_ix - md.aplicado_ix;
                            let paso_y = total_iy - md.aplicado_iy;
                            if (paso_x != 0 || paso_y != 0)
                                && mover_pixeles_seleccion(
                                    &mut model, paso_x, paso_y,
                                )
                            {
                                if let Some(m) = model.mover_drag.as_mut() {
                                    m.aplicado_ix = total_ix;
                                    m.aplicado_iy = total_iy;
                                }
                                // Coalesce: todo el drag = un solo Undo.
                                let etiqueta =
                                    model.seleccionada.map(|i| (i, "mover_sel"));
                                pushear_snapshot(&mut model, etiqueta);
                            }
                        }
                    }
                }
            }
            Msg::FinalizarSeleccion => {
                // Si veníamos arrastrando contenido, cerramos ese drag y
                // dejamos la selección donde quedó (siguió al contenido).
                if model.mover_drag.take().is_some() {
                    if let Some(rect) = model.seleccion {
                        model.estado = format!(
                            "movida a ({},{})",
                            rect.x0, rect.y0
                        );
                    }
                    return model;
                }
                // Commit final: si el rect quedó válido al fin del drag
                // ya está en `seleccion`. Si era un click sin
                // movimiento (área cero), `seleccion` quedó None
                // — limpiamos también el drag y avisamos.
                model.seleccion_drag = None;
                if let Some(rect) = model.seleccion {
                    model.estado = format!(
                        "selección {}×{} @ ({},{})",
                        rect.x1 - rect.x0,
                        rect.y1 - rect.y0,
                        rect.x0,
                        rect.y0
                    );
                } else {
                    model.estado = "selección vacía — Esc o re-drag".into();
                }
            }
            Msg::LimpiarSeleccion => {
                model.seleccion = None;
                model.seleccion_drag = None;
                model.estado = "selección limpia".into();
            }
            Msg::SeleccionarTodo => {
                let w = model.lienzo.width;
                let h = model.lienzo.height;
                if w > 0 && h > 0 {
                    model.seleccion =
                        Some(RectImagen { x0: 0, y0: 0, x1: w, y1: h });
                    model.seleccion_drag = None;
                    model.mover_drag = None;
                    model.estado = format!("seleccionado todo ({}×{})", w, h);
                }
            }
            Msg::ExpandirSeleccion(delta) => {
                if let Some(rect) = model.seleccion {
                    let w = model.lienzo.width;
                    let h = model.lienzo.height;
                    model.seleccion = expandir_rect(rect, delta, w, h);
                    model.estado = match model.seleccion {
                        Some(r) => {
                            format!("selección {}×{}", r.x1 - r.x0, r.y1 - r.y0)
                        }
                        None => "selección colapsada — limpia".into(),
                    };
                } else {
                    model.estado = "no hay selección que ajustar".into();
                }
            }
            Msg::RecortarASeleccion => {
                if recortar_lienzo_a_seleccion(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::LimpiarSeleccionEnCapa => {
                if limpiar_seleccion_en_capa(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::RellenarSeleccionEnCapa => {
                if rellenar_seleccion_en_capa(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::DuplicarSeleccionACapa => {
                if duplicar_seleccion_a_capa(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::CopiarSeleccion => {
                // No destructivo: nunca snapshotea.
                copiar_seleccion(&mut model);
            }
            Msg::CortarSeleccion => {
                if cortar_seleccion(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::PegarPortapapeles => {
                if pegar_portapapeles(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::MoverSeleccion { dx, dy } => {
                // Coalesce por capa: una ráfaga de flechas (o nudges
                // sucesivos) colapsa a un solo Undo, como el drag de
                // opacidad. Captura el id ANTES de mover (mover puede
                // dejar la selección fuera, pero la capa no cambia).
                let id = model.seleccionada;
                if mover_pixeles_seleccion(&mut model, dx, dy) {
                    let etiqueta = id.map(|i| (i, "mover_sel"));
                    pushear_snapshot(&mut model, etiqueta);
                }
            }
            Msg::RecogerColor { lx, ly, rw, rh } => {
                if let Some(img) = model.imagen.as_ref() {
                    let bytes = img.data.data();
                    match recoger_color_en(
                        bytes,
                        img.width,
                        img.height,
                        lx,
                        ly,
                        rw,
                        rh,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    ) {
                        Some(rgba) => {
                            model.color_picked = Some(rgba);
                            model.estado = format!(
                                "color · #{:02X}{:02X}{:02X} α={}",
                                rgba[0], rgba[1], rgba[2], rgba[3]
                            );
                        }
                        None => {
                            // Click cayó fuera de la imagen (en el pad del
                            // fit-contain o en el borde). Dejamos
                            // `color_picked` tal cual y avisamos.
                            model.estado = "color · fuera de la imagen".into();
                        }
                    }
                }
            }
            Msg::RellenarFlood { lx, ly, rw, rh } => {
                // Convertir el click local a coord-imagen (misma inversa
                // que el marquee) y floodear desde ahí.
                if let Some((ix, iy)) = local_a_imagen(
                    lx,
                    ly,
                    rw,
                    rh,
                    model.lienzo.width,
                    model.lienzo.height,
                    model.factor_zoom,
                    model.pan_x,
                    model.pan_y,
                ) {
                    let sx = ix.floor() as u32;
                    let sy = iy.floor() as u32;
                    if rellenar_flood_en_capa(&mut model, sx, sy) {
                        pushear_snapshot(&mut model, None);
                    }
                } else {
                    model.estado = "balde · fuera de la imagen".into();
                }
            }
            Msg::IniciarTrazo { lx, ly, rw, rh } => {
                if let Some((ix, iy)) = local_a_imagen(
                    lx,
                    ly,
                    rw,
                    rh,
                    model.lienzo.width,
                    model.lienzo.height,
                    model.factor_zoom,
                    model.pan_x,
                    model.pan_y,
                ) {
                    let cx = ix.floor() as i32;
                    let cy = iy.floor() as i32;
                    model.pincel_drag = Some(PincelDrag {
                        cur_lx: lx,
                        cur_ly: ly,
                        rw,
                        rh,
                        last_ix: cx,
                        last_iy: cy,
                    });
                    let borrar = model.herramienta == Herramienta::Borrador;
                    let radio = model.radio_pincel;
                    let dureza = model.dureza_pincel;
                    // Shift + click con un punto previo: trazo en LÍNEA
                    // RECTA desde el último punto hasta acá (Photoshop).
                    // Si no, una pincelada puntual.
                    let sim = model.simetria;
                    let cambio = match (model.shift_held, model.ultimo_pincel) {
                        (true, Some((px, py))) => pincel_segmento_en_capa(
                            &mut model, px, py, cx, cy, radio, borrar, dureza,
                            sim,
                        ),
                        _ => pincel_punto_en_capa(
                            &mut model, cx, cy, radio, borrar, dureza, sim,
                        ),
                    };
                    if cambio {
                        let etiqueta = model.seleccionada.map(|i| (i, "pincel"));
                        pushear_snapshot(&mut model, etiqueta);
                    }
                    model.ultimo_pincel = Some((cx, cy));
                }
            }
            Msg::ContinuarTrazo { dx, dy } => {
                if let Some(pd) = model.pincel_drag.as_mut() {
                    pd.cur_lx += dx;
                    pd.cur_ly += dy;
                    let pd = *pd;
                    if let Some((ix, iy)) = local_a_imagen(
                        pd.cur_lx,
                        pd.cur_ly,
                        pd.rw,
                        pd.rh,
                        model.lienzo.width,
                        model.lienzo.height,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    ) {
                        let nx = ix.floor() as i32;
                        let ny = iy.floor() as i32;
                        let borrar = model.herramienta == Herramienta::Borrador;
                        let radio = model.radio_pincel;
                        let dureza = model.dureza_pincel;
                        let sim = model.simetria;
                        if pincel_segmento_en_capa(
                            &mut model,
                            pd.last_ix,
                            pd.last_iy,
                            nx,
                            ny,
                            radio,
                            borrar,
                            dureza,
                            sim,
                        ) {
                            let etiqueta =
                                model.seleccionada.map(|i| (i, "pincel"));
                            pushear_snapshot(&mut model, etiqueta);
                        }
                        // Avanzar el último punto aunque el segmento no
                        // cambiara (p.ej. pintar sobre el mismo color):
                        // el trazo sigue desde donde está el cursor.
                        if let Some(p) = model.pincel_drag.as_mut() {
                            p.last_ix = nx;
                            p.last_iy = ny;
                        }
                        // Persistir el último punto para el ancla del
                        // próximo trazo recto con Shift.
                        model.ultimo_pincel = Some((nx, ny));
                    }
                }
            }
            Msg::FinalizarTrazo => {
                model.pincel_drag = None;
                // Cortar el coalesce: el próximo trazo es un Undo aparte.
                model.ultima_etiqueta_snapshot = None;
            }
            Msg::BumpRadioPincel(delta) => {
                model.radio_pincel =
                    (model.radio_pincel + delta).clamp(0, RADIO_PINCEL_MAX);
                model.estado =
                    format!("radio pincel {} px", model.radio_pincel * 2 + 1);
            }
            Msg::BumpDurezaPincel(delta) => {
                model.dureza_pincel =
                    (model.dureza_pincel + delta).clamp(0.0, 1.0);
                model.estado = format!(
                    "dureza pincel {}%",
                    (model.dureza_pincel * 100.0).round() as i32
                );
            }
            Msg::SetShift(v) => {
                model.shift_held = v;
            }
            Msg::CiclarSimetria => {
                model.simetria = model.simetria.siguiente();
                model.estado = format!("simetría: {}", model.simetria.etiqueta());
            }
            Msg::IniciarDegradado { lx, ly, rw, rh } => {
                if let Some((ix, iy)) = local_a_imagen(
                    lx,
                    ly,
                    rw,
                    rh,
                    model.lienzo.width,
                    model.lienzo.height,
                    model.factor_zoom,
                    model.pan_x,
                    model.pan_y,
                ) {
                    model.gradiente_drag = Some(GradienteDrag {
                        ancla_ix: ix as f32,
                        ancla_iy: iy as f32,
                        cur_lx: lx,
                        cur_ly: ly,
                        rw,
                        rh,
                    });
                }
            }
            Msg::AjustarDegradado { dx, dy } => {
                if let Some(g) = model.gradiente_drag.as_mut() {
                    g.cur_lx += dx;
                    g.cur_ly += dy;
                }
            }
            Msg::FinalizarDegradado => {
                if let Some(g) = model.gradiente_drag.take() {
                    if let Some((bx, by)) = local_a_imagen(
                        g.cur_lx,
                        g.cur_ly,
                        g.rw,
                        g.rh,
                        model.lienzo.width,
                        model.lienzo.height,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    ) {
                        if rellenar_gradiente_en_capa(
                            &mut model,
                            g.ancla_ix,
                            g.ancla_iy,
                            bx as f32,
                            by as f32,
                        ) {
                            pushear_snapshot(&mut model, None);
                        }
                    }
                }
            }
            Msg::AgregarMascara => {
                if agregar_mascara(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AgregarMascaraDeSeleccion => {
                if agregar_mascara_de_seleccion(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::InvertirMascara => {
                if invertir_mascara(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::QuitarMascara => {
                if quitar_mascara(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AplicarMascara => {
                if aplicar_mascara(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::ToggleEditarMascara => {
                model.editando_mascara = !model.editando_mascara;
                model.estado = if model.editando_mascara {
                    format!(
                        "editando máscara (pincel → {} · borrador oculta)",
                        model.valor_mascara
                    )
                } else {
                    "editando contenido".into()
                };
            }
            Msg::BumpValorMascara(delta) => {
                let nuevo = (model.valor_mascara as i32 + delta).clamp(0, 255);
                model.valor_mascara = nuevo as u8;
                model.estado = format!("valor pincel máscara {} (gris)", model.valor_mascara);
            }
            Msg::Exportar(formato) => {
                // Path en CWD con timestamp Unix — sin file picker (la app
                // todavía no tiene). La extensión la elige el formato; el
                // usuario ve el path final en el header.
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let ext = extension_export(formato);
                let ruta = std::path::PathBuf::from(format!("tullpu-export-{ts}.{ext}"));
                model.estado = match tullpu_render::exportar(
                    &model.lienzo,
                    &model.almacen,
                    &ruta,
                    formato,
                ) {
                    Ok(_) => format!("exportado → {}", ruta.display()),
                    Err(e) => format!("export falló: {e}"),
                };
            }
            Msg::MenuOpen(idx) => {
                model.menu_open = idx;
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.context_menu = None;
                model.edit_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.context_menu = None;
                model = handle_menu_command(model, &cmd, handle);
            }
            Msg::RightPressAt { x, y } => {
                // Si estamos renombrando una capa, el right-click abre el
                // menú de edición de TEXTO sobre el input. Si no, el menú
                // contextual de capa/selección. El menú contextual de
                // Llimphi clampa al viewport, así que un pequeño desfase de
                // ancla es irrelevante para un MVP.
                if model.renombrando.is_some() {
                    model.context_menu = None;
                    model.edit_menu = Some((x, y));
                } else {
                    model.edit_menu = None;
                    model.context_menu = Some((x, y));
                }
            }
            Msg::EditMenuAction(action) => {
                model.edit_menu = None;
                if let Some((_, input)) = model.renombrando.as_mut() {
                    let _ = editmenu::apply(
                        input.editor_mut(),
                        action,
                        &mut model.clipboard,
                    );
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = llimphi_theme::Theme::dark();
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let cabecera = header(
            &theme,
            &model.lienzo,
            &model.estado,
            &model.proveedor_etiqueta,
            model.factor_zoom,
            model.herramienta,
            model.color_picked,
        );
        // El panel del lienzo lleva el right-click → menú contextual. Sus
        // coords locales no son las de ventana (vive bajo menubar+header),
        // pero el menú clampa al viewport, suficiente para el MVP.
        let lienzo = panel_lienzo(&theme, model)
            .on_right_click_at(|x, y, _w, _h| Some(Msg::RightPressAt { x, y }));
        let centro = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            panel_capas(&theme, model),
            lienzo,
            panel_ops(&theme, model),
        ]);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menubar, cabecera, centro])
    }

    fn on_wheel(
        _model: &Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Msg> {
        // Sólo zoom-eamos si el cursor está sobre el lienzo. Si está en
        // los paneles laterales, dejamos pasar (futuro: scroll vertical
        // del panel de capas si crece). delta.y > 0 ⇒ scroll hacia abajo ⇒
        // zoom out (convención CSS — ver `WheelDelta`).
        let rect = lienzo_rect_get()?;
        if !dentro_de_rect(rect, cursor.0, cursor.1) {
            return None;
        }
        let mult = ZOOM_BASE.powf(-delta.y);
        Some(Msg::Zoom {
            mult,
            ancla: Some((rect, cursor.0, cursor.1)),
        })
    }

    fn on_file_drop(_model: &Model, path: PathBuf) -> Option<Msg> {
        // Cualquier archivo soltado se procesa por la misma vía que el
        // picker. Si no es PNG/JPEG la decodificación falla y el estado
        // refleja el error — sin diálogo modal, sin preflight.
        Some(Msg::FileDrop(path))
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        use llimphi_ui::KeyState;
        // Picker abierto: el módulo decide qué hacer con cada tecla
        // (input, navegación, apply, escape). Tiene prioridad sobre los
        // atajos globales para que escribir en el filtro no abra otro popup.
        if let Some(state) = model.picker.as_ref() {
            if let Some(pm) = picker::on_key(state, event) {
                return Some(Msg::Picker(pm));
            }
            return None;
        }
        // Renombrando una capa: las teclas van al text-input, salvo Enter
        // (confirma) y Escape (cancela). Mismo patrón que el picker: el
        // modo modal absorbe los atajos globales.
        if model.renombrando.is_some() {
            if event.state == KeyState::Pressed {
                match &event.key {
                    Key::Named(NamedKey::Enter) => return Some(Msg::ConfirmarRenombrar),
                    Key::Named(NamedKey::Escape) => return Some(Msg::CancelarRenombrar),
                    _ => {}
                }
            }
            return Some(Msg::TeclaRenombrar(event.clone()));
        }
        // Sincronizar el estado vivo de Shift: el handler de click no
        // recibe modifiers, así que lo trackeamos desde la tecla para
        // habilitar el trazo recto (Shift+click). Las dos Shift (izq/der)
        // llegan como NamedKey::Shift.
        if matches!(event.key, Key::Named(NamedKey::Shift)) {
            return Some(Msg::SetShift(event.state == KeyState::Pressed));
        }
        // Ctrl+P abre el fuzzy picker (mismo atajo que nada y VS Code).
        if picker::open_shortcut(event) {
            return Some(Msg::Picker(PickerMsg::Open));
        }
        hotkey_a_msg(model, event)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let theme = llimphi_theme::Theme::dark();
        // Prioridad 1: menú de edición de texto (sólo durante renombrado).
        if let Some((x, y)) = model.edit_menu {
            if let Some((_, input)) = model.renombrando.as_ref() {
                let flags = EditFlags::from_editor(input.editor(), input.is_masked());
                return Some(context_menu_view(editmenu::edit_context_menu(
                    (x, y),
                    viewport_of(),
                    &theme,
                    flags,
                    Msg::EditMenuAction,
                    Msg::CloseMenus,
                )));
            }
        }
        // Prioridad 2: menú contextual de capa/selección.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_canvas(model, &theme, x, y));
        }
        // Prioridad 3: dropdown del menú principal.
        if model.menu_open.is_some() {
            let menu = app_menu(model);
            if let Some(v) = menubar_overlay(&menubar_spec(&menu, model, &theme)) {
                return Some(v);
            }
        }
        // Prioridad 4: el picker (overlay preexistente).
        let state = model.picker.as_ref()?;
        let palette = PickerPalette::from_theme(&theme);
        let panel = picker::view(
            state,
            &model.imagenes_disponibles,
            &model.raiz,
            &palette,
            Msg::Picker,
        );
        // Envuelvo el panel en un contenedor con padding lateral generoso
        // para centrarlo visualmente sobre el lienzo — el módulo devuelve
        // un View de `100% × 220px` que sin esto se pegaría al borde.
        Some(
            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                padding: Rect {
                    left: length(120.0_f32),
                    right: length(120.0_f32),
                    top: length(80.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![panel]),
        )
    }
}

// =============================================================================
//  Menú principal + menú contextual + menú de edición de texto
// =============================================================================

/// Viewport para clampear overlays. La app no trackea el tamaño de
/// ventana, así que usamos las dims iniciales — el menú clampa a esto y un
/// resize sólo desfasa el clamp, no rompe la usabilidad (MVP).
fn viewport_of() -> (f32, f32) {
    let (w, h) = Tullpu::initial_size();
    (w as f32, h as f32)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a AppMenu,
    model: &Model,
    theme: &'a llimphi_theme::Theme,
) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Menú principal de tullpu. Refleja en gris el estado real (sin capa
/// seleccionada / sin historial / sin selección). Sólo comandos que mapean
/// a `Msg` ya existentes — nada inventado.
fn app_menu(model: &Model) -> AppMenu {
    let hay_capa = model.seleccionada.is_some();
    let hay_sel = model.seleccion.is_some();
    let can_undo = model.cursor_historial > 0;
    let can_redo = model.cursor_historial + 1 < model.historial.len();

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Shift+Z");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut duplicar = MenuItem::new("Duplicar capa", "edit.duplicar").shortcut("Ctrl+D");
    let mut eliminar = MenuItem::new("Eliminar capa", "edit.eliminar");
    let mut combinar = MenuItem::new("Combinar hacia abajo", "edit.combinar");
    if !hay_capa {
        duplicar = duplicar.disabled();
        eliminar = eliminar.disabled();
        combinar = combinar.disabled();
    }
    let aplanar = MenuItem::new("Aplanar visibles", "edit.aplanar").separated();

    let mut copiar = MenuItem::new("Copiar", "edit.copiar").shortcut("Ctrl+C").separated();
    let mut cortar = MenuItem::new("Cortar", "edit.cortar").shortcut("Ctrl+X");
    let mut dup_sel = MenuItem::new("Duplicar selección a capa", "edit.dup_sel").shortcut("Ctrl+J");
    let mut recortar = MenuItem::new("Recortar a selección", "edit.recortar_sel");
    let mut limpiar_sel = MenuItem::new("Limpiar selección", "sel.limpiar").shortcut("Esc");
    if !hay_sel {
        copiar = copiar.disabled();
        cortar = cortar.disabled();
        dup_sel = dup_sel.disabled();
        recortar = recortar.disabled();
        limpiar_sel = limpiar_sel.disabled();
    }
    let pegar = MenuItem::new("Pegar", "edit.pegar").shortcut("Ctrl+V");
    let sel_todo = MenuItem::new("Seleccionar todo", "sel.todo").shortcut("Ctrl+A").separated();

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Abrir imagen…", "file.abrir").shortcut("Ctrl+P"))
                .item(MenuItem::new("Exportar PNG", "file.png").shortcut("Ctrl+S").separated())
                .item(MenuItem::new("Exportar JPEG", "file.jpeg"))
                .item(MenuItem::new("Exportar WebP", "file.webp").shortcut("Ctrl+Shift+S")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(duplicar)
                .item(eliminar)
                .item(combinar)
                .item(aplanar)
                .item(copiar)
                .item(cortar)
                .item(pegar)
                .item(dup_sel)
                .item(sel_todo)
                .item(limpiar_sel)
                .item(recortar),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Acercar", "view.zoom_in").shortcut("+"))
                .item(MenuItem::new("Alejar", "view.zoom_out").shortcut("-"))
                .item(MenuItem::new("Restablecer vista", "view.reset").shortcut("0").separated())
                .item(MenuItem::new("Rotar horario", "view.rotar_cw"))
                .item(MenuItem::new("Rotar antihorario", "view.rotar_ccw"))
                .item(MenuItem::new("Recorte automático", "view.autotrim")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id (barra o contextual) al `Msg` real y lo
/// despacha. Todos los ids mapean a acciones que ya existían.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    let sel = model.seleccionada;
    let msg = match cmd {
        "file.abrir" => Some(Msg::Picker(PickerMsg::Open)),
        "file.png" => Some(Msg::Exportar(tullpu_render::FormatoExport::Png)),
        "file.jpeg" => Some(Msg::Exportar(tullpu_render::FormatoExport::Jpeg { calidad: 90 })),
        "file.webp" => Some(Msg::Exportar(tullpu_render::FormatoExport::Webp)),
        "edit.undo" => Some(Msg::Undo),
        "edit.redo" => Some(Msg::Redo),
        "edit.duplicar" => sel.map(Msg::Duplicar),
        "edit.eliminar" => sel.map(Msg::Eliminar),
        "edit.combinar" => sel.map(Msg::Combinar),
        "edit.aplanar" => Some(Msg::AplanarVisibles),
        "edit.copiar" => Some(Msg::CopiarSeleccion),
        "edit.cortar" => Some(Msg::CortarSeleccion),
        "edit.pegar" => Some(Msg::PegarPortapapeles),
        "edit.dup_sel" => Some(Msg::DuplicarSeleccionACapa),
        "edit.recortar_sel" => Some(Msg::RecortarASeleccion),
        "sel.todo" => Some(Msg::SeleccionarTodo),
        "sel.limpiar" => Some(Msg::LimpiarSeleccion),
        "view.zoom_in" => Some(Msg::Zoom { mult: ZOOM_BASE, ancla: None }),
        "view.zoom_out" => Some(Msg::Zoom { mult: 1.0 / ZOOM_BASE, ancla: None }),
        "view.reset" => Some(Msg::ResetVista),
        "view.rotar_cw" => Some(Msg::RotarLienzo { cw: true }),
        "view.rotar_ccw" => Some(Msg::RotarLienzo { cw: false }),
        "view.autotrim" => Some(Msg::AutotrimLienzo),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => None,
    };
    match msg {
        Some(m) => Tullpu::update(model, m, handle),
        None => model,
    }
}

/// Menú contextual sobre el lienzo/capa. Refleja en gris el estado real
/// (sin capa, sin selección, portapapeles vacío). Sólo acciones existentes.
fn context_menu_canvas(
    model: &Model,
    theme: &llimphi_theme::Theme,
    x: f32,
    y: f32,
) -> View<Msg> {
    let hay_capa = model.seleccionada.is_some();
    let hay_sel = model.seleccion.is_some();
    let hay_clip = model.portapapeles.is_some();
    let header = model
        .seleccionada
        .and_then(|id| model.lienzo.capas.iter().find(|c| c.id == id))
        .map(|c| format!("capa · {}", c.nombre))
        .unwrap_or_else(|| "lienzo".to_string());

    let dis = |it: ContextMenuItem, on: bool| if on { it } else { it.disabled() };

    let mut items = vec![
        dis(ContextMenuItem::action("Duplicar capa").with_shortcut("Ctrl+D"), hay_capa),
        dis(ContextMenuItem::action("Combinar hacia abajo"), hay_capa),
        ContextMenuItem::action("Aplanar visibles"),
        ContextMenuItem::separator(),
        dis(ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C"), hay_sel),
        dis(ContextMenuItem::action("Cortar").with_shortcut("Ctrl+X"), hay_sel),
        dis(ContextMenuItem::action("Pegar").with_shortcut("Ctrl+V"), hay_clip),
        dis(ContextMenuItem::action("Duplicar selección a capa").with_shortcut("Ctrl+J"), hay_sel),
        ContextMenuItem::separator(),
        ContextMenuItem::action("Seleccionar todo").with_shortcut("Ctrl+A"),
        dis(ContextMenuItem::action("Limpiar selección"), hay_sel),
    ];
    items.push(dis(
        ContextMenuItem::action("Eliminar capa").destructive(),
        hay_capa,
    ));

    // Mapeo posicional índice → command id de `handle_menu_command`. Los
    // separadores ocupan slot vacío (nunca enganchan click).
    let cmds: Vec<&'static str> = vec![
        "edit.duplicar",
        "edit.combinar",
        "edit.aplanar",
        "",
        "edit.copiar",
        "edit.cortar",
        "edit.pegar",
        "edit.dup_sel",
        "",
        "sel.todo",
        "sel.limpiar",
        "edit.eliminar",
    ];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(),
        header: Some(header),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(theme),
    })
}

fn main() {
    llimphi_ui::run::<Tullpu>();
}


#[cfg(test)]
mod pruebas;
