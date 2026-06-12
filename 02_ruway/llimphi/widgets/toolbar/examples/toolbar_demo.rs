//! Demo interactiva del toolbar: grupo de navegación + toggles de vista +
//! acciones (una deshabilitada). El texto de abajo refleja el último click.
//!
//! `cargo run -p llimphi-widget-toolbar --example toolbar_demo --release`

use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems,
};
use llimphi_ui::{run, App, Handle, View};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};

struct Demo;

struct Model {
    vista: usize,
    dual: bool,
    ultimo: String,
}

#[derive(Clone)]
enum Msg {
    Subir,
    Vista(usize),
    Dual,
    Nueva,
}

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "toolbar · demo"
    }

    fn init(_handle: &Handle<Self::Msg>) -> Self::Model {
        Model { vista: 0, dual: false, ultimo: "(sin acciones)".into() }
    }

    fn update(model: Self::Model, msg: Self::Msg, _h: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Subir => m.ultimo = "subir".into(),
            Msg::Vista(v) => {
                m.vista = v;
                m.ultimo = format!("vista {v}");
            }
            Msg::Dual => {
                m.dual = !m.dual;
                m.ultimo = format!("dual: {}", m.dual);
            }
            Msg::Nueva => m.ultimo = "nueva carpeta".into(),
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let pal = ToolbarPalette::from_theme(&theme);
        let vistas = [Icon::Rows, Icon::Table, Icon::Grid, Icon::Image];
        let barra = toolbar_view(
            vec![
                ToolbarGroup::new(vec![ToolbarItem::new(
                    |_s, c| icon_view(Icon::ChevronUp, c, 1.7),
                    Msg::Subir,
                )
                .with_label("subir")]),
                ToolbarGroup::new(
                    vistas
                        .iter()
                        .enumerate()
                        .map(|(i, ic)| {
                            let ic = *ic;
                            ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), Msg::Vista(i))
                                .active(model.vista == i)
                        })
                        .collect(),
                ),
                ToolbarGroup::new(vec![
                    ToolbarItem::new(|_s, c| icon_view(Icon::Columns, c, 1.7), Msg::Dual)
                        .active(model.dual),
                    ToolbarItem::new(|_s, c| icon_view(Icon::Plus, c, 1.7), Msg::Nueva)
                        .with_label("carpeta")
                        .enabled(false),
                ]),
            ],
            36.0,
            &pal,
        );
        let estado = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: llimphi_ui::llimphi_layout::taffy::Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text(format!("último: {}", model.ultimo), 13.0, theme.fg_text);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![barra, estado])
    }
}

fn main() {
    run::<Demo>();
}
