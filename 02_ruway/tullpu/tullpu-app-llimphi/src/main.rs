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
mod texto;
mod view;
mod viewport;

use std::path::PathBuf;

use std::sync::Arc;
use std::time::{Duration, Instant};

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
    context_menu_view, context_menu_view_ex, ContextMenuExtras, ContextMenuItem,
    ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_edit_menu::{self as editmenu, EditFlags};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_toast::{toast_stack_view, Toast};

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

/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);

/// Empuja un toast al stack y programa su expiración (worker que duerme
/// `TOAST_TTL` y reentra con `Msg::ToastExpire`). El propio widget anima su
/// entrada/salida — no hace falta un tick de repaint.
fn push_toast(model: &mut Model, handle: &Handle<Msg>, toast: Toast) {
    let id = toast.id;
    model.toasts.push(toast);
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
}

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
            Msg::AgregarAjuste(op) => {
                // La capa de ajuste no necesita madre: opera sobre el
                // compuesto inferior. Se apila encima de la seleccionada.
                let nombre = format!("ajuste · {}", op_etiqueta(&op));
                let mut nueva = Capa::ajuste(nombre, op);
                let nuevo_id = nueva.id;
                // Hereda el grupo de la capa seleccionada para caer en el mismo
                // scope (si la selección está dentro de una carpeta).
                nueva.grupo = model
                    .seleccionada
                    .and_then(|id| model.lienzo.capa(id))
                    .and_then(|c| c.grupo);
                model.lienzo.apilar(nueva);
                model.seleccionada = Some(nuevo_id);
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::VoltearCapa { horizontal } => {
                if voltear_capa_activa(&mut model, horizontal) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Agrupar(id) => {
                if let Some(gid) = model.lienzo.agrupar(&[id], "grupo") {
                    model.seleccionada = Some(gid);
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                    model.estado = "capa agrupada".into();
                }
            }
            Msg::ToggleClipping(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.clipping = !c.clipping;
                    let on = c.clipping;
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                    model.estado =
                        if on { "máscara de recorte ON" } else { "máscara de recorte OFF" }.into();
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
                let id = model.next_toast;
                model.next_toast += 1;
                if agregar_capa_desde_archivo(&mut model, &path) {
                    pushear_snapshot(&mut model, None);
                    let nombre = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("imagen")
                        .to_string();
                    push_toast(
                        &mut model,
                        handle,
                        Toast::success(id, format!("📥 importada · {nombre}"), TOAST_TTL),
                    );
                } else {
                    let motivo = model.estado.clone();
                    push_toast(&mut model, handle, Toast::error(id, motivo, TOAST_TTL));
                }
            }
            Msg::Undo => {
                if aplicar_undo(&mut model) {
                    ajustar_seleccion_tras_restaurar(&mut model);
                    aplicar_y_recomponer(&mut model);
                    model.estado = format!(
                        "↶ undo · {}/{}",
                        model.hist.cursor() + 1,
                        model.hist.len()
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
                        model.hist.cursor() + 1,
                        model.hist.len()
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
                        img.image.width,
                        img.image.height,
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
                model.seleccion_mascara = None;
                model.seleccion_overlay = None;
                    }
                }
            }
            Msg::AjustarSeleccion { dx, dy } => {
                if let Some(drag) = model.seleccion_drag.as_mut() {
                    drag.cur_lx += dx;
                    drag.cur_ly += dy;
                    let drag = *drag;
                    model.seleccion_mascara = None;
                    model.seleccion_overlay = None;
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
                model.seleccion_mascara = None;
                model.seleccion_overlay = None;
                model.seleccion_drag = None;
                model.estado = "selección limpia".into();
            }
            Msg::InvertirSeleccion => {
                invertir_seleccion(&mut model);
            }
            Msg::SeleccionarTodo => {
                let w = model.lienzo.width;
                let h = model.lienzo.height;
                if w > 0 && h > 0 {
                    model.seleccion =
                        Some(RectImagen { x0: 0, y0: 0, x1: w, y1: h });
                    model.seleccion_mascara = None;
                    model.seleccion_overlay = None;
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
                    model.seleccion_mascara = None;
                    model.seleccion_overlay = None;
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
                    let bytes = img.image.data.data();
                    match recoger_color_en(
                        bytes,
                        img.image.width,
                        img.image.height,
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
            Msg::SeleccionarVarita { lx, ly, rw, rh } => {
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
                    if seleccionar_por_color(&mut model, sx, sy) {
                        // La selección no es parte del DAG de imagen — sin
                        // snapshot, como el resto de las ops de marquee.
                    }
                } else {
                    model.estado = "varita · fuera de la imagen".into();
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
                model.hist.invalidar_etiqueta();
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
            Msg::AgregarTexto { lx, ly, rw, rh } => {
                if let Some((ix, iy)) = local_a_imagen(
                    lx, ly, rw, rh,
                    model.lienzo.width, model.lienzo.height,
                    model.factor_zoom, model.pan_x, model.pan_y,
                ) {
                    let id = agregar_capa_texto(&mut model, ix.floor() as u32, iy.floor() as u32);
                    let mut input = TextInputState::new();
                    input.set_text("Texto".to_string());
                    model.editando_texto = Some((id, input));
                    pushear_snapshot(&mut model, None);
                    model.estado = "texto · escribí · Enter/Esc cierra".into();
                } else {
                    model.estado = "texto · fuera de la imagen".into();
                }
            }
            Msg::AgregarRectangulo => {
                let (w, h) = (model.lienzo.width as f32, model.lienzo.height as f32);
                let color = model.color_picked.unwrap_or([60, 120, 220, 255]);
                let params = tullpu_core::ParamsVector::rectangulo(
                    w * 0.25, h * 0.25, w * 0.5, h * 0.5, color,
                );
                agregar_capa_vector(&mut model, params, "rectángulo");
                pushear_snapshot(&mut model, None);
                model.estado = "vector · rectángulo agregado".into();
            }
            Msg::AgregarElipse => {
                let (w, h) = (model.lienzo.width as f32, model.lienzo.height as f32);
                let color = model.color_picked.unwrap_or([220, 90, 60, 255]);
                let params = tullpu_core::ParamsVector::elipse(
                    w * 0.5, h * 0.5, w * 0.3, h * 0.3, color,
                );
                agregar_capa_vector(&mut model, params, "elipse");
                pushear_snapshot(&mut model, None);
                model.estado = "vector · elipse agregada".into();
            }
            Msg::AgregarRectRedondeado => {
                let (w, h) = (model.lienzo.width as f32, model.lienzo.height as f32);
                let color = model.color_picked.unwrap_or([90, 180, 130, 255]);
                let params = tullpu_core::ParamsVector::rect_redondeado(
                    w * 0.25, h * 0.25, w * 0.5, h * 0.5, (w.min(h)) * 0.08, color,
                );
                agregar_capa_vector(&mut model, params, "rect redondeado");
                pushear_snapshot(&mut model, None);
                model.estado = "vector · rect redondeado".into();
            }
            Msg::AgregarEstrella => {
                let (w, h) = (model.lienzo.width as f32, model.lienzo.height as f32);
                let color = model.color_picked.unwrap_or([230, 200, 60, 255]);
                let r = w.min(h) * 0.35;
                let params = tullpu_core::ParamsVector::estrella(w * 0.5, h * 0.5, r, r * 0.42, 5, color);
                agregar_capa_vector(&mut model, params, "estrella");
                pushear_snapshot(&mut model, None);
                model.estado = "vector · estrella".into();
            }
            Msg::AgregarPoligono => {
                let (w, h) = (model.lienzo.width as f32, model.lienzo.height as f32);
                let color = model.color_picked.unwrap_or([120, 150, 230, 255]);
                let params = tullpu_core::ParamsVector::poligono_regular(
                    w * 0.5, h * 0.5, w.min(h) * 0.35, 6, color,
                );
                agregar_capa_vector(&mut model, params, "hexágono");
                pushear_snapshot(&mut model, None);
                model.estado = "vector · hexágono".into();
            }
            Msg::AgregarLinea => {
                let (w, h) = (model.lienzo.width as f32, model.lienzo.height as f32);
                let color = model.color_picked.unwrap_or([20, 20, 20, 255]);
                let params = tullpu_core::ParamsVector::linea(
                    w * 0.2, h * 0.2, w * 0.8, h * 0.8, color, (w.min(h) * 0.01).max(2.0),
                );
                agregar_capa_vector(&mut model, params, "línea");
                pushear_snapshot(&mut model, None);
                model.estado = "vector · línea".into();
            }
            Msg::PlumaPress { lx, ly, rw, rh } => {
                if pluma_press(&mut model, lx, ly, rw, rh) {
                    pushear_snapshot(&mut model, None);
                    model.estado = "pluma · vértice · Enter cierra".into();
                }
            }
            Msg::PlumaArrastrar { dx, dy } => {
                pluma_arrastrar(&mut model, dx, dy);
            }
            Msg::PlumaSoltar => {
                let movido = model.pluma_ancla.take().is_some() || model.pluma_control.take().is_some();
                if movido {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::PlumaCerrar => {
                pluma_cerrar(&mut model);
                pushear_snapshot(&mut model, None);
                model.estado = "pluma · path cerrado".into();
            }
            Msg::VectorRelleno => {
                let color = model.color_picked.unwrap_or([60, 120, 220, 255]);
                editar_vector_seleccionado(&mut model, |p| p.relleno = Some(color));
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorRellenoQuitar => {
                editar_vector_seleccionado(&mut model, |p| p.relleno = None);
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorTrazo => {
                let color = model.color_picked.unwrap_or([20, 20, 20, 255]);
                editar_vector_seleccionado(&mut model, |p| {
                    p.trazo = Some(color);
                    if p.ancho_trazo < 1.0 {
                        p.ancho_trazo = 2.0;
                    }
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorTrazoQuitar => {
                editar_vector_seleccionado(&mut model, |p| p.trazo = None);
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorAnchoTrazo(delta) => {
                editar_vector_seleccionado(&mut model, |p| {
                    p.ancho_trazo = (p.ancho_trazo + delta).clamp(0.0, 200.0);
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorGradienteLineal => {
                let color = model.color_picked.unwrap_or([60, 120, 220, 255]);
                let transp = [color[0], color[1], color[2], 0];
                editar_vector_seleccionado(&mut model, |p| {
                    let (x0, y0, x1, y1) = bbox_path(p);
                    p.gradiente = Some(tullpu_core::Gradiente::lineal(x0, (y0 + y1) * 0.5, x1, (y0 + y1) * 0.5, color, transp));
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorGradienteRadial => {
                let color = model.color_picked.unwrap_or([60, 120, 220, 255]);
                let transp = [color[0], color[1], color[2], 0];
                editar_vector_seleccionado(&mut model, |p| {
                    let (x0, y0, x1, y1) = bbox_path(p);
                    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
                    let r = (((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt() * 0.5).max(1.0);
                    p.gradiente = Some(tullpu_core::Gradiente::radial(cx, cy, r, color, transp));
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorGradienteQuitar => {
                editar_vector_seleccionado(&mut model, |p| p.gradiente = None);
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorTrazoCap => {
                use tullpu_core::CapTrazo::*;
                editar_estilo_trazo(&mut model, |e| {
                    e.cap = match e.cap {
                        Plano => Redondo,
                        Redondo => Cuadrado,
                        Cuadrado => Plano,
                    };
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorTrazoJoin => {
                use tullpu_core::JoinTrazo::*;
                editar_estilo_trazo(&mut model, |e| {
                    e.join = match e.join {
                        Punta => Redondo,
                        Redondo => Bisel,
                        Bisel => Punta,
                    };
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::VectorTrazoDash => {
                editar_estilo_trazo(&mut model, |e| {
                    e.dash = if e.dash.is_empty() { vec![12.0, 7.0] } else { Vec::new() };
                });
                pushear_snapshot(&mut model, None);
            }
            Msg::ToggleSnap => {
                model.snap_grid = if model.snap_grid.is_some() { None } else { Some(16.0) };
                model.estado = match model.snap_grid {
                    Some(g) => format!("snap a grilla {g:.0} px"),
                    None => "snap libre".into(),
                };
            }
            Msg::AlinearCentro { h, v } => {
                alinear_vector_centro(&mut model, h, v);
                pushear_snapshot(&mut model, None);
            }
            Msg::BooleanoUnion => {
                if let Some(id) = model.seleccionada {
                    if combinar_booleano(&mut model, id, tullpu_ops::OpBooleano::Union) {
                        pushear_snapshot(&mut model, None);
                    }
                }
            }
            Msg::BooleanoInter => {
                if let Some(id) = model.seleccionada {
                    if combinar_booleano(&mut model, id, tullpu_ops::OpBooleano::Interseccion) {
                        pushear_snapshot(&mut model, None);
                    }
                }
            }
            Msg::BooleanoResta => {
                if let Some(id) = model.seleccionada {
                    if combinar_booleano(&mut model, id, tullpu_ops::OpBooleano::Resta) {
                        pushear_snapshot(&mut model, None);
                    }
                }
            }
            Msg::TextoTecla(ev) => {
                let actualizar = if let Some((id, input)) = model.editando_texto.as_mut() {
                    input.apply_key(&ev);
                    Some((*id, input.text()))
                } else {
                    None
                };
                if let Some((id, txt)) = actualizar {
                    editar_params_texto(&mut model, id, |p| p.texto = txt);
                }
            }
            Msg::TextoTamano(delta) => {
                // Aplica al texto en edición, o al texto seleccionado.
                let target = model
                    .editando_texto
                    .as_ref()
                    .map(|(id, _)| *id)
                    .or_else(|| {
                        model.seleccionada.filter(|id| {
                            model.lienzo.capa(*id).map_or(false, |c| c.params_texto().is_some())
                        })
                    });
                if let Some(id) = target {
                    editar_params_texto(&mut model, id, |p| {
                        p.tamano = (p.tamano + delta).clamp(4.0, 512.0)
                    });
                }
            }
            Msg::TerminarTexto => {
                if model.editando_texto.take().is_some() {
                    pushear_snapshot(&mut model, None);
                    model.estado = "texto listo".into();
                }
            }
            Msg::SetAlt(v) => {
                model.alt_held = v;
            }
            Msg::IniciarClon { lx, ly, rw, rh } => {
                let punto = local_a_imagen(
                    lx, ly, rw, rh,
                    model.lienzo.width, model.lienzo.height,
                    model.factor_zoom, model.pan_x, model.pan_y,
                );
                if let Some((ix, iy)) = punto {
                    let (ix, iy) = (ix.floor() as i32, iy.floor() as i32);
                    if model.alt_held {
                        // Alt+click fija el origen del clon.
                        model.clon_ancla = Some((ix, iy));
                        model.clon_offset = None;
                        model.estado = "clon · origen fijado".into();
                    } else if let Some((ax, ay)) = model.clon_ancla {
                        // Arranca un trazo: offset = origen − inicio.
                        let off = (ax - ix, ay - iy);
                        model.clon_offset = Some(off);
                        model.pincel_drag = Some(PincelDrag {
                            cur_lx: lx, cur_ly: ly, rw, rh, last_ix: ix, last_iy: iy,
                        });
                        let radio = model.radio_pincel;
                        let dureza = model.dureza_pincel;
                        let sanar = model.herramienta == Herramienta::Sanar;
                        let etiqueta = model.seleccionada.map(|i| (i, if sanar { "sanar" } else { "clon" }));
                        let cambio = if sanar {
                            sanar_punto_en_capa(&mut model, ix, iy, off.0, off.1, radio, dureza)
                        } else {
                            clonar_punto_en_capa(&mut model, ix, iy, off.0, off.1, radio, dureza)
                        };
                        if cambio {
                            pushear_snapshot(&mut model, etiqueta);
                        }
                    } else {
                        model.estado = "clon · Alt+click para fijar el origen".into();
                    }
                }
            }
            Msg::ContinuarClon { dx, dy } => {
                if let (Some(pd), Some(off)) = (model.pincel_drag, model.clon_offset) {
                    let mut pd = pd;
                    pd.cur_lx += dx;
                    pd.cur_ly += dy;
                    if let Some((ix, iy)) = local_a_imagen(
                        pd.cur_lx, pd.cur_ly, pd.rw, pd.rh,
                        model.lienzo.width, model.lienzo.height,
                        model.factor_zoom, model.pan_x, model.pan_y,
                    ) {
                        let (nx, ny) = (ix.floor() as i32, iy.floor() as i32);
                        let radio = model.radio_pincel;
                        let dureza = model.dureza_pincel;
                        let sanar = model.herramienta == Herramienta::Sanar;
                        let etiqueta = model.seleccionada.map(|i| (i, if sanar { "sanar" } else { "clon" }));
                        let cambio = if sanar {
                            sanar_segmento_en_capa(
                                &mut model, pd.last_ix, pd.last_iy, nx, ny, off.0, off.1, radio, dureza,
                            )
                        } else {
                            clonar_segmento_en_capa(
                                &mut model, pd.last_ix, pd.last_iy, nx, ny, off.0, off.1, radio, dureza,
                            )
                        };
                        if cambio {
                            pushear_snapshot(&mut model, etiqueta);
                        }
                        pd.last_ix = nx;
                        pd.last_iy = ny;
                    }
                    model.pincel_drag = Some(pd);
                }
            }
            Msg::FinalizarClon => {
                model.pincel_drag = None;
                model.clon_offset = None;
            }
            Msg::IniciarLazo { lx, ly, rw, rh } => {
                let mut puntos = Vec::new();
                if let Some((ix, iy)) = local_a_imagen(
                    lx, ly, rw, rh,
                    model.lienzo.width, model.lienzo.height,
                    model.factor_zoom, model.pan_x, model.pan_y,
                ) {
                    puntos.push((ix.floor() as i32, iy.floor() as i32));
                }
                model.lazo_drag = Some(LazoDrag { cur_lx: lx, cur_ly: ly, rw, rh, puntos });
            }
            Msg::ContinuarLazo { dx, dy } => {
                // Acumula la posición y agrega un vértice nuevo si cambió de
                // píxel-imagen respecto al último (evita polilíneas densísimas).
                let punto = model.lazo_drag.as_ref().and_then(|l| {
                    let (clx, cly) = (l.cur_lx + dx, l.cur_ly + dy);
                    local_a_imagen(
                        clx, cly, l.rw, l.rh,
                        model.lienzo.width, model.lienzo.height,
                        model.factor_zoom, model.pan_x, model.pan_y,
                    )
                    .map(|(ix, iy)| (clx, cly, ix.floor() as i32, iy.floor() as i32))
                });
                if let Some(l) = model.lazo_drag.as_mut() {
                    l.cur_lx += dx;
                    l.cur_ly += dy;
                    if let Some((_, _, ix, iy)) = punto {
                        if l.puntos.last() != Some(&(ix, iy)) {
                            l.puntos.push((ix, iy));
                        }
                    }
                }
            }
            Msg::FinalizarLazo => {
                if let Some(l) = model.lazo_drag.take() {
                    seleccionar_lazo(&mut model, &l.puntos);
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
                let id = model.next_toast;
                model.next_toast += 1;
                match tullpu_render::exportar(
                    &model.lienzo,
                    &model.almacen,
                    &ruta,
                    formato,
                ) {
                    Ok(_) => {
                        model.estado = format!("exportado → {}", ruta.display());
                        push_toast(
                            &mut model,
                            handle,
                            Toast::success(id, format!("💾 exportado → {}", ruta.display()), TOAST_TTL),
                        );
                    }
                    Err(e) => {
                        model.estado = format!("export falló: {e}");
                        push_toast(
                            &mut model,
                            handle,
                            Toast::error(id, format!("export falló: {e}"), TOAST_TTL),
                        );
                    }
                }
            }
            Msg::ExportarPsd => {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let ruta = std::path::PathBuf::from(format!("tullpu-export-{ts}.psd"));
                let id = model.next_toast;
                model.next_toast += 1;
                let resultado = foreign_psd::exportar_psd(&model.lienzo, &model.almacen)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| std::fs::write(&ruta, bytes).map_err(|e| e.to_string()));
                match resultado {
                    Ok(_) => {
                        let n = model.lienzo.capas.len();
                        model.estado = format!("exportado → {} ({n} capas)", ruta.display());
                        push_toast(
                            &mut model,
                            handle,
                            Toast::success(id, format!("💾 PSD → {} ({n} capas)", ruta.display()), TOAST_TTL),
                        );
                    }
                    Err(e) => {
                        model.estado = format!("export PSD falló: {e}");
                        push_toast(&mut model, handle, Toast::error(id, format!("export PSD falló: {e}"), TOAST_TTL));
                    }
                }
            }
            Msg::ExportarSvg => {
                // Junta las capas vectoriales en orden visual (fondo→frente).
                let vectores: Vec<tullpu_core::ParamsVector> = model
                    .lienzo
                    .capas
                    .iter()
                    .filter_map(|c| c.params_vector().cloned())
                    .collect();
                let id = model.next_toast;
                model.next_toast += 1;
                if vectores.is_empty() {
                    model.estado = "export SVG: no hay capas vectoriales".into();
                    push_toast(&mut model, handle, Toast::error(id, "SVG: no hay capas vectoriales", TOAST_TTL));
                } else {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let ruta = std::path::PathBuf::from(format!("tullpu-export-{ts}.svg"));
                    let svg = foreign_svg::exportar_svg(&vectores, model.lienzo.width, model.lienzo.height);
                    match std::fs::write(&ruta, svg) {
                        Ok(_) => {
                            let n = vectores.len();
                            model.estado = format!("exportado → {} ({n} vectores)", ruta.display());
                            push_toast(&mut model, handle, Toast::success(id, format!("💾 SVG → {} ({n} vectores)", ruta.display()), TOAST_TTL));
                        }
                        Err(e) => {
                            model.estado = format!("export SVG falló: {e}");
                            push_toast(&mut model, handle, Toast::error(id, format!("export SVG falló: {e}"), TOAST_TTL));
                        }
                    }
                }
            }
            Msg::MenuOpen(idx) => {
                model.menu_open = idx;
                model.menu_active = usize::MAX;
                if idx.is_some() {
                    model.menu_anim =
                        Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        model.menu_open = None;
                        model.menu_active = usize::MAX;
                        model.context_menu = None;
                        model = handle_menu_command(model, &cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.context_menu = None;
                model.edit_menu = None;
                model.edit_active = usize::MAX;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
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
                    model.edit_active = usize::MAX;
                    model.edit_anim =
                        Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                } else {
                    model.edit_menu = None;
                    model.context_menu = Some((x, y));
                }
            }
            Msg::EditNav(dir) => {
                if let Some((_, input)) = model.renombrando.as_ref() {
                    let flags = EditFlags::from_editor(input.editor(), input.is_masked());
                    model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
                }
            }
            Msg::EditActivate => {
                let action = model.renombrando.as_ref().and_then(|(_, input)| {
                    let flags = EditFlags::from_editor(input.editor(), input.is_masked());
                    editmenu::edit_menu_action_at(flags, model.edit_active)
                });
                if let Some(action) = action {
                    return Tullpu::update(model, Msg::EditMenuAction(action), handle);
                }
            }
            Msg::EditMenuAction(action) => {
                model.edit_menu = None;
                model.edit_active = usize::MAX;
                if let Some((_, input)) = model.renombrando.as_mut() {
                    let _ = editmenu::apply(
                        input.editor_mut(),
                        action,
                        &mut model.clipboard,
                    );
                }
            }
            Msg::ToastExpire(id) => {
                model.toasts.retain(|t| t.id != id);
            }
            Msg::IniciarTransform => {
                iniciar_transform(&mut model);
            }
            Msg::TransformPress { lx, ly, rw, rh } => {
                transform_press(&mut model, lx, ly, rw, rh);
            }
            Msg::TransformArrastrar { dx, dy } => {
                transform_arrastrar(&mut model, dx, dy);
            }
            Msg::TransformSoltar => {
                if let Some(t) = model.transform.as_mut() {
                    t.agarre = None;
                }
            }
            Msg::ConfirmarTransform => {
                confirmar_transform(&mut model);
            }
            Msg::CancelarTransform => {
                cancelar_transform(&mut model);
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
        let root = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menubar, cabecera, centro]);

        // Overlay de toasts (bottom-right): confirmaciones/errores de
        // export e import. La app filtra los vivos; el widget anima su
        // entrada/salida por su cuenta.
        let now = Instant::now();
        let alive: Vec<Toast> =
            model.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
        if alive.is_empty() {
            root
        } else {
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .children(vec![
                root,
                toast_stack_view(&alive, viewport_of(), Msg::ToastExpire),
            ])
        }
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
        // Menú principal abierto: ←/→ cambian de menú raíz (con wrap),
        // ↑/↓ navegan la fila, Enter ejecuta, Esc cierra. Consume la tecla.
        if let Some(mi) = model.menu_open {
            if event.state == KeyState::Pressed {
                let n = app_menu(model).menus.len().max(1);
                return Some(match &event.key {
                    Key::Named(NamedKey::Escape) => Msg::CloseMenus,
                    Key::Named(NamedKey::ArrowLeft) => Msg::MenuOpen(Some((mi + n - 1) % n)),
                    Key::Named(NamedKey::ArrowRight) => Msg::MenuOpen(Some((mi + 1) % n)),
                    Key::Named(NamedKey::ArrowDown) => Msg::MenuNav(1),
                    Key::Named(NamedKey::ArrowUp) => Msg::MenuNav(-1),
                    Key::Named(NamedKey::Enter) => Msg::MenuActivate,
                    _ => return None,
                });
            }
            return None;
        }
        // Menú de edición de texto abierto (sólo durante renombrado):
        // ↑/↓ navegan, Enter ejecuta, Esc cierra. Tiene prioridad sobre el
        // ruteo de teclas al text-input del renombrado.
        if model.edit_menu.is_some() {
            if event.state == KeyState::Pressed {
                return Some(match &event.key {
                    Key::Named(NamedKey::Escape) => Msg::CloseMenus,
                    Key::Named(NamedKey::ArrowDown) => Msg::EditNav(1),
                    Key::Named(NamedKey::ArrowUp) => Msg::EditNav(-1),
                    Key::Named(NamedKey::Enter) => Msg::EditActivate,
                    _ => return None,
                });
            }
            return None;
        }
        // Editando una capa de texto: las teclas escriben el contenido (se
        // re-rasteriza en vivo), salvo Enter/Escape que cierran la edición.
        if model.editando_texto.is_some() {
            if event.state == KeyState::Pressed {
                match &event.key {
                    Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => {
                        return Some(Msg::TerminarTexto)
                    }
                    _ => {}
                }
            }
            return Some(Msg::TextoTecla(event.clone()));
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
        // Modo transformación libre activo: Enter aplica, Escape cancela.
        // Absorbe estas dos teclas; el resto cae al ruteo normal de abajo
        // (para que `s`/`r`/etc. no rompan, simplemente no hacen nada útil).
        if model.transform.is_some() && event.state == KeyState::Pressed {
            match &event.key {
                Key::Named(NamedKey::Enter) => return Some(Msg::ConfirmarTransform),
                Key::Named(NamedKey::Escape) => return Some(Msg::CancelarTransform),
                _ => {}
            }
        }
        // Sincronizar el estado vivo de Shift: el handler de click no
        // recibe modifiers, así que lo trackeamos desde la tecla para
        // habilitar el trazo recto (Shift+click). Las dos Shift (izq/der)
        // llegan como NamedKey::Shift.
        if matches!(event.key, Key::Named(NamedKey::Shift)) {
            return Some(Msg::SetShift(event.state == KeyState::Pressed));
        }
        // Ídem Alt: el tampón de clonado lo usa para fijar el origen.
        if matches!(event.key, Key::Named(NamedKey::Alt)) {
            return Some(Msg::SetAlt(event.state == KeyState::Pressed));
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
                let mut spec = editmenu::edit_context_menu(
                    (x, y),
                    viewport_of(),
                    &theme,
                    flags,
                    Msg::EditMenuAction,
                    Msg::CloseMenus,
                );
                spec.active = model.edit_active;
                return Some(context_menu_view_ex(
                    spec,
                    ContextMenuExtras {
                        appear: model.edit_anim.value(),
                        ..Default::default()
                    },
                ));
            }
        }
        // Prioridad 2: menú contextual de capa/selección.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_canvas(model, &theme, x, y));
        }
        // Prioridad 3: dropdown del menú principal.
        if model.menu_open.is_some() {
            let menu = app_menu(model);
            if let Some(v) = menubar_overlay_animated(
                &menubar_spec(&menu, model, &theme),
                model.menu_active,
                model.menu_anim.value(),
            ) {
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
    let can_undo = model.hist.puede_deshacer();
    let can_redo = model.hist.puede_rehacer();

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
                .item(MenuItem::new("Exportar WebP", "file.webp").shortcut("Ctrl+Shift+S"))
                .item(MenuItem::new("Exportar PSD (capas)", "file.psd"))
                .item(MenuItem::new("Exportar SVG (vectores)", "file.svg")),
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
                .item(MenuItem::new("Invertir selección", "sel.invertir"))
                .item(limpiar_sel)
                .item(recortar),
        )
        .menu(
            Menu::new("Capa")
                .item(MenuItem::new("Agrupar", "capa.agrupar"))
                .item(MenuItem::new("Máscara de recorte", "capa.clipping"))
                .item(MenuItem::new("Voltear capa ↔", "capa.voltear_h"))
                .item(MenuItem::new("Voltear capa ↕", "capa.voltear_v"))
                .item(MenuItem::new("Transformar libre (Ctrl+T)", "capa.transformar").separated())
                .item(MenuItem::new("Ajuste: Brillo", "capa.ajuste.brillo"))
                .item(MenuItem::new("Ajuste: Contraste", "capa.ajuste.contraste"))
                .item(MenuItem::new("Ajuste: Niveles", "capa.ajuste.niveles"))
                .item(MenuItem::new("Ajuste: Curvas", "capa.ajuste.curvas"))
                .item(MenuItem::new("Ajuste: Saturación", "capa.ajuste.saturacion"))
                .item(MenuItem::new("Ajuste: Tonalidad", "capa.ajuste.tonalidad"))
                .item(MenuItem::new("Ajuste: Invertir", "capa.ajuste.invertir")),
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
        "file.psd" => Some(Msg::ExportarPsd),
        "file.svg" => Some(Msg::ExportarSvg),
        "edit.undo" => Some(Msg::Undo),
        "edit.redo" => Some(Msg::Redo),
        "edit.duplicar" => sel.map(Msg::Duplicar),
        "edit.eliminar" => sel.map(Msg::Eliminar),
        "edit.combinar" => sel.map(Msg::Combinar),
        "edit.aplanar" => Some(Msg::AplanarVisibles),
        "capa.agrupar" => sel.map(Msg::Agrupar),
        "capa.clipping" => sel.map(Msg::ToggleClipping),
        "capa.voltear_h" => Some(Msg::VoltearCapa { horizontal: true }),
        "capa.voltear_v" => Some(Msg::VoltearCapa { horizontal: false }),
        "capa.transformar" => Some(Msg::IniciarTransform),
        "capa.ajuste.invertir" => Some(Msg::AgregarAjuste(tullpu_core::OpLocal::Invertir)),
        "capa.ajuste.curvas" => {
            Some(Msg::AgregarAjuste(tullpu_core::OpLocal::curvas_identidad()))
        }
        "capa.ajuste.brillo" => {
            Some(Msg::AgregarAjuste(tullpu_core::OpLocal::Brillo { delta: 0.0 }))
        }
        "capa.ajuste.contraste" => {
            Some(Msg::AgregarAjuste(tullpu_core::OpLocal::Contraste { factor: 1.0 }))
        }
        "capa.ajuste.niveles" => Some(Msg::AgregarAjuste(tullpu_core::OpLocal::Niveles {
            entrada_min: 0.0,
            entrada_max: 1.0,
            gamma: 1.0,
        })),
        "capa.ajuste.saturacion" => {
            Some(Msg::AgregarAjuste(tullpu_core::OpLocal::Saturacion { factor: 1.0 }))
        }
        "capa.ajuste.tonalidad" => {
            Some(Msg::AgregarAjuste(tullpu_core::OpLocal::Tonalidad { grados: 0.0 }))
        }
        "edit.copiar" => Some(Msg::CopiarSeleccion),
        "edit.cortar" => Some(Msg::CortarSeleccion),
        "edit.pegar" => Some(Msg::PegarPortapapeles),
        "edit.dup_sel" => Some(Msg::DuplicarSeleccionACapa),
        "edit.recortar_sel" => Some(Msg::RecortarASeleccion),
        "sel.todo" => Some(Msg::SeleccionarTodo),
        "sel.invertir" => Some(Msg::InvertirSeleccion),
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
