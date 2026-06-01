//! raymi-llimphi — el frontend de calendario y contactos sobre Llimphi.
//!
//! Dos modos, conmutables desde la barra superior:
//! - **Calendario** — grilla del mes (6×7) con chips de eventos coloreados por
//!   calendario, navegación de mes y "Hoy"; a la derecha, la **agenda del día**
//!   seleccionado (instancias con hora y asunto).
//! - **Contactos** — lista buscable a la izquierda y ficha del contacto a la
//!   derecha (avatar con iniciales, correos, teléfonos, organización, nota).
//!
//! Es un frontend **intercambiable** sobre el backend agnóstico de `raymi-core`:
//! el demo lo cablea a `MockBackend`; un `raymi-app` futuro lo cableará a un
//! puente CalDAV/CardDAV. Igual que `paloma`, el crate no implementa `App`: expone
//! `Model` + `Msg` + funciones libres que un binario delega desde su `impl App`.

use std::time::{SystemTime, UNIX_EPOCH};

use llimphi_theme::Theme;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::TextInputState;

use raymi_core::time::{self, CivilDate};
use raymi_core::{CalStore, DavBackend};

pub mod demo;
mod view;

/// Modo activo de la app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Calendar,
    Contacts,
}

/// El modelo del cliente: el backend (calendario + contactos), la caché y la
/// selección de la UI. `'static` para `App::Model`; vive en el hilo del compositor.
pub struct Model {
    backend: Box<dyn DavBackend>,
    store: CalStore,
    mode: Mode,
    /// Mes mostrado en la grilla.
    view_year: i64,
    view_month: u32,
    /// Día seleccionado (medianoche UTC, s Unix).
    selected_day: i64,
    /// Hoy (medianoche UTC), para resaltar la celda.
    today: i64,
    /// Caja de búsqueda de contactos.
    search: TextInputState,
    search_focused: bool,
    /// `UID` del contacto seleccionado.
    selected_contact: Option<String>,
    pub status: String,
    pub theme: Theme,
}

impl Model {
    /// Construye el modelo sobre `backend`, sincroniza todo (calendarios +
    /// eventos + libretas + contactos) y abre el mes actual con el día de hoy
    /// seleccionado. Best-effort: si un sync falla, ese panel queda vacío.
    pub fn new(backend: Box<dyn DavBackend>, theme: Theme) -> Self {
        let now = now_unix();
        let today = time::start_of_day(now);
        let (date, _, _, _) = time::to_civil(now);

        let mut model = Self {
            backend,
            store: CalStore::new(),
            mode: Mode::Calendar,
            view_year: date.year,
            view_month: date.month,
            selected_day: today,
            today,
            search: TextInputState::new(),
            search_focused: false,
            selected_contact: None,
            status: String::from("raymi"),
            theme,
        };
        model.resync();
        model
    }

    /// (Re)sincroniza todo desde el backend: calendarios + eventos + libretas +
    /// contactos. Best-effort; deja el estado contado.
    fn resync(&mut self) {
        if let Err(e) = self.store.sync_calendars(&*self.backend) {
            self.status = format!("error al listar calendarios: {e}");
            return;
        }
        let cal_ids: Vec<String> = self.store.calendars().iter().map(|c| c.id.clone()).collect();
        for id in &cal_ids {
            let _ = self.store.sync_events(&*self.backend, id);
        }
        if self.store.sync_address_books(&*self.backend).is_ok() {
            let books: Vec<String> = self.store.address_books().iter().map(|b| b.id.clone()).collect();
            for b in &books {
                let _ = self.store.sync_contacts(&*self.backend, b);
            }
        }
        self.status = format!(
            "{} calendario(s) · {} contacto(s)",
            self.store.calendars().len(),
            self.store.search_contacts("").len(),
        );
    }

    /// Avanza/retrocede el mes mostrado por `delta` meses.
    fn shift_month(&mut self, delta: i64) {
        let d = time::add_months(CivilDate { year: self.view_year, month: self.view_month, day: 1 }, delta);
        self.view_year = d.year;
        self.view_month = d.month;
    }

    /// Vuelve al mes de hoy y selecciona el día de hoy.
    fn go_today(&mut self) {
        let (date, _, _, _) = time::to_civil(self.today);
        self.view_year = date.year;
        self.view_month = date.month;
        self.selected_day = self.today;
    }

    fn selected_contact_uid(&self) -> Option<&str> {
        self.selected_contact.as_deref()
    }
}

/// Las transiciones de la UI.
#[derive(Clone)]
pub enum Msg {
    /// Cambiar de modo (Calendario / Contactos).
    SetMode(Mode),
    /// Mes anterior / siguiente en la grilla.
    PrevMonth,
    NextMonth,
    /// Volver al mes de hoy.
    Today,
    /// Re-sincronizar desde el backend.
    Refresh,
    /// Seleccionar un día (medianoche UTC).
    SelectDay(i64),
    /// Enfocar/desenfocar la búsqueda de contactos.
    ContactSearchFocus(bool),
    /// Tecla mientras la búsqueda tiene foco.
    ContactSearchKey(KeyEvent),
    /// Seleccionar un contacto por `UID`.
    SelectContact(String),
}

/// La transición Elm.
pub fn update(mut model: Model, msg: Msg, _handle: &llimphi_ui::Handle<Msg>) -> Model {
    match msg {
        Msg::SetMode(m) => {
            model.mode = m;
            model.search_focused = false;
        }
        Msg::PrevMonth => model.shift_month(-1),
        Msg::NextMonth => model.shift_month(1),
        Msg::Today => model.go_today(),
        Msg::Refresh => model.resync(),
        Msg::SelectDay(day) => model.selected_day = day,
        Msg::SelectContact(uid) => model.selected_contact = Some(uid),
        Msg::ContactSearchFocus(on) => model.search_focused = on,
        Msg::ContactSearchKey(event) => {
            if event.state == KeyState::Pressed {
                match &event.key {
                    Key::Named(NamedKey::Escape) => {
                        model.search.clear();
                        model.search_focused = false;
                    }
                    _ => {
                        model.search.apply_key(&event);
                    }
                }
            }
        }
    }
    model
}

/// El árbol de la UI.
pub fn view(model: &Model) -> View<Msg> {
    view::root(model)
}

/// Atajos globales. Con la búsqueda enfocada, las teclas van a ella. Si no:
/// flechas ←/→ cambian de mes, `t` va a hoy, `c`/`g` alternan modo.
pub fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
    if model.mode == Mode::Contacts && model.search_focused {
        return Some(Msg::ContactSearchKey(event.clone()));
    }
    if event.state != KeyState::Pressed || event.modifiers.ctrl || event.modifiers.alt {
        return None;
    }
    match &event.key {
        Key::Named(NamedKey::F5) => Some(Msg::Refresh),
        Key::Named(NamedKey::ArrowLeft) if model.mode == Mode::Calendar => Some(Msg::PrevMonth),
        Key::Named(NamedKey::ArrowRight) if model.mode == Mode::Calendar => Some(Msg::NextMonth),
        Key::Character(ch) if ch.eq_ignore_ascii_case("t") => Some(Msg::Today),
        Key::Character(ch) if ch.eq_ignore_ascii_case("g") => Some(Msg::SetMode(Mode::Calendar)),
        Key::Character(ch) if ch.eq_ignore_ascii_case("k") => Some(Msg::SetMode(Mode::Contacts)),
        _ => None,
    }
}

/// Rueda del mouse en el calendario → cambia de mes.
pub fn on_wheel(
    model: &Model,
    delta: WheelDelta,
    _cursor: (f32, f32),
    _mods: llimphi_ui::Modifiers,
) -> Option<Msg> {
    if model.mode != Mode::Calendar {
        return None;
    }
    let lines = (delta.y * 3.0).round() as i32;
    if lines > 0 {
        Some(Msg::NextMonth)
    } else if lines < 0 {
        Some(Msg::PrevMonth)
    } else {
        None
    }
}

/// Segundos Unix actuales (UTC). El frontend es un binario normal, así que sí
/// puede leer el reloj del sistema (a diferencia del núcleo agnóstico).
fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// Reexport para que el binario del demo arme su `impl App` con tipos estables.
pub use llimphi_ui::Handle;
