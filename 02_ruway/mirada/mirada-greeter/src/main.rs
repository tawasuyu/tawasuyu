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
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::context_menu_view;
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
    /// Clipboard del sistema, compartido por el menú de edición.
    clipboard: SystemClipboard,
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
}

#[derive(Clone)]
enum Msg {
    Focus(Field),
    /// Tecla a aplicar al campo focado (`TextInputState::apply_key`).
    EditKey(KeyEvent),
    Submit,
    AuthDone(Result<UserInfo, AuthError>),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Right-click sobre la ventana → abre el menú de edición en `(x, y)`
    /// operando sobre el campo focuseado.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
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
            clipboard: SystemClipboard::new(),
            menu_open: None,
            edit_menu: None,
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
        // Con un menú abierto, Esc lo cierra y el resto se ignora para no
        // teclear «detrás» del menú.
        if model.menu_open.is_some() || model.edit_menu.is_some() {
            if matches!(&e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::CloseMenus);
            }
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
                emit_ticket(&SessionTicket::new(user));
                handle.quit();
            }
            Msg::AuthDone(Err(e)) => {
                m.status = Status::Failed(e.to_string());
                m.pass.clear();
                m.focus = Field::Pass;
            }
            Msg::MenuOpen(idx) => m.menu_open = idx,
            Msg::CloseMenus => {
                m.menu_open = None;
                m.edit_menu = None;
            }
            Msg::MenuCommand(cmd) => return handle_menu_command(m, cmd, handle),
            Msg::EditMenuOpen(x, y) => {
                // Mientras autenticamos no abrimos el menú de edición.
                if !matches!(m.status, Status::Authenticating) {
                    m.edit_menu = Some((x, y));
                }
            }
            Msg::EditMenuAction(action) => return apply_edit_menu_action(m, action),
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let input_palette = TextInputPalette::from_theme(&theme);

        let title = row(28.0, "carmen", 22.0, theme.fg_text);
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
        .fill(theme.bg_panel)
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

        // Zona central que aloja la tarjeta de login. Ocupa todo el
        // espacio sobrante bajo la barra de menú.
        let body = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![card]);

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
            return Some(context_menu_view(editmenu::edit_context_menu(
                (x, y),
                viewport,
                &theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            )));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay(&menubar_spec(&menu, model, &theme))
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

    AppMenu::new()
        .menu(
            Menu::new("Sesión")
                .item(iniciar)
                .item(MenuItem::new("Ir a usuario", "session.user").separated())
                .item(MenuItem::new("Ir a contraseña", "session.pass")),
        )
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

