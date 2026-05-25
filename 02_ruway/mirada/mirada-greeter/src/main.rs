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
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

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
    user: TextInputState,
    pass: TextInputState,
    focus: Field,
    status: Status,
}

#[derive(Clone)]
enum Msg {
    Focus(Field),
    /// Tecla a aplicar al campo focado (`TextInputState::apply_key`).
    EditKey(KeyEvent),
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
            user: TextInputState::new(),
            pass: TextInputState::masked(),
            focus: Field::User,
            status: Status::Idle,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Mientras esperamos a PAM, no aceptamos input.
        if matches!(model.status, Status::Authenticating) {
            return None;
        }
        match &e.key {
            Key::Named(NamedKey::Tab) => Some(Msg::Focus(toggle(model.focus))),
            Key::Named(NamedKey::Enter) => {
                if model.focus == Field::User {
                    Some(Msg::Focus(Field::Pass))
                } else {
                    Some(Msg::Submit)
                }
            }
            _ => {
                // Todo lo demás se delega al widget — `apply_key` decide
                // si la consume (printable, Backspace) o no.
                Some(Msg::EditKey(e.clone()))
            }
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Focus(f) => m.focus = f,
            Msg::EditKey(ev) => {
                let dst = match m.focus {
                    Field::User => &mut m.user,
                    Field::Pass => &mut m.pass,
                };
                if dst.apply_key(&ev) {
                    // Tipear limpia el error previo — el usuario está
                    // corrigiendo.
                    if matches!(m.status, Status::Failed(_)) {
                        m.status = Status::Idle;
                    }
                }
            }
            Msg::Submit => {
                if matches!(m.status, Status::Authenticating) {
                    return m;
                }
                let user = m.user.text().trim().to_string();
                if user.is_empty() {
                    m.status = Status::Failed("ingresá un usuario".into());
                    m.focus = Field::User;
                    return m;
                }
                let secret = m.pass.text().to_string();
                let auth = Arc::clone(&m.auth);
                m.status = Status::Authenticating;
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
        let palette = Palette::default();
        let input_palette = TextInputPalette::default();

        let title = row(28.0, "carmen", 22.0, palette.fg_text);
        let subtitle = row(16.0, "iniciá tu sesión", 12.0, palette.fg_muted);

        let user_cap = row(14.0, "usuario", 10.0, palette.fg_muted);
        let user_box = text_input_view(
            &model.user,
            "ingresá tu usuario",
            model.focus == Field::User,
            &input_palette,
            Msg::Focus(Field::User),
        );

        let pass_cap = row(14.0, "contraseña", 10.0, palette.fg_muted);
        let pass_box = text_input_view(
            &model.pass,
            "·······",
            model.focus == Field::Pass,
            &input_palette,
            Msg::Focus(Field::Pass),
        );

        let (status_msg, status_color) = match &model.status {
            Status::Idle => (String::new(), palette.fg_muted),
            Status::Authenticating => ("verificando…".to_string(), palette.fg_muted),
            Status::Failed(m) => (m.clone(), palette.destructive),
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
        .fill(palette.bg_panel)
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
        .fill(palette.bg_app)
        .children(vec![card])
    }
}

fn toggle(f: Field) -> Field {
    match f {
        Field::User => Field::Pass,
        Field::Pass => Field::User,
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

// ---------------------------------------------------------------------
// Paleta
// ---------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Palette {
    bg_app: Color,
    bg_panel: Color,
    fg_text: Color,
    fg_muted: Color,
    destructive: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg_app: Color::from_rgba8(14, 16, 22, 255),
            bg_panel: Color::from_rgba8(22, 26, 36, 255),
            fg_text: Color::from_rgba8(214, 222, 232, 255),
            fg_muted: Color::from_rgba8(140, 152, 170, 255),
            destructive: Color::from_rgba8(220, 110, 110, 255),
        }
    }
}
