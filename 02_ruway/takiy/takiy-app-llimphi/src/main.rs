// En release sobre Windows: subsistema GUI (sin consola negra detrás).
// No-op en Linux/otros targets — preserva `cargo check --workspace`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! `takiy-app-llimphi` — piano roll visor + reproductor sobre Llimphi.
//!
//! Carga un `Score` (built-in o desde `TAKIY_SCORE_JSON`), lo pinta como
//! grid pitch×beats y reproduce con Space. La síntesis es osciladores
//! (`takiy-synth::OscRenderer`) o SF2 (`MultiProgramRenderer` si
//! `TAKIY_SF2` apunta a un soundfont); el audio sale por el device
//! default (`takiy-playback::Player` sobre cpal).
//!
//! La lógica editable (Score + selección + pista activa) vive en
//! [`takiy_app::EditorState`] — testeada headless en `examples/smoke.rs`.
//! Acá quedan sólo el bridge Llimphi y la integración con el `Player`,
//! repartidos en módulos bin-only: `msg` (Msg + drag + hit-test),
//! `appmodel` (Model), `audio` (sf2/render/play), `paint` (piano roll) y
//! `update` (actualizar + audition). Este archivo deja el `impl App` y el
//! ruteo de teclado/rueda.
//!
//! Controles:
//!
//! - `Space`      — toca / detiene el score.
//! - `Ctrl+E`     — exporta el score actual a SMF (.mid).
//! - `Ctrl+R`     — render offline del score actual a WAV (44100 Hz / estéreo /
//!                  16-bit PCM) ignorando metrónomo y count-in.
//! - `Tab`        — cicla la pista activa.
//! - `N`          — crea una pista nueva y la activa.
//! - Click izq.   — agrega una nota (o selecciona la existente bajo el cursor).
//! - Drag izq.    — mueve / redimensiona la nota, o mueve un punto de automación.
//! - Click der.   — borra la nota / dot bajo el cursor.
//! - Wheel        — desplaza la ventana vertical de pitches en semitonos.
//! - `Alt+D/R/V/P/C/K` — delay / reverb / automación de volumen·pan / limpiar / snap-key.
//! - `←/→`, `↑/↓` — mueve la nota seleccionada ±1 beat / ±1 semitono.
//! - `+/-`, `[/]` — alarga·acorta / velocity de la nota seleccionada.
//! - `Del`/`⌫`    — borra la nota seleccionada. `Ctrl+⌫` borra la pista activa.
//! - `S` guarda · `,/.` tempo · `p/P` programa GM · `Esc` cierra.

mod appmodel;
mod audio;
mod chrome;
mod msg;
mod overview;
mod paint;
mod proyecto;
mod record;
mod update;
mod waveedit;

use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, PaintRect, View,
    WheelDelta,
};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_overlay_animated, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::Tween;
use llimphi_theme::motion;
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_widget_toast::toast_stack_view;
use takiy_app::{describe_key, load_score_or_demo, pitch_range_with_offset, EditMsg};
use takiy_playback::Player;

use crate::appmodel::{Model, Screen};
use crate::audio::load_sf2;
use crate::msg::Msg;
use crate::paint::paint_piano_roll;
use crate::update::{actualizar, build_editor};

struct Takiy;

impl App for Takiy {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "takiy · piano roll (llimphi)"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 640)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let (score, source) = load_score_or_demo();
        let editor = build_editor(score);
        eprintln!(
            "takiy · cargado {source} ({} pistas, {:.1} beats)",
            editor.score.tracks().len(),
            editor.score.duration_beats()
        );

        let (player, status) = match Player::open() {
            Ok(p) => {
                let s = format!(
                    "Space = play · device {} Hz / {} ch",
                    p.sample_rate(),
                    p.channels()
                );
                eprintln!("takiy · {s}");
                (Some(p), s)
            }
            Err(e) => {
                eprintln!("takiy · sin audio: {e}");
                (None, format!("sin audio: {e}"))
            }
        };

        let target_sr = player.as_ref().map(Player::sample_rate).unwrap_or(44_100);
        let (sf2, engine) = load_sf2(&editor.score, target_sr);

        // Tick periódico ~20 Hz. Sirve para repintar el cursor de
        // reproducción y detectar fin de buffer sin tocar el callback.
        handle.spawn_periodic(std::time::Duration::from_millis(50), || Msg::Tick);

        let mut editor = editor;
        editor.save_path = std::env::var_os("TAKIY_SCORE_JSON").map(std::path::PathBuf::from);

        // Proyecto inicial: envuelve el score cargado en un proyecto
        // versionado (DAG). Más proyectos se abren con «＋» del rail.
        let proy_dir = std::env::var_os("TAKIY_PROJ_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("takiy-proyectos"));
        let proyecto0 = takiy_proyecto::Proyecto::nuevo(source.clone(), editor.score.clone());

        Model {
            editor,
            source,
            theme: Theme::dark(),
            player,
            sf2,
            engine,
            playing: false,
            status,
            playback_bpm: 120.0,
            last_rect: None,
            drag: None,
            auto_pending: None,
            midi_offset: 0,
            last_audition_at: None,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
            // El mixer abierto de arranque: el usuario ve sus pistas y los
            // controles de transporte sin tener que descubrir un atajo.
            left_active: Some(crate::chrome::DockItem::Pistas),
            right_active: None,
            left_w: crate::chrome::DEFAULT_PANEL_W,
            right_w: crate::chrome::DEFAULT_PANEL_W,
            // El proyecto se abre en el panorama de pistas (tipo Audacity);
            // clickear un carril entra al piano roll de esa pista.
            screen: Screen::Overview,
            onda_peaks: std::collections::HashMap::new(),
            wave_sel: None,
            recording: None,
            proyectos: vec![proyecto0],
            proy_activo: 0,
            proy_dir,
            ver_versiones: true,
            ver_pistas: true,
            toasts: Vec::new(),
            next_toast: 0,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        actualizar(model, msg, handle)
    }

    fn on_wheel(
        _model: &Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Msg> {
        if modifiers.ctrl || modifiers.alt || modifiers.shift {
            return None;
        }
        // `delta.y` viene normalizado a "líneas" (positivo arriba). Lo
        // proyectamos directo a semitonos: una "línea" de rueda mueve
        // un semitono. Si una rueda física pisa más de un escalón, ya
        // viene multiplicada por llimphi-ui.
        let steps = delta.y.round() as i32;
        if steps == 0 {
            return None;
        }
        Some(Msg::ScrollMidi { delta: steps })
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        // Modo grabación: el teclado es un piano. Intercepta TODO (incluido
        // key-up, que el resto de la app ignora) antes de los atajos.
        if model.recording.is_some() {
            return record_key(model, event);
        }
        if event.state != KeyState::Pressed {
            return None;
        }
        let allow_repeat = matches!(
            &event.key,
            Key::Named(
                NamedKey::ArrowLeft
                    | NamedKey::ArrowRight
                    | NamedKey::ArrowUp
                    | NamedKey::ArrowDown
                    | NamedKey::Delete
                    | NamedKey::Backspace
            )
        );
        if event.repeat && !allow_repeat {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu().menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        match &event.key {
            Key::Named(NamedKey::Space) if event.modifiers.ctrl => Some(Msg::PlayWithCountIn),
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            Key::Named(NamedKey::Tab) => Some(Msg::Edit(EditMsg::CycleTrack)),
            // Esc: si hay un menú abierto lo cierra; si no, sale.
            Key::Named(NamedKey::Escape)
                if model.menu_open.is_some() || model.context_menu.is_some() =>
            {
                Some(Msg::CloseMenus)
            }
            // Esc: en el editor de una pista vuelve al panorama; en el
            // panorama sale de la app.
            Key::Named(NamedKey::Escape) if model.screen == Screen::Track => {
                Some(Msg::OpenOverview)
            }
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            Key::Named(NamedKey::ArrowLeft) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: -1.0, d_semitones: 0 }))
            }
            Key::Named(NamedKey::ArrowRight) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: 1.0, d_semitones: 0 }))
            }
            Key::Named(NamedKey::ArrowUp) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: 1 }))
            }
            Key::Named(NamedKey::ArrowDown) => {
                Some(Msg::Edit(EditMsg::MoveSelected { d_beat: 0.0, d_semitones: -1 }))
            }
            Key::Named(NamedKey::Backspace) if event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::DeleteActiveTrack))
            }
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => {
                Some(Msg::Edit(EditMsg::DeleteSelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("n") => Some(Msg::Edit(EditMsg::NewTrack)),
            // Mixer per-track (F3.a): Alt+M/S/[/] manejan la pista activa.
            // Vienen ANTES de los handlers sin modifiers para que las
            // versiones con Alt no caigan en metrónomo o velocity.
            Key::Character(s) if s.eq_ignore_ascii_case("m") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleMuteActive))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("s") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleSoloActive))
            }
            Key::Character(s) if s == "D" && event.modifiers.alt && event.modifiers.shift => {
                Some(Msg::Edit(EditMsg::CycleMasterDelayTime))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("d") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleMasterDelay))
            }
            Key::Character(s) if s == "R" && event.modifiers.alt && event.modifiers.shift => {
                Some(Msg::Edit(EditMsg::CycleMasterReverbRoom))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("r") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleMasterReverb))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("v") && event.modifiers.alt => {
                Some(Msg::AnchorVolumeAutomation)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("p") && event.modifiers.alt => {
                Some(Msg::AnchorPanAutomation)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("c") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ClearActiveAutomation))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("k") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::ToggleSnapToKey))
            }
            Key::Character(s) if (s == "[" || s == "{") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActiveVolume { delta: -0.1 }))
            }
            Key::Character(s) if (s == "]" || s == "}") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActiveVolume { delta: 0.1 }))
            }
            Key::Character(s) if (s == "," || s == "<") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActivePan { delta: -0.1 }))
            }
            Key::Character(s) if (s == "." || s == ">") && event.modifiers.alt => {
                Some(Msg::Edit(EditMsg::NudgeActivePan { delta: 0.1 }))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("m") => Some(Msg::ToggleMetronome),
            Key::Character(s) if s.eq_ignore_ascii_case("l") => Some(Msg::ToggleLoop),
            Key::Character(s) if s.eq_ignore_ascii_case("q") => Some(Msg::CycleSnap),
            Key::Character(s) if s == "k" => Some(Msg::Edit(EditMsg::CycleKeyRoot)),
            Key::Character(s) if s == "K" => Some(Msg::Edit(EditMsg::CycleKeyMode)),
            Key::Character(s) if s.eq_ignore_ascii_case("z") && event.modifiers.ctrl && event.modifiers.shift => {
                Some(Msg::Redo)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("z") && event.modifiers.ctrl => Some(Msg::Undo),
            Key::Character(s) if s.eq_ignore_ascii_case("y") && event.modifiers.ctrl => Some(Msg::Redo),
            Key::Character(s) if s.eq_ignore_ascii_case("c") && event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::CopySelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("x") && event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::CutSelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("v") && event.modifiers.ctrl => {
                // Paste al beat 0; el playhead-aware paste se agrega
                // cuando expongamos position_beats al on_key handler.
                Some(Msg::PasteAtPlayhead)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("d") && event.modifiers.ctrl => {
                Some(Msg::Edit(EditMsg::DuplicateSelected))
            }
            Key::Character(s) if s.eq_ignore_ascii_case("e") && event.modifiers.ctrl => {
                Some(Msg::ExportMidi)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("r") && event.modifiers.ctrl => {
                Some(Msg::ExportWav)
            }
            Key::Character(s) if s.eq_ignore_ascii_case("s") => Some(Msg::Save),
            Key::Character(s) if s == "+" || s == "=" => {
                Some(Msg::Edit(EditMsg::ResizeSelected { d_beat: 0.5 }))
            }
            Key::Character(s) if s == "-" || s == "_" => {
                Some(Msg::Edit(EditMsg::ResizeSelected { d_beat: -0.5 }))
            }
            Key::Character(s) if s == "[" || s == "{" => {
                Some(Msg::Edit(EditMsg::NudgeVelocity { delta: -10 }))
            }
            Key::Character(s) if s == "]" || s == "}" => {
                Some(Msg::Edit(EditMsg::NudgeVelocity { delta: 10 }))
            }
            Key::Character(s) if s == "," => Some(Msg::Edit(EditMsg::NudgeTempo { delta: -5.0 })),
            Key::Character(s) if s == "." => Some(Msg::Edit(EditMsg::NudgeTempo { delta: 5.0 })),
            Key::Character(s) if s == "p" => Some(Msg::NudgeProgram { delta: -1 }),
            Key::Character(s) if s == "P" => Some(Msg::NudgeProgram { delta: 1 }),
            _ => None,
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let sk = scene_key(model);
        let score = model.editor.score.clone();
        let source = model.source.clone();
        let engine = model.engine.clone();
        let status = model.status.clone();
        let playing = model.playing;
        let active_track = model.editor.active_track;
        let selected = model.editor.selected;
        let playback_position_seconds = model
            .player
            .as_ref()
            .filter(|_| playing)
            .map(|p| p.position_seconds());
        let playback_bpm = model.playback_bpm;
        let loop_region = model.editor.loop_region;
        let metronome_on = model.editor.metronome_beats_per_bar.is_some();
        let snap_label = model.editor.snap.label();
        let undo_depth = model.editor.history.len();
        let key_label = describe_key(&model.editor.score.key);
        let key_scale = model.editor.score.key.clone();
        let snap_to_key = model.editor.snap_to_key;
        let (min_midi, max_midi) = pitch_range_with_offset(&score, model.midi_offset);
        let total_beats = score
            .duration_beats()
            .max(8.0)
            .max(loop_region.map(|(_, t)| t).unwrap_or(0.0));

        let score_paint = score;

        // Barra de menú principal arriba; el piano roll ocupa el resto.
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model));
        // Barra de herramientas bajo el menú (compartida por panorama y
        // editor; en el editor lleva además el botón «‹ pistas»).
        let toolbar = chrome::toolbar_bar(model, &theme);

        // Panorama de pistas (tipo Audacity): la pantalla con la que se
        // abre el proyecto. El piano roll queda detrás de un click.
        if matches!(model.screen, Screen::Overview) {
            let body = scene_body(overview::body(model, &theme), sk);
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .fill(theme.bg_app)
            .children(vec![menubar, toolbar, body]);
        }

        // Modo grabación: teclado-piano que graba MIDI (tiene prioridad
        // sobre el editor de la pista).
        if model.recording.is_some() {
            let body = scene_body(record::body(model, &theme), sk);
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .fill(theme.bg_app)
            .children(vec![menubar, toolbar, body]);
        }

        // Editor de la pista abierta: si está en modo onda, el editor de
        // forma de onda; si no, el piano roll de siempre.
        let active_is_onda = model
            .editor
            .score
            .track(active_track)
            .map(|t| matches!(t.view, takiy_core::TrackView::Onda))
            .unwrap_or(false);
        if active_is_onda {
            let body = scene_body(waveedit::body(model, &theme), sk);
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .fill(theme.bg_app)
            .children(vec![menubar, toolbar, body]);
        }

        let canvas = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(theme.bg_app)
        // El press se resuelve en `update()` para que el drag posterior
        // tenga `(rw, rh)` cacheado en el modelo — `draggable_at` no recibe
        // el rect del nodo, sólo (lx0, ly0) y los deltas.
        .on_click_at(|lx, ly, rw, rh| Some(Msg::PressAt { lx, ly, rw, rh }))
        .draggable_at(|phase, dx, dy, lx0, ly0| {
            Some(Msg::DragNote { phase, dx, dy, lx0, ly0 })
        })
        // Right-click: el handler de update decide si borra el objeto
        // bajo el cursor (nota/dot) o abre el menú contextual sobre la
        // selección. El offset MENU_H lleva la coord local del canvas a
        // coord de ventana para anclar el overlay.
        .on_right_click_at(|lx, ly, rw, rh| Some(Msg::RightPressAt { lx, ly, rw, rh }))
        .paint_with(move |scene, ts, rect: PaintRect| {
            paint_piano_roll(
                scene, ts, rect, &score_paint, &source, &engine, &status, playing,
                active_track, selected, playback_position_seconds, playback_bpm,
                loop_region, metronome_on, snap_label, undo_depth,
                &key_label, key_scale.as_ref(), snap_to_key,
                min_midi, max_midi, total_beats, theme,
            );
        });

        // Centro = canvas con los paneles de los sidebars en panes
        // resizables (mismo patrón que cosmos: panel del item activo como
        // pane al costado, divisor arrastrable). El rail va aparte, como
        // columna acoplada a cada borde.
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
        // Sidebar de proyecto (siempre presente en el piano roll):
        // Versiones + Pistas, con su divisor arrastrable.
        core = splitter_two(
            Direction::Row,
            proyecto::panel(model, &theme),
            PaneSize::Fixed(model.left_w),
            core,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetDockWidth(chrome::DockSide::Left, dx)),
                DragPhase::End => None,
            },
            &sp,
        );

        // El core crece para ocupar el espacio entre los dos rails.
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
            proyecto::rail(model, &theme),
            center,
            chrome::rail(chrome::DockSide::Right, model, &theme),
        ]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menubar, toolbar, scene_body(body, sk)])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // Capa de menús: prioridad al contextual de la nota seleccionada;
        // si no, el dropdown del menú principal.
        let menu_layer = if let Some((x, y)) = model.context_menu {
            Some(context_menu_for_selection(model, x, y))
        } else {
            let menu = app_menu();
            menubar_overlay_animated(
                &menubar_spec(&menu, model),
                model.menu_active,
                model.menu_anim.value(),
            )
        };

        // Capa de toasts (export/guardado/errores): vive sobre cualquier
        // pantalla, filtrando los expirados.
        let now = std::time::Instant::now();
        let alive: Vec<_> = model.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
        let toast_layer = (!alive.is_empty())
            .then(|| toast_stack_view(&alive, viewport_of(model), Msg::ToastExpire));

        match (menu_layer, toast_layer) {
            (None, None) => None,
            (Some(m), None) => Some(m),
            (None, Some(t)) => Some(t),
            (Some(m), Some(t)) => Some(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![m, t]),
            ),
        }
    }
}

/// `key` estable de la escena actual: grabación / panorama / editor de
/// onda / piano roll, por proyecto. Cambia sólo al pasar de una escena a
/// otra (incluido cambiar de proyecto) → dispara la transición de entrada
/// del cuerpo. Deliberadamente NO incluye la pista activa: ciclar pistas
/// con Tab dentro del piano roll no es un cambio de escena (sería un
/// slide en cada Tab), pero pasar de una pista midi a una onda sí lo es
/// porque cambia el editor —y eso ya lo capta el discriminante.
fn scene_key(model: &Model) -> u64 {
    let disc: u64 = if model.recording.is_some() {
        0
    } else if matches!(model.screen, Screen::Overview) {
        1
    } else {
        let onda = model
            .editor
            .score
            .track(model.editor.active_track)
            .map(|t| matches!(t.view, takiy_core::TrackView::Onda))
            .unwrap_or(false);
        if onda { 2 } else { 3 }
    };
    disc * 1_000_000 + model.proy_activo as u64
}

/// Envuelve el cuerpo de una pantalla con la transición de entrada de
/// escena (fade + leve slide-up) anclada a `scene_key`.
fn scene_body(body: View<Msg>, key: u64) -> View<Msg> {
    body.animated_enter_from(key, motion::SLOW, Affine::translate((0.0, 24.0)))
}

/// Viewport para clampear overlays: reconstruye el tamaño de ventana a
/// partir del rect del canvas (`last_rect`) sumando el cromo que lo
/// rodea — rails, paneles y divisores en horizontal; menú + toolbar en
/// vertical. Si todavía no se pintó, cae a `initial_size()`.
fn viewport_of(model: &Model) -> (f32, f32) {
    use crate::chrome::{DockSide, RAIL_W, TOOLBAR_H};
    match model.last_rect {
        Some((w, h)) => {
            const SPLITTER_W: f32 = 6.0;
            let mut extra_w = RAIL_W * 2.0;
            // El sidebar de proyecto (izquierda) está siempre presente.
            extra_w += model.left_w + SPLITTER_W;
            if chrome::active_of(model, DockSide::Right).is_some() {
                extra_w += model.right_w + SPLITTER_W;
            }
            (w + extra_w, h + MENU_H + TOOLBAR_H)
        }
        None => {
            let (w, h) = Takiy::initial_size();
            (w as f32, h as f32)
        }
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
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

/// El menú principal del piano roll. Sólo comandos que mapean a
/// `Msg`/`EditMsg` reales ya existentes — nada inventado.
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

/// Traduce un command id (de la barra o del contextual) al `Msg`/`EditMsg`
/// real y lo dispatcha. Todos los ids mapean a acciones que ya existían.
fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    let msg = match cmd {
        "file.save" => Some(Msg::Save),
        "file.export_midi" => Some(Msg::ExportMidi),
        "file.export_wav" => Some(Msg::ExportWav),
        "file.quit" => Some(Msg::Quit),
        "edit.undo" => Some(Msg::Undo),
        "edit.redo" => Some(Msg::Redo),
        "edit.copy" => Some(Msg::Edit(EditMsg::CopySelected)),
        "edit.cut" => Some(Msg::Edit(EditMsg::CutSelected)),
        "edit.paste" => Some(Msg::PasteAtPlayhead),
        "edit.duplicate" => Some(Msg::Edit(EditMsg::DuplicateSelected)),
        "edit.delete" => Some(Msg::Edit(EditMsg::DeleteSelected)),
        "edit.new_track" => Some(Msg::Edit(EditMsg::NewTrack)),
        "edit.cycle_track" => Some(Msg::Edit(EditMsg::CycleTrack)),
        "edit.delete_track" => Some(Msg::Edit(EditMsg::DeleteActiveTrack)),
        "play.toggle" => Some(Msg::TogglePlay),
        "play.countin" => Some(Msg::PlayWithCountIn),
        "play.metronome" => Some(Msg::ToggleMetronome),
        "play.loop" => Some(Msg::ToggleLoop),
        "view.snap" => Some(Msg::CycleSnap),
        "view.snap_key" => Some(Msg::Edit(EditMsg::ToggleSnapToKey)),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

/// Menú contextual sobre la nota seleccionada. Refleja en gris el estado
/// real del editor (clipboard vacío deshabilita "Pegar"). Sólo acciones
/// existentes.
fn context_menu_for_selection(model: &Model, x: f32, y: f32) -> View<Msg> {
    let has_clipboard = !model.editor.clipboard.is_empty();
    let header = model
        .editor
        .selected
        .and_then(|(t, i)| model.editor.score.track(t).and_then(|tr| tr.notes().get(i)).copied())
        .map(|n| format!("nota midi {}", n.pitch.midi()))
        .unwrap_or_else(|| "selección".to_string());

    let mut items = vec![
        ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C"),
        ContextMenuItem::action("Cortar").with_shortcut("Ctrl+X"),
        ContextMenuItem::action("Duplicar").with_shortcut("Ctrl+D"),
    ];
    let paste = ContextMenuItem::action("Pegar al playhead").with_shortcut("Ctrl+V");
    items.push(if has_clipboard { paste } else { paste.disabled() });
    items.push(ContextMenuItem::separator());
    items.push(ContextMenuItem::action("Borrar").with_shortcut("Del").destructive());

    // Mapeo de índice de item → command id de `handle_menu_command`.
    let cmds: Vec<&'static str> =
        vec!["edit.copy", "edit.cut", "edit.duplicate", "edit.paste", "", "edit.delete"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(header),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

/// Ruteo de teclado en modo grabación: las teclas mapeadas tocan/graban
/// notas (key-down → note on, key-up → note off), `←/→` corren la octava,
/// `Espacio` alterna el fondo y `Esc` detiene.
fn record_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
    let base = model.recording.as_ref().map(|r| r.base_octave).unwrap_or(4);
    match event.state {
        KeyState::Pressed => match &event.key {
            Key::Named(NamedKey::Escape) => Some(Msg::ToggleRecord),
            Key::Named(NamedKey::ArrowUp) => Some(Msg::RecordOctave(1)),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::RecordOctave(-1)),
            Key::Named(NamedKey::Space) => Some(Msg::RecordToggleBacking),
            Key::Character(s)
                if !event.repeat && !event.modifiers.ctrl && !event.modifiers.alt =>
            {
                record::key_to_midi(s, base).map(Msg::RecordKeyDown)
            }
            _ => None,
        },
        KeyState::Released => match &event.key {
            Key::Character(s) => record::key_to_midi(s, base).map(Msg::RecordKeyUp),
            _ => None,
        },
    }
}

fn main() {
    llimphi_ui::run::<Takiy>();
}
