//! Texto seleccionable **fuera del editor**: arrastrá el mouse sobre los
//! párrafos para resaltar y Ctrl/Cmd+C para copiar al portapapeles. La
//! selección la maneja el runtime (`View::selectable(key)`) — la app no
//! guarda estado de selección en su `Model`.
//!
//! Corre con: `cargo run -p llimphi-ui --example selectable_text --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Rect, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

struct Demo;

const PARRAFOS: [&str; 3] = [
    "Arrastrá el cursor sobre este texto para seleccionarlo. La selección \
     vive en el runtime de Llimphi, no en el Model de la app.",
    "Cada párrafo tiene su propia key estable; empezar a arrastrar en otro \
     reemplaza la selección anterior. Ctrl+C (o Cmd+C en macOS) copia el \
     rango resaltado al portapapeles del sistema.",
    "No es un editor: es texto de sólo lectura que igual se puede leer, \
     resaltar y copiar — labels, párrafos, celdas, salidas de consola.",
];

impl App for Demo {
    type Model = ();
    type Msg = ();

    fn title() -> &'static str {
        "llimphi · texto seleccionable"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {}

    fn update(_: Self::Model, _: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {}

    fn view(_: &Self::Model) -> View<Self::Msg> {
        let mut children: Vec<View<()>> = vec![View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            ..Default::default()
        })
        .text_aligned(
            "Texto seleccionable (arrastrá + Ctrl/Cmd+C)",
            22.0,
            Color::from_rgba8(230, 240, 250, 255),
            Alignment::Start,
        )];

        for (i, p) in PARRAFOS.iter().enumerate() {
            children.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: Dimension::auto(),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    *p,
                    16.0,
                    Color::from_rgba8(205, 214, 226, 255),
                    Alignment::Start,
                )
                // La línea clave: cada párrafo es seleccionable con una key
                // estable (su índice). El resaltado + copy los hace el runtime.
                .selectable(i as u64),
            );
        }

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
                left: length(40.0_f32),
                right: length(40.0_f32),
                top: length(36.0_f32),
                bottom: length(36.0_f32),
            },
            align_items: Some(AlignItems::Start),
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 24, 32, 255))
        .children(children)
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
