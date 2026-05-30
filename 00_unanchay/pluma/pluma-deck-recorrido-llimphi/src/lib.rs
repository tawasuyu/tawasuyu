//! Frontend Llimphi del modo `Recorrido` de `pluma-deck-core` — presentación
//! espacial tipo Prezi: un lienzo 2D infinito con marcos colocados en
//! coordenadas de mundo y una cámara que vuela entre ellos.
//!
//! La lógica vive entera en `pluma-deck-core` (cámara, ruta, máquina de
//! interacción); aquí sólo hay **pintura** (`View::paint_with` aplicando el
//! transform de la cámara) y el cableado de eventos. Sigue la regla #2 del
//! repo: la UI es un frontend intercambiable sobre un `*-core` agnóstico.
//!
//! El host arma su `App` así:
//! - `view`: nodo a pantalla completa con [`recorrido_view`] (registra el rect
//!   del panel en un side-channel para que `on_wheel` sepa el tamaño).
//! - `on_wheel`: lee [`panel_actual`] y despacha un zoom-a-cursor.
//! - drag sobre el nodo: `RecorridoState::arrastrar_delta` (pan libre).
//! - flechas: `siguiente`/`anterior` + un tick periódico que llama
//!   `RecorridoState::avanzar(dt)` para animar el vuelo.

use std::sync::{Mutex, OnceLock};

use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Mix};
use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment, TextBlock};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::{PaintRect, View};

use pluma_deck_core::{Camara, ContenidoMarco, Recorrido, RecorridoState, Rect};

/// Base del zoom por "clic" de rueda (igual criterio que tullpu: `1.1`).
pub const ZOOM_BASE: f64 = 1.1;

// ---- Side-channel del rect del panel -------------------------------------
//
// `App::on_wheel` recibe el cursor absoluto pero no el tamaño del viewport.
// El `paint_with` de [`recorrido_view`] escribe el rect del panel cada frame
// y `on_wheel`/handlers lo leen. Mismo patrón que `tullpu` (`LIENZO_RECT`).

static PANEL_RECT: OnceLock<Mutex<Option<Rect>>> = OnceLock::new();

fn panel_set(r: Rect) {
    let cell = PANEL_RECT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(r);
    }
}

/// Último rect del panel (px de pantalla) registrado por [`recorrido_view`].
/// `None` hasta el primer frame pintado. Lo usan `on_wheel` (zoom-a-cursor),
/// `siguiente`/`anterior` (encuadre) en el `update` del host.
pub fn panel_actual() -> Option<Rect> {
    PANEL_RECT.get()?.lock().ok().and_then(|g| *g)
}

/// `true` si `(cx, cy)` (px de pantalla) cae dentro de `panel`.
pub fn dentro(panel: Rect, cx: f32, cy: f32) -> bool {
    let (cx, cy) = (cx as f64, cy as f64);
    cx >= panel.x && cx <= panel.x + panel.w && cy >= panel.y && cy <= panel.y + panel.h
}

// ---- Colores del lienzo (no temáticos todavía; placeholder sobrio) -------

const FONDO: Color = Color::from_rgba8(18, 20, 28, 255);
const MARCO_FONDO: Color = Color::from_rgba8(38, 42, 56, 255);
const MARCO_BORDE: Color = Color::from_rgba8(80, 86, 104, 255);
const MARCO_ACENTO: Color = Color::from_rgba8(120, 180, 255, 255);
const TEXTO: Color = Color::from_rgba8(225, 230, 240, 235);

/// Nodo a pantalla completa que pinta el recorrido y registra el rect del
/// panel. `Msg` es libre: el caller suele colgarle un `.draggable(...)` para
/// el pan — esta función no lo impone para no fijar el tipo de mensaje.
pub fn recorrido_view<Msg: 'static>(rec: &Recorrido, state: &RecorridoState) -> View<Msg> {
    // Clonamos lo mínimo para que el closure `Send + Sync` sobreviva al frame.
    let marcos = rec.marcos.clone();
    let paso_id = rec.pasos.get(state.paso).copied();
    let camara = state.camara;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(FONDO)
    .paint_with(move |scene, ts, rect: PaintRect| {
        panel_set(to_rect(rect));
        pintar(scene, ts, rect, &marcos, paso_id, &camara);
    })
}

fn to_rect(r: PaintRect) -> Rect {
    Rect::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64)
}

/// Affine mundo→pantalla de una cámara, dado el rect del panel.
/// `pantalla = centro_panel + escala(zoom) · rot(-rot) · (mundo - centro)`.
fn world_to_screen_affine(cam: &Camara, panel: Rect) -> Affine {
    let pcx = panel.x + panel.w * 0.5;
    let pcy = panel.y + panel.h * 0.5;
    Affine::translate((pcx, pcy))
        * Affine::scale(cam.zoom)
        * Affine::rotate(-cam.rot_rad)
        * Affine::translate((-cam.centro.0, -cam.centro.1))
}

fn pintar(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: PaintRect,
    marcos: &[pluma_deck_core::Marco],
    paso_id: Option<pluma_deck_core::MarcoId>,
    cam: &Camara,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let panel = to_rect(rect);
    let w2s = world_to_screen_affine(cam, panel);

    // Clip al panel: un marco con zoom-in no debe derramar fuera del nodo.
    let node = KurboRect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &node);

    for m in marcos {
        let (mcx, mcy) = m.rect.centro();
        // Giro propio del marco alrededor de su centro, encadenado al mundo→pantalla.
        let xf = w2s
            * Affine::translate((mcx, mcy))
            * Affine::rotate(m.rot_rad)
            * Affine::translate((-mcx, -mcy));
        let kr = KurboRect::new(
            m.rect.x,
            m.rect.y,
            m.rect.x + m.rect.w,
            m.rect.y + m.rect.h,
        );
        scene.fill(Fill::NonZero, xf, MARCO_FONDO, None, &kr);
        let actual = paso_id == Some(m.id);
        let (grosor, color) = if actual { (3.0, MARCO_ACENTO) } else { (1.0, MARCO_BORDE) };
        scene.stroke(&Stroke::new(grosor), xf, color, None, &kr);

        // Etiqueta: centrada en el centro de pantalla del marco. Tamaño escala
        // con el zoom (clamp para que siga legible muy lejos / muy cerca).
        if let ContenidoMarco::Etiqueta(t) = &m.contenido {
            if t.is_empty() {
                continue;
            }
            let (sx, sy) = cam.world_to_screen((mcx, mcy), panel);
            let ancho_px = (m.rect.w * cam.zoom) as f32;
            if ancho_px < 12.0 {
                continue; // demasiado chico para texto
            }
            let size_px = ((16.0 * cam.zoom) as f32).clamp(9.0, 40.0);
            let block = TextBlock {
                text: t,
                size_px,
                color: TEXTO,
                origin: (sx - ancho_px as f64 * 0.5, sy - size_px as f64 * 0.6),
                max_width: Some(ancho_px),
                alignment: Alignment::Center,
                line_height: 1.2,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, block.color, block.origin);
        }
    }

    scene.pop_layer();
}
