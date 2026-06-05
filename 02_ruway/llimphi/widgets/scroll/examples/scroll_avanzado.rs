//! Showcase de scroll avanzado (Tier 5): **app-bar colapsable** (sliver) +
//! lista scrolleable + **inercia (fling)**. Un único `offset` en el Model
//! maneja el colapso del header y el scroll del cuerpo; los botones "Fling"
//! sueltan una velocidad que decae con [`fling_step`] vía un ticker periódico.
//!
//! Corré con:
//! ```text
//! cargo run -p llimphi-widget-scroll --example scroll_avanzado --release
//! ```

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::{App, Handle, View};
use llimphi_widget_scroll::{
    clamp_offset, fling_settled, fling_step, sliver_app_bar, sliver_max_offset, ScrollPalette,
    FLING_FRICTION,
};

const HEADER_MAX: f32 = 200.0;
const HEADER_MIN: f32 = 56.0;
const VIEWPORT: f32 = 560.0;
const ROW_H: f32 = 46.0;
const N_ROWS: usize = 40;
const CONTENT_LEN: f32 = N_ROWS as f32 * ROW_H;
const DT: f32 = 1.0 / 60.0;

#[derive(Clone)]
enum Msg {
    /// Delta de scroll en px (rueda / arrastre de barra) a sumar al offset.
    ScrollBy(f32),
    /// Soltar una inercia con esta velocidad inicial (px/s).
    Fling(f32),
    /// Tick del ticker: avanza la inercia si hay.
    Tick,
}

struct Model {
    offset: f32,
    velocity: f32,
    theme: Theme,
}

fn max_off() -> f32 {
    sliver_max_offset(CONTENT_LEN, VIEWPORT, HEADER_MAX, HEADER_MIN)
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · scroll avanzado (sliver + fling)"
    }

    fn initial_size() -> (u32, u32) {
        (720, VIEWPORT as u32)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Ticker de inercia ~60 fps (mismo patrón que `approach`).
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
        Model { offset: 0.0, velocity: 0.0, theme: Theme::dark() }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::ScrollBy(d) => {
                model.velocity = 0.0; // un scroll manual corta la inercia
                model.offset = clamp_offset(model.offset + d, max_off() + VIEWPORT, VIEWPORT);
                // (clamp_offset usa content/viewport; acá el "content" efectivo
                // es max_off + viewport, así max_offset(...) == max_off.)
            }
            Msg::Fling(v) => model.velocity = v,
            Msg::Tick => {
                if model.velocity != 0.0 {
                    let (v, delta) = fling_step(model.velocity, DT, FLING_FRICTION);
                    model.offset =
                        clamp_offset(model.offset + delta, max_off() + VIEWPORT, VIEWPORT);
                    // Frenar en los topes o al asentarse.
                    if fling_settled(v) || model.offset <= 0.0 || model.offset >= max_off() {
                        model.velocity = 0.0;
                    } else {
                        model.velocity = v;
                    }
                }
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = &model.theme;
        let pal = ScrollPalette::from_theme(t);

        // Lista (cuerpo del sliver): filas alternadas.
        let rows: Vec<View<Msg>> = (0..N_ROWS)
            .map(|i| {
                let bg = if i % 2 == 0 { t.bg_panel } else { t.bg_panel_alt };
                View::new(Style {
                    size: Size { width: percent(1.0), height: length(ROW_H) },
                    align_items: Some(AlignItems::Center),
                    padding: Rect { left: length(20.0), right: length(20.0), top: length(0.0), bottom: length(0.0) },
                    ..Default::default()
                })
                .fill(bg)
                .text(format!("Fila {:02}", i + 1), 18.0, t.fg_text)
            })
            .collect();
        let list = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0), height: length(CONTENT_LEN) },
            ..Default::default()
        })
        .children(rows);

        let theme = t.clone();
        let sliver = sliver_app_bar(
            model.offset,
            HEADER_MAX,
            HEADER_MIN,
            move |frac| header(&theme, frac),
            list,
            CONTENT_LEN,
            VIEWPORT,
            Msg::ScrollBy,
            &pal,
        );

        View::new(Style {
            size: Size { width: percent(1.0), height: percent(1.0) },
            ..Default::default()
        })
        .fill(t.bg_app)
        .children(vec![sliver])
    }
}

/// Header colapsable: el título encoge con `frac` y el subtítulo + botones de
/// fling se desvanecen al colapsar (el `clip` del header los recorta).
fn header(t: &Theme, frac: f32) -> View<Msg> {
    let title_size = 34.0 - 14.0 * frac; // 34 → 20
    // Fondo que se aclara al colapsar (de accent a panel).
    let title_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0), height: length(HEADER_MIN) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: Rect { left: length(20.0), right: length(16.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style { ..Default::default() })
            .text("Scroll avanzado", title_size, t.fg_text),
        fling_buttons(t),
    ]);

    let subtitle = View::new(Style {
        size: Size { width: percent(1.0), height: length(28.0) },
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(20.0), right: length(20.0), top: length(0.0), bottom: length(0.0) },
        ..Default::default()
    })
    .alpha(1.0 - frac) // se desvanece al colapsar
    .text("Tier 5 · app-bar colapsable + inercia · rueda para scrollear", 15.0, t.fg_muted);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0), height: length(HEADER_MAX) },
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .children(vec![title_row, subtitle])
}

fn fling_buttons(t: &Theme) -> View<Msg> {
    let btn = |label: &str, v: f32| {
        View::new(Style {
            size: Size { width: length(96.0), height: length(34.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(t.bg_button)
        .hover_fill(t.bg_button_hover)
        .radius(8.0)
        .text(label.to_string(), 15.0, t.fg_text)
        .on_click(Msg::Fling(v))
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: Size { width: length(8.0), height: length(0.0) },
        ..Default::default()
    })
    .children(vec![btn("Fling ▲", -2600.0), btn("Fling ▼", 2600.0)])
}

fn main() {
    llimphi_ui::run::<Demo>();
}
