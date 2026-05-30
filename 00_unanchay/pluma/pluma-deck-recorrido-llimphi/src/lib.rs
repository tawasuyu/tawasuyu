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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Blob, Color, Fill, Image as PenikoImage, ImageFormat, Mix};
use llimphi_ui::llimphi_text::{draw_layout_xf, layout_block, measurement, Alignment, TextBlock};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::{PaintRect, View};

use pluma_deck_core::{Camara, ContenidoMarco, MarcoId, Recorrido, RecorridoState, Rect};

/// Base del zoom por "clic" de rueda (igual criterio que tullpu: `1.1`).
pub const ZOOM_BASE: f64 = 1.1;

type Scene = llimphi_ui::llimphi_raster::vello::Scene;
type Ts = llimphi_ui::llimphi_text::Typesetter;

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

// ---- Caché de imágenes decodificadas -------------------------------------
//
// `ContenidoMarco::Imagen` guarda bytes **codificados** (PNG/JPEG/WebP): el
// core es agnóstico al render. Decodificarlos a RGBA8 en cada frame sería
// carísimo, así que se decodifica una vez y se cachea la `peniko::Image`
// (que es barata de clonar — su `Blob` es `Arc`). La clave `(id, len)` detecta
// el caso de reemplazar la imagen de un marco por otra de distinto tamaño.

static IMG_CACHE: OnceLock<Mutex<HashMap<(MarcoId, usize), PenikoImage>>> = OnceLock::new();

/// Devuelve la `peniko::Image` del marco `id`, decodificando+cacheando la
/// primera vez. `None` si los bytes no son una imagen válida.
fn imagen_cacheada(id: MarcoId, bytes: &[u8]) -> Option<PenikoImage> {
    let cell = IMG_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = cell.lock().ok()?;
    if let Some(img) = g.get(&(id, bytes.len())) {
        return Some(img.clone());
    }
    let img = decodificar(bytes)?;
    g.insert((id, bytes.len()), img.clone());
    Some(img)
}

fn decodificar(bytes: &[u8]) -> Option<PenikoImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    let blob = Blob::from(img.into_raw());
    Some(PenikoImage::new(blob, ImageFormat::Rgba8, w, h))
}

// ---- Pintura por marco ---------------------------------------------------
//
// El `paint_with` corre cada frame con un closure `Send + Sync`. Para no clonar
// los bytes de imagen (ni re-decodificar) por frame, `recorrido_view` precocina
// cada marco a una `Pintura` ligera: el texto se clona (barato) y la imagen se
// resuelve a una `peniko::Image` cacheada (clon barato).

enum Pintura {
    Etiqueta(String),
    Texto { titulo: Option<String>, parrafos: Vec<String> },
    Imagen(PenikoImage),
    Nada,
}

struct MarcoPintura {
    id: MarcoId,
    rect: Rect,
    rot_rad: f64,
    pintura: Pintura,
}

// ---- Colores del lienzo (no temáticos todavía; placeholder sobrio) -------

const FONDO: Color = Color::from_rgba8(18, 20, 28, 255);
const MARCO_FONDO: Color = Color::from_rgba8(38, 42, 56, 255);
const MARCO_BORDE: Color = Color::from_rgba8(80, 86, 104, 255);
const MARCO_ACENTO: Color = Color::from_rgba8(120, 180, 255, 255);
const TEXTO: Color = Color::from_rgba8(225, 230, 240, 235);
const TEXTO_TENUE: Color = Color::from_rgba8(186, 194, 210, 225);

/// Nodo a pantalla completa que pinta el recorrido y registra el rect del
/// panel. `Msg` es libre: el caller suele colgarle un `.draggable(...)` para
/// el pan — esta función no lo impone para no fijar el tipo de mensaje.
pub fn recorrido_view<Msg: 'static>(rec: &Recorrido, state: &RecorridoState) -> View<Msg> {
    // Precocinamos cada marco a una `Pintura` ligera (texto clonado, imagen
    // resuelta a peniko::Image cacheada) para no clonar bytes ni re-decodificar
    // por frame, y para que el closure `Send + Sync` sobreviva sin los bytes.
    let pinturas: Vec<MarcoPintura> = rec
        .marcos
        .iter()
        .map(|m| {
            let pintura = match &m.contenido {
                ContenidoMarco::Etiqueta(t) if !t.is_empty() => Pintura::Etiqueta(t.clone()),
                ContenidoMarco::Texto { titulo, parrafos } => {
                    Pintura::Texto { titulo: titulo.clone(), parrafos: parrafos.clone() }
                }
                ContenidoMarco::Imagen { bytes, .. } => {
                    imagen_cacheada(m.id, bytes).map(Pintura::Imagen).unwrap_or(Pintura::Nada)
                }
                _ => Pintura::Nada,
            };
            MarcoPintura { id: m.id, rect: m.rect, rot_rad: m.rot_rad, pintura }
        })
        .collect();
    let paso_id = rec.pasos.get(state.paso).copied();
    let camara = state.camara;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(FONDO)
    .paint_with(move |scene, ts, rect: PaintRect| {
        panel_set(to_rect(rect));
        pintar(scene, ts, rect, &pinturas, paso_id, &camara);
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
    marcos: &[MarcoPintura],
    paso_id: Option<MarcoId>,
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

        // La imagen se pinta encajada en el marco (respeta giro/zoom vía `xf`).
        if let Pintura::Imagen(img) = &m.pintura {
            pintar_imagen(scene, xf, &m.rect, img);
        }

        let actual = paso_id == Some(m.id);
        let (grosor, color) = if actual { (3.0, MARCO_ACENTO) } else { (1.0, MARCO_BORDE) };
        scene.stroke(&Stroke::new(grosor), xf, color, None, &kr);

        // El texto se pinta en el **espacio local del marco** (origen en su
        // esquina sup-izq, ejes alineados al marco, 1 unidad = 1 px de pantalla),
        // así sigue el giro del marco como lo hace su borde. Los tamaños de
        // fuente se clampan por zoom para seguir legibles lejos/cerca.
        let local = marco_local_xf(cam, panel, &m.rect, m.rot_rad);
        let w_px = (m.rect.w * cam.zoom) as f32;
        let h_px = (m.rect.h * cam.zoom) as f32;
        match &m.pintura {
            Pintura::Etiqueta(t) => pintar_etiqueta(scene, ts, local, w_px, h_px, cam.zoom, t),
            Pintura::Texto { titulo, parrafos } => {
                pintar_texto(scene, ts, local, w_px, h_px, cam.zoom, titulo.as_deref(), parrafos);
            }
            Pintura::Imagen(_) | Pintura::Nada => {}
        }
    }

    scene.pop_layer();
}

/// Pinta `img` encajada en el rect del marco preservando aspect ratio,
/// centrada y clipeada al marco (en su espacio transformado, así respeta el
/// giro propio). `xf` es el mundo→pantalla del marco ya con su rotación.
fn pintar_imagen(scene: &mut Scene, xf: Affine, rect: &Rect, img: &PenikoImage) {
    let (iw, ih) = (img.width as f64, img.height as f64);
    if iw <= 0.0 || ih <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let s = (rect.w / iw).min(rect.h / ih);
    let (dw, dh) = (iw * s, ih * s);
    let ox = rect.x + (rect.w - dw) * 0.5;
    let oy = rect.y + (rect.h - dh) * 0.5;
    let kr = KurboRect::new(rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
    // Clip al rect del marco en su propio espacio (xf incluye giro+zoom).
    scene.push_layer(Mix::Clip, 1.0, xf, &kr);
    let img_xf = xf * Affine::translate((ox, oy)) * Affine::scale(s);
    scene.draw_image(img, img_xf);
    scene.pop_layer();
}

/// Afín que lleva coordenadas **locales del marco** a pantalla: el origen (0,0)
/// es su esquina superior-izquierda, los ejes están alineados al marco (rotados
/// según el giro del marco *relativo a la cámara*) y 1 unidad local = 1 px de
/// pantalla. Pintar texto en este espacio hace que siga el giro del marco igual
/// que su borde, sin que el zoom deforme el tamaño de fuente (que se clampa).
fn marco_local_xf(cam: &Camara, panel: Rect, rect: &Rect, rot_rad: f64) -> Affine {
    let (scx, scy) = cam.world_to_screen(rect.centro(), panel);
    // Giro del marco visto en pantalla: la cámara rota -rot, el marco +rot_rad.
    let ang = rot_rad - cam.rot_rad;
    let (w_px, h_px) = (rect.w * cam.zoom, rect.h * cam.zoom);
    Affine::translate((scx, scy))
        * Affine::rotate(ang)
        * Affine::translate((-w_px * 0.5, -h_px * 0.5))
}

/// Etiqueta de una línea centrada en el marco, en su espacio local (sigue el
/// giro). El tamaño escala con el zoom (clamp para seguir legible lejos/cerca).
fn pintar_etiqueta(scene: &mut Scene, ts: &mut Ts, local: Affine, w_px: f32, h_px: f32, zoom: f64, t: &str) {
    if w_px < 12.0 {
        return; // demasiado chico para texto
    }
    let size_px = ((16.0 * zoom) as f32).clamp(9.0, 40.0);
    let block = TextBlock {
        text: t,
        size_px,
        color: TEXTO,
        // Centrado vertical aproximado dentro del marco; el `max_width` + center
        // resuelven el horizontal.
        origin: (0.0, (h_px as f64 - size_px as f64) * 0.5),
        max_width: Some(w_px),
        alignment: Alignment::Center,
        line_height: 1.2,
        italic: false,
        font_family: None,
    };
    let layout = layout_block(ts, &block);
    draw_layout_xf(scene, &layout, block.color, local * Affine::translate(block.origin));
}

/// Contenido de "slide": título (si hay) + párrafos, fluidos desde la esquina
/// superior-izquierda del marco, clipeados a su rect — todo en el espacio local
/// del marco, así el texto gira con él. El apilado usa la altura medida del
/// título.
fn pintar_texto(
    scene: &mut Scene,
    ts: &mut Ts,
    local: Affine,
    w_px: f32,
    h_px: f32,
    zoom: f64,
    titulo: Option<&str>,
    parrafos: &[String],
) {
    if w_px < 40.0 || h_px < 24.0 {
        return; // demasiado chico para texto fluido
    }
    let pad = ((12.0 * zoom) as f32).clamp(5.0, 22.0);
    let inner_w = (w_px - 2.0 * pad).max(8.0);
    let left = pad as f64;
    let mut y = pad as f64;

    // Clip al rect del marco en su espacio local (gira con el marco).
    let clip = KurboRect::new(0.0, 0.0, w_px as f64, h_px as f64);
    scene.push_layer(Mix::Clip, 1.0, local, &clip);

    if let Some(tt) = titulo.filter(|s| !s.is_empty()) {
        let size = ((22.0 * zoom) as f32).clamp(12.0, 46.0);
        let block = TextBlock {
            text: tt,
            size_px: size,
            color: TEXTO,
            origin: (left, y),
            max_width: Some(inner_w),
            alignment: Alignment::Start,
            line_height: 1.15,
            italic: false,
            font_family: None,
        };
        let layout = layout_block(ts, &block);
        let medida = measurement(&layout);
        draw_layout_xf(scene, &layout, TEXTO, local * Affine::translate((left, y)));
        y += medida.height as f64 + ((10.0 * zoom) as f32).clamp(4.0, 18.0) as f64;
    }

    if !parrafos.is_empty() {
        let cuerpo = parrafos.join("\n\n");
        let size = ((15.0 * zoom) as f32).clamp(9.0, 32.0);
        let block = TextBlock {
            text: &cuerpo,
            size_px: size,
            color: TEXTO_TENUE,
            origin: (left, y),
            max_width: Some(inner_w),
            alignment: Alignment::Start,
            line_height: 1.35,
            italic: false,
            font_family: None,
        };
        let layout = layout_block(ts, &block);
        draw_layout_xf(scene, &layout, TEXTO_TENUE, local * Affine::translate((left, y)));
    }

    scene.pop_layer();
}
