//! `supay-doom-llimphi` — Fase 1 del proyecto supay.
//!
//! Doom real corriendo dentro de una ventana Llimphi. La simulación
//! C (doomgeneric) avanza un tick por cada `Msg::Tick` a 35 Hz; el
//! framebuffer 320×200 ARGB que el motor llena se pinta como
//! `View::image` con aspect-fit. Las pulsaciones de teclado de
//! Llimphi se traducen a códigos Doom (`KEY_FIRE`, `KEY_USE`, los
//! `KEY_*ARROW`) y se encolan en el motor con `push_key`.
//!
//! ## Controles
//!
//! - **WASD / flechas** — moverse / girar (los manda como
//!   `KEY_UPARROW`, etc.; doomgeneric los maneja con su config
//!   nativa de Doom).
//! - **,/.** — strafe izquierdo / derecho.
//! - **Ctrl o Enter** — disparar (`KEY_FIRE`).
//! - **Space** — usar / abrir puertas (`KEY_USE`).
//! - **Shift** — correr.
//! - **Tab** — mapa.
//! - **Esc** — menú del juego (Doom abre su menú; segundo Esc cierra ventana).
//! - **Y/N** — confirmaciones del menú.
//!
//! ## Requiere
//!
//! 1. Vendoring de doomgeneric. Ver `supay-core/vendor/README.md`.
//! 2. Un WAD legalmente distribuible (DOOM1.WAD shareware) en el
//!    cwd desde donde se corre. Sin WAD, doomgeneric aborta.
//!
//! En modo stub (sin vendor), la app arranca igual y pinta un mensaje
//! explicando qué falta — útil para verificar el plumbing Llimphi.

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Color, Image, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use supay_core::{keys, DoomEngine, SnapshotPair, DOOM_HEIGHT, DOOM_PIXELS, DOOM_WIDTH};

const TICK_HZ: u64 = 35; // canónico de Doom
const TICK_MS: u64 = 1_000 / TICK_HZ;

struct Model {
    engine: DoomEngine,
    tick: u64,
    /// Framebuffer del último frame del motor, ya en formato Rgba8
    /// (lo que `View::image` espera).
    framebuffer_rgba: Vec<u8>,
    /// Fase 2: par de snapshots (prev + next) capturados desde
    /// doomgeneric — el renderer 3D futuro interpolará entre los dos
    /// para correr más rápido que 35 Hz. Por ahora sólo lo mantenemos
    /// vivo para validar el plumbing y mostrar conteos en el header.
    snapshots: SnapshotPair,
}

#[derive(Clone)]
enum Msg {
    Tick,
    Key(KeyEvent),
    Quit,
}

struct Supay;

impl App for Supay {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "supay · fase 1 · doom"
    }

    fn initial_size() -> (u32, u32) {
        // 4× la resolución original de Doom = 1280×800 — el aspect-fit
        // de `View::image` se encarga del resto. Bordes negros si la
        // relación no encaja.
        (1280, 800)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        // Args estilo argv. `-iwad doom1.wad` busca el WAD en cwd.
        // `-nosound` apaga el subsistema de audio entero: nuestros
        // stubs devuelven 0 / NULL y eso confunde a `S_StartSound`
        // que va a leer "lump 0" del WAD como si fuera un efecto
        // de sonido — segfault. Con `-nosound` el motor ni intenta.
        let args = vec![
            "doomgeneric".to_string(),
            "-iwad".to_string(),
            "doom1.wad".to_string(),
            "-nosound".to_string(),
        ];
        Model {
            engine: DoomEngine::new(args),
            tick: 0,
            framebuffer_rgba: vec![0; DOOM_PIXELS * 4],
            snapshots: SnapshotPair::new(),
        }
    }

    fn on_key(_: &Model, e: &KeyEvent) -> Option<Msg> {
        if matches!(&e.key, Key::Named(NamedKey::F12)) && e.state == KeyState::Pressed {
            // F12 cierra la ventana — Esc lo manejamos como KEY_ESCAPE del juego.
            return Some(Msg::Quit);
        }
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Quit => handle.quit(),
            Msg::Tick => {
                m.tick = m.tick.wrapping_add(1);
                m.engine.tick();
                refresh_framebuffer(&mut m);
                // Fase 2: capturamos snapshot tras el tick. El renderer
                // de Fase 3 lo consumirá; por ahora vive como evidencia
                // de que el plumbing funciona.
                let snap = m.engine.capture_scene(m.tick);
                m.snapshots.push(snap);
            }
            Msg::Key(e) => {
                if let Some(doom_key) = translate_key(&e.key, e.text.as_deref()) {
                    let pressed = e.state == KeyState::Pressed;
                    m.engine.push_key(pressed, doom_key);
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let header = header_bar(model, &theme);
        let body = if model.engine.real {
            game_pane(model)
        } else {
            stub_message_pane(&theme)
        };
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(0, 0, 0, 255))
        .children(vec![header, body])
    }
}

fn header_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let mode = if model.engine.real { "ENGINE REAL" } else { "STUB" };
    let title = model.engine.title();
    let title = if title.is_empty() { "supay-doom".to_string() } else { title };
    // Fase 2: stats del snapshot más reciente.
    let scene = model
        .snapshots
        .next()
        .map(|s| {
            format!(
                "scene[w={} sec={} spr={}]",
                s.walls.len(),
                s.sectors.len(),
                s.sprites.len()
            )
        })
        .unwrap_or_else(|| "scene[—]".to_string());
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        format!("{title}  ·  tick {}  ·  {}  ·  {}", model.tick, mode, scene),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

fn game_pane(model: &Model) -> View<Msg> {
    // Reconstruimos el peniko::Image por frame — `Blob::from` clona los
    // bytes para que el Image sea owning. Costo aceptable a 320×200×4
    // = 256 KB/frame.
    let blob = Blob::from(model.framebuffer_rgba.clone());
    let image = Image::new(blob, ImageFormat::Rgba8, DOOM_WIDTH as u32, DOOM_HEIGHT as u32);
    // Patrón flex correcto para "ocupá el resto de la columna":
    // `flex_basis = 0` + `flex_grow = 1` + `height = auto`. Si en su
    // lugar pidiéramos `height = percent(1.0)`, taffy le daría el
    // 100% del padre (= ventana entera) y la imagen se solaparía
    // con el header de 24 px.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .image(image)
}

fn stub_message_pane(theme: &Theme) -> View<Msg> {
    let title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(60.0_f32),
        },
        padding: Rect {
            left: length(32.0_f32),
            right: length(32.0_f32),
            top: length(40.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "supay-doom-llimphi corre en modo STUB".to_string(),
        24.0,
        theme.fg_text,
        Alignment::Start,
    );
    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(32.0_f32),
            right: length(32.0_f32),
            top: length(10.0_f32),
            bottom: length(32.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "Para correr Doom real:\n\n\
         1.  cd 02_ruway/supay/supay-core/vendor\n\
         2.  git clone https://github.com/ozkl/doomgeneric.git\n\
         3.  Bajá DOOM1.WAD (shareware) al cwd:\n\
         \n     curl -O https://distro.ibiblio.org/slitaz/sources/packages/d/doom1.wad\n\n\
         4.  cargo run -p supay-doom-llimphi --release\n\n\
         La ventana Llimphi pintará el framebuffer 320×200 del motor\n\
         original con aspect-fit a la resolución de la ventana.\n\n\
         F12 cierra la ventana en cualquier momento.".to_string(),
        13.0,
        theme.fg_muted,
        Alignment::Start,
    )
    .fill(Color::from_rgba8(10, 10, 14, 255));
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![title, body])
}

/// Copia el framebuffer ARGB (`0xAARRGGBB` little-endian u32) que el
/// motor llenó al buffer Rgba8 que `peniko::Image` espera. Cuatro
/// canales contiguos por píxel.
fn refresh_framebuffer(m: &mut Model) {
    let src = m.engine.framebuffer();
    let dst = &mut m.framebuffer_rgba;
    debug_assert_eq!(src.len(), DOOM_PIXELS);
    debug_assert_eq!(dst.len(), DOOM_PIXELS * 4);
    for (i, px) in src.iter().enumerate() {
        let r = ((px >> 16) & 0xff) as u8;
        let g = ((px >> 8) & 0xff) as u8;
        let b = (px & 0xff) as u8;
        // doomgeneric llena alpha en 0; lo forzamos a 255 para que el
        // píxel sea opaco — sino el `View::image` lo mezclaría con el
        // fondo.
        let a = 0xff_u8;
        let o = i * 4;
        dst[o] = r;
        dst[o + 1] = g;
        dst[o + 2] = b;
        dst[o + 3] = a;
    }
}

/// Traduce un evento de teclado Llimphi a código Doom. Devolver
/// `None` significa "esta tecla no le importa al motor".
fn translate_key(key: &Key, text: Option<&str>) -> Option<u8> {
    use keys::*;
    // Primero las teclas con nombre estable.
    if let Key::Named(named) = key {
        return Some(match named {
            NamedKey::ArrowUp => KEY_UPARROW,
            NamedKey::ArrowDown => KEY_DOWNARROW,
            NamedKey::ArrowLeft => KEY_LEFTARROW,
            NamedKey::ArrowRight => KEY_RIGHTARROW,
            NamedKey::Escape => KEY_ESCAPE,
            NamedKey::Enter => KEY_ENTER,
            NamedKey::Tab => KEY_TAB,
            NamedKey::Shift => KEY_RSHIFT,
            NamedKey::Space => KEY_USE,
            NamedKey::Control => KEY_FIRE,
            _ => return None,
        });
    }
    // Luego las que vienen como texto.
    let ch = text.and_then(|s| s.chars().next())?;
    Some(match ch.to_ascii_lowercase() {
        'w' => KEY_UPARROW,
        's' => KEY_DOWNARROW,
        'a' => KEY_LEFTARROW,
        'd' => KEY_RIGHTARROW,
        ',' => KEY_STRAFE_L,
        '.' => KEY_STRAFE_R,
        ' ' => KEY_USE,
        'y' => KEY_Y,
        'n' => KEY_N,
        _ => return None,
    })
}

fn main() {
    llimphi_ui::run::<Supay>();
}
