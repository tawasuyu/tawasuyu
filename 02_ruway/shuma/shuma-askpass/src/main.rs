//! `shuma-askpass` — popup de contraseña compatible con `SUDO_ASKPASS`.
//!
//! `sudo -A` invoca el binario referenciado por `$SUDO_ASKPASS`; éste debe:
//! - aceptar el prompt como `argv[1]` (opcional),
//! - escribir la pass a stdout y terminar con exit 0 al confirmar,
//! - terminar con exit !=0 sin stdout si el usuario cancela.
//!
//! El bin abre una ventana Llimphi modal pequeña con un text-input `masked`,
//! botones Cancelar/OK y atajos Enter/Esc. Al cerrarse, lee el resultado del
//! singleton compartido y lo emite a stdout / pone el exit code.
//!
//! Para que `sudo -A` lo encuentre, shuma-exec exporta `SUDO_ASKPASS` al PTY.

use std::sync::Mutex;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Singleton del resultado: `Some(pass)` si el usuario confirmó; `None` si
/// canceló o cerró la ventana sin confirmar. El `main` lo lee tras `run`.
static RESULT: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone)]
enum Msg {
    Key(KeyEvent),
    Confirm,
    Cancel,
}

struct Model {
    prompt: String,
    input: TextInputState,
    theme: Theme,
}

struct Askpass;

impl App for Askpass {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "shuma · autenticación"
    }

    fn app_id() -> Option<&'static str> {
        Some("shuma.askpass")
    }

    fn initial_size() -> (u32, u32) {
        (400, 190)
    }

    fn init(_h: &Handle<Self::Msg>) -> Self::Model {
        // `argv[1]` lo trae sudo con el prompt resuelto ("[sudo] password
        // for user:"); si no, default explícito.
        let prompt = std::env::args()
            .nth(1)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Contraseña:".to_string());
        Model {
            prompt,
            input: TextInputState::masked(),
            theme: Theme::dark(),
        }
    }

    fn on_key(_model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Key(e) => match &e.key {
                Key::Named(NamedKey::Escape) => {
                    handle.quit();
                }
                Key::Named(NamedKey::Enter) => {
                    if let Ok(mut g) = RESULT.lock() {
                        *g = Some(m.input.text());
                    }
                    handle.quit();
                }
                _ => {
                    let _ = m.input.apply_key(&e);
                }
            },
            Msg::Confirm => {
                if let Ok(mut g) = RESULT.lock() {
                    *g = Some(m.input.text());
                }
                handle.quit();
            }
            Msg::Cancel => {
                handle.quit();
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = &model.theme;
        let prompt = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            model.prompt.clone(),
            14.0,
            theme.fg_text,
            Alignment::Start,
        );

        let tpal = TextInputPalette::from_theme(theme);
        let input = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(36.0_f32),
            },
            ..Default::default()
        })
        .children(vec![text_input_view(
            &model.input,
            "•••••••",
            true,
            &tpal,
            Msg::Confirm, // click sobre el box no cambia foco (siempre focado)
        )]);

        let cancelar = View::new(Style {
            size: Size {
                width: length(120.0_f32),
                height: length(34.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_button)
        .hover_fill(theme.bg_button_hover)
        .radius(5.0)
        .text_aligned("Cancelar".to_string(), 12.0, theme.fg_text, Alignment::Center)
        .on_click(Msg::Cancel);
        let ok = View::new(Style {
            size: Size {
                width: length(120.0_f32),
                height: length(34.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(5.0)
        .text_aligned("Aceptar".to_string(), 12.0, theme.bg_app, Alignment::Center)
        .on_click(Msg::Confirm);
        let botones = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(40.0_f32),
            },
            gap: Size {
                width: length(10.0_f32),
                height: length(0.0_f32),
            },
            justify_content: Some(JustifyContent::End),
            align_items: Some(AlignItems::Center),
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(12.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![cancelar, ok]);

        let card = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            padding: Rect {
                left: length(20.0_f32),
                right: length(20.0_f32),
                top: length(18.0_f32),
                bottom: length(18.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(10.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(8.0)
        .children(vec![prompt, input, botones]);

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(14.0_f32),
                right: length(14.0_f32),
                top: length(14.0_f32),
                bottom: length(14.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![card])
    }
}

fn main() {
    bitacora::abrir("shuma");
    llimphi_ui::run::<Askpass>();
    // Tras `run`, el bucle terminó. Si el usuario confirmó, escupimos la
    // pass a stdout (sin newline trailing — algunos askpass strict cortan
    // en `\n` y otros no, la pass del usuario *podría* tener LF; nos
    // alineamos al formato que usa `ssh-askpass`: sólo lo que tipeó).
    let pass = RESULT.lock().ok().and_then(|mut g| g.take());
    if let Some(p) = pass {
        // `print!` sin trailing newline — algunos consumidores la dejan
        // entera; `sudo` y `ssh` toleran un único `\n` final, así que lo
        // agregamos como hace ssh-askpass clásico.
        println!("{p}");
        std::process::exit(0);
    }
    std::process::exit(1);
}
