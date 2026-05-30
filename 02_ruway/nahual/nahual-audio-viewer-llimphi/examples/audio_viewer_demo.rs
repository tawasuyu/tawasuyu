//! Showcase de `nahual-audio-viewer-llimphi`.
//!
//! `cargo run -p nahual-audio-viewer-llimphi --example audio_viewer_demo --release -- /path/clip.mp3`
//!
//! Sin argumento abre nada (placeholder); con un archivo WAV/MP3/FLAC/
//! Opus/OGG lo reproduce y muestra el espectro. Espacio: play/pausa.

use std::path::PathBuf;
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use nahual_audio_viewer_llimphi::{audio_viewer_view, AudioViewerPalette, AudioViewerState};

const TICK: Duration = Duration::from_millis(33); // ~30 Hz

struct Model {
    state: AudioViewerState,
}

#[derive(Clone)]
enum Msg {
    Tick,
    TogglePlay,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · audio viewer showcase"
    }

    fn initial_size() -> (u32, u32) {
        (820, 420)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK, || Msg::Tick);
        let state = match std::env::args().nth(1).map(PathBuf::from) {
            Some(p) => AudioViewerState::open(&p),
            None => AudioViewerState::default(),
        };
        Model { state }
    }

    fn on_key(_model: &Model, e: &KeyEvent) -> Option<Msg> {
        if e.state == KeyState::Pressed && matches!(&e.key, Key::Named(NamedKey::Space)) {
            return Some(Msg::TogglePlay);
        }
        None
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => model.state.tick(TICK),
            Msg::TogglePlay => model.state.toggle_play(),
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = AudioViewerPalette::default();
        let viewer = audio_viewer_view::<Msg>(&model.state, &palette);
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![viewer])
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
