//! `pineal-gpu-demo` — starfield warp 3D sobre el camino GPU directo.
//!
//! Es el **primer consumidor real de `GpuSceneCanvas`** (Fase 4 del SDD
//! de pineal). El resto del catálogo sólo tenía el backend GPU
//! documentado y testeado; aquí se pinta de verdad: el demo emite hasta
//! 1 M de `fill_rect` por frame contra el trait `Canvas`, pero detrás
//! del trait está `GpuSceneCanvas` empujando a un `GpuBatch` que despacha
//! todo en **una sola draw call instanciada** (P3 del SDD).
//!
//! Reparto de responsabilidades (Elm + el split CPU/GPU del SDD):
//!
//! - `update(Tick)` avanza la simulación en CPU: cada estrella vuela
//!   hacia el observador (`z -= speed`) y se recicla al fondo al cruzar
//!   el plano cercano. Estado en un `Vec<f32>` plano interleaved
//!   `[x,y,z, ...]`, mutado in-place (P1 + P2: zero boxing, zero alloc
//!   en hot path).
//! - `gpu_paint_with(...)` proyecta perspectiva y dibuja: lee el campo
//!   de estrellas, calcula el pixel de cada una contra el rect del nodo
//!   y llama `canvas.fill_rect(...)`. Ni el demo ni el painter saben de
//!   `wgpu` — sólo hablan el trait `Canvas`.
//!
//! El campo se comparte entre `update` (escribe) y el painter (lee) por
//! `Arc<Mutex<StarField>>`. Ambos corren en el hilo de UI, así que el
//! mutex nunca se disputa; el `Arc` existe sólo porque la closure GPU
//! debe ser `'static` y no puede tomar prestado el `Model`.
//!
//! Las `GpuPipelines` (shaders + render pipelines) se compilan una vez y
//! se cachean en un `OnceLock` que vive en el `Model`, no en `view()` —
//! recompilarlas por frame mataría el framerate.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as PadRect;
use llimphi_ui::llimphi_raster::peniko::Color as PenikoColor;
use llimphi_ui::llimphi_raster::{GpuBatch, GpuPipelines};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, KeyEvent, KeyState, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};

use pineal_render::{Canvas, Color, GpuSceneCanvas, Rect};

/// La intermediate del frame es `Rgba8Unorm` (ver
/// `llimphi-compositor::GpuPaintFn`). Las pipelines deben coincidir.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const FRAME_PERIOD: Duration = Duration::from_millis(16);
const NEAR: f32 = 0.06;
const FAR: f32 = 1.0;
const WARP_SPEED: f32 = 0.012;

/// Densidades cicladas desde el menú. La última (1 M) es la que el SDD
/// daba como techo del camino GPU directo.
const DENSITIES: [usize; 4] = [50_000, 200_000, 500_000, 1_000_000];

#[derive(Clone)]
enum Msg {
    Tick,
    /// Cicla a la siguiente densidad de estrellas y reconstruye el campo.
    CycleDensity,
    /// Pausa/reanuda el warp (las estrellas quedan congeladas).
    TogglePause,
    MenuOpen(Option<usize>),
    MenuCommand(String),
    CloseMenus,
    CycleTheme,
    ContextMenuOpen(f32, f32),
}

/// Campo de estrellas en coordenadas de cámara. `xyz` interleaved
/// `[x0,y0,z0, x1,y1,z1, ...]`; x,y ∈ [-1,1], z ∈ (NEAR, FAR].
struct StarField {
    xyz: Vec<f32>,
    count: usize,
    rng: u32,
}

impl StarField {
    fn new(count: usize) -> Self {
        let mut field = StarField {
            xyz: vec![0.0; count * 3],
            count,
            rng: 0x9E37_79B9,
        };
        for i in 0..count {
            field.respawn(i, true);
        }
        field
    }

    /// xorshift32 → f32 en [-1, 1).
    fn unit(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Reposiciona la estrella `i`. Con `fresh` la siembra a una `z`
    /// aleatoria en todo el rango (arranque); si no, la manda al fondo
    /// (reciclaje cuando cruzó el plano cercano).
    fn respawn(&mut self, i: usize, fresh: bool) {
        let x = self.unit();
        let y = self.unit();
        let z = if fresh {
            // Aleatoria en (NEAR, FAR] para no arrancar todas en fila.
            NEAR + (self.unit() * 0.5 + 0.5) * (FAR - NEAR)
        } else {
            FAR
        };
        let b = i * 3;
        self.xyz[b] = x;
        self.xyz[b + 1] = y;
        self.xyz[b + 2] = z;
    }

    /// Un paso de simulación: todas vuelan hacia el observador; las que
    /// cruzan el plano cercano se reciclan al fondo. Zero alloc.
    fn advance(&mut self) {
        for i in 0..self.count {
            let zi = i * 3 + 2;
            let z = self.xyz[zi] - WARP_SPEED;
            if z <= NEAR {
                self.respawn(i, false);
            } else {
                self.xyz[zi] = z;
            }
        }
    }
}

struct Model {
    field: Arc<Mutex<StarField>>,
    pipelines: Arc<OnceLock<GpuPipelines>>,
    density_idx: usize,
    paused: bool,
    frame: u64,
    theme: Theme,
    menu_open: Option<usize>,
    context_menu: Option<(f32, f32)>,
}

struct GpuDemo;

impl App for GpuDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pineal — starfield warp (GPU directo)"
    }
    fn initial_size() -> (u32, u32) {
        (1100, 700)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(FRAME_PERIOD, || Msg::Tick);
        let density_idx = 1; // 200 K por defecto
        Model {
            field: Arc::new(Mutex::new(StarField::new(DENSITIES[density_idx]))),
            pipelines: Arc::new(OnceLock::new()),
            density_idx,
            paused: false,
            frame: 0,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                if !model.paused {
                    model.field.lock().unwrap().advance();
                    model.frame = model.frame.wrapping_add(1);
                }
            }
            Msg::CycleDensity => {
                model.density_idx = (model.density_idx + 1) % DENSITIES.len();
                let n = DENSITIES[model.density_idx];
                model.field = Arc::new(Mutex::new(StarField::new(n)));
            }
            Msg::TogglePause => model.paused = !model.paused,
            Msg::MenuOpen(which) => {
                model.menu_open = which;
                model.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                return handle_menu_command(model, &cmd, handle);
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.context_menu = None;
            }
            Msg::CycleTheme => model.theme = Theme::next_after(model.theme.name),
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "pineal — starfield warp · GpuSceneCanvas → 1 draw call instanciada".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let stats = format!(
            "estrellas = {}    {}    frame = {}    backend = GpuBatch (wgpu, Rgba8Unorm)    \
             [D] densidad · [espacio] pausa · click-derecho menú",
            fmt_count(DENSITIES[model.density_idx]),
            if model.paused { "PAUSA" } else { "warp" },
            model.frame,
        );
        let stats_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        // El panel del cielo: fondo negro por vello + estrellas por GPU
        // directo encima (LoadOp::Load preserva el fondo).
        let field = model.field.clone();
        let pipelines = model.pipelines.clone();
        let sky = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .fill(PenikoColor::from_rgba8(2, 3, 9, 255))
        .gpu_paint_with(move |device, queue, encoder, target, rect, viewport| {
            let pipes = pipelines.get_or_init(|| GpuPipelines::new(device, TARGET_FORMAT));
            let mut batch = GpuBatch::new(pipes);

            // Proyección perspectiva contra el rect del nodo (pixels
            // absolutos del scene). Todo lo que sigue es el caller
            // decidiendo "qué pintar"; el "cómo" es del backend.
            {
                let mut canvas = GpuSceneCanvas::new(&mut batch);
                let cx = rect.x + rect.w * 0.5;
                let cy = rect.y + rect.h * 0.5;
                let kx = rect.w * 0.05;
                let ky = rect.h * 0.05;
                let f = model_field_snapshot(&field);
                let xyz = &f.xyz;
                for i in 0..f.count {
                    let b = i * 3;
                    let (x, y, z) = (xyz[b], xyz[b + 1], xyz[b + 2]);
                    let inv = 1.0 / z;
                    let sx = cx + x * inv * kx;
                    let sy = cy + y * inv * ky;
                    // Cull fuera del rect (barato; evita rects enormes).
                    if sx < rect.x || sx > rect.x + rect.w || sy < rect.y || sy > rect.y + rect.h {
                        continue;
                    }
                    // Cerca = grande y brillante; lejos = punto tenue.
                    let bright = ((FAR - z) / (FAR - NEAR)).clamp(0.0, 1.0);
                    let size = 0.6 + bright * bright * 2.6;
                    let half = size * 0.5;
                    let a = 0.25 + bright * 0.75;
                    // Tinte ligeramente frío hacia el azul-blanco.
                    let col = Color::rgba(0.78 + bright * 0.22, 0.84 + bright * 0.16, 1.0, a);
                    canvas.fill_rect(
                        Rect { x: sx - half, y: sy - half, w: size, h: size },
                        col,
                    );
                }
            }

            batch.flush(
                device,
                queue,
                encoder,
                target,
                (viewport.0 as f32, viewport.1 as f32),
                wgpu::LoadOp::Load,
            );
        });

        let sky_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![sky]);

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            padding: PadRect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(14.0_f32),
                bottom: length(14.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, stats_row, sky_panel]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, body])
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        match event.text.as_deref() {
            Some("d") | Some("D") => Some(Msg::CycleDensity),
            Some(" ") => Some(Msg::TogglePause),
            _ => None,
        }
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        if let Some((x, y)) = model.context_menu {
            let items = vec![
                ContextMenuItem::action("Más estrellas"),
                ContextMenuItem::action("Pausa / reanudar"),
                ContextMenuItem::action("Cambiar tema"),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| match i {
                0 => Msg::CycleDensity,
                1 => Msg::TogglePause,
                _ => Msg::CycleTheme,
            });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport: viewport_of(model),
                header: Some("Starfield".to_string()),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model))
    }
}

/// Clona el snapshot del campo bajo el lock (libera el mutex antes de
/// la proyección). El campo es `Vec<f32>` plano: el clone es un memcpy
/// contiguo, sin punteros ni boxing (P1).
fn model_field_snapshot(field: &Arc<Mutex<StarField>>) -> FieldSnapshot {
    let g = field.lock().unwrap();
    FieldSnapshot { xyz: g.xyz.clone(), count: g.count }
}

struct FieldSnapshot {
    xyz: Vec<f32>,
    count: usize,
}

fn fmt_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{} M", n / 1_000_000)
    } else {
        format!("{} K", n / 1_000)
    }
}

fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = GpuDemo::initial_size();
    (w as f32, h as f32)
}

fn menubar_spec<'a>(menu: &'a AppMenu, model: &'a Model) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(Menu::new("Archivo").item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q")))
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Más estrellas", "view.density"))
                .item(MenuItem::new("Pausa / reanudar", "view.pause"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.quit" => std::process::exit(0),
        "view.density" => {
            handle.dispatch(Msg::CycleDensity);
            model
        }
        "view.pause" => {
            handle.dispatch(Msg::TogglePause);
            model
        }
        "view.theme" => {
            handle.dispatch(Msg::CycleTheme);
            model
        }
        _ => model,
    }
}

fn main() {
    llimphi_ui::run::<GpuDemo>();
}
