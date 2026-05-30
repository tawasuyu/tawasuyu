//! Demo de **imagen dentro del marco** (cierra el pendiente menor de la Fase 3
//! del §6.sexies): un marco cuyo contenido es `ContenidoMarco::Imagen`.
//!
//! El core guarda los bytes **codificados** (aquí un PNG generado en memoria,
//! sin tocar disco) + sus dimensiones; el frontend Llimphi los decodifica una
//! vez, los cachea y los pinta encajados en el marco preservando aspect ratio
//! —respetando giro y zoom de la cámara igual que el texto—.
//!
//! Mezcla slides de texto con dos marcos-imagen (uno girado, para lucir que la
//! imagen vuela con el marco). Controles iguales que `recorrido_demo`:
//!   - **→ / ↓ / Espacio / Enter**: paso siguiente.   **← / ↑**: anterior.
//!   - **rueda**: zoom-a-cursor.   **arrastrar**: paneo libre.
//!
//! Corre con:
//!   `cargo run -p pluma-deck-recorrido-llimphi --example recorrido_imagen_demo --release`

use std::time::Duration;

use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use pluma_deck_core::{ContenidoMarco, Marco, Recorrido, RecorridoState, Rect, RejillaOpts};
use pluma_deck_recorrido_llimphi::{dentro, panel_actual, recorrido_view, ZOOM_BASE};

const PANEL_INICIAL: Rect = Rect { x: 0.0, y: 0.0, w: 1100.0, h: 720.0 };

#[derive(Clone)]
enum Msg {
    Zoom { mult: f64, cursor: (f32, f32) },
    Pan { dx: f32, dy: f32 },
    Siguiente,
    Anterior,
    Tick,
}

struct Model {
    rec: Recorrido,
    state: RecorridoState,
}

/// Genera un PNG en memoria: degradado diagonal con una rejilla, para tener una
/// imagen reconocible sin depender de un asset en disco. Devuelve los bytes PNG
/// codificados (lo que viaja en `ContenidoMarco::Imagen`) + sus dimensiones.
fn png_demo(w: u32, h: u32, base: (u8, u8, u8)) -> (Vec<u8>, u32, u32) {
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        // Degradado diagonal sobre el color base + rejilla cada 40 px.
        let t = (fx + fy) * 0.5;
        let mezcla = |c: u8| (c as f32 * (0.35 + 0.65 * t)) as u8;
        let rejilla = (x % 40 == 0 || y % 40 == 0) as u8 * 40;
        *px = image::Rgba([
            mezcla(base.0).saturating_add(rejilla),
            mezcla(base.1).saturating_add(rejilla),
            mezcla(base.2).saturating_add(rejilla),
            255,
        ]);
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("codificar PNG demo");
    (bytes, w, h)
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · recorrido (imagen dentro del marco)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let slide = |t: &str, ps: &[&str]| ContenidoMarco::Texto {
            titulo: Some(t.into()),
            parrafos: ps.iter().map(|s| s.to_string()).collect(),
        };
        let (png_a, wa, ha) = png_demo(480, 320, (90, 150, 230));
        let (png_b, wb, hb) = png_demo(360, 360, (230, 140, 90));

        let contenidos = vec![
            slide(
                "Imagen nativa en el marco",
                &[
                    "El marco siguiente lleva una imagen: el core guarda bytes PNG; el frontend los rasteriza y encaja.",
                    "La imagen vuela con la cámara — zoom y giro la transforman como a cualquier marco.",
                ],
            ),
            ContenidoMarco::Imagen { bytes: png_a, ancho: wa, alto: ha },
            slide("Aspect ratio preservado", &["La imagen se centra y encaja sin deformarse, clipeada al marco."]),
        ];
        let mut rec = Recorrido::en_rejilla(
            contenidos,
            RejillaOpts { cols: 3, marco_w: 640.0, marco_h: 420.0, gap_x: 240.0, gap_y: 200.0 },
        );
        // Marco-imagen suelto y girado, para lucir que la imagen sigue el giro.
        let id = (rec.marcos.len() + 1) as u64;
        rec.agregar_marco(
            Marco::new(id, Rect::new(220.0, 760.0, 360.0, 360.0), ContenidoMarco::Imagen { bytes: png_b, ancho: wb, alto: hb })
                .con_giro(0.18),
        );
        rec.pasos.push(id);

        let mut state = RecorridoState::new();
        state.saltar_a_paso(&rec, 0, PANEL_INICIAL);
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
        Model { rec, state }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let panel = panel_actual().unwrap_or(PANEL_INICIAL);
        match msg {
            Msg::Zoom { mult, cursor } => model.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel),
            Msg::Pan { dx, dy } => model.state.arrastrar_delta(dx as f64, dy as f64),
            Msg::Siguiente => {
                model.state.siguiente(&model.rec, panel);
            }
            Msg::Anterior => {
                model.state.anterior(&model.rec, panel);
            }
            Msg::Tick => {
                model.state.avanzar(1.0 / 60.0);
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        recorrido_view(&model.rec, &model.state).draggable(|phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::Pan { dx, dy }),
            DragPhase::End => None,
        })
    }

    fn on_wheel(_m: &Self::Model, delta: WheelDelta, cursor: (f32, f32), _mods: Modifiers) -> Option<Self::Msg> {
        let panel = panel_actual()?;
        if !dentro(panel, cursor.0, cursor.1) {
            return None;
        }
        Some(Msg::Zoom { mult: ZOOM_BASE.powf(-delta.y as f64), cursor })
    }

    fn on_key(_m: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        match &ev.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Space) => Some(Msg::Siguiente),
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => Some(Msg::Anterior),
            _ => None,
        }
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
