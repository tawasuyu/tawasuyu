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
use raymi_store::CalDb;

mod editor;
pub mod demo;
mod view;

pub use editor::{ContactDraft, ContactField, Editor, EventDraft, EventField};

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
    /// Editor abierto (evento/contacto) o ninguno.
    editor: Editor,
    /// Caché en disco (offline-first). `None` → sin persistencia (demo).
    db: Option<CalDb>,
    /// Id de cuenta — clave de la caché en disco.
    account_id: String,
    pub status: String,
    pub theme: Theme,
}

impl Model {
    /// Construye el modelo sobre `backend` sin persistencia (demo): sincroniza
    /// todo y abre el mes actual con el día de hoy seleccionado. Best-effort: si
    /// un sync falla, ese panel queda vacío.
    pub fn new(backend: Box<dyn DavBackend>, theme: Theme) -> Self {
        Self::build(backend, theme, None, "default".to_string())
    }

    /// Igual que [`Model::new`] pero **offline-first** con caché en disco:
    /// hidrata el `store` desde `db` antes de tocar la red (pinta al instante lo
    /// último conocido), luego sincroniza y vuelca el resultado al disco. Si la
    /// red falla, queda lo hidratado.
    pub fn with_persistence(
        backend: Box<dyn DavBackend>,
        theme: Theme,
        db: CalDb,
        account_id: String,
    ) -> Self {
        Self::build(backend, theme, Some(db), account_id)
    }

    /// Constructor común: arma el modelo, hidrata desde disco si hay `db`, abre
    /// el mes de hoy y sincroniza.
    fn build(backend: Box<dyn DavBackend>, theme: Theme, db: Option<CalDb>, account_id: String) -> Self {
        let now = now_unix();
        let today = time::start_of_day(now);
        let (date, _, _, _) = time::to_civil(now);

        // Offline-first: lo que haya en disco se ve antes del primer viaje de red.
        let store = match &db {
            Some(db) => db.hydrate(&account_id),
            None => CalStore::new(),
        };

        let mut model = Self {
            backend,
            store,
            mode: Mode::Calendar,
            view_year: date.year,
            view_month: date.month,
            selected_day: today,
            today,
            search: TextInputState::new(),
            search_focused: false,
            selected_contact: None,
            editor: Editor::None,
            db,
            account_id,
            status: String::from("raymi"),
            theme,
        };
        model.resync();
        model
    }

    /// (Re)sincroniza todo desde el backend: calendarios + eventos + libretas +
    /// contactos. Best-effort; deja el estado contado. Si la red falla, conserva
    /// lo que ya tenía (lo hidratado del disco). Tras un sync exitoso vuelca la
    /// caché a disco si hay persistencia.
    fn resync(&mut self) {
        if let Err(e) = self.store.sync_calendars(&*self.backend) {
            self.status = format!("sin red · {} en caché ({e})", self.store.calendars().len());
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
        self.persist();
        self.recount();
    }

    /// Vuelca la caché a disco si hay persistencia (best-effort: un fallo de
    /// disco no rompe la UI).
    fn persist(&self) {
        if let Some(db) = &self.db {
            let _ = db.snapshot(&self.account_id, &self.store);
        }
    }

    /// Recalcula la barra de estado con los conteos actuales.
    fn recount(&mut self) {
        self.status = format!(
            "{} calendario(s) · {} contacto(s)",
            self.store.calendars().len(),
            self.store.search_contacts("").len(),
        );
    }

    // ── editores: crear / editar / borrar ─────────────────────────────────

    /// Primer calendario disponible (destino por defecto de un evento nuevo).
    fn default_calendar(&self) -> Option<String> {
        self.store.calendars().first().map(|c| c.id.clone())
    }

    /// Primera libreta disponible (destino por defecto de un contacto nuevo).
    fn default_book(&self) -> Option<String> {
        self.store.address_books().first().map(|b| b.id.clone())
    }

    fn open_new_event(&mut self) {
        match self.default_calendar() {
            Some(cal) => self.editor = Editor::Event(EventDraft::new(cal, self.selected_day)),
            None => self.status = "no hay calendarios donde crear un evento".into(),
        }
    }

    fn open_edit_event(&mut self, calendar: &str, uid: &str) {
        if let Some(e) = self.store.events(calendar).iter().find(|e| e.uid == uid).cloned() {
            self.editor = Editor::Event(EventDraft::from_event(&e));
        }
    }

    fn open_new_contact(&mut self) {
        match self.default_book() {
            Some(book) => self.editor = Editor::Contact(ContactDraft::new(book)),
            None => self.status = "no hay libretas donde crear un contacto".into(),
        }
    }

    fn open_edit_contact(&mut self, uid: &str) {
        if let Some(c) = self.store.search_contacts("").into_iter().find(|c| c.uid == uid).cloned() {
            self.editor = Editor::Contact(ContactDraft::from_contact(&c));
        }
    }

    /// Avanza el calendario destino del evento en edición al siguiente de la lista.
    fn cycle_event_calendar(&mut self) {
        let ids: Vec<String> = self.store.calendars().iter().map(|c| c.id.clone()).collect();
        if let Editor::Event(d) = &mut self.editor {
            if let Some(pos) = ids.iter().position(|id| id == &d.calendar) {
                d.calendar = ids[(pos + 1) % ids.len()].clone();
            }
        }
    }

    /// Guarda el evento en edición: lo envía al backend y, si lo acepta, lo
    /// aplica a la caché y la persiste. En error, deja el editor abierto y avisa.
    fn save_event(&mut self) {
        let Editor::Event(d) = std::mem::replace(&mut self.editor, Editor::None) else { return };
        let uid = d.uid.clone().unwrap_or_else(|| new_uid("evt"));
        match d.build(uid) {
            Some(ev) => match self.backend.put_event(&ev) {
                Ok(()) => {
                    self.store.upsert_event(ev);
                    self.persist();
                    self.recount();
                }
                Err(e) => {
                    self.status = format!("no se pudo guardar el evento: {e}");
                    self.editor = Editor::Event(d);
                }
            },
            None => {
                self.status = "fecha u hora inválida (usá AAAA-MM-DD y HH:MM)".into();
                self.editor = Editor::Event(d);
            }
        }
    }

    /// Borra el evento en edición (si era existente).
    fn delete_event(&mut self) {
        let Editor::Event(d) = std::mem::replace(&mut self.editor, Editor::None) else { return };
        let Some(uid) = d.uid.clone() else { return }; // nuevo sin guardar: sólo cierra
        match self.backend.delete_event(&d.calendar, &uid) {
            Ok(()) => {
                self.store.remove_event(&d.calendar, &uid);
                self.persist();
                self.recount();
            }
            Err(e) => {
                self.status = format!("no se pudo borrar el evento: {e}");
                self.editor = Editor::Event(d);
            }
        }
    }

    fn save_contact(&mut self) {
        let Editor::Contact(d) = std::mem::replace(&mut self.editor, Editor::None) else { return };
        let uid = d.uid.clone().unwrap_or_else(|| new_uid("card"));
        match d.build(uid.clone()) {
            Some(c) => match self.backend.put_contact(&c) {
                Ok(()) => {
                    self.store.upsert_contact(c);
                    self.selected_contact = Some(uid);
                    self.persist();
                    self.recount();
                }
                Err(e) => {
                    self.status = format!("no se pudo guardar el contacto: {e}");
                    self.editor = Editor::Contact(d);
                }
            },
            None => {
                self.status = "el contacto necesita un nombre".into();
                self.editor = Editor::Contact(d);
            }
        }
    }

    fn delete_contact(&mut self) {
        let Editor::Contact(d) = std::mem::replace(&mut self.editor, Editor::None) else { return };
        let Some(uid) = d.uid.clone() else { return };
        match self.backend.delete_contact(&d.address_book, &uid) {
            Ok(()) => {
                self.store.remove_contact(&d.address_book, &uid);
                if self.selected_contact.as_deref() == Some(uid.as_str()) {
                    self.selected_contact = None;
                }
                self.persist();
                self.recount();
            }
            Err(e) => {
                self.status = format!("no se pudo borrar el contacto: {e}");
                self.editor = Editor::Contact(d);
            }
        }
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

    // ── editores ──────────────────────────────────────────────────────────
    /// Abrir el editor de evento nuevo (en el día seleccionado).
    NewEvent,
    /// Abrir el editor de un evento existente (`calendar`, `uid`).
    EditEvent { calendar: String, uid: String },
    /// Enfocar un campo del editor de evento.
    EventFocus(EventField),
    /// Tecla en el editor de evento.
    EventKey(KeyEvent),
    /// Alternar "día completo".
    EventToggleAllDay,
    /// Pasar el evento al siguiente calendario.
    EventCycleCalendar,
    /// Guardar / borrar el evento en edición.
    SaveEvent,
    DeleteEvent,
    /// Abrir el editor de contacto nuevo.
    NewContact,
    /// Editar el contacto seleccionado (`uid`).
    EditContact(String),
    /// Enfocar un campo del editor de contacto.
    ContactFocus(ContactField),
    /// Tecla en el editor de contacto.
    ContactKey(KeyEvent),
    /// Guardar / borrar el contacto en edición.
    SaveContact,
    DeleteContact,
    /// Cerrar cualquier editor sin guardar.
    CloseEditor,
    /// No hace nada (absorbe clicks dentro de la tarjeta del editor).
    Noop,
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

        // ── editores ──────────────────────────────────────────────────────
        Msg::NewEvent => model.open_new_event(),
        Msg::EditEvent { calendar, uid } => model.open_edit_event(&calendar, &uid),
        Msg::EventFocus(field) => {
            if let Editor::Event(d) = &mut model.editor {
                d.focus = field;
            }
        }
        Msg::EventToggleAllDay => {
            if let Editor::Event(d) = &mut model.editor {
                d.all_day = !d.all_day;
            }
        }
        Msg::EventCycleCalendar => model.cycle_event_calendar(),
        Msg::EventKey(event) => apply_editor_key(&mut model, event, true),
        Msg::SaveEvent => model.save_event(),
        Msg::DeleteEvent => model.delete_event(),
        Msg::NewContact => model.open_new_contact(),
        Msg::EditContact(uid) => model.open_edit_contact(&uid),
        Msg::ContactFocus(field) => {
            if let Editor::Contact(d) = &mut model.editor {
                d.focus = field;
            }
        }
        Msg::ContactKey(event) => apply_editor_key(&mut model, event, false),
        Msg::SaveContact => model.save_contact(),
        Msg::DeleteContact => model.delete_contact(),
        Msg::CloseEditor => model.editor = Editor::None,
        Msg::Noop => {}
    }
    model
}

/// Encamina una tecla al campo enfocado del editor abierto. Escape cierra; Tab
/// cicla el foco; el resto va al `TextInputState` activo. `is_event` distingue
/// qué editor está activo (las dos ramas comparten estructura).
fn apply_editor_key(model: &mut Model, event: KeyEvent, is_event: bool) {
    if event.state != KeyState::Pressed {
        return;
    }
    match &event.key {
        Key::Named(NamedKey::Escape) => model.editor = Editor::None,
        Key::Named(NamedKey::Tab) => match &mut model.editor {
            Editor::Event(d) if is_event => d.focus = d.focus.next(),
            Editor::Contact(d) if !is_event => d.focus = d.focus.next(),
            _ => {}
        },
        _ => match &mut model.editor {
            Editor::Event(d) if is_event => {
                d.focused_mut().apply_key(&event);
            }
            Editor::Contact(d) if !is_event => {
                d.focused_mut().apply_key(&event);
            }
            _ => {}
        },
    }
}

/// El árbol de la UI.
pub fn view(model: &Model) -> View<Msg> {
    view::root(model)
}

/// La capa modal: el editor de evento/contacto cuando hay uno abierto.
pub fn view_overlay(model: &Model) -> Option<View<Msg>> {
    match &model.editor {
        Editor::None => None,
        Editor::Event(d) => Some(view::event_editor(model, d)),
        Editor::Contact(d) => Some(view::contact_editor(model, d)),
    }
}

/// Atajos globales. Con la búsqueda enfocada, las teclas van a ella. Si no:
/// flechas ←/→ cambian de mes, `t` va a hoy, `c`/`g` alternan modo.
pub fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
    // Con un editor abierto, las teclas son suyas (Esc/Tab/escritura).
    match &model.editor {
        Editor::Event(_) => return Some(Msg::EventKey(event.clone())),
        Editor::Contact(_) => return Some(Msg::ContactKey(event.clone())),
        Editor::None => {}
    }
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
        Key::Character(ch) if ch.eq_ignore_ascii_case("n") => Some(match model.mode {
            Mode::Calendar => Msg::NewEvent,
            Mode::Contacts => Msg::NewContact,
        }),
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

/// Un `UID` razonablemente único para un evento o contacto nuevo: nanos del
/// reloj (monótonos en la práctica para clicks humanos) + sufijo de dominio.
fn new_uid(prefix: &str) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}-{nanos:x}@raymi")
}

// Reexport para que el binario del demo arme su `impl App` con tipos estables.
pub use llimphi_ui::Handle;
