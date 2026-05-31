//! Grabador de pantalla Llimphi — la integración UI del lado INPUT de
//! `media`. Un botón Rec/Stop, un timer y el estado de la grabación;
//! por debajo, el loop `ScreenSource (X11) + MicSource (cpal) →
//! media-recorder-webm → .webm AV1+Opus nativo`, sin ffmpeg.
//!
//! El bucle Elm de Llimphi (`update`/`view`) corre en el hilo de la UI
//! y **no debe bloquear**. La grabación es trabajo largo y pesado
//! (encode AV1 por frame), así que vive en un hilo de fondo lanzado con
//! [`Handle::spawn`]: la clausura corre el loop hasta que el flag de
//! stop se levanta y, al terminar, **devuelve** un `Msg::Finished` que
//! el bucle Elm recibe en `update` — cero locks compartidos con la UI
//! salvo el handle clonable del recorder.
//!
//! Corre con: `cargo run -p media-recorder-app --release`
//! (necesita `$DISPLAY`; el micrófono es opcional — sin él graba
//! video-solo).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};

use media_core::{AudioSource, FrameSource};
use media_recorder_webm::{
    default_recording_path, RecordedAudioSource, RecordedFrameSource, WebmRecorder,
    WebmRecorderSettings,
};
use media_source_capture::{
    MicSource, ScreenOptions, ScreenSource, WaylandScreenOptions, WaylandScreenSource,
};

const FPS: u32 = 30;

/// Backend de pantalla elegido en runtime: X11 o Wayland (wlr). Un enum
/// en vez de `Box<dyn FrameSource>` para no depender de un impl de Box.
enum Screen {
    X11(ScreenSource),
    Wayland(WaylandScreenSource),
}

impl FrameSource for Screen {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        match self {
            Screen::X11(s) => s.tick(dt, buf),
            Screen::Wayland(s) => s.tick(dt, buf),
        }
    }
}

/// Abre la pantalla con el backend adecuado: Wayland si hay
/// `$WAYLAND_DISPLAY` (con fallback a X11/XWayland si falla), si no X11.
fn open_screen(fps: u32) -> Result<Screen, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match WaylandScreenSource::open(WaylandScreenOptions {
            fps,
            ..Default::default()
        }) {
            Ok(s) => return Ok(Screen::Wayland(s)),
            Err(e) => {
                // wlr-screencopy no disponible (GNOME/KDE): si hay X11
                // (XWayland) seguimos por ahí; si no, devolvemos el error.
                if std::env::var_os("DISPLAY").is_none() {
                    return Err(format!("wayland: {e}"));
                }
            }
        }
    }
    ScreenSource::open(ScreenOptions {
        fps,
        ..Default::default()
    })
    .map(Screen::X11)
    .map_err(|e| format!("pantalla: {e}"))
}

/// Resumen liviano (Clone) de una grabación cerrada, para viajar en el
/// `Msg`.
#[derive(Clone)]
struct RecLite {
    path: String,
    frames: usize,
    audio_packets: usize,
    kib: f64,
}

#[derive(Clone)]
enum Msg {
    Start,
    Stop,
    Tick,
    Finished(Result<RecLite, String>),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal o contextual — se traduce al
    /// `Msg`/efecto real (iniciar/detener grabación, alternar tema, salir).
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana. Origen de la raíz es 0,0 ⇒ local == ventana.
    ContextMenuOpen(f32, f32),
    /// Alterna el tema claro/oscuro de la app.
    ToggleTheme,
}

enum RecState {
    Idle,
    Recording { since: Instant, path: String },
    Stopping,
    Saved(RecLite),
    Failed(String),
}

struct Model {
    state: RecState,
    rec: WebmRecorder,
    stop: Arc<AtomicBool>,
    /// Segundos transcurridos, refrescados por `Tick` mientras graba.
    elapsed_secs: u64,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Menú contextual: ancla `(x, y)` en ventana. `None` cerrado. La app
    /// no tiene campos de texto editables, así que el contextual mapea a
    /// comandos de grabación reales — no a edición.
    context_menu: Option<(f32, f32)>,
    /// Tema oscuro (default) o claro. El menú Ver lo alterna.
    dark: bool,
    /// Tamaño aproximado del viewport para anclar overlays. Sin hook de
    /// resize en llimphi-ui, lo fijamos al `initial_size`.
    viewport: (f32, f32),
}

struct RecorderApp;

impl App for RecorderApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "media · grabador de pantalla"
    }

    fn initial_size() -> (u32, u32) {
        (560, 380)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Timer de refresco del cronómetro (no-op cuando no graba).
        handle.spawn_periodic(Duration::from_millis(500), || Msg::Tick);
        Model {
            state: RecState::Idle,
            rec: WebmRecorder::with_settings(WebmRecorderSettings {
                fps_num: FPS,
                fps_den: 1,
                ..Default::default()
            }),
            stop: Arc::new(AtomicBool::new(false)),
            elapsed_secs: 0,
            menu_open: None,
            context_menu: None,
            dark: true,
            viewport: (560.0, 380.0),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Start => {
                if matches!(model.state, RecState::Recording { .. } | RecState::Stopping) {
                    return model; // ya grabando.
                }
                model.stop.store(false, Ordering::Release);
                let path = default_recording_path(std::env::current_dir().unwrap_or_default());
                let path_str = path.display().to_string();

                let rec = model.rec.clone();
                let stop = model.stop.clone();
                // Trabajo pesado en background; devuelve Msg::Finished al cerrar.
                handle.spawn(move || record_loop(rec, stop, path));

                model.elapsed_secs = 0;
                model.state = RecState::Recording {
                    since: Instant::now(),
                    path: path_str,
                };
            }
            Msg::Stop => {
                if let RecState::Recording { .. } = model.state {
                    model.stop.store(true, Ordering::Release);
                    model.state = RecState::Stopping;
                }
            }
            Msg::Tick => {
                if let RecState::Recording { since, .. } = &model.state {
                    model.elapsed_secs = since.elapsed().as_secs();
                }
            }
            Msg::Finished(res) => {
                model.state = match res {
                    Ok(lite) => RecState::Saved(lite),
                    Err(e) => RecState::Failed(e),
                };
            }
            Msg::MenuOpen(which) => {
                model.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                model.context_menu = None;
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.context_menu = None;
                return handle_menu_command(model, &cmd, handle);
            }
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                model.context_menu = Some((x, y));
            }
            Msg::ToggleTheme => {
                model.dark = !model.dark;
            }
        }
        model
    }

    /// Atajos globales: `Esc` cierra cualquier menú abierto.
    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        if matches!(event.key, Key::Named(NamedKey::Escape))
            && (model.menu_open.is_some() || model.context_menu.is_some())
        {
            return Some(Msg::CloseMenus);
        }
        None
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Prioridad: menú contextual > dropdown del menú principal.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu(model, x, y));
        }
        let menu = app_menu(model);
        menubar_overlay(&menubar_spec(&menu, model))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // --- Barra de menú principal: primer hijo del column raíz. ---
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));

        // Colores de fondo / sub-línea según el tema activo.
        let bg = if model.dark {
            rgb(18, 22, 30)
        } else {
            rgb(238, 240, 245)
        };
        let muted = if model.dark {
            rgb(150, 160, 175)
        } else {
            rgb(90, 100, 115)
        };

        // --- Estado / cabecera ---
        let (status, status_color) = match &model.state {
            RecState::Idle => (
                "listo para grabar".to_string(),
                if model.dark {
                    rgb(170, 180, 195)
                } else {
                    rgb(70, 80, 95)
                },
            ),
            RecState::Recording { .. } => (
                format!("● REC  {}", fmt_mmss(model.elapsed_secs)),
                rgb(240, 90, 90),
            ),
            RecState::Stopping => ("guardando…".to_string(), rgb(230, 200, 90)),
            RecState::Saved(l) => (
                format!("✓ {} frames · {:.0} KiB", l.frames, l.kib),
                rgb(120, 210, 150),
            ),
            RecState::Failed(_) => ("error".to_string(), rgb(240, 110, 110)),
        };
        let header = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(status, 40.0, status_color);

        // --- Sub-línea: path / detalle de audio / mensaje de error ---
        let detail = match &model.state {
            RecState::Idle => "pantalla + micrófono → .webm AV1+Opus".to_string(),
            RecState::Recording { path, .. } => path.clone(),
            RecState::Stopping => "muxeando AV1+Opus…".to_string(),
            RecState::Saved(l) => {
                let audio = if l.audio_packets > 0 {
                    format!("{} paquetes Opus", l.audio_packets)
                } else {
                    "video-solo".to_string()
                };
                format!("{}  ·  {}", l.path, audio)
            }
            RecState::Failed(e) => e.clone(),
        };
        let subline = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(detail, 15.0, muted);

        // --- Botón Rec/Stop ---
        let recording = matches!(model.state, RecState::Recording { .. });
        let stopping = matches!(model.state, RecState::Stopping);
        let (label, fill, msg) = if recording {
            ("■ Detener", rgb(220, 70, 70), Msg::Stop)
        } else if stopping {
            ("…", rgb(120, 120, 130), Msg::Stop)
        } else {
            ("● Grabar", rgb(70, 200, 130), Msg::Start)
        };
        let mut button = View::new(Style {
            size: Size {
                width: length(220.0_f32),
                height: length(64.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(fill)
        .radius(14.0)
        .text(label, 26.0, rgb(12, 24, 18));
        if !stopping {
            button = button.on_click(msg);
        }
        let button_row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(64.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![button]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(20.0_f32),
            },
            padding: Rect {
                left: length(28.0_f32),
                right: length(28.0_f32),
                top: length(28.0_f32),
                bottom: length(28.0_f32),
            },
            ..Default::default()
        })
        .fill(bg)
        // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el
        // menú contextual del grabador.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, header, subline, button_row])
    }
}

/// Tema activo de la app (claro/oscuro) según el flag del modelo.
fn app_theme(model: &Model) -> llimphi_theme::Theme {
    if model.dark {
        llimphi_theme::Theme::dark()
    } else {
        llimphi_theme::Theme::light()
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model) -> MenuBarSpec<'a, Msg> {
    // El theme va por valor en un slot estático según el flag — el spec
    // toma `&Theme`, así que necesitamos una referencia con vida 'static.
    static DARK: std::sync::OnceLock<llimphi_theme::Theme> = std::sync::OnceLock::new();
    static LIGHT: std::sync::OnceLock<llimphi_theme::Theme> = std::sync::OnceLock::new();
    let theme: &'static llimphi_theme::Theme = if model.dark {
        DARK.get_or_init(llimphi_theme::Theme::dark)
    } else {
        LIGHT.get_or_init(llimphi_theme::Theme::light)
    };
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: model.viewport,
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del grabador. Archivo / Grabación / Ver / Ayuda.
/// Sin "Editar": la app no tiene campos de texto editables. Sólo entran
/// comandos que mapean a acciones reales (iniciar/detener grabación,
/// alternar tema, salir). Los ítems de grabación se agrisan según el
/// estado real (no se puede iniciar si ya graba, ni detener si está idle).
fn app_menu(model: &Model) -> AppMenu {
    let recording = matches!(model.state, RecState::Recording { .. });
    let stopping = matches!(model.state, RecState::Stopping);

    let mut iniciar = MenuItem::new("Iniciar grabación", "rec.start");
    if recording || stopping {
        iniciar = iniciar.disabled();
    }
    let mut detener = MenuItem::new("Detener grabación", "rec.stop");
    if !recording {
        detener = detener.disabled();
    }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q")),
        )
        .menu(
            Menu::new("Grabación")
                .item(iniciar)
                .item(detener),
        )
        .menu(
            Menu::new("Ver").item(MenuItem::new(
                if model.dark {
                    "Tema claro"
                } else {
                    "Tema oscuro"
                },
                "view.theme",
            )),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id del menú (principal o contextual) al `Msg`/efecto
/// real. Los ids de grabación despachan los mismos `Msg::Start`/`Msg::Stop`
/// que el botón Rec/Stop.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "rec.start" => handle.dispatch(Msg::Start),
        "rec.stop" => handle.dispatch(Msg::Stop),
        "view.theme" => handle.dispatch(Msg::ToggleTheme),
        "file.quit" => std::process::exit(0),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
    model
}

/// Menú contextual del grabador. Como la app no tiene campos de texto
/// editables, el contextual NO ofrece edición: mapea a los comandos de
/// grabación y tema reales (los mismos que botón y menú principal). Los
/// ítems no disponibles según el estado se omiten.
fn context_menu(model: &Model, x: f32, y: f32) -> View<Msg> {
    let recording = matches!(model.state, RecState::Recording { .. });
    let stopping = matches!(model.state, RecState::Stopping);

    // Construimos (label, command-id) según el estado real.
    let mut entries: Vec<(&str, &str)> = Vec::new();
    if recording {
        entries.push(("Detener grabación", "rec.stop"));
    } else if !stopping {
        entries.push(("Iniciar grabación", "rec.start"));
    }
    entries.push((
        if model.dark { "Tema claro" } else { "Tema oscuro" },
        "view.theme",
    ));

    let items: Vec<ContextMenuItem> = entries
        .iter()
        .map(|(label, _)| ContextMenuItem::action(*label))
        .collect();
    let ids: Vec<String> = entries.iter().map(|(_, id)| id.to_string()).collect();

    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        ids.get(i)
            .map(|id| Msg::MenuCommand(id.clone()))
            .unwrap_or(Msg::CloseMenus)
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: model.viewport,
        header: Some("grabador".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&app_theme(model)),
    })
}

/// Loop de grabación, en hilo de fondo. Devuelve el `Msg` que el bucle
/// Elm recibe al cerrar.
fn record_loop(rec: WebmRecorder, stop: Arc<AtomicBool>, path: PathBuf) -> Msg {
    let screen = match open_screen(FPS) {
        Ok(s) => s,
        Err(e) => return Msg::Finished(Err(e)),
    };

    // Micrófono opcional: sin él, grabación video-solo.
    let mic = MicSource::open_default().ok();
    let (a_sr, a_ch) = mic
        .as_ref()
        .map(|m| (m.sample_rate(), m.channels()))
        .unwrap_or((0, 0));

    let mut recorded_v = RecordedFrameSource::new(screen, rec.clone());
    let mut recorded_a = mic.map(|m| RecordedAudioSource::new(m, rec.clone()));

    let dt = Duration::from_micros(1_000_000 / FPS as u64);
    let mut vbuf = Vec::new();
    let mut abuf: Vec<f32> = Vec::new();

    // Cebar dimensiones (start() las exige).
    let prime_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if recorded_v.tick(dt, &mut vbuf).is_some() {
            break;
        }
        if Instant::now() >= prime_deadline {
            return Msg::Finished(Err("no llegaron frames de la pantalla".into()));
        }
        std::thread::sleep(dt / 2);
    }

    if let Err(e) = rec.start(&path) {
        return Msg::Finished(Err(format!("start: {e}")));
    }

    let mut last_audio = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let _ = recorded_v.tick(dt, &mut vbuf);
        if let Some(ra) = recorded_a.as_mut() {
            let frames = (a_sr as f64 * last_audio.elapsed().as_secs_f64()) as usize;
            if frames > 0 {
                abuf.clear();
                abuf.resize(frames * a_ch.max(1) as usize, 0.0);
                ra.fill(&mut abuf, a_sr, a_ch);
                last_audio = Instant::now();
            }
        }
        std::thread::sleep(dt / 2);
    }

    match rec.stop() {
        Ok((out, summary)) => Msg::Finished(Ok(RecLite {
            path: out.display().to_string(),
            frames: summary.video_frames,
            audio_packets: summary.audio_packets,
            kib: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) as f64 / 1024.0,
        })),
        Err(e) => Msg::Finished(Err(format!("stop: {e}"))),
    }
}

/// `mm:ss` desde segundos.
fn fmt_mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

#[inline]
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn main() {
    llimphi_ui::run::<RecorderApp>();
}
