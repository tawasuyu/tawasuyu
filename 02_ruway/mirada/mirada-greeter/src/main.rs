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

mod rain;
mod sessions;
mod state;

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

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
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;

/// `app_id` con el que el compositor reconoce y compone el greeter.
const GREETER_APP_ID: &str = "carmen.greeter";

/// Autenticador compartible entre el hilo de UI y el de fondo.
type DynAuth = Arc<dyn Authenticator + Send + Sync>;

fn main() {
    rimay_localize::init();
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
    /// Sesiones de escritorio descubiertas en el sistema (la 0 es mirada).
    sessions: Vec<sessions::Session>,
    /// Índice de la sesión elegida dentro de `sessions`.
    session_idx: usize,
    /// Clipboard del sistema, compartido por el menú de edición.
    clipboard: SystemClipboard,
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    menu_anim: Tween<f32>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    edit_active: usize,
    /// Animación de aparición del menú de edición.
    edit_anim: Tween<f32>,
    /// ¿Pintar el fondo de lluvia de glifos (rusty rain)?
    rain_enabled: bool,
    /// Paleta del fondo de lluvia.
    rain_color: state::RainColor,
    /// Reloj del fondo (segundos), avanzado por `Msg::RainTick`.
    rain_t: f32,
}

#[derive(Clone)]
enum Msg {
    Focus(Field),
    /// Tecla a aplicar al campo focado (`TextInputState::apply_key`).
    EditKey(KeyEvent),
    Submit,
    AuthDone(Result<UserInfo, AuthError>),
    /// Avanza la sesión elegida (con wrap) — clic en el selector de la
    /// tarjeta.
    CycleSession(i32),
    /// Fija la sesión elegida por índice — elección desde el menú.
    PickSession(usize),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Right-click sobre la ventana → abre el menú de edición en `(x, y)`
    /// operando sobre el campo focuseado.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición.
    EditMenuAction(EditAction),
    /// Navegación ↑/↓ por la fila activa del menú principal.
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de animación de aparición/swap (re-render).
    MenuTick,
    /// Navegación ↑/↓ por la fila activa del menú de edición.
    EditNav(i32),
    /// Enter sobre la fila activa del menú de edición.
    EditActivate,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Tick del fondo de lluvia — avanza el reloj y repinta.
    RainTick,
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

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let saved = state::GreeterState::load();
        let sessions = sessions::discover();

        // Prerellena el último usuario y arranca el foco directo en la
        // contraseña si ya hay un nombre recordado.
        let mut user = TextInputState::new();
        if !saved.last_user.is_empty() {
            user.set_text(saved.last_user.clone());
        }
        let focus = if saved.last_user.is_empty() {
            Field::User
        } else {
            Field::Pass
        };

        // Restaura el último escritorio elegido buscándolo por nombre (los
        // índices no son estables entre arranques: las sesiones del sistema
        // pueden aparecer/desaparecer).
        let session_idx = sessions
            .iter()
            .position(|s| s.name == saved.last_session)
            .unwrap_or(0);

        // Si el fondo está encendido, arranca el reloj de animación (~30 fps).
        if saved.rain_enabled {
            handle.spawn_periodic(Duration::from_millis(33), || Msg::RainTick);
        }

        Model {
            auth: pick_authenticator(),
            user,
            pass: TextInputState::masked(),
            focus,
            status: Status::Idle,
            sessions,
            session_idx,
            clipboard: SystemClipboard::new(),
            menu_open: None,
            edit_menu: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            rain_enabled: saved.rain_enabled,
            rain_color: saved.rain_color,
            rain_t: 0.0,
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
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => {
                    return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                }
                Key::Named(NamedKey::ArrowRight) => {
                    return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                }
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                _ => return None,
            }
        }
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
        }
        match &e.key {
            Key::Named(NamedKey::Tab) => Some(Msg::Focus(toggle(model.focus))),
            // ↑/↓ cambian de escritorio sin tocar el ratón (los campos de una
            // línea no usan las flechas verticales, así que quedan libres).
            Key::Named(NamedKey::ArrowUp) => Some(Msg::CycleSession(-1)),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::CycleSession(1)),
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
                    m.status = Status::Failed(rimay_localize::t("greeter-error-empty-user"));
                    m.focus = Field::User;
                    return m;
                }
                let secret = m.pass.text().to_string();
                let auth = Arc::clone(&m.auth);
                m.status = Status::Authenticating;
                handle.spawn(move || Msg::AuthDone(auth.authenticate(&user, &secret)));
            }
            Msg::AuthDone(Ok(user)) => {
                // El comando de la sesión elegida viaja en el tiquet. Vacío
                // (sesión nativa mirada) ⇒ el compositor usa su autostart.
                let chosen = m.sessions.get(m.session_idx);
                let exec = chosen.map(|s| s.exec.clone()).unwrap_or_default();
                let foreign = chosen.map(|s| s.foreign).unwrap_or(false);
                // Recuerda usuario + escritorio (y la config del fondo) para
                // el próximo login.
                state::GreeterState {
                    last_user: m.user.text().trim().to_string(),
                    last_session: chosen.map(|s| s.name.clone()).unwrap_or_default(),
                    rain_enabled: m.rain_enabled,
                    rain_color: m.rain_color,
                }
                .save();
                let ticket = SessionTicket::new(user);
                let ticket = if exec.is_empty() {
                    ticket
                } else {
                    ticket.with_session(exec).foreign(foreign)
                };
                emit_ticket(&ticket);
                handle.quit();
            }
            Msg::CycleSession(dir) => {
                let n = m.sessions.len().max(1) as i32;
                let cur = m.session_idx as i32;
                m.session_idx = (((cur + dir) % n + n) % n) as usize;
            }
            Msg::PickSession(i) => {
                if i < m.sessions.len() {
                    m.session_idx = i;
                }
                m.menu_open = None;
            }
            Msg::AuthDone(Err(e)) => {
                m.status = Status::Failed(e.to_string());
                m.pass.clear();
                m.focus = Field::Pass;
            }
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        return handle_menu_command(m, cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let (input, masked) = focused_input(&m);
                let flags = EditFlags::from_editor(input.editor(), masked);
                m.edit_active = editmenu::edit_menu_step(flags, m.edit_active, dir);
            }
            Msg::EditActivate => {
                let (input, masked) = focused_input(&m);
                let flags = EditFlags::from_editor(input.editor(), masked);
                if let Some(a) = editmenu::edit_menu_action_at(flags, m.edit_active) {
                    return apply_edit_menu_action(m, a);
                }
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                m.edit_active = usize::MAX;
            }
            Msg::MenuCommand(cmd) => return handle_menu_command(m, cmd, handle),
            Msg::EditMenuOpen(x, y) => {
                // Mientras autenticamos no abrimos el menú de edición.
                if !matches!(m.status, Status::Authenticating) {
                    m.edit_menu = Some((x, y));
                    m.edit_active = usize::MAX;
                    m.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::EditMenuAction(action) => return apply_edit_menu_action(m, action),
            Msg::RainTick => {
                // Avanza el reloj del fondo. Se envuelve para no perder
                // precisión `f32` en sesiones largas del greeter.
                m.rain_t = (m.rain_t + 0.033) % 100_000.0;
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let input_palette = TextInputPalette::from_theme(&theme);

        // Barrita de acento sobre el título — el toque de color del DM.
        let accent_bar = View::new(Style {
            size: Size {
                width: length(46.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(2.0);

        let title = row(30.0, "carmen", 23.0, theme.fg_text);
        let subtitle = row(
            16.0,
            &rimay_localize::t("greeter-subtitle"),
            12.0,
            theme.fg_muted,
        );

        let user_cap = row(
            14.0,
            &rimay_localize::t("greeter-label-user"),
            10.0,
            theme.fg_muted,
        );
        let user_box = text_input_view(
            &model.user,
            &rimay_localize::t("greeter-placeholder-user"),
            model.focus == Field::User,
            &input_palette,
            Msg::Focus(Field::User),
        );

        let pass_cap = row(
            14.0,
            &rimay_localize::t("greeter-label-password"),
            10.0,
            theme.fg_muted,
        );
        let pass_box = text_input_view(
            &model.pass,
            "·······",
            model.focus == Field::Pass,
            &input_palette,
            Msg::Focus(Field::Pass),
        );

        let (status_msg, status_color) = match &model.status {
            Status::Idle => (String::new(), theme.fg_muted),
            Status::Authenticating => (
                rimay_localize::t("greeter-status-authenticating"),
                theme.fg_muted,
            ),
            Status::Failed(m) => (m.clone(), theme.fg_destructive),
        };
        let status_line = row(16.0, &status_msg, 11.0, status_color);

        // Selector de sesión: una pastilla «‹ nombre · tipo ›». Siempre hay
        // al menos «mirada» y «mirada · pata», así que las flechas sirven.
        let sess = model.sessions.get(model.session_idx);
        let sess_name = sess.map(|s| s.name.clone()).unwrap_or_else(|| "mirada".into());
        let sess_kind = sess.map(|s| s.kind.tag()).unwrap_or("wayland");
        let sess_cap = row(14.0, "Escritorio", 10.0, theme.fg_muted);
        let arrow = |glyph: &str, msg: Msg| {
            View::new(Style {
                size: Size {
                    width: length(30.0_f32),
                    height: length(28.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(theme.bg_button)
            .radius(7.0)
            .text_aligned(glyph.to_string(), 14.0, theme.fg_text, Alignment::Center)
            .on_click(msg)
        };
        let sess_center = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(28.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(7.0)
        .text_aligned(
            format!("{sess_name} · {sess_kind}"),
            11.0,
            theme.fg_text,
            Alignment::Center,
        );
        let session_selector = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            gap: Size {
                width: length(6.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            arrow("‹", Msg::CycleSession(-1)),
            sess_center,
            arrow("›", Msg::CycleSession(1)),
        ]);

        // Botón de entrar: la acción primaria, en color de acento. Mientras
        // autentica se atenúa y cambia de rótulo.
        let busy = matches!(model.status, Status::Authenticating);
        let (btn_label, btn_fill) = if busy {
            ("Entrando…", theme.bg_button)
        } else {
            ("Entrar", theme.accent)
        };
        let enter_btn = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(38.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(btn_fill)
        .radius(9.0)
        .text_aligned(
            btn_label.to_string(),
            13.0,
            Color::from_rgba8(245, 246, 250, 255),
            Alignment::Center,
        )
        .on_click(Msg::Submit);

        let card = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(360.0_f32),
                height: Dimension::auto(),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(11.0_f32),
            },
            padding: Rect {
                left: length(32.0_f32),
                right: length(32.0_f32),
                top: length(30.0_f32),
                bottom: length(26.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(14.0)
        .children(vec![
            accent_bar,
            title,
            subtitle,
            spacer(6.0),
            user_cap,
            user_box,
            pass_cap,
            pass_box,
            status_line,
            spacer(2.0),
            sess_cap,
            session_selector,
            spacer(6.0),
            enter_btn,
            spacer(2.0),
            row(13.0, "↑/↓: escritorio  ·  Enter: entrar", 9.0, theme.fg_muted),
            row(13.0, "Ctrl+Alt+F1…F12: consola  ·  Ctrl+Alt+⌫: salir", 9.0, theme.fg_muted),
        ]);

        // Zona central que aloja la tarjeta de login. Ocupa todo el
        // espacio sobrante bajo la barra de menú. Si el fondo de lluvia está
        // activo, su `paint_with` pinta detrás de la tarjeta (el painter de un
        // nodo corre antes que sus hijos).
        let mut body = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_app);
        if model.rain_enabled {
            let t = model.rain_t;
            let bright = rain_bright(model.rain_color, &theme);
            body = body.paint_with(move |scene, ts, rect| {
                rain::paint(scene, ts, rect, t, bright);
            });
        }
        let body = body.children(vec![card]);

        // Raíz en columna: barra de menú arriba + cuerpo centrado. El
        // right-click se engancha en la raíz (origen 0,0 ⇒ las coords
        // locales ya son de ventana) y abre el menú de edición sobre el
        // campo focuseado.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(vec![menubar, body])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        let theme = Theme::dark();
        let (w, h) = Self::initial_size();
        let viewport = (w as f32, h as f32);
        // El menú de edición tiene prioridad si está abierto.
        if let Some((x, y)) = model.edit_menu {
            let (input, masked) = focused_input(model);
            let flags = EditFlags::from_editor(input.editor(), masked);
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                viewport,
                &theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras { appear: model.edit_anim.value(), ..Default::default() },
            ));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// El campo de texto focuseado + si está enmascarado.
fn focused_input(model: &Model) -> (&TextInputState, bool) {
    match model.focus {
        Field::User => (&model.user, model.user.is_masked()),
        Field::Pass => (&model.pass, model.pass.is_masked()),
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Greeter::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Construye el menú principal del greeter reflejando el estado real del
/// campo focuseado (Cortar/Copiar grises sin selección o si enmascarado).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let (input, masked) = focused_input(model);
    let editor = input.editor();
    let has_sel = editor.has_selection();
    let can_undo = editor.can_undo();
    let can_redo = editor.can_redo();
    let has_text = !editor.is_empty();
    let busy = matches!(model.status, Status::Authenticating);

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo { undo = undo.disabled(); }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo { redo = redo.disabled(); }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    // Enmascarado o sin selección ⇒ no se puede cortar/copiar.
    if !has_sel || masked { cut = cut.disabled(); copy = copy.disabled(); }
    let paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall").shortcut("Ctrl+A").separated();
    if !has_text { sel_all = sel_all.disabled(); }

    let mut iniciar = MenuItem::new("Iniciar sesión", "session.submit").shortcut("Enter");
    if busy { iniciar = iniciar.disabled(); }

    // Menú "Sesión": acciones de login + la lista de sesiones descubiertas.
    // La elegida lleva «●»; el resto «  ».
    let mut sesion = Menu::new("Sesión")
        .item(iniciar)
        .item(MenuItem::new("Ir a usuario", "session.user"))
        .item(MenuItem::new("Ir a contraseña", "session.pass"));
    for (i, s) in model.sessions.iter().enumerate() {
        let mark = if i == model.session_idx { "● " } else { "   " };
        let label = format!("{mark}{} · {}", s.name, s.kind.tag());
        let mut item = MenuItem::new(label, format!("session.pick.{i}"));
        if i == 0 {
            item = item.separated();
        }
        sesion = sesion.item(item);
    }

    AppMenu::new()
        .menu(sesion)
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
}

/// Traduce el `command` del menú principal al `Msg` real y lo despacha.
fn handle_menu_command(mut model: Model, command: String, handle: &Handle<Msg>) -> Model {
    model.menu_open = None;
    // Elección de sesión: «session.pick.<idx>».
    if let Some(rest) = command.strip_prefix("session.pick.") {
        if let Ok(i) = rest.parse::<usize>() {
            return Greeter::update(model, Msg::PickSession(i), handle);
        }
        return model;
    }
    let target = match command.as_str() {
        "session.submit" => Some(Msg::Submit),
        "session.user" => Some(Msg::Focus(Field::User)),
        "session.pass" => Some(Msg::Focus(Field::Pass)),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        _ => None,
    };
    match target {
        Some(Msg::Submit) => Greeter::update(model, Msg::Submit, handle),
        Some(msg) => Greeter::update(model, msg, handle),
        None => model,
    }
}

/// Aplica una acción del menú de edición al campo focuseado. Limpia el
/// error previo si el contenido cambió (el usuario está corrigiendo).
fn apply_edit_menu_action(mut model: Model, action: EditAction) -> Model {
    model.edit_menu = None;
    let r = {
        let mut clip = std::mem::replace(&mut model.clipboard, SystemClipboard::new());
        let editor = match model.focus {
            Field::User => model.user.editor_mut(),
            Field::Pass => model.pass.editor_mut(),
        };
        let r = editmenu::apply(editor, action, &mut clip);
        model.clipboard = clip;
        r
    };
    if r.changed() && matches!(model.status, Status::Failed(_)) {
        model.status = Status::Idle;
    }
    model
}

/// Resuelve el color base (RGB brillante) del fondo de lluvia. `Accent` toma
/// el acento del tema; el resto son paletas fijas.
fn rain_bright(color: state::RainColor, theme: &Theme) -> (u8, u8, u8) {
    match color {
        state::RainColor::Green => (120, 255, 140),
        state::RainColor::Red => (255, 90, 80),
        state::RainColor::Amber => (255, 200, 90),
        state::RainColor::Cyan => (110, 235, 255),
        state::RainColor::Accent => {
            let c = theme.accent.to_rgba8();
            (c.r, c.g, c.b)
        }
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

/// Un hueco vertical de `h` px — separa grupos dentro de la tarjeta.
fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        ..Default::default()
    })
}

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

