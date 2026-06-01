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
    parse_address_list, Address, MailBackend, MailStore, MessageId, OutgoingMessage, Thread,
};

mod view;

/// Campo enfocado del formulario de redacción.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeField {
    To,
    Subject,
    Body,
}

impl ComposeField {
    /// El siguiente campo en el ciclo Tab (To → Subject → Body → To).
    fn next(self) -> Self {
        match self {
            ComposeField::To => ComposeField::Subject,
            ComposeField::Subject => ComposeField::Body,
            ComposeField::Body => ComposeField::To,
        }
    }
}

/// Estado del compositor (modal de redacción). Vive sólo mientras se redacta;
/// `None` en el modelo significa "no hay redacción abierta".
pub struct Compose {
    pub to: TextInputState,
    pub subject: TextInputState,
    pub body: TextInputState,
    pub focus: ComposeField,
    /// Si es una respuesta, el mensaje original (alimenta `In-Reply-To` y
    /// `References`). El asunto/destinatario ya vienen prellenados.
    pub in_reply_to: Option<MessageId>,
    pub references: Vec<MessageId>,
}

impl Compose {
    fn empty() -> Self {
        Self {
            to: TextInputState::new(),
            subject: TextInputState::new(),
            body: TextInputState::new(),
            focus: ComposeField::To,
            in_reply_to: None,
            references: Vec::new(),
        }
    }

    /// El input del campo enfocado, para rutear las teclas.
    fn focused_mut(&mut self) -> &mut TextInputState {
        match self.focus {
            ComposeField::To => &mut self.to,
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
    /// Redacción en curso; `None` si el modal está cerrado.
    compose: Option<Compose>,
    /// Última línea de estado (resultado de un sync/envío).
    pub status: String,
    pub theme: Theme,
}

impl Model {
    /// Construye el modelo sobre `backend`, sincroniza los buzones y abre el
    /// primero (típicamente `INBOX`). Best-effort: si un sync falla, el panel
    /// queda vacío y el estado lo dice.
    pub fn new(backend: Box<dyn MailBackend>, me: Address, theme: Theme) -> Self {
        let mut store = MailStore::new();
        let mut status = String::from("paloma · sin sincronizar");
        if let Err(e) = store.sync_mailboxes(&*backend) {
            status = format!("error al listar buzones: {e}");
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
            compose: None,
            status,
            theme,
        };
        if let Some(name) = first {
            model.open_mailbox(&name);
        }
        model
    }

    /// Trae y abre `mailbox`: sincroniza sus mensajes, reconstruye hilos y
    /// limpia la selección de hilo.
    fn open_mailbox(&mut self, mailbox: &str) {
        match self.store.sync_messages(&*self.backend, mailbox) {
            Ok(()) => {
                self.threads = self.store.threads(mailbox);
                self.selected_mailbox = Some(mailbox.to_string());
                self.selected_thread = None;
                self.list_scroll = 0;
                self.status = format!(
                    "{mailbox} · {} hilos · {} sin leer",
                    self.threads.len(),
                    self.store.unread_count(mailbox),
                );
            }
            Err(e) => {
                self.status = format!("error al traer {mailbox}: {e}");
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
        self.status = format!("{mailbox} · {} sin leer", self.store.unread_count(&mailbox));
    }

    /// El hilo abierto, si lo hay.
    fn current_thread(&self) -> Option<&Thread> {
        self.selected_thread.and_then(|i| self.threads.get(i))
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
    /// Abrir el compositor en blanco.
    ComposeOpen,
    /// Abrir el compositor como respuesta al último mensaje del hilo abierto.
    ComposeReply,
    /// Cerrar el compositor sin enviar.
    ComposeClose,
    /// Cambiar el campo enfocado del compositor.
    ComposeFocus(ComposeField),
    /// Tecla mientras el compositor está abierto (va al campo enfocado).
    ComposeKey(KeyEvent),
    /// Enviar lo redactado.
    ComposeSend,
    /// Re-traer el buzón activo desde el backend.
    Refresh,
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
    let out = OutgoingMessage {
        from: model.me.clone(),
        to,
        cc: Vec::new(),
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
            model.status = "enviado".into();
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
    if event.state != KeyState::Pressed || event.modifiers.ctrl || event.modifiers.alt {
        return None;
    }
    match &event.key {
        Key::Named(NamedKey::F5) => Some(Msg::Refresh),
        Key::Character(ch) if ch.eq_ignore_ascii_case("c") => Some(Msg::ComposeOpen),
        Key::Character(ch) if ch.eq_ignore_ascii_case("r") => {
            model.current_thread().map(|_| Msg::ComposeReply)
        }
        _ => None,
    }
}

/// Rueda del mouse → scroll de la lista de hilos.
pub fn on_wheel(
    _model: &Model,
    delta: WheelDelta,
    _cursor: (f32, f32),
    _mods: llimphi_ui::Modifiers,
) -> Option<Msg> {
    let lines = (delta.y * 3.0).round() as i32;
    (lines != 0).then_some(Msg::ScrollList(-lines))
}

// Reexport para que el binario del demo arme su `impl App` con tipos estables.
pub use llimphi_ui::Handle;
