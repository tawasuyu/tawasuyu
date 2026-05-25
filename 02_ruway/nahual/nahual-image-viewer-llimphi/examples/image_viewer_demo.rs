//! Showcase de `nahual-image-viewer-llimphi`.
//!
//! Modo archivo: `cargo run -p nahual-image-viewer-llimphi --example image_viewer_demo --release -- /path/a/imagen.png`
//! Modo procedural (sin args): genera un degradado in-memory para
//! validar el path de pintado sin depender de un archivo real.

use std::path::PathBuf;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::{App, Handle, View};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImagePreviewState, ImageViewerPalette,
    DEFAULT_IMAGE_BYTES_MAX,
};

const PROC_W: u32 = 512;
const PROC_H: u32 = 320;

struct Model {
    state: ImagePreviewState,
    path: Option<PathBuf>,
}

#[derive(Clone)]
enum Msg {}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · image viewer showcase"
    }

    fn initial_size() -> (u32, u32) {
        (960, 700)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let arg = std::env::args().nth(1).map(PathBuf::from);
        match arg {
            Some(p) => Model {
                state: load_image(&p, DEFAULT_IMAGE_BYTES_MAX),
                path: Some(p),
            },
            None => Model {
                state: procedural_state(),
                path: None,
            },
        }
    }

    fn update(model: Model, _: Msg, _: &Handle<Msg>) -> Model {
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = ImageViewerPalette::default();
        let viewer = image_viewer_view::<Msg>(&model.state, model.path.as_deref(), &palette);
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

/// Genera un degradado RGB diagonal in-memory: rojo en (0,0), verde
/// abajo, azul a la derecha. Permite ver que la pintura realmente sale
/// sin pedir un archivo afuera.
fn procedural_state() -> ImagePreviewState {
    let mut pixels = Vec::with_capacity((PROC_W * PROC_H * 4) as usize);
    for y in 0..PROC_H {
        for x in 0..PROC_W {
            let r = 255 - (x * 255 / PROC_W.max(1)) as u8;
            let g = (y * 255 / PROC_H.max(1)) as u8;
            let b = (x * 255 / PROC_W.max(1)) as u8;
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
            pixels.push(255);
        }
    }
    let blob = Blob::from(pixels);
    let image = Image::new(blob, ImageFormat::Rgba8, PROC_W, PROC_H);
    ImagePreviewState::Image {
        image,
        width: PROC_W,
        height: PROC_H,
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
