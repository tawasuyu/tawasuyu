//! Showcase de `nahual-video-viewer-llimphi`.
//!
//! Modo archivo: `cargo run -p nahual-video-viewer-llimphi --example video_viewer_demo --release -- /path/clip.ivf`
//! Modo procedural (sin args): usa el `TestCard` de media-core (gradiente
//! animado + círculo) para validar el pintado sin un archivo real.

use std::path::PathBuf;
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::{App, Handle, View};
use media_core::TestCard;
use nahual_video_viewer_llimphi::{video_viewer_view, VideoViewerPalette, VideoViewerState};

const TICK: Duration = Duration::from_millis(33); // ~30 Hz

struct Model {
    state: VideoViewerState,
}

#[derive(Clone)]
enum Msg {
    Tick,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · video viewer showcase"
    }

    fn initial_size() -> (u32, u32) {
        (960, 700)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK, || Msg::Tick);
        let arg = std::env::args().nth(1).map(PathBuf::from);
        let state = match arg {
            Some(p) => {
                let ext = p
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(str::to_ascii_lowercase);
                match ext.as_deref() {
                    Some("webm" | "mkv") => VideoViewerState::open_webm(&p),
                    _ => VideoViewerState::open_av1(&p),
                }
            }
            None => VideoViewerState::from_source(
                Box::new(TestCard::new(512, 320, 30.0)),
                "testcard 512×320",
                512,
                320,
                None,
            ),
        };
        Model { state }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                model.state.tick(TICK);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = VideoViewerPalette::default();
        let viewer = video_viewer_view::<Msg>(&model.state, &palette);
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
