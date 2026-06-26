//! Pantallazo headless de `takiy-app-llimphi` — el piano roll real.
//!
//! Monta la **view real** de la app (menubar arriba + canvas con
//! `paint_piano_roll`, el mismo painter del binario) sobre un
//! [`EditorState`] sembrado con una pieza creíble en La menor: cuatro
//! pistas (melodía / arpegio / bajo / acordes), ~70 notas, tonalidad
//! activa (filas en escala resaltadas), región de loop, automación de
//! volumen y pan en la pista activa, nota seleccionada, master delay +
//! reverb prendidos y el cursor de reproducción congelado en el beat 6.5
//! — nada depende de la hora actual.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p takiy-app-llimphi --example pantallazo_takiy --release -- [out.png]`
#![allow(dead_code)]

// El painter, los mensajes, el modelo y el cromo viven en módulos
// bin-only del crate: los incluimos por `#[path]` para montar exactamente
// la misma view que la app (menubar + toolbar + rails de dientes +
// paneles + canvas).
#[path = "../src/msg.rs"]
mod msg;
#[path = "../src/paint.rs"]
mod paint;
#[path = "../src/appmodel.rs"]
mod appmodel;
#[path = "../src/chrome.rs"]
mod chrome;
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/overview.rs"]
mod overview;
#[path = "../src/waveedit.rs"]
mod waveedit;
#[path = "../src/record.rs"]
mod record;

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_motion::Tween;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint as paint_view, DragPhase, PaintRect, View};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use takiy_app::{describe_key, pitch_range_with_offset, EditorState, Snap};
use takiy_core::{
    AutomationLane, DelayParams, Pitch, PitchClass, ReverbParams, Scale, Score, ScoreNote, Track,
    WaveLayer, WaveOp,
};

use crate::appmodel::Model;
use crate::chrome::DockItem;
use crate::msg::Msg;
use crate::paint::paint_piano_roll;

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Atajo: nota por clase + octava (panic si queda fuera del rango MIDI —
/// con datos demo fijos no puede pasar).
fn nota(class: PitchClass, octave: i32, beat: f32, dur: f32, vel: u8) -> ScoreNote {
    ScoreNote::new(
        Pitch::from_class_octave(class, octave).expect("pitch demo válido"),
        beat,
        dur,
        vel,
    )
}

/// Compone la pieza demo: 16 beats en La menor sobre la progresión
/// Am — F — C — G (un compás de 4/4 cada acorde). Cuatro pistas con roles
/// distintos para que la paleta por pista del painter se vea entera.
fn score_demo() -> Score {
    use PitchClass::*;
    let mut score = Score::new(112.0);
    score.key = Some(Scale::natural_minor(A));
    score.master_delay = Some(DelayParams::default());
    score.master_reverb = Some(ReverbParams { room_size: 0.85, ..Default::default() });

    // — melodía (pista 0, activa): frase cantábile con corcheas y blancas.
    let mut melodia = Track::new("melodía");
    melodia.volume = 0.95;
    melodia.pan = -0.2;
    // La automación de la pista activa se pinta como overlay: una curva
    // de volumen (naranja) que crece hacia el clímax del compás 3 y una
    // de pan (cyan) que pasea el lead de izquierda a derecha.
    melodia.volume_automation = Some(AutomationLane::default());
    if let Some(lane) = melodia.volume_automation.as_mut() {
        for (b, v) in [(0.0, 0.55), (4.0, 0.8), (8.0, 1.15), (12.0, 0.9), (16.0, 0.65)] {
            lane.add_point(b, v);
        }
    }
    melodia.pan_automation = Some(AutomationLane::default());
    if let Some(lane) = melodia.pan_automation.as_mut() {
        for (b, v) in [(0.0, -0.45), (6.0, 0.1), (10.0, 0.4), (16.0, -0.1)] {
            lane.add_point(b, v);
        }
    }
    for n in [
        // compás 1 (Am)
        nota(A, 4, 0.0, 1.0, 104),
        nota(C, 5, 1.0, 0.5, 96),
        nota(B, 4, 1.5, 0.5, 88),
        nota(A, 4, 2.0, 1.0, 100),
        nota(E, 5, 3.0, 1.0, 112),
        // compás 2 (F)
        nota(F, 4, 4.0, 0.5, 84),
        nota(A, 4, 4.5, 0.5, 90),
        nota(C, 5, 5.0, 1.0, 102),
        nota(A, 4, 6.0, 1.0, 92),
        nota(F, 4, 7.0, 1.0, 86),
        // compás 3 (C) — clímax
        nota(E, 4, 8.0, 1.0, 90),
        nota(G, 4, 9.0, 0.5, 96),
        nota(C, 5, 9.5, 0.5, 104),
        nota(E, 5, 10.0, 1.5, 118), // ← nota seleccionada en el pantallazo
        nota(D, 5, 11.5, 0.5, 98),
        // compás 4 (G) — resolución
        nota(D, 5, 12.0, 1.0, 100),
        nota(B, 4, 13.0, 1.0, 94),
        nota(G, 4, 14.0, 0.75, 88),
        nota(A, 4, 15.0, 1.0, 106),
    ] {
        melodia.add(n);
    }
    score.add_track(melodia);

    // — arpegio (pista 1): corcheas continuas dibujando cada acorde, una
    //   octava por encima de la melodía para que cada pista tenga su
    //   registro propio y ninguna tape a otra en el grid.
    let mut arpegio = Track::new("arpegio");
    arpegio.volume = 0.7;
    arpegio.pan = 0.3;
    let compases: [[(PitchClass, i32); 4]; 4] = [
        [(A, 5), (C, 6), (E, 6), (C, 6)], // Am
        [(F, 5), (A, 5), (C, 6), (A, 5)], // F
        [(G, 5), (C, 6), (E, 6), (C, 6)], // C
        [(G, 5), (B, 5), (D, 6), (B, 5)], // G
    ];
    for (bar, notas) in compases.iter().enumerate() {
        for rep in 0..2 {
            for (i, (pc, oct)) in notas.iter().enumerate() {
                let beat = bar as f32 * 4.0 + rep as f32 * 2.0 + i as f32 * 0.5;
                let vel = if i == 0 { 88 } else { 72 };
                arpegio.add(nota(*pc, *oct, beat, 0.45, vel));
            }
        }
    }
    score.add_track(arpegio);

    // — bajo (pista 2): fundamentales y quintas en blancas.
    let mut bajo = Track::new("bajo");
    bajo.volume = 1.0;
    for (pc, oct, beat) in [
        (A, 2, 0.0), (E, 2, 2.0),  // Am
        (F, 2, 4.0), (C, 3, 6.0),  // F
        (C, 2, 8.0), (G, 2, 10.0), // C
        (G, 2, 12.0), (D, 3, 14.0), // G
    ] {
        bajo.add(nota(pc, oct, beat, 2.0, 110));
    }
    score.add_track(bajo);

    // — acordes (pista 3): tríadas en redondas que rellenan la armonía,
    //   voicings por debajo de la melodía (tope en E4 < F4 melódico).
    let mut acordes = Track::new("acordes");
    acordes.volume = 0.55;
    acordes.pan = 0.1;
    let triadas: [[(PitchClass, i32); 3]; 4] = [
        [(A, 3), (C, 4), (E, 4)], // Am
        [(F, 3), (A, 3), (C, 4)], // F
        [(C, 3), (E, 3), (G, 3)], // C
        [(G, 3), (B, 3), (D, 4)], // G
    ];
    for (bar, triada) in triadas.iter().enumerate() {
        for (pc, oct) in triada {
            acordes.add(nota(*pc, *oct, bar as f32 * 4.0, 4.0, 64));
        }
    }
    score.add_track(acordes);

    score
}

/// Construye el `EditorState` demo: el mismo estado que tendría la app a
/// mitad de una sesión de edición (loop armado, metrónomo, snap fino,
/// snap-a-tonalidad, varias ediciones en el undo stack, nota seleccionada).
fn editor_demo() -> EditorState {
    let score = score_demo();
    let mut editor = EditorState::with_score(score);
    editor.set_loop_region(Some((4.0, 12.0)));
    editor.toggle_metronome(); // on · 4/4
    editor.snap = Snap::Eighth;
    editor.snap_to_key = true;
    // Profundidad de undo creíble: como si hubiera 7 ediciones previas.
    for _ in 0..7 {
        editor.history.push(editor.score.clone());
    }
    // Selección: el clímax de la melodía (E5 en el beat 10).
    let sel_idx = editor
        .score
        .track(0)
        .map(|t| {
            t.notes()
                .iter()
                .position(|n| (n.start - 10.0).abs() < 1e-3 && n.pitch.midi() == 76)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    editor.selected = Some((0, sel_idx));
    editor
}

/// El menú principal del piano roll — calco de `app_menu()` en src/main.rs
/// (cerrado en el pantallazo, así que sólo se ven los rótulos).
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Guardar", "file.save").shortcut("S"))
                .item(MenuItem::new("Exportar MIDI…", "file.export_midi").shortcut("Ctrl+E"))
                .item(MenuItem::new("Exportar WAV…", "file.export_wav").shortcut("Ctrl+R"))
                .item(MenuItem::new("Salir", "file.quit").shortcut("Esc").separated()),
        )
        .menu(
            Menu::new("Editar")
                .item(MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z"))
                .item(MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y"))
                .item(MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C").separated())
                .item(MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X"))
                .item(MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V"))
                .item(MenuItem::new("Duplicar", "edit.duplicate").shortcut("Ctrl+D"))
                .item(MenuItem::new("Borrar selección", "edit.delete").shortcut("Del").separated())
                .item(MenuItem::new("Pista nueva", "edit.new_track").shortcut("N"))
                .item(MenuItem::new("Ciclar pista", "edit.cycle_track").shortcut("Tab"))
                .item(MenuItem::new("Borrar pista activa", "edit.delete_track")),
        )
        .menu(
            Menu::new("Reproducción")
                .item(MenuItem::new("Tocar / Detener", "play.toggle").shortcut("Space"))
                .item(MenuItem::new("Tocar con count-in", "play.countin").shortcut("Ctrl+Space"))
                .item(MenuItem::new("Metrónomo", "play.metronome").shortcut("M").separated())
                .item(MenuItem::new("Loop", "play.loop").shortcut("L")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Ciclar snap", "view.snap").shortcut("Q"))
                .item(MenuItem::new("Snap a tonalidad", "view.snap_key").shortcut("Alt+K")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Construye el `Model` real de la app (sin Player ni SF2 — el pantallazo
/// no abre device de audio) con el mixer abierto a la izquierda y los
/// efectos a la derecha, para que se vean los dos sidebars de dientes.
fn model_demo(theme: Theme) -> Model {
    Model {
        editor: editor_demo(),
        source: "demo built-in".to_string(),
        theme,
        player: None,
        sf2: None,
        engine: "engine osc".to_string(),
        playing: true,
        status: "Space = play · device 48000 Hz / 2 ch".to_string(),
        playback_bpm: 112.0,
        last_rect: None,
        drag: None,
        auto_pending: None,
        midi_offset: 0,
        last_audition_at: None,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        context_menu: None,
        left_active: Some(DockItem::Pistas),
        right_active: Some(DockItem::Efectos),
        left_w: chrome::DEFAULT_PANEL_W,
        right_w: chrome::DEFAULT_PANEL_W,
        screen: appmodel::Screen::Track,
        onda_peaks: std::collections::HashMap::new(),
        wave_sel: None,
        recording: None,
    }
}

/// Misma composición que `Takiy::view()` (src/main.rs): menubar + toolbar +
/// cuerpo (rail izq · panel · canvas · panel · rail der), con los mismos
/// builders del cromo. Los handlers de click/drag se omiten — acá nadie
/// clickea.
fn view_demo(model: &Model, theme: Theme) -> View<Msg> {
    let score = model.editor.score.clone();
    let source = model.source.clone();
    let engine = model.engine.clone();
    let status = model.status.clone();
    let playing = model.playing;
    let active_track = model.editor.active_track;
    let selected = model.editor.selected;
    let playback_bpm = model.playback_bpm;
    // Cursor congelado en el beat 6.5 (mitad del loop) — determinista.
    let playback_position_seconds = Some(6.5 * 60.0 / playback_bpm);
    let loop_region = model.editor.loop_region;
    let metronome_on = model.editor.metronome_beats_per_bar.is_some();
    let snap_label = model.editor.snap.label();
    let undo_depth = model.editor.history.len();
    let key_label = describe_key(&model.editor.score.key);
    let key_scale = model.editor.score.key.clone();
    let snap_to_key = model.editor.snap_to_key;
    let (min_midi, max_midi) = pitch_range_with_offset(&score, 0);
    let total_beats = score
        .duration_beats()
        .max(8.0)
        .max(loop_region.map(|(_, t)| t).unwrap_or(0.0));

    let menu = app_menu();
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });

    let toolbar = chrome::toolbar_bar(model, &theme);

    let canvas = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .paint_with(move |scene, ts, rect: PaintRect| {
        paint_piano_roll(
            scene, ts, rect, &score, &source, &engine, &status, playing,
            active_track, selected, playback_position_seconds, playback_bpm,
            loop_region, metronome_on, snap_label, undo_depth,
            &key_label, key_scale.as_ref(), snap_to_key,
            min_midi, max_midi, total_beats, theme,
        );
    });

    let sp = SplitterPalette::from_theme(&theme);
    let mut core = canvas;
    if let Some(rp) = chrome::panel(chrome::DockSide::Right, model, &theme) {
        core = splitter_two(
            Direction::Row,
            core,
            PaneSize::Flex,
            rp,
            PaneSize::Fixed(model.right_w),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetDockWidth(chrome::DockSide::Right, dx)),
                DragPhase::End => None,
            },
            &sp,
        );
    }
    if let Some(lp) = chrome::panel(chrome::DockSide::Left, model, &theme) {
        core = splitter_two(
            Direction::Row,
            lp,
            PaneSize::Fixed(model.left_w),
            core,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetDockWidth(chrome::DockSide::Left, dx)),
                DragPhase::End => None,
            },
            &sp,
        );
    }

    let center = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![core]);

    let body = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(0.0_f32) },
        min_size: Size { width: length(0.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        chrome::rail(chrome::DockSide::Left, model, &theme),
        center,
        chrome::rail(chrome::DockSide::Right, model, &theme),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, toolbar, body])
}

/// Panorama de pistas (tipo Audacity) montado con el `overview::body`
/// real: menubar + toolbar + carriles. Pone dos pistas en modo onda
/// (con sus picos calculados) y dos en midi para mostrar ambos carriles.
fn overview_view(model: &Model, theme: Theme) -> View<Msg> {
    let menu = app_menu();
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });
    let toolbar = chrome::toolbar_bar(model, &theme);
    let body = overview::body(model, &theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, toolbar, body])
}

/// Editor de onda montado con el `waveedit::body` real: menubar +
/// toolbar (con «‹ pistas») + barra de ops + forma de onda con una
/// selección y ediciones aplicadas (silencio + fade out).
fn waveedit_view(model: &Model, theme: Theme) -> View<Msg> {
    let menu = app_menu();
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });
    let toolbar = chrome::toolbar_bar(model, &theme);
    let body = waveedit::body(model, &theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, toolbar, body])
}

/// Modo grabación montado con `record::body` real: HUD + teclado de
/// piano con un acorde (C E G) "apretado" para mostrar el realce.
fn record_view(model: &Model, theme: Theme) -> View<Msg> {
    let menu = app_menu();
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });
    let toolbar = chrome::toolbar_bar(model, &theme);
    let body = record::body(model, &theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, toolbar, body])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/takiy.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let theme = Theme::dark(); // el theme canónico de la app (src/main.rs)
    let mut model = model_demo(theme);
    // `TAKIY_RECORD=1` rinde el modo grabación (teclado-piano con acorde).
    let root = if std::env::var_os("TAKIY_RECORD").is_some() {
        model.screen = appmodel::Screen::Track;
        let mut held = std::collections::HashMap::new();
        held.insert(60u8, 0.0_f32); // C4
        held.insert(64u8, 0.0_f32); // E4
        held.insert(67u8, 0.0_f32); // G4
        model.recording = Some(appmodel::RecState {
            track: 0,
            started_at: std::time::Instant::now(),
            bpm: 112.0,
            backing: true,
            base_octave: 4,
            held,
            count: 7,
            last_beat: 3.5,
        });
        record_view(&model, theme)
    } else if std::env::var_os("TAKIY_WAVE").is_some() {
        model.screen = appmodel::Screen::Track;
        model.editor.active_track = 0;
        if let Some(t) = model.editor.score.track_mut(0) {
            t.view = takiy_core::TrackView::Onda;
            // Silenciar [4,6) + fade out al final, para que la edición se vea.
            t.wave = Some(WaveLayer {
                ops: vec![
                    WaveOp::Silence { from: 4.0, to: 6.0 },
                    WaveOp::FadeOut { from: 12.0, to: 16.0 },
                ],
            });
        }
        let peaks = overview::compute_onda_peaks(&model.editor.score, 0);
        model.onda_peaks.insert(0, peaks);
        model.wave_sel = Some((4.0, 6.0));
        waveedit_view(&model, theme)
    } else if std::env::var_os("TAKIY_OVERVIEW").is_some() {
        model.screen = appmodel::Screen::Overview;
        // Pistas pares en onda (con picos), impares en midi.
        let n = model.editor.score.tracks().len();
        for i in 0..n {
            if i % 2 == 0 {
                if let Some(t) = model.editor.score.track_mut(i) {
                    t.view = takiy_core::TrackView::Onda;
                }
                let peaks = overview::compute_onda_peaks(&model.editor.score, i);
                model.onda_peaks.insert(i, peaks);
            }
        }
        overview_view(&model, theme)
    } else {
        view_demo(&model, theme)
    };

    // view → layout → scene (misma secuencia que el eventloop real).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint_view(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-takiy"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_takiy: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
