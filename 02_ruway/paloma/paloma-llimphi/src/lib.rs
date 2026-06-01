//! paloma-llimphi — el frontend del correo sobre Llimphi.
//!
//! Tres paneles, de izquierda a derecha:
//! - **Buzones** — la lista de carpetas (`INBOX`, `Sent`, …) con su rol y el
//!   contador de no-leídos. Click selecciona y trae sus mensajes.
//! - **Hilos** — las conversaciones del buzón activo, recientes primero. Cada
//!   fila: asunto, remitente del último mensaje, extracto, cantidad y un punto
//!   de acento si hay no-leídos. Click abre el hilo (y lo marca como leído).
//! - **Lectura** — los mensajes del hilo seleccionado, apilados del más viejo
//!   al más nuevo (de · para · fecha · cuerpo de texto). Botón *Responder*.
//!
//! Es un frontend **intercambiable** sobre el `MailBackend` agnóstico de
//! `paloma-core`: el demo lo cablea a `MockBackend`; `paloma-app` lo cableará
//! a `NetBackend` (IMAP+SMTP). No conoce red ni protocolo — sólo el `trait`.
//!
//! El crate no implementa `App` directamente: expone `Model` + `Msg` + las
//! funciones libres (`update`/`view`/`view_overlay`/`on_key`/`on_wheel`) que
//! un binario delga desde su propio `impl App`. Así cada anfitrión inyecta el
//! backend que quiera en su `init` (el `App::init` de Llimphi no toma args).

use llimphi_theme::Theme;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::TextInputState;

use paloma_core::{
    parse_address_list, Address, Flags, MailBackend, MailStore, MessageId, OutgoingMessage, Thread,
};

pub mod demo;
mod view;

/// Campo enfocado del formulario de redacción.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeField {
    To,
    Cc,
    Subject,
    Body,
}

impl ComposeField {
    /// El siguiente campo en el ciclo Tab (To → Cc → Subject → Body → To).
    fn next(self) -> Self {
        match self {
            ComposeField::To => ComposeField::Cc,
            ComposeField::Cc => ComposeField::Subject,
            ComposeField::Subject => ComposeField::Body,
            ComposeField::Body => ComposeField::To,
        }
    }
}

/// Estado del compositor (modal de redacción). Vive sólo mientras se redacta;
/// `None` en el modelo significa "no hay redacción abierta".
pub struct Compose {
    pub to: TextInputState,
    pub cc: TextInputState,
    pub subject: TextInputState,
    pub body: TextInputState,
    pub focus: ComposeField,
    /// Si es una respuesta, el mensaje original (alimenta `In-Reply-To` y
    /// `References`). El asunto/destinatario ya vienen prellenados.
    pub in_reply_to: Option<MessageId>,
    pub references: Vec<MessageId>,
    /// Firmar el saliente con la identidad Ed25519 de la cuenta (gancho de
    /// `agora`; hoy es una preferencia de UI hasta integrar el keystore).
    pub sign: bool,
}

impl Compose {
    fn empty() -> Self {
        Self {
            to: TextInputState::new(),
            cc: TextInputState::new(),
            subject: TextInputState::new(),
            body: TextInputState::new(),
            focus: ComposeField::To,
            in_reply_to: None,
            references: Vec::new(),
            sign: false,
        }
    }

    /// El input del campo enfocado, para rutear las teclas.
    fn focused_mut(&mut self) -> &mut TextInputState {
        match self.focus {
            ComposeField::To => &mut self.to,
            ComposeField::Cc => &mut self.cc,
            ComposeField::Subject => &mut self.subject,
            ComposeField::Body => &mut self.body,
        }
    }
}

/// El modelo del cliente: el backend (cualquiera), la caché local y la
/// selección actual de la UI. `'static` para encajar en `App::Model`; no
/// necesita `Send` porque vive en el hilo del compositor.
pub struct Model {
    /// El transporte. Boxeado y agnóstico: mock en el demo, IMAP/SMTP en prod.
    backend: Box<dyn MailBackend>,
    store: MailStore,
    /// Dirección propia, para el `From` de los envíos.
    me: Address,
    /// Buzón activo (clave en el store); `None` antes del primer sync.
    selected_mailbox: Option<String>,
    /// Hilos del buzón activo, recientes primero. Cacheados al seleccionar.
    threads: Vec<Thread>,
    /// Índice del hilo abierto dentro de `threads`; `None` si ninguno.
    selected_thread: Option<usize>,
    /// Offset de scroll de la lista de hilos (en filas).
    list_scroll: usize,
    /// Offset de scroll del panel de lectura (en píxeles).
    read_scroll: f32,
    /// Redacción en curso; `None` si el modal está cerrado.
    compose: Option<Compose>,
    /// Caja de búsqueda de texto. Con contenido, el panel central muestra
    /// resultados (mensajes que matchean) en vez de los hilos del buzón.
    search: TextInputState,
    /// `true` si la caja de búsqueda tiene el foco del teclado.
    search_focused: bool,
    /// Modo de búsqueda: `false` = exacta (texto), `true` = semántica. La
    /// semántica (embeddings vía `rimay`) está pendiente de integrar; el toggle
    /// existe y, por ahora, cae a la exacta avisándolo.
    search_semantic: bool,
    /// Caché en disco (offline-first). `None` = sin persistencia (demos).
    db: Option<paloma_store::MailDb>,
    /// Identificador de la cuenta — clave en la caché en disco.
    account_id: String,
    /// Última línea de estado (resultado de un sync/envío).
    pub status: String,
    pub theme: Theme,
}

impl Model {
    /// Construye el modelo sobre `backend` **sin persistencia** (demos): no toca
    /// disco, sincroniza buzones de red y abre el primero.
    pub fn new(backend: Box<dyn MailBackend>, me: Address, theme: Theme) -> Self {
        Self::build(backend, me, theme, None, "demo".to_string())
    }

    /// Como [`Self::new`] pero con **caché en disco**: precarga lo último
    /// conocido (offline-first), refresca contra el backend y persiste el
    /// resultado bajo `account_id`.
    pub fn with_persistence(
        backend: Box<dyn MailBackend>,
        me: Address,
        theme: Theme,
        db: paloma_store::MailDb,
        account_id: impl Into<String>,
    ) -> Self {
        Self::build(backend, me, theme, Some(db), account_id.into())
    }

    fn build(
        backend: Box<dyn MailBackend>,
        me: Address,
        theme: Theme,
        db: Option<paloma_store::MailDb>,
        account_id: String,
    ) -> Self {
        let mut store = MailStore::new();
        // Offline-first: pintar lo cacheado antes de tocar la red.
        if let Some(d) = &db {
            let cached = d.load_mailboxes(&account_id);
            if !cached.is_empty() {
                store.ingest_mailboxes(cached);
            }
        }
        // Refrescar la lista de buzones de red; si funciona, persistirla.
        let mut status = String::from("paloma · sin sincronizar");
        match store.sync_mailboxes(&*backend) {
            Ok(()) => {
                if let Some(d) = &db {
                    let _ = d.save_mailboxes(&account_id, store.mailboxes());
                }
            }
            Err(e) => {
                if store.mailboxes().is_empty() {
                    status = format!("sin conexión y sin caché: {e}");
                } else {
                    status = format!("offline · buzones desde caché ({e})");
                }
            }
        }
        let first = store.mailboxes().first().map(|m| m.name.clone());
        let mut model = Self {
            backend,
            store,
            me,
            selected_mailbox: None,
            threads: Vec::new(),
            selected_thread: None,
            list_scroll: 0,
            read_scroll: 0.0,
            compose: None,
            search: TextInputState::new(),
            search_focused: false,
            search_semantic: false,
            db,
            account_id,
            status,
            theme,
        };
        if let Some(name) = first {
            model.open_mailbox(&name);
        }
        model
    }

    /// Abre el mensaje `id`: salta a su buzón y selecciona el hilo que lo
    /// contiene. Usado al elegir un resultado de búsqueda.
    fn open_message(&mut self, id: &MessageId) {
        let Some(mailbox) = self.store.message(id).map(|m| m.mailbox.clone()) else { return };
        self.open_mailbox(&mailbox);
        if let Some(idx) = self.threads.iter().position(|t| t.message_ids.contains(id)) {
            self.open_thread(idx);
        }
    }

    /// Trae y abre `mailbox`: sincroniza sus mensajes (o los lee de la caché si
    /// no hay red), reconstruye hilos y limpia la selección de hilo. Persiste el
    /// snapshot fresco en disco cuando la red responde.
    fn open_mailbox(&mut self, mailbox: &str) {
        let db = self.db.clone();
        let account = self.account_id.clone();
        match self.store.sync_messages(&*self.backend, mailbox) {
            Ok(()) => {
                self.threads = self.store.threads(mailbox);
                self.selected_mailbox = Some(mailbox.to_string());
                self.selected_thread = None;
                self.list_scroll = 0;
                if let Some(d) = &db {
                    let _ = d.save_messages(&account, mailbox, self.store.messages(mailbox));
                }
                self.status = format!(
                    "{mailbox} · {} hilos · {} sin leer",
                    self.threads.len(),
                    self.store.unread_count(mailbox),
                );
            }
            Err(e) => {
                // Offline: caer a la caché en disco si la hay.
                let cached = db.as_ref().map(|d| d.load_messages(&account, mailbox)).unwrap_or_default();
                if cached.is_empty() {
                    self.status = format!("error al traer {mailbox}: {e}");
                } else {
                    self.store.ingest(mailbox, cached);
                    self.threads = self.store.threads(mailbox);
                    self.selected_mailbox = Some(mailbox.to_string());
                    self.selected_thread = None;
                    self.list_scroll = 0;
                    self.status = format!(
                        "{mailbox} · offline · {} hilos desde caché",
                        self.threads.len(),
                    );
                }
            }
        }
    }

    /// Marca todos los mensajes del hilo `idx` como leídos y lo selecciona.
    fn open_thread(&mut self, idx: usize) {
        let Some(mailbox) = self.selected_mailbox.clone() else { return };
        let Some(thread) = self.threads.get(idx) else { return };
        let ids: Vec<MessageId> = thread.message_ids.clone();
        for id in &ids {
            let _ = self.store.mark_seen(&*self.backend, &mailbox, id);
        }
        // Recalcular hilos (cambiaron los no-leídos); el orden por fecha es
        // estable, así que el índice sigue apuntando al mismo hilo.
        self.threads = self.store.threads(&mailbox);
        self.selected_thread = Some(idx);
        self.read_scroll = 0.0;
        // Reflejar el estado de leído en la caché en disco.
        if let Some(d) = self.db.clone() {
            let _ = d.save_messages(&self.account_id, &mailbox, self.store.messages(&mailbox));
        }
        self.status = format!("{mailbox} · {} sin leer", self.store.unread_count(&mailbox));
    }

    /// El hilo abierto, si lo hay.
    fn current_thread(&self) -> Option<&Thread> {
        self.selected_thread.and_then(|i| self.threads.get(i))
    }

    /// El `Message-ID` del mensaje más reciente del hilo abierto.
    fn current_newest(&self) -> Option<MessageId> {
        self.current_thread().and_then(|t| t.message_ids.last().cloned())
    }

    /// Persiste el snapshot de un buzón en la caché en disco (si la hay).
    fn persist_mailbox(&self, mailbox: &str) {
        if let Some(d) = self.db.clone() {
            let _ = d.save_messages(&self.account_id, mailbox, self.store.messages(mailbox));
        }
    }

    /// Alterna un flag de un mensaje (estrella o leído), local + backend, y
    /// reconstruye los hilos. `flip` recibe los flags actuales y devuelve los
    /// nuevos.
    fn toggle_flag(&mut self, id: &MessageId, flip: impl Fn(Flags) -> Flags) {
        let Some(mailbox) = self.selected_mailbox.clone() else { return };
        let Some(new_flags) = self.store.message(id).map(|m| flip(m.flags)) else { return };
        let _ = self.store.set_flags(&*self.backend, &mailbox, id, new_flags);
        self.threads = self.store.threads(&mailbox);
        self.persist_mailbox(&mailbox);
    }

    /// Marca el hilo abierto como borrado (`\Deleted`) y lo saca de la bandeja.
    fn delete_current_thread(&mut self) {
        let Some(mailbox) = self.selected_mailbox.clone() else { return };
        let Some(thread) = self.current_thread() else { return };
        let ids: Vec<MessageId> = thread.message_ids.clone();
        for id in &ids {
            if let Some(flags) = self.store.message(id).map(|m| Flags { deleted: true, ..m.flags }) {
                let _ = self.store.set_flags(&*self.backend, &mailbox, id, flags);
            }
        }
        self.threads = self.store.threads(&mailbox);
        self.selected_thread = None;
        self.read_scroll = 0.0;
        self.persist_mailbox(&mailbox);
        self.status = format!("{mailbox} · hilo enviado a la papelera");
    }
}

/// Las transiciones de la UI.
#[derive(Clone)]
pub enum Msg {
    /// Click en un buzón de la izquierda.
    SelectMailbox(String),
    /// Click en un hilo de la lista central.
    SelectThread(usize),
    /// Scroll de la lista de hilos (líneas; +abajo).
    ScrollList(i32),
    /// Scroll del panel de lectura (líneas; +abajo).
    ScrollRead(i32),
    /// Abrir el compositor en blanco.
    ComposeOpen,
    /// Abrir el compositor como respuesta al último mensaje del hilo abierto.
    ComposeReply,
    /// Abrir el compositor reenviando el último mensaje del hilo abierto.
    ComposeForward,
    /// Alternar la firma Ed25519 del saliente (gancho de agora).
    ComposeToggleSign,
    /// Cerrar el compositor sin enviar.
    ComposeClose,
    /// Alternar la estrella (`\Flagged`) de un mensaje.
    ToggleStar(MessageId),
    /// Alternar leído/no-leído (`\Seen`) de un mensaje.
    ToggleSeen(MessageId),
    /// Enviar el hilo abierto a la papelera (`\Deleted`).
    DeleteThread,
    /// Cambiar el campo enfocado del compositor.
    ComposeFocus(ComposeField),
    /// Tecla mientras el compositor está abierto (va al campo enfocado).
    ComposeKey(KeyEvent),
    /// Enviar lo redactado.
    ComposeSend,
    /// Re-traer el buzón activo desde el backend.
    Refresh,
    /// Enfocar/desenfocar la caja de búsqueda.
    SearchFocus(bool),
    /// Tecla mientras la búsqueda tiene el foco.
    SearchKey(KeyEvent),
    /// Abrir un mensaje (típicamente, un resultado de búsqueda).
    OpenMessage(MessageId),
    /// Cambiar el modo de búsqueda (false = exacta, true = semántica).
    SearchMode(bool),
    /// Pedir el render HTML enriquecido de un mensaje (gancho de puriy).
    ViewRich(MessageId),
}

/// La transición Elm. Toma el modelo por valor y lo devuelve mutado.
pub fn update(mut model: Model, msg: Msg, _handle: &llimphi_ui::Handle<Msg>) -> Model {
    match msg {
        Msg::SelectMailbox(name) => model.open_mailbox(&name),
        Msg::SelectThread(idx) => model.open_thread(idx),
        Msg::ScrollList(lines) => {
            let max = model.threads.len().saturating_sub(1);
            if lines > 0 {
                model.list_scroll = (model.list_scroll + lines as usize).min(max);
            } else {
                model.list_scroll = model.list_scroll.saturating_sub((-lines) as usize);
            }
        }
        Msg::ScrollRead(lines) => {
            const STEP: f32 = 40.0;
            let max = view::reading_content_height(&model) - READ_VIEWPORT_H;
            let max = max.max(0.0);
            model.read_scroll = (model.read_scroll + lines as f32 * STEP).clamp(0.0, max);
        }
        Msg::Refresh => {
            if let Some(name) = model.selected_mailbox.clone() {
                model.open_mailbox(&name);
            }
        }
        Msg::ComposeOpen => model.compose = Some(Compose::empty()),
        Msg::ComposeClose => model.compose = None,
        Msg::ComposeReply => {
            if let Some(original) = model
                .current_thread()
                .and_then(|t| t.message_ids.last())
                .and_then(|id| model.store.message(id))
            {
                let out = OutgoingMessage::reply_to(original, model.me.clone());
                let mut c = Compose::empty();
                c.to.set_text(
                    out.to.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "),
                );
                c.subject.set_text(out.subject);
                c.focus = ComposeField::Body;
                c.in_reply_to = out.in_reply_to;
                c.references = out.references;
                model.compose = Some(c);
            }
        }
        Msg::ComposeForward => {
            if let Some(original) = model.current_newest().and_then(|id| model.store.message(&id)) {
                let out = OutgoingMessage::forward(original, model.me.clone());
                let mut c = Compose::empty();
                c.subject.set_text(out.subject);
                c.body.set_text(out.body_text);
                c.focus = ComposeField::To;
                model.compose = Some(c);
            }
        }
        Msg::ComposeToggleSign => {
            if let Some(c) = model.compose.as_mut() {
                c.sign = !c.sign;
            }
        }
        Msg::ToggleStar(id) => model.toggle_flag(&id, |f| Flags { flagged: !f.flagged, ..f }),
        Msg::ToggleSeen(id) => model.toggle_flag(&id, |f| Flags { seen: !f.seen, ..f }),
        Msg::DeleteThread => model.delete_current_thread(),
        Msg::ComposeFocus(field) => {
            if let Some(c) = model.compose.as_mut() {
                c.focus = field;
            }
        }
        Msg::ComposeKey(event) => {
            if let Some(c) = model.compose.as_mut() {
                if event.state == KeyState::Pressed {
                    match &event.key {
                        Key::Named(NamedKey::Escape) => model.compose = None,
                        Key::Named(NamedKey::Tab) => c.focus = c.focus.next(),
                        _ => {
                            c.focused_mut().apply_key(&event);
                        }
                    }
                }
            }
        }
        Msg::ComposeSend => model = send_compose(model),
        Msg::SearchFocus(on) => model.search_focused = on,
        Msg::SearchMode(semantic) => {
            model.search_semantic = semantic;
            if semantic {
                model.status = "búsqueda semántica (rimay): pendiente — usando exacta".into();
            }
        }
        Msg::ViewRich(_id) => {
            model.status = "HTML enriquecido vía puriy: pendiente (texto despojado por ahora)".into();
        }
        Msg::OpenMessage(id) => {
            model.open_message(&id);
            model.search_focused = false;
        }
        Msg::SearchKey(event) => {
            if event.state == KeyState::Pressed {
                match &event.key {
                    Key::Named(NamedKey::Escape) => {
                        model.search.clear();
                        model.search_focused = false;
                    }
                    Key::Named(NamedKey::Enter) => {
                        // Enter abre el primer resultado.
                        let q = model.search.text();
                        let first = model.store.search(&q).first().map(|m| m.id.clone());
                        if let Some(id) = first {
                            model.open_message(&id);
                            model.search_focused = false;
                        }
                    }
                    _ => {
                        model.search.apply_key(&event);
                        model.list_scroll = 0;
                    }
                }
            }
        }
    }
    model
}

/// Arma el `OutgoingMessage` desde el compositor, lo envía por el backend y
/// refresca el buzón `Sent` si está abierto. Sin destinatario válido, no hace
/// nada salvo avisar.
fn send_compose(mut model: Model) -> Model {
    let Some(c) = model.compose.as_ref() else { return model };
    let to = parse_address_list(&c.to.text());
    if to.is_empty() {
        model.status = "no se puede enviar: falta un destinatario válido".into();
        return model;
    }
    let signed = c.sign;
    let out = OutgoingMessage {
        from: model.me.clone(),
        to,
        cc: parse_address_list(&c.cc.text()),
        bcc: Vec::new(),
        subject: c.subject.text(),
        body_text: c.body.text(),
        body_html: None,
        in_reply_to: c.in_reply_to.clone(),
        references: c.references.clone(),
    };
    match model.backend.send(&out) {
        Ok(_id) => {
            model.compose = None;
            model.status = if signed { "enviado · firmado (Ed25519)" } else { "enviado" }.into();
            // Si estamos viendo Sent, reflejar el envío recién aterrizado.
            if model.selected_mailbox.as_deref() == Some("Sent") {
                model.open_mailbox("Sent");
            }
        }
        Err(e) => model.status = format!("error al enviar: {e}"),
    }
    model
}

/// El árbol de la UI. Delega en el módulo `view`.
pub fn view(model: &Model) -> View<Msg> {
    view::root(model)
}

/// El overlay del compositor, si está abierto.
pub fn view_overlay(model: &Model) -> Option<View<Msg>> {
    model.compose.as_ref().map(|c| view::compose_modal(model, c))
}

/// Atajos globales. Con el compositor abierto, todas las teclas van a él
/// (salvo que `update` las interprete como Esc/Tab). Cerrado: `c` redacta,
/// `r` responde, `F5` refresca.
pub fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
    if model.compose.is_some() {
        return Some(Msg::ComposeKey(event.clone()));
    }
    if model.search_focused {
        return Some(Msg::SearchKey(event.clone()));
    }
    if event.state != KeyState::Pressed || event.modifiers.ctrl || event.modifiers.alt {
        return None;
    }
    match &event.key {
        Key::Named(NamedKey::F5) => Some(Msg::Refresh),
        Key::Named(NamedKey::Delete) => model.current_thread().map(|_| Msg::DeleteThread),
        Key::Character(ch) if ch.as_str() == "/" => Some(Msg::SearchFocus(true)),
        Key::Character(ch) if ch.eq_ignore_ascii_case("c") => Some(Msg::ComposeOpen),
        Key::Character(ch) if ch.eq_ignore_ascii_case("r") => {
            model.current_thread().map(|_| Msg::ComposeReply)
        }
        Key::Character(ch) if ch.eq_ignore_ascii_case("f") => {
            model.current_thread().map(|_| Msg::ComposeForward)
        }
        _ => None,
    }
}

/// Rueda del mouse → scroll de la lista de hilos.
pub fn on_wheel(
    _model: &Model,
    delta: WheelDelta,
    cursor: (f32, f32),
    _mods: llimphi_ui::Modifiers,
) -> Option<Msg> {
    let lines = (delta.y * 3.0).round() as i32;
    if lines == 0 {
        return None;
    }
    // El panel de lectura es la franja derecha; el resto scrollea la lista.
    if cursor.0 > READING_PANEL_X {
        Some(Msg::ScrollRead(-lines))
    } else {
        Some(Msg::ScrollList(-lines))
    }
}

/// X (px) a partir de la cual empieza el panel de lectura (ancho buzones +
/// ancho hilos). Decide a qué panel va la rueda del mouse.
const READING_PANEL_X: f32 = 200.0 + 340.0;
/// Alto aproximado del viewport de lectura (ventana − toolbar − header −
/// status). Sólo se usa para acotar el scroll; el sobre-scroll se autocorrige.
const READ_VIEWPORT_H: f32 = 600.0;

// Reexport para que el binario del demo arme su `impl App` con tipos estables.
pub use llimphi_ui::Handle;
