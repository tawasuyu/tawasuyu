//! `mirada-greeter` — el greeter del escritorio carmen.
//!
//! Una ventana GPUI de login. El compositor (`mirada-compositor`) la
//! arranca como proceso hijo cuando bootea en modo greeter, la compone a
//! pantalla completa (la reconoce por `app_id = "carmen.greeter"`) y le
//! lee el stdout.
//!
//! Flujo: el usuario teclea usuario + contraseña, el greeter autentica
//! con [`brahman_auth`], y en éxito **imprime un [`SessionTicket`] a
//! stdout** y termina. El compositor parsea esa línea, hace el traspaso
//! a modo sesión (setuid al usuario + arranque) sin reiniciar el
//! servidor gráfico — la «mutación atómica» del DM.
//!
//! Backend de autenticación (ver [`pick_authenticator`]):
//! - por defecto, PAM contra el servicio `carmen`;
//! - `MIRADA_GREETER_MOCK="usuario:secreto"` usa el mock, para iterar la
//!   UI en cajas sin PAM o con el greeter anidado en otro escritorio.

use std::io::Write;
use std::sync::Arc;

use brahman_auth::{
    AuthError, Authenticator, MockAuthenticator, PamAuthenticator, SessionTicket, UserInfo,
    DEFAULT_SERVICE,
};
use gpui::{
    div, prelude::*, px, App, Application, Bounds, Context, Entity, IntoElement, Render,
    SharedString, Window, WindowBounds, WindowOptions,
};
use nahual_theme::Theme;
use nahual_widget_text_input::{TextInput, TextInputEvent};

/// `app_id` con el que el compositor reconoce y compone el greeter.
const GREETER_APP_ID: &str = "carmen.greeter";

/// Autenticador compartible entre el hilo de UI y el de fondo.
type DynAuth = Arc<dyn Authenticator + Send + Sync>;

fn main() {
    Application::new().run(|cx: &mut App| {
        Theme::install_default(cx);
        let auth = pick_authenticator();
        let bounds = Bounds::centered(None, gpui::size(px(960.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                app_id: Some(GREETER_APP_ID.into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Greeter::new(auth, window, cx)),
        )
        .expect("abrir la ventana del greeter");
        cx.activate(true);
    });
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

/// Estado del intento de login en curso.
enum Status {
    /// Esperando que el usuario teclee.
    Idle,
    /// Autenticación en vuelo (en el hilo de fondo).
    Authenticating,
    /// Último intento falló; el mensaje es para mostrar.
    Failed(String),
}

struct Greeter {
    auth: DynAuth,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    status: Status,
}

impl Greeter {
    fn new(auth: DynAuth, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        let username = cx.new(|cx| TextInput::new("", cx).with_placeholder("usuario"));
        let password = cx.new(|cx| {
            TextInput::new("", cx)
                .with_placeholder("contraseña")
                .with_mask()
        });

        // Enter en «usuario» pasa el foco a «contraseña».
        cx.subscribe_in(&username, window, |this, _u, ev, window, cx| {
            if let TextInputEvent::Confirmed(_) = ev {
                this.password.read(cx).request_focus(window);
            }
        })
        .detach();

        // Enter en «contraseña» dispara la autenticación.
        cx.subscribe_in(&password, window, |this, _p, ev, window, cx| {
            if let TextInputEvent::Confirmed(_) = ev {
                this.submit(window, cx);
            }
        })
        .detach();

        // Foco inicial en «usuario».
        username.read(cx).request_focus(window);

        Self {
            auth,
            username,
            password,
            status: Status::Idle,
        }
    }

    /// Valida el formulario y lanza la autenticación en el hilo de fondo
    /// (PAM puede tardar — `pam_unix` demora ~2 s ante un fallo).
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Authenticating) {
            return; // intento ya en curso
        }
        let user = self.username.read(cx).text().trim().to_string();
        let secret = self.password.read(cx).text().to_string();
        if user.is_empty() {
            self.status = Status::Failed("ingresá un usuario".into());
            self.username.read(cx).request_focus(window);
            cx.notify();
            return;
        }

        self.status = Status::Authenticating;
        cx.notify();

        let auth = Arc::clone(&self.auth);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { auth.authenticate(&user, &secret) })
                .await;
            let _ = this.update(cx, |me, cx| me.finish(result, cx));
        })
        .detach();
    }

    /// Procesa el resultado de la autenticación.
    fn finish(&mut self, result: Result<UserInfo, AuthError>, cx: &mut Context<Self>) {
        match result {
            Ok(user) => {
                // El compositor lee esta línea del stdout del greeter.
                emit_ticket(&SessionTicket::new(user));
                cx.quit();
            }
            Err(e) => {
                self.status = Status::Failed(e.to_string());
                // Limpia la contraseña; el foco ya está en ese campo.
                self.password.update(cx, |p, cx| p.set_text("", cx));
                cx.notify();
            }
        }
    }
}

impl Render for Greeter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let (status_msg, status_color) = match &self.status {
            Status::Idle => (SharedString::default(), theme.fg_muted),
            Status::Authenticating => (SharedString::from("verificando…"), theme.fg_muted),
            Status::Failed(m) => (SharedString::from(m.clone()), theme.accent_destructive()),
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.bg_app)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .w(px(320.0))
                    .p(px(28.0))
                    .bg(theme.bg_panel)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(12.0))
                    .child(
                        div()
                            .text_size(px(22.0))
                            .text_color(theme.fg_text)
                            .child("carmen"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.fg_muted)
                            .child("iniciá tu sesión"),
                    )
                    .child(caption(&theme, "usuario"))
                    .child(self.username.clone())
                    .child(caption(&theme, "contraseña"))
                    .child(self.password.clone())
                    .child(
                        div()
                            .h(px(16.0))
                            .text_size(px(11.0))
                            .text_color(status_color)
                            .child(status_msg),
                    ),
            )
    }
}

/// Etiqueta pequeña sobre un campo del formulario.
fn caption(theme: &Theme, text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(10.0))
        .text_color(theme.fg_muted)
        .child(text)
}

/// Imprime el tiquet a stdout y fuerza el flush antes de terminar.
fn emit_ticket(ticket: &SessionTicket) {
    println!("{}", ticket.to_line());
    let _ = std::io::stdout().flush();
}
