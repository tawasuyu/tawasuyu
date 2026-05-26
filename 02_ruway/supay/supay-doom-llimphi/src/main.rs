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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, JustifyContent, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Color, Image, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use supay_core::{keys, DoomEngine, SnapshotPair, DOOM_HEIGHT, DOOM_PIXELS, DOOM_WIDTH};
use supay_render_llimphi::{scene_view, RenderConfig, WadAtlas};
use supay_wad::Wad;

// =====================================================================
// Paleta Supay — riffs sobre la identidad del Doom clásico:
// negro carbón de cabina, crimson de sangre BFG, ámbar de chasis, gris
// hueso del HUD. Pensada para verse igual sobre wayland o framebuffer.
// =====================================================================
const COLOR_BG_ABYSS: Color = Color::from_rgba8(0, 0, 0, 255);
const COLOR_BG_PANEL: Color = Color::from_rgba8(12, 8, 8, 255);
const COLOR_BG_SUNKEN: Color = Color::from_rgba8(6, 4, 4, 255);
const COLOR_CRIMSON: Color = Color::from_rgba8(180, 30, 30, 255);
const COLOR_CRIMSON_DEEP: Color = Color::from_rgba8(90, 14, 14, 255);
const COLOR_AMBER: Color = Color::from_rgba8(232, 168, 76, 255);
const COLOR_BONE: Color = Color::from_rgba8(216, 204, 188, 255);
const COLOR_DUST: Color = Color::from_rgba8(132, 124, 116, 255);
const COLOR_GREEN_CRT: Color = Color::from_rgba8(140, 188, 96, 255);
const COLOR_RULE: Color = Color::from_rgba8(48, 16, 16, 255);

const HEADER_HEIGHT: f32 = 44.0;
const FOOTER_HEIGHT: f32 = 24.0;
const FRAME_THICKNESS: f32 = 4.0;

const TICK_HZ: u64 = 35; // canónico de Doom
const TICK_MS: u64 = 1_000 / TICK_HZ;
const TICK_PERIOD: Duration = Duration::from_millis(TICK_MS);
/// Frame rate del renderer 3D — desacoplado del tick del motor. A 60 Hz
/// la interpolación entre snapshots se vuelve visible (cada tick de
/// 28.5 ms cubre ~1.7 frames de 16.6 ms, así el alpha barre 0→1 en pasos
/// de ~0.58). En Framebuffer mode el redraw también ayuda al smoothing
/// del scaling del FB.
const FRAME_MS: u64 = 1_000 / 60;

/// Modo de visualización. Toggleable con F3 — Fase 1 vs Fase 3.0.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Pinta el framebuffer 320×200 ARGB del motor original
    /// (renderer software clásico de Doom).
    Framebuffer,
    /// Renderer 3D moderno consumiendo snapshots interpolados
    /// (Fase 3.0 — supay-render-llimphi sobre vello).
    Scene3d,
}

struct Model {
    engine: DoomEngine,
    tick: u64,
    /// Framebuffer del último frame del motor, ya en formato Rgba8
    /// (lo que `View::image` espera).
    framebuffer_rgba: Vec<u8>,
    /// Fase 2: par de snapshots (prev + next) capturados desde
    /// doomgeneric — el renderer 3D (Fase 3) interpola entre los dos
    /// para correr más rápido que 35 Hz.
    snapshots: SnapshotPair,
    /// Instante real del último `Msg::Tick` — el renderer 3D lo usa
    /// para calcular `alpha = elapsed / TICK_PERIOD` al pintar.
    last_tick_at: Instant,
    view_mode: ViewMode,
    /// Fase 3.3: atlas WAD compartido con el renderer. `None` si el WAD
    /// no pudo cargarse (modo stub o doom1.wad ausente en cwd); en ese
    /// caso el renderer cae a las paletas hardcoded de 3.1.
    atlas: Option<Arc<WadAtlas>>,
    /// Conjunto de pic_idx ya registrados en el atlas. Cuando aparece
    /// un sector con un pic_idx nuevo, lo resolvemos vía
    /// `engine.flat_name` y lo añadimos.
    known_pics: std::collections::HashSet<u16>,
    /// Análogo para spritenums (Fase 3.4): cada vez que un mobj nuevo
    /// aparece, registramos su 4-char base name en el atlas.
    known_sprites: std::collections::HashSet<u16>,
}

#[derive(Clone)]
enum Msg {
    /// Tick del motor Doom (35 Hz). Avanza la simulación + captura
    /// snapshot.
    Tick,
    /// Redraw a 60 Hz para que el closure de paint_with recompute
    /// `alpha = elapsed / TICK_PERIOD` y la interpolación entre
    /// snapshots sea visible. No toca el model más allá del rebuild.
    Frame,
    Key(KeyEvent),
    ToggleViewMode,
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
        handle.spawn_periodic(Duration::from_millis(FRAME_MS), || Msg::Frame);
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
        // Cargamos el WAD desde el mismo path que pasamos al motor.
        // Si falla (no existe, mal formato), seguimos sin atlas — el
        // renderer cae a las paletas hardcoded de 3.1 sin romperse.
        let atlas = match Wad::open("doom1.wad") {
            Ok(wad) => Some(Arc::new(WadAtlas::new(wad, HashMap::new()))),
            Err(e) => {
                eprintln!("supay: WAD no cargado ({e}) — renderer 3D usará paletas fallback");
                None
            }
        };
        Model {
            engine: DoomEngine::new(args),
            tick: 0,
            framebuffer_rgba: vec![0; DOOM_PIXELS * 4],
            snapshots: SnapshotPair::new(),
            last_tick_at: Instant::now(),
            view_mode: ViewMode::Framebuffer,
            atlas,
            known_pics: std::collections::HashSet::new(),
            known_sprites: std::collections::HashSet::new(),
        }
    }

    fn on_key(_: &Model, e: &KeyEvent) -> Option<Msg> {
        if e.state == KeyState::Pressed {
            if matches!(&e.key, Key::Named(NamedKey::F12)) {
                // F12 cierra la ventana — Esc lo manejamos como KEY_ESCAPE del juego.
                return Some(Msg::Quit);
            }
            if matches!(&e.key, Key::Named(NamedKey::F3)) {
                // F3 alterna framebuffer ↔ renderer 3D (Fase 1 ↔ Fase 3.0).
                return Some(Msg::ToggleViewMode);
            }
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
                // Fase 2: snapshot tras cada tick.
                let snap = m.engine.capture_scene(m.tick);
                // Fase 3.3: registrar en el atlas cualquier pic_idx
                // nuevo que aparezca en sectores. WadAtlas usa interior
                // mutability — el Arc compartido con el renderer ve
                // los nombres nuevos sin necesidad de reconstruirlo.
                if let Some(atlas) = m.atlas.as_ref() {
                    for sec in snap.sectors.iter() {
                        for pic in [sec.floor_pic, sec.ceiling_pic] {
                            if m.known_pics.insert(pic) {
                                if let Some(name) = m.engine.flat_name(pic) {
                                    atlas.set_flat_name(pic, name);
                                }
                            }
                        }
                    }
                    for spr in snap.sprites.iter() {
                        if m.known_sprites.insert(spr.sprite) {
                            if let Some(name) = m.engine.sprite_name(spr.sprite) {
                                atlas.set_sprite_name(spr.sprite, name);
                            }
                        }
                    }
                }
                m.snapshots.push(snap);
                m.last_tick_at = Instant::now();
            }
            Msg::Frame => {
                // No-op a nivel de modelo. Existe sólo para que Llimphi
                // dispare un view rebuild + redraw y el closure de
                // paint_with recompute `alpha` desde Instant::now() —
                // así la interpolación entre snapshots se vuelve
                // visible aún cuando no haya entrada del usuario.
            }
            Msg::Key(e) => {
                if let Some(doom_key) = translate_key(&e.key, e.text.as_deref()) {
                    let pressed = e.state == KeyState::Pressed;
                    m.engine.push_key(pressed, doom_key);
                }
            }
            Msg::ToggleViewMode => {
                m.view_mode = match m.view_mode {
                    ViewMode::Framebuffer => ViewMode::Scene3d,
                    ViewMode::Scene3d => ViewMode::Framebuffer,
                };
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let header = header_bar(model);
        let body = match model.view_mode {
            ViewMode::Framebuffer => {
                if model.engine.real {
                    framed_pane(game_pane(model))
                } else {
                    stub_message_pane()
                }
            }
            ViewMode::Scene3d => framed_pane(scene_view(
                &model.snapshots,
                model.last_tick_at,
                TICK_PERIOD,
                RenderConfig {
                    atlas: model.atlas.clone(),
                    ..RenderConfig::default()
                },
            )),
        };
        let footer = footer_bar(model);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(COLOR_BG_ABYSS)
        .children(vec![header, body, footer])
    }
}

// =====================================================================
// Header — banda alta con el logo a la izquierda y un HUD a la derecha:
// pill de modo (REAL o STUB), tick monoespaciado, tag de vista, recuento
// de la escena. Look "BFG console".
// =====================================================================
fn header_bar(model: &Model) -> View<Msg> {
    let logo_text = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "SUPAY · DOOM".to_string(),
        22.0,
        COLOR_CRIMSON,
        Alignment::Start,
    );
    let logo_sub = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "PHASE 3.11 · LLIMPHI BUILD".to_string(),
        9.0,
        COLOR_AMBER,
        Alignment::Start,
    );
    let logo = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(18.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![logo_text, logo_sub]);

    let mode_id = if model.engine.real {
        "supay-mode-real"
    } else {
        "supay-mode-stub"
    };
    let mode_bg = if model.engine.real { COLOR_CRIMSON_DEEP } else { COLOR_BG_SUNKEN };
    let mode_fg = if model.engine.real { COLOR_AMBER } else { COLOR_DUST };
    let mode_pill = View::new(Style {
        size: Size {
            width: length(110.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(mode_bg)
    .radius(3.0)
    .text_aligned(
        rimay_localize::t(mode_id),
        11.0,
        mode_fg,
        Alignment::Center,
    );

    let scene = model
        .snapshots
        .next()
        .map(|s| {
            format!(
                "w={} sec={} spr={}",
                s.walls.len(),
                s.sectors.len(),
                s.sprites.len()
            )
        })
        .unwrap_or_else(|| "—".to_string());
    let view_tag = rimay_localize::t(match model.view_mode {
        ViewMode::Framebuffer => "supay-view-fb",
        ViewMode::Scene3d => "supay-view-3d",
    });
    let stats = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(18.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!(
            "TICK {:06}    {}    SCENE [{}]",
            model.tick, view_tag, scene
        ),
        11.0,
        COLOR_BONE,
        Alignment::End,
    );

    let right = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::End),
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![stats, mode_pill]);

    let inner = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_HEIGHT),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(COLOR_BG_PANEL)
    .children(vec![logo, right]);

    // Banda 1 px crimson como underline al header — separador del juego.
    let rule = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(COLOR_RULE);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_HEIGHT + 1.0),
        },
        ..Default::default()
    })
    .children(vec![inner, rule])
}

// =====================================================================
// Footer — banda baja con los controles, color CRT verde sobre negro.
// =====================================================================
fn footer_bar(model: &Model) -> View<Msg> {
    let id = if model.engine.real {
        "supay-controls-hint"
    } else {
        "supay-stub-controls-hint"
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(FOOTER_HEIGHT),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(COLOR_BG_SUNKEN)
    .text_aligned(
        rimay_localize::t(id),
        10.0,
        COLOR_GREEN_CRT,
        Alignment::Start,
    )
}

// =====================================================================
// Marco crimson — envuelve cualquier contenido (FB o renderer 3D) con
// un border de 4 px en rojo profundo. Truco "outer fill = border color,
// inner fill = real content".
// =====================================================================
fn framed_pane(content: View<Msg>) -> View<Msg> {
    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(COLOR_BG_ABYSS)
    .children(vec![content]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(FRAME_THICKNESS),
            right: length(FRAME_THICKNESS),
            top: length(FRAME_THICKNESS),
            bottom: length(FRAME_THICKNESS),
        },
        ..Default::default()
    })
    .fill(COLOR_CRIMSON_DEEP)
    .children(vec![inner])
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

// =====================================================================
// Stub screen — pantalla "no hay WAD" rediseñada como console-prompt:
// banner crimson arriba, pasos numerados con su comando en color CRT,
// pie con el porqué técnico del motor.
// =====================================================================
fn stub_message_pane() -> View<Msg> {
    let banner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(120.0_f32),
        },
        padding: Rect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(36.0_f32),
            bottom: length(0.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(44.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            "SUPAY · DOOM".to_string(),
            32.0,
            COLOR_CRIMSON,
            Alignment::Start,
        ),
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            rimay_localize::t("supay-stub-title"),
            13.0,
            COLOR_DUST,
            Alignment::Start,
        ),
    ]);

    let mut steps: Vec<View<Msg>> = Vec::new();
    for (n, (title_id, cmd_id)) in [
        ("supay-stub-step-1", "supay-stub-step-1-cmd"),
        ("supay-stub-step-2", "supay-stub-step-2-cmd"),
        ("supay-stub-step-3", "supay-stub-step-3-cmd"),
    ]
    .iter()
    .enumerate()
    {
        steps.push(stub_step((n as u32) + 1, title_id, cmd_id));
    }

    let body = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(8.0_f32),
            bottom: length(20.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .children(steps);

    let foot = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(36.0_f32),
        },
        padding: Rect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(0.0_f32),
            bottom: length(12.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t("supay-stub-footer"),
        11.0,
        COLOR_DUST,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(COLOR_BG_PANEL)
    .children(vec![banner, body, foot])
}

/// Un paso del setup: bullet ámbar con el número + título en hueso +
/// comando shell debajo en verde CRT sobre una caja sunken.
fn stub_step(n: u32, title_id: &'static str, cmd_id: &'static str) -> View<Msg> {
    let bullet = View::new(Style {
        size: Size {
            width: length(28.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{:02}", n),
        13.0,
        COLOR_AMBER,
        Alignment::Center,
    );

    let title = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(22.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t(title_id),
        14.0,
        COLOR_BONE,
        Alignment::Start,
    );

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![bullet, title]);

    let cmd = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
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
    .fill(COLOR_BG_SUNKEN)
    .radius(2.0)
    .text_aligned(
        rimay_localize::t(cmd_id),
        11.0,
        COLOR_GREEN_CRT,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![head, cmd])
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
    rimay_localize::init();
    llimphi_ui::run::<Supay>();
}
