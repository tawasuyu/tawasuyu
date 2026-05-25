//! `mirada-greeter` — el greeter del escritorio carmen.
//!
//! Ventana Llimphi de login. El compositor (`mirada-compositor`) la arranca
//! como proceso hijo cuando bootea en modo greeter, la compone a pantalla
//! completa (la reconoce por `app_id = "carmen.greeter"`) y le lee el stdout.
//!
//! Flujo: el usuario teclea usuario + contraseña, el greeter autentica con
//! [`auth_core`], y en éxito **imprime un [`SessionTicket`] a stdout** y
//! termina. El compositor parsea esa línea, hace el traspaso a modo sesión
//! (setuid al usuario + arranque) sin reiniciar el servidor gráfico — la
//! «mutación atómica» del DM.
//!
//! Backend de autenticación (ver [`pick_authenticator`]):
//! - por defecto, PAM contra el servicio `carmen`;
//! - `MIRADA_GREETER_MOCK="usuario:secreto"` usa el mock, para iterar la UI
//!   en cajas sin PAM o con el greeter anidado en otro escritorio.

use std::io::Write;
use std::sync::Arc;

use auth_core::{
    AuthError, Authenticator, MockAuthenticator, PamAuthenticator, SessionTicket, UserInfo,
    DEFAULT_SERVICE,
};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

/// `app_id` con el que el compositor reconoce y compone el greeter.
const GREETER_APP_ID: &str = "carmen.greeter";

/// Autenticador compartible entre el hilo de UI y el de fondo.
type DynAuth = Arc<dyn Authenticator + Send + Sync>;

fn main() {
    llimphi_ui::run::<Greeter>();
}

/// Elige el backend de autenticación según el entorno.
fn pick_authenticator() -> DynAuth {
    // Modo dev: credenciales fijas, sin tocar PAM.
    if let Ok(spec) = std::env::var("MIRADA_GREETER_MOCK") {
        if let Some((user, secret)) = spec.split_once(':') {
            eprintln!("mirada-greeter · backend mock (usuario «{user}»)");
            return Arc::new(MockAuthenticator::new().with_user(user, secret));
        }
        eprintln!("mirada-greeter · MIRADA_GREETER_MOCK mal formado (falta «:»), ignorado");
    }
    // Camino real: PAM. Servicio sobreescribible con `MIRADA_GREETER_PAM`.
    let service =
        std::env::var("MIRADA_GREETER_PAM").unwrap_or_else(|_| DEFAULT_SERVICE.to_string());
    eprintln!("mirada-greeter · backend PAM (servicio «{service}»)");
    Arc::new(PamAuthenticator::new(service))
}

/// Imprime el tiquet a stdout y fuerza el flush antes de terminar.
fn emit_ticket(ticket: &SessionTicket) {
    println!("{}", ticket.to_line());
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------------
// Modelo + mensajes
// ---------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    User,
    Pass,
}

enum Status {
    Idle,
    Authenticating,
    Failed(String),
}

struct Model {
    auth: DynAuth,
    user: String,
    pass: String,
    focus: Field,
    status: Status,
}

#[derive(Clone)]
enum Msg {
    Focus(Field),
    Insert(String),
    Backspace,
    Submit,
    AuthDone(Result<UserInfo, AuthError>),
}

// ---------------------------------------------------------------------
// Bucle Elm
// ---------------------------------------------------------------------

struct Greeter;

impl App for Greeter {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "carmen · greeter"
    }

    fn app_id() -> Option<&'static str> {
        Some(GREETER_APP_ID)
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        Model {
            auth: pick_authenticator(),
            user: String::new(),
            pass: String::new(),
            focus: Field::User,
            status: Status::Idle,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Mientras esperamos a PAM, no aceptamos input: el campo queda
        // "congelado" hasta que vuelva el resultado.
        if matches!(model.status, Status::Authenticating) {
            return None;
        }
        match &e.key {
            Key::Named(NamedKey::Tab) => Some(Msg::Focus(match model.focus {
                Field::User => Field::Pass,
                Field::Pass => Field::User,
            })),
            Key::Named(NamedKey::Enter) => {
                if model.focus == Field::User {
                    Some(Msg::Focus(Field::Pass))
                } else {
                    Some(Msg::Submit)
                }
            }
            Key::Named(NamedKey::Backspace) => Some(Msg::Backspace),
            _ => {
                // Sólo letras imprimibles (filtra modifiers, flechas, etc).
                let text = e.text.as_ref()?;
                if text.is_empty() || text.chars().any(|c| c.is_control()) {
                    None
                } else {
                    Some(Msg::Insert(text.clone()))
                }
            }
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Focus(f) => m.focus = f,
            Msg::Insert(s) => {
                match m.focus {
                    Field::User => m.user.push_str(&s),
                    Field::Pass => m.pass.push_str(&s),
                }
                // Tipear limpia el error previo — el usuario está corrigiendo.
                if matches!(m.status, Status::Failed(_)) {
                    m.status = Status::Idle;
                }
            }
            Msg::Backspace => {
                match m.focus {
                    Field::User => {
                        m.user.pop();
                    }
                    Field::Pass => {
                        m.pass.pop();
                    }
                }
                if matches!(m.status, Status::Failed(_)) {
                    m.status = Status::Idle;
                }
            }
            Msg::Submit => {
                if matches!(m.status, Status::Authenticating) {
                    return m;
                }
                let user = m.user.trim().to_string();
                if user.is_empty() {
                    m.status = Status::Failed("ingresá un usuario".into());
                    m.focus = Field::User;
                    return m;
                }
                let secret = m.pass.clone();
                let auth = Arc::clone(&m.auth);
                m.status = Status::Authenticating;
                // PAM puede tardar ~2 s ante un fallo: lo lanzamos a un hilo
                // de fondo y reentramos al `update` con `AuthDone` cuando
                // termine — la ventana sigue respondiendo mientras tanto.
                handle.spawn(move || Msg::AuthDone(auth.authenticate(&user, &secret)));
            }
            Msg::AuthDone(Ok(user)) => {
                emit_ticket(&SessionTicket::new(user));
                handle.quit();
            }
            Msg::AuthDone(Err(e)) => {
                m.status = Status::Failed(e.to_string());
                m.pass.clear();
                m.focus = Field::Pass;
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // Paleta análoga al `nahual-theme` dark default — sobria, alto contraste.
        let bg_app = Color::from_rgba8(14, 16, 22, 255);
        let bg_panel = Color::from_rgba8(22, 26, 36, 255);
        let bg_input = Color::from_rgba8(16, 20, 28, 255);
        let bg_input_focus = Color::from_rgba8(20, 26, 38, 255);
        let border = Color::from_rgba8(46, 54, 70, 255);
        let border_focus = Color::from_rgba8(110, 140, 220, 255);
        let fg_text = Color::from_rgba8(214, 222, 232, 255);
        let fg_muted = Color::from_rgba8(140, 152, 170, 255);
        let fg_placeholder = Color::from_rgba8(95, 105, 122, 255);
        let destructive = Color::from_rgba8(220, 110, 110, 255);

        let title = row(28.0, "carmen", 22.0, fg_text);
        let subtitle = row(16.0, "iniciá tu sesión", 12.0, fg_muted);

        let user_cap = row(14.0, "usuario", 10.0, fg_muted);
        let user_box = input_box(
            &model.user,
            "ingresá tu usuario",
            false,
            model.focus == Field::User,
            fg_text,
            fg_placeholder,
            bg_input,
            bg_input_focus,
            border,
            border_focus,
            Msg::Focus(Field::User),
        );

        let pass_cap = row(14.0, "contraseña", 10.0, fg_muted);
        let pass_box = input_box(
            &model.pass,
            "·······",
            true,
            model.focus == Field::Pass,
            fg_text,
            fg_placeholder,
            bg_input,
            bg_input_focus,
            border,
            border_focus,
            Msg::Focus(Field::Pass),
        );

        let (status_msg, status_color) = match &model.status {
            Status::Idle => (String::new(), fg_muted),
            Status::Authenticating => ("verificando…".to_string(), fg_muted),
            Status::Failed(m) => (m.clone(), destructive),
        };
        let status_line = row(16.0, &status_msg, 11.0, status_color);

        let card = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(320.0_f32),
                height: Dimension::auto(),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(10.0_f32),
            },
            padding: Rect {
                left: length(28.0_f32),
                right: length(28.0_f32),
                top: length(28.0_f32),
                bottom: length(28.0_f32),
            },
            ..Default::default()
        })
        .fill(bg_panel)
        .radius(12.0)
        .children(vec![
            title,
            subtitle,
            user_cap,
            user_box,
            pass_cap,
            pass_box,
            status_line,
        ]);

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![card])
    }
}

// ---------------------------------------------------------------------
// Helpers de vista
// ---------------------------------------------------------------------

/// Fila de ancho completo con un texto a la izquierda.
fn row(height: f32, text: &str, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

/// Caja de input: borde 1 px (rect padre coloreado), relleno, texto con
/// caret simulado cuando tiene foco, y `on_click` que pasa el foco aquí.
#[allow(clippy::too_many_arguments)]
fn input_box(
    value: &str,
    placeholder: &str,
    mask: bool,
    focused: bool,
    fg_text: Color,
    fg_placeholder: Color,
    bg_input: Color,
    bg_input_focus: Color,
    border: Color,
    border_focus: Color,
    on_focus: Msg,
) -> View<Msg> {
    let is_empty = value.is_empty();
    let shown = if is_empty {
        placeholder.to_string()
    } else if mask {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    // El caret se simula como bloque al final del texto. Sin blink: el
    // foco se distingue por el color de borde + bg ya cambiados.
    let display = if focused && !is_empty {
        format!("{shown}\u{2588}")
    } else {
        shown
    };
    let text_color = if is_empty { fg_placeholder } else { fg_text };
    let (bg, border_color) = if focused {
        (bg_input_focus, border_focus)
    } else {
        (bg_input, border)
    };

    // Inner: el rect con bg y el texto. Sin on_click — lo hereda el padre.
    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .text_aligned(display, 13.0, text_color, Alignment::Start);

    // Outer: el borde (1 px de padding pintado en `border_color`).
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(border_color)
    .radius(4.0)
    .on_click(on_focus)
    .children(vec![inner])
}
