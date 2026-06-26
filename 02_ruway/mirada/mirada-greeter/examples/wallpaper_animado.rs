//! `wallpaper_animado` — un **wallpaper animado de ejemplo**, ejecutable.
//!
//! Corré:  `cargo run -p mirada-greeter --example wallpaper_animado --release`
//!
//! Es una app Llimphi mínima que pinta un **plasma** que se mueve: un reloj
//! (`spawn_periodic` ~30 fps) avanza `t`, y `view()` repinta el fondo entero con
//! `paint_with` en función de `t`. Es exactamente el patrón que usa el fondo
//! animado del greeter (`mirada-greeter/src/bg.rs` + `plasma.rs`), aislado en un
//! ejemplo auto-contenido para poder *verlo* sin levantar el login ni el
//! compositor.
//!
//! **Por qué es sólo un ejemplo (y no el wallpaper del escritorio todavía):** el
//! compositor compone su fondo como un `MemoryRenderBuffer` por salida que
//! **cachea** y sólo rearma cuando cambia (color/gradiente/`procedural` estático/
//! imagen/slideshow). NO existe una fuente `animated` que se repinte por frame, ni
//! está expuesta en wawa-panel. Llevar esto al escritorio es la idea «wallpaper
//! dinámico» del `PLAN.md` §«Wallpaper dinámico/video»: agregar
//! `WallpaperSpec::Animated`, repintar el buffer por frame, y exponer la fuente
//! en el `Schema`. Este ejemplo es el motor de pintura listo para esa mudanza.
//!
//! Teclas:  `Espacio` cambia la paleta · `Esc` sale.

use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KRect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};

/// Paletas de base para el plasma (color al que tienden los lóbulos). El
/// `Espacio` rota entre ellas.
const PALETAS: [(u8, u8, u8); 4] = [
    (60, 130, 230),  // azul
    (210, 80, 150),  // magenta
    (70, 200, 150),  // verde agua
    (235, 150, 60),  // ámbar
];

struct WallpaperAnimado;

#[derive(Clone)]
struct Model {
    /// Reloj de la animación en segundos (lo avanza `Msg::Tick`).
    t: f32,
    /// Índice en [`PALETAS`].
    paleta: usize,
}

#[derive(Clone)]
enum Msg {
    Tick,
    SiguientePaleta,
    Salir,
}

impl App for WallpaperAnimado {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "wallpaper animado · ejemplo"
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // ~30 fps: el latido que hace que el fondo se mueva. Mismo recurso que
        // el greeter (`Msg::RainTick`).
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        Model { t: 0.0, paleta: 0 }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => model.t += 0.033,
            Msg::SiguientePaleta => model.paleta = (model.paleta + 1) % PALETAS.len(),
            Msg::Salir => handle.quit(),
        }
        model
    }

    fn on_key(_model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Space) => Some(Msg::SiguientePaleta),
            Key::Named(NamedKey::Escape) => Some(Msg::Salir),
            _ => None,
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = model.t;
        let base = PALETAS[model.paleta];
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(8, 10, 16, 255))
        .paint_with(move |scene, ts, rect| plasma(scene, ts, rect, t, base))
    }
}

/// Lado de la celda del plasma en px. Más chico = más fino y más caro.
const CELL: f32 = 16.0;

/// Pinta un frame del plasma sobre `rect` para el reloj `t` y el color `base`.
/// Render **puro** (misma técnica que `mirada-greeter/src/plasma.rs`): una grilla
/// donde cada celda suma cuatro fuentes seno (dos lineales, una diagonal y una
/// radial pulsante); el valor mapea a una rampa oscuro→base→blanco. `ts` no se usa.
fn plasma(scene: &mut vello::Scene, _ts: &mut Typesetter, rect: PaintRect, t: f32, base: (u8, u8, u8)) {
    if rect.w < CELL || rect.h < CELL {
        return;
    }
    let cols = (rect.w / CELL).ceil() as i32;
    let rows = (rect.h / CELL).ceil() as i32;
    let cx = cols as f32 / 2.0;
    let cy = rows as f32 / 2.0;
    for gy in 0..rows {
        for gx in 0..cols {
            let fx = gx as f32 / 8.0;
            let fy = gy as f32 / 8.0;
            // Centro pulsante de la fuente radial.
            let mcx = cx + 6.0 * (t * 0.5).sin();
            let mcy = cy + 4.0 * (t * 0.43).cos();
            let dx = (gx as f32 - mcx) / 7.0;
            let dy = (gy as f32 - mcy) / 7.0;
            let rad = (dx * dx + dy * dy).sqrt();
            let v = (fx + t * 0.9).sin()
                + (fy + t * 0.7).cos()
                + ((fx + fy) * 0.5 + t).sin()
                + (rad * 2.0 - t * 1.3).sin();
            let n = ((v / 4.0 + 1.0) * 0.5).clamp(0.0, 1.0); // a [0, 1]
            let x0 = (rect.x + gx as f32 * CELL) as f64;
            let y0 = (rect.y + gy as f32 * CELL) as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                rampa(n, base),
                None,
                &KRect::new(x0, y0, x0 + CELL as f64, y0 + CELL as f64),
            );
        }
    }
}

/// Rampa de color del plasma: `n` en `[0,1]` va de oscuro → `base` → blanco.
fn rampa(n: f32, base: (u8, u8, u8)) -> Color {
    let (br, bg, bb) = (base.0 as f32, base.1 as f32, base.2 as f32);
    let (r, g, b) = if n < 0.5 {
        let f = n / 0.5;
        (br * (0.10 + 0.90 * f), bg * (0.10 + 0.90 * f), bb * (0.10 + 0.90 * f))
    } else {
        let f = (n - 0.5) / 0.5;
        (br + (255.0 - br) * f, bg + (255.0 - bg) * f, bb + (255.0 - bb) * f)
    };
    Color::from_rgba8(r as u8, g as u8, b as u8, 255)
}

fn main() {
    llimphi_ui::run::<WallpaperAnimado>();
}
