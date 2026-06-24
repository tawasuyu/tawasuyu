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
    parse_address_list, Address, Flags, MailBackend, MailCuerpo, MailSignature, MailStore, Message,
    MessageId, OutgoingMessage, Thread,
};

pub mod demo;
mod view;

/// Nombre del buzón local donde aterriza el correo del rail P2P (Eje 3.B).
pub const SUYU_MAILBOX: &str = "Suyu";

/// Motor de **búsqueda por significado**, inyectado por el anfitrión.
///
/// El cómputo de embeddings es async y pega contra el `rimay-verbo-daemon`; no
/// puede vivir en el bucle Elm síncrono. Por eso el motor recibe el [`Handle`]
/// y **despacha [`Msg::SemanticResults`]** cuando termina, en vez de devolver.
/// `Model::semantic == None` (demos, o sin daemon) ⇒ el modo semántico cae a la
/// búsqueda exacta. La implementación concreta vive en el anfitrión
/// (`paloma-app`), sobre `paloma-semantic` + un runtime async.
pub trait SemanticEngine: Send {
    /// Embebe lo que falte de `corpus`, lo rankea contra `query` por similitud
    /// coseno, y despacha `Msg::SemanticResults(ids ordenados)` por `handle`.
    /// No bloquea: corre todo en su propio runtime.
    fn search(&self, query: String, corpus: Vec<Message>, handle: llimphi_ui::Handle<Msg>);
}

/// Asistente LLM, inyectado por el anfitrión. Igual que [`SemanticEngine`]: el
/// trabajo es async (pega contra el modelo vía `pluma-llm`, local con Ollama o
/// remoto), corre fuera del hilo de UI y **despacha** el resultado como `Msg`.
/// `Model::llm == None` (sin backend / sin opt-in) ⇒ los botones ✨ no aparecen.
/// La implementación concreta vive en `paloma-app`.
pub trait LlmAssistant: Send {
    /// Resume el hilo (`thread_text`, texto plano) → despacha `Msg::LlmSummary`
    /// con el resumen, o `Msg::LlmError` si falla.
    fn summarize(&self, thread_text: String, handle: llimphi_ui::Handle<Msg>);
    /// Redacta un borrador de respuesta al hilo → despacha `Msg::LlmDraft` con
    /// el cuerpo, o `Msg::LlmError` si falla.
    fn draft_reply(&self, thread_text: String, handle: llimphi_ui::Handle<Msg>);
    /// Traduce `text` a `target_lang` (multilienzo) → despacha
    /// `Msg::LlmTranslation { lang, text }`, o `Msg::LlmError` si falla.
    fn translate(&self, text: String, target_lang: String, handle: llimphi_ui::Handle<Msg>);
}

/// Identidad firmante del usuario, inyectada por el anfitrión (Eje 3:
/// soberanía). Firma los bytes canónicos de un saliente con la `Keypair` de
/// `agora`. Síncrono y rápido (Ed25519); no necesita runtime. `Model::signer ==
/// None` ⇒ los salientes van sin firmar aunque se tilde "Firmar".
pub trait MailSigner: Send {
    /// Firma `canonical`; devuelve `(pubkey 32 bytes, firma 64 bytes)`.
    fn sign(&self, canonical: &[u8]) -> ([u8; 32], [u8; 64]);
}

/// Enlace al **rail soberano** P2P (Eje 3.B), inyectado por el anfitrión. Envía
/// un `Message` a una identidad `agora` (sin SMTP): el anfitrión lo sella
/// (`paloma-rail`) y lo entrega por el transporte. La recepción es push: el
/// anfitrión despacha `Msg::RailReceived` cuando llega un sobre. `Model::rail ==
/// None` ⇒ no hay buzón "Suyu" ni enrutado por el rail.
pub trait RailLink: Send {
    /// Entrega `msg` a la identidad `to` (32 bytes de clave pública).
    fn send(&self, to: [u8; 32], msg: &Message) -> Result<(), String>;
    /// La dirección del rail de **este** usuario (`<hex>@suyu`), para compartir.
    fn my_address(&self) -> String;
}

/// Crea **avales** (web-of-trust, Eje 3): firma con la identidad del usuario que
/// `subject` (clave pública) es alguien conocido. Lo inyecta el anfitrión (tiene
/// la `Keypair`). `None` ⇒ no se puede avalar desde la UI.
pub trait Voucher: Send {
    /// Firma un aval por `subject` con etiqueta `display`; devuelve la atestación.
    fn vouch(&self, subject: [u8; 32], display: &str) -> paloma_trust::Attestation;
}

/// Confianza de identidad de un remitente (pubkey↔persona).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderTrust {
    /// Está en tu libreta: identidad conocida directa.
    Direct(String),
    /// No está, pero un contacto tuyo lo avaló (transitivo a un salto).
    Vouched(String),
    /// Firmado e íntegro, pero de identidad desconocida (TOFU).
    Unknown,
}

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
    /// Lienzos multilienzo (Eje 4): versiones del cuerpo en otros idiomas que el
    /// LLM derivó y viajarán con el mensaje. Vacío = sólo el cuerpo principal.
    pub cuerpos: Vec<MailCuerpo>,
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
            cuerpos: Vec::new(),
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
    /// Modo de búsqueda: `false` = exacta (texto), `true` = semántica
    /// (embeddings vía `rimay`). Sin [`Model::semantic`] inyectado, el modo
    /// semántico cae a la exacta.
    search_semantic: bool,
    /// Motor de búsqueda por significado, inyectado por el anfitrión. `None` en
    /// demos o si no hay daemon de embeddings.
    semantic: Option<Box<dyn SemanticEngine>>,
    /// Última tanda de resultados semánticos (ids rankeados), o `None` si no se
    /// buscó aún / la consulta se vació. Llega async vía `Msg::SemanticResults`.
    semantic_results: Option<Vec<MessageId>>,
    /// `true` mientras hay una búsqueda semántica en vuelo (embebiendo/rankeando).
    semantic_busy: bool,
    /// Asistente LLM inyectado por el anfitrión. `None` sin backend disponible.
    llm: Option<Box<dyn LlmAssistant>>,
    /// Resumen LLM del hilo abierto (banner sobre la lectura). Se limpia al
    /// cambiar de hilo. `None` = no se pidió / se descartó.
    summary: Option<String>,
    /// `true` mientras se está resumiendo el hilo.
    summary_busy: bool,
    /// `true` mientras se redacta un borrador de respuesta con el LLM.
    draft_busy: bool,
    /// Identidad firmante (Ed25519/agora), inyectada por el anfitrión. `None` =
    /// sin identidad → los salientes no se firman.
    signer: Option<Box<dyn MailSigner>>,
    /// Enlace al rail P2P (Eje 3.B), inyectado por el anfitrión. `None` = sin
    /// buzón "Suyu" ni enrutado por el rail.
    rail: Option<Box<dyn RailLink>>,
    /// Idioma del lector (de wawa-config): el lienzo que se autoselecciona al
    /// leer (multilienzo, Eje 4).
    reader_lang: String,
    /// Idioma elegido a mano para ver el hilo abierto; `None` = auto (lee
    /// `reader_lang`). Lo fija el selector de lienzos. Se limpia al cambiar hilo.
    view_lang: Option<String>,
    /// Libreta de contactos: alias → dirección (correo o rail). Resuelve el
    /// campo "Para" al redactar. Vacía si no hay archivo.
    contacts: paloma_contacts::Contactbook,
    /// Ruta del archivo de contactos (para persistir altas). `None` = sin disco.
    contacts_path: Option<std::path::PathBuf>,
    /// Red de avales (web-of-trust transitiva sobre agora).
    trust: paloma_trust::TrustStore,
    /// Ruta del archivo de avales (para persistir). `None` = sin disco.
    trust_path: Option<std::path::PathBuf>,
    /// Generador de avales (inyectado; tiene la `Keypair`). `None` = no avalar.
    voucher: Option<Box<dyn Voucher>>,
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
        // Inicializar rimay-localize (idempotente si ya fue llamado).
        rimay_localize::init();
        // Cargar el idioma global configurado en wawa-config (también el idioma
        // del lector para el multilienzo).
        let reader_lang = wawa_config::WawaConfig::load().lang;
        let _ = rimay_localize::set_locale(&reader_lang);

        let mut store = MailStore::new();
        // Offline-first: pintar lo cacheado antes de tocar la red.
        if let Some(d) = &db {
            let cached = d.load_mailboxes(&account_id);
            if !cached.is_empty() {
                store.ingest_mailboxes(cached);
            }
        }
        // Refrescar la lista de buzones de red; si funciona, persistirla.
        let mut status = rimay_localize::t("paloma-status-init");
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
            semantic: None,
            semantic_results: None,
            semantic_busy: false,
            llm: None,
            summary: None,
            summary_busy: false,
            draft_busy: false,
            signer: None,
            rail: None,
            reader_lang,
            view_lang: None,
            contacts: paloma_contacts::Contactbook::new(),
            contacts_path: None,
            trust: paloma_trust::TrustStore::new(),
            trust_path: None,
            voucher: None,
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

    /// Inyecta el motor de búsqueda por significado (lo hace el anfitrión tras
    /// construir el modelo). Sin esto, el modo semántico cae a la exacta.
    pub fn attach_semantic(&mut self, engine: Box<dyn SemanticEngine>) {
        self.semantic = engine.into();
    }

    /// Todos los mensajes cacheados, deduplicados por `Message-ID` (un mismo
    /// mensaje puede aparecer en varios buzones, p. ej. etiquetas de Gmail). Es
    /// el corpus que se le pasa al motor semántico para embeber/rankear.
    fn corpus(&self) -> Vec<Message> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for mb in self.store.mailboxes() {
            for m in self.store.messages(&mb.name) {
                if seen.insert(m.id.clone()) {
                    out.push(m.clone());
                }
            }
        }
        out
    }

    /// Inyecta el asistente LLM (lo hace el anfitrión tras construir el modelo).
    /// Sin esto, los botones ✨ (resumir / borrador) no se muestran.
    pub fn attach_llm(&mut self, assistant: Box<dyn LlmAssistant>) {
        self.llm = assistant.into();
    }

    /// ¿Hay asistente LLM disponible? (Gobierna si la UI muestra los botones ✨.)
    pub fn llm_available(&self) -> bool {
        self.llm.is_some()
    }

    /// Inyecta la identidad firmante (Ed25519/agora). Sin esto, "Firmar" en el
    /// compositor no produce firma.
    pub fn attach_signer(&mut self, signer: Box<dyn MailSigner>) {
        self.signer = signer.into();
    }

    /// ¿Hay identidad para firmar salientes?
    pub fn signer_available(&self) -> bool {
        self.signer.is_some()
    }

    /// Inyecta el enlace al rail P2P y crea el buzón local "Suyu" (donde
    /// aterriza el correo soberano recibido). Lo hace el anfitrión al arrancar.
    pub fn attach_rail(&mut self, rail: Box<dyn RailLink>) {
        self.store.pin_mailbox(paloma_core::Mailbox::new(SUYU_MAILBOX));
        self.rail = rail.into();
    }

    /// ¿Hay rail P2P disponible? (Gobierna el buzón "Suyu" y el enrutado.)
    pub fn rail_available(&self) -> bool {
        self.rail.is_some()
    }

    /// La dirección del rail de este usuario (`<hex>@suyu`), para compartir con
    /// contactos. `None` si no hay rail.
    pub fn my_rail_address(&self) -> Option<String> {
        self.rail.as_ref().map(|r| r.my_address())
    }

    /// Inyecta la libreta de contactos cargada del disco y su ruta (para
    /// persistir altas). Lo hace el anfitrión al arrancar.
    pub fn set_contacts(&mut self, book: paloma_contacts::Contactbook, path: std::path::PathBuf) {
        self.contacts = book;
        self.contacts_path = Some(path);
    }

    /// Cuántos contactos hay en la libreta.
    pub fn contacts_len(&self) -> usize {
        self.contacts.len()
    }

    /// Inyecta la red de avales cargada del disco + su ruta, y el generador de
    /// avales (con la `Keypair`). Lo hace el anfitrión al arrancar.
    pub fn set_trust(
        &mut self,
        store: paloma_trust::TrustStore,
        path: std::path::PathBuf,
        voucher: Box<dyn Voucher>,
    ) {
        self.trust = store;
        self.trust_path = Some(path);
        self.voucher = voucher.into();
    }

    /// Las claves públicas de los contactos del rail (en las que se confía
    /// directo) — base de la confianza transitiva.
    fn direct_rail_keys(&self) -> Vec<[u8; 32]> {
        self.contacts
            .all()
            .iter()
            .filter_map(|c| paloma_rail::parse_rail_address(&c.address))
            .collect()
    }

    /// Confianza de identidad del remitente de `m` (pubkey↔persona): directa
    /// (en la libreta), avalada (un contacto lo vouchea, transitivo a un salto),
    /// o desconocida. Para el rail la dirección es la clave pública, así que esto
    /// + firma `Verified` = identidad criptográfica.
    pub fn sender_trust(&self, m: &Message) -> SenderTrust {
        if let Some(name) = self.contacts.name_for(&m.from.email) {
            return SenderTrust::Direct(name.to_string());
        }
        if let Some(pk) = paloma_rail::parse_rail_address(&m.from.email) {
            let trusted = self.direct_rail_keys();
            if let Some(attester) = self.trust.vouched_by(&pk, &trusted) {
                let by = self
                    .contacts
                    .name_for(&paloma_rail::rail_address(&attester))
                    .unwrap_or("?")
                    .to_string();
                return SenderTrust::Vouched(by);
            }
        }
        SenderTrust::Unknown
    }

    /// Idioma efectivo de lectura (multilienzo): el elegido a mano, si no el del
    /// lector. Decide qué lienzo muestra el panel de lectura.
    pub fn effective_view_lang(&self) -> &str {
        self.view_lang.as_deref().unwrap_or(&self.reader_lang)
    }

    /// El idioma elegido a mano para ver (None = auto).
    pub fn view_lang(&self) -> Option<&str> {
        self.view_lang.as_deref()
    }

    /// El resumen LLM del hilo abierto, si se pidió.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// ¿Se está resumiendo el hilo ahora mismo?
    pub fn summary_busy(&self) -> bool {
        self.summary_busy
    }

    /// ¿Se está redactando un borrador con el LLM ahora mismo?
    pub fn draft_busy(&self) -> bool {
        self.draft_busy
    }

    /// El hilo abierto como texto plano (asunto + cada mensaje: de + cuerpo),
    /// para pasárselo al asistente LLM. `None` si no hay hilo abierto.
    fn thread_plain_text(&self) -> Option<String> {
        let thread = self.current_thread()?;
        let mut out = String::new();
        if !thread.subject.is_empty() {
            out.push_str("Asunto: ");
            out.push_str(&thread.subject);
            out.push_str("\n\n");
        }
        for id in &thread.message_ids {
            if let Some(m) = self.store.message(id) {
                out.push_str("De: ");
                out.push_str(&m.from.to_string());
                out.push('\n');
                out.push_str(&m.display_body());
                out.push_str("\n\n---\n\n");
            }
        }
        Some(out)
    }

    /// ¿Está activo el modo semántico **y** hay motor inyectado? (Si no, la UI
    /// muestra los resultados exactos.)
    pub fn semantic_active(&self) -> bool {
        self.search_semantic && self.semantic.is_some()
    }

    /// ¿Hay una búsqueda semántica en vuelo?
    pub fn semantic_busy(&self) -> bool {
        self.semantic_busy
    }

    /// Los mensajes de la última tanda semántica, en orden de relevancia y ya
    /// resueltos a `&Message` (descarta ids que ya no estén en la caché).
    /// `None` si todavía no se buscó nada por significado.
    pub fn semantic_hits(&self) -> Option<Vec<&Message>> {
        self.semantic_results
            .as_ref()
            .map(|ids| ids.iter().filter_map(|id| self.store.message(id)).collect())
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
        // Buzones locales (rail "Suyu"): no existen en el backend IMAP; se leen
        // directo de la caché local, sin tocar la red.
        if self.store.is_pinned(mailbox) {
            self.threads = self.store.threads(mailbox);
            self.selected_mailbox = Some(mailbox.to_string());
            self.selected_thread = None;
            self.list_scroll = 0;
            self.status = format!("{mailbox} · {} hilos (rail P2P)", self.threads.len());
            return;
        }
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
        // El resumen LLM es por hilo: al cambiar de hilo, se descarta.
        self.summary = None;
        self.summary_busy = false;
        // El idioma de lectura vuelve a auto (el del lector) en cada hilo.
        self.view_lang = None;
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
    /// Resultados de una búsqueda semántica (ids rankeados), despachados por el
    /// motor cuando termina de embeber/rankear fuera del hilo de UI.
    SemanticResults(Vec<MessageId>),
    /// Pedir el render HTML enriquecido de un mensaje (gancho de puriy).
    ViewRich(MessageId),
    /// Pedir al LLM un resumen del hilo abierto.
    Summarize,
    /// Resumen del hilo, devuelto por el asistente LLM.
    LlmSummary(String),
    /// Descartar el banner de resumen.
    DismissSummary,
    /// Pedir al LLM un borrador de respuesta al hilo abierto.
    DraftReply,
    /// Borrador de respuesta del LLM: abre el compositor con el cuerpo redactado.
    LlmDraft(String),
    /// Falla del asistente LLM (la muestra la barra de estado).
    LlmError(String),
    /// Un mensaje llegó por el rail P2P: se ingiere en el buzón "Suyu".
    RailReceived(Message),
    /// Mostrar la propia dirección del rail en la barra de estado (para copiar).
    ShowRailAddress,
    /// Pedir al LLM derivar un lienzo del cuerpo en redacción a `lang` (Eje 4).
    DeriveCuerpo(String),
    /// Lienzo traducido devuelto por el LLM: se agrega a la redacción.
    LlmTranslation { lang: String, text: String },
    /// Elegir el idioma con el que se lee el hilo (`None` = auto). Selector de
    /// lienzos del panel de lectura.
    SetViewLang(Option<String>),
    /// Guardar el remitente del hilo abierto en la libreta de contactos.
    SaveSenderContact,
    /// Avalar (web-of-trust) al remitente del hilo abierto: firmás que su
    /// identidad es conocida, para que tus contactos la reconozcan por vos.
    VouchSender,
}

/// Dispara una búsqueda por significado: arma el corpus y se lo entrega al
/// motor, que embebe/rankea async y despacha `Msg::SemanticResults`. Sin motor
/// o con consulta vacía, no hace nada (limpia resultados).
fn fire_semantic(model: &mut Model, handle: &llimphi_ui::Handle<Msg>) {
    let query = model.search.text();
    if query.trim().is_empty() {
        model.semantic_results = None;
        model.semantic_busy = false;
        return;
    }
    let Some(engine) = model.semantic.as_ref() else { return };
    let corpus = model.corpus();
    model.semantic_busy = true;
    model.semantic_results = None;
    model.status = rimay_localize::t("paloma-status-search-semantic-running");
    engine.search(query, corpus, handle.clone());
}

/// Arma un compositor de **respuesta** al hilo abierto (Para/Asunto/References
/// prellenados, cuerpo vacío). `None` si no hay hilo o mensaje. Lo comparten la
/// respuesta manual (`ComposeReply`) y el borrador LLM (`LlmDraft`).
fn reply_compose(model: &Model) -> Option<Compose> {
    let original = model
        .current_thread()
        .and_then(|t| t.message_ids.last())
        .and_then(|id| model.store.message(id))?;
    let out = OutgoingMessage::reply_to(original, model.me.clone());
    let mut c = Compose::empty();
    c.to.set_text(out.to.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
    c.subject.set_text(out.subject);
    c.focus = ComposeField::Body;
    c.in_reply_to = out.in_reply_to;
    c.references = out.references;
    Some(c)
}

/// La transición Elm. Toma el modelo por valor y lo devuelve mutado.
pub fn update(mut model: Model, msg: Msg, handle: &llimphi_ui::Handle<Msg>) -> Model {
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
            if let Some(c) = reply_compose(&model) {
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
            model.semantic_results = None;
            model.semantic_busy = false;
            if semantic {
                if model.semantic.is_some() {
                    // Con motor y consulta ya escrita, buscá de una.
                    if model.search.text().trim().is_empty() {
                        model.status = rimay_localize::t("paloma-status-search-semantic");
                    } else {
                        fire_semantic(&mut model, handle);
                    }
                } else {
                    // Sin motor: el modo semántico cae a la exacta.
                    model.status = rimay_localize::t("paloma-status-search-semantic-fallback");
                }
            }
        }
        Msg::SemanticResults(ids) => {
            model.semantic_busy = false;
            let n = ids.len();
            model.semantic_results = Some(ids);
            model.status =
                rimay_localize::t_args("paloma-status-search-semantic-done", &[("n", n.to_string().into())]);
        }
        Msg::ViewRich(_id) => {
            model.status = rimay_localize::t("paloma-status-view-rich");
        }
        Msg::Summarize => {
            if let (Some(llm), Some(text)) = (model.llm.as_ref(), model.thread_plain_text()) {
                model.summary_busy = true;
                model.summary = None;
                model.status = rimay_localize::t("paloma-status-llm-summarizing");
                llm.summarize(text, handle.clone());
            }
        }
        Msg::LlmSummary(s) => {
            model.summary = Some(s);
            model.summary_busy = false;
            model.status = rimay_localize::t("paloma-status-llm-summary-done");
        }
        Msg::DismissSummary => model.summary = None,
        Msg::DraftReply => {
            if let (Some(llm), Some(text)) = (model.llm.as_ref(), model.thread_plain_text()) {
                model.draft_busy = true;
                model.status = rimay_localize::t("paloma-status-llm-drafting");
                llm.draft_reply(text, handle.clone());
            }
        }
        Msg::LlmDraft(body) => {
            model.draft_busy = false;
            if let Some(mut c) = reply_compose(&model) {
                c.body.set_text(body);
                model.compose = Some(c);
                model.status = rimay_localize::t("paloma-status-llm-draft-done");
            }
        }
        Msg::LlmError(e) => {
            model.summary_busy = false;
            model.draft_busy = false;
            model.status = e;
        }
        Msg::RailReceived(mut msg) => {
            msg.mailbox = SUYU_MAILBOX.to_string();
            model.store.add_message(SUYU_MAILBOX, msg);
            // Si estamos mirando "Suyu", refrescar la lista de hilos.
            if model.selected_mailbox.as_deref() == Some(SUYU_MAILBOX) {
                model.threads = model.store.threads(SUYU_MAILBOX);
            }
            model.status = rimay_localize::t("paloma-status-rail-received");
        }
        Msg::ShowRailAddress => {
            if let Some(addr) = model.my_rail_address() {
                model.status = format!("tu dirección del rail: {addr}");
            }
        }
        Msg::DeriveCuerpo(lang) => {
            // Multilienzo: el LLM traduce el cuerpo en redacción a `lang`.
            if let (Some(llm), Some(c)) = (model.llm.as_ref(), model.compose.as_ref()) {
                let body = c.body.text();
                if !body.trim().is_empty() {
                    model.draft_busy = true;
                    model.status = rimay_localize::t("paloma-status-llm-translating");
                    llm.translate(body, lang, handle.clone());
                }
            }
        }
        Msg::LlmTranslation { lang, text } => {
            model.draft_busy = false;
            if let Some(c) = model.compose.as_mut() {
                // Reemplaza el lienzo de ese idioma si ya existía.
                c.cuerpos.retain(|x| !x.lang.eq_ignore_ascii_case(&lang));
                c.cuerpos.push(MailCuerpo { lang: lang.clone(), tone: None, body_text: text });
                model.status =
                    rimay_localize::t("paloma-status-llm-lienzo-done").replace("{lang}", &lang);
            }
        }
        Msg::SetViewLang(lang) => model.view_lang = lang,
        Msg::SaveSenderContact => {
            // Guarda el remitente del mensaje más reciente del hilo abierto.
            if let Some(m) = model.current_newest().and_then(|id| model.store.message(&id)) {
                let addr = m.from.email.clone();
                let name = m.from.display_name().to_string();
                let name = if name.is_empty() || name == addr { addr.clone() } else { name };
                let nuevo = model.contacts.upsert(&name, &addr);
                if let Some(p) = &model.contacts_path {
                    let _ = model.contacts.save(p);
                }
                model.status = rimay_localize::t_args(
                    if nuevo { "paloma-status-contact-added" } else { "paloma-status-contact-updated" },
                    &[("name", name.into())],
                );
            }
        }
        Msg::VouchSender => {
            // Avalar al remitente: firma que su identidad (clave del rail) es
            // conocida. Sólo aplica a remitentes con identidad del rail.
            let info = model.current_newest().and_then(|id| model.store.message(&id)).and_then(|m| {
                paloma_rail::parse_rail_address(&m.from.email)
                    .map(|pk| (pk, m.from.display_name().to_string(), m.from.email.clone()))
            });
            match (info, model.voucher.as_ref()) {
                (Some((pk, name, addr)), Some(voucher)) => {
                    let display = if name.is_empty() || name == addr { addr } else { name };
                    let aval = voucher.vouch(pk, &display);
                    if model.trust.add(aval) {
                        if let Some(p) = &model.trust_path {
                            let _ = model.trust.save(p);
                        }
                    }
                    model.status = rimay_localize::t_args("paloma-status-vouched", &[("name", display.into())]);
                }
                (None, _) => model.status = rimay_localize::t("paloma-status-vouch-not-rail"),
                (_, None) => {}
            }
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
                        if model.semantic_active() {
                            // En semántico, Enter dispara la búsqueda por
                            // significado (el resultado llega async).
                            fire_semantic(&mut model, handle);
                        } else {
                            // Exacta: Enter abre el primer resultado.
                            let q = model.search.text();
                            let first = model.store.search(&q).first().map(|m| m.id.clone());
                            if let Some(id) = first {
                                model.open_message(&id);
                                model.search_focused = false;
                            }
                        }
                    }
                    _ => {
                        model.search.apply_key(&event);
                        model.list_scroll = 0;
                        // En semántico, tipear invalida la tanda anterior: hay
                        // que volver a presionar Enter para re-rankear.
                        if model.semantic_active() {
                            model.semantic_results = None;
                        }
                    }
                }
            }
        }
    }
    model
}

/// Un `Message-ID` fresco para un mensaje del rail (sin servidor que lo asigne).
fn fresh_rail_id() -> MessageId {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    MessageId(format!("<rail-{nanos}@suyu>"))
}

/// Arma y envía lo redactado, **enrutando por destinatario**: las direcciones
/// `@suyu` van por el rail P2P (selladas Ed25519, sin SMTP) y el resto por SMTP.
/// Sin destinatario válido, sólo avisa.
fn send_compose(mut model: Model) -> Model {
    let Some(c) = model.compose.as_ref() else { return model };
    // Libreta: expandir alias del "Para" a sus direcciones antes de parsear.
    let to_expanded = model.contacts.expand(&c.to.text());
    let all_to = parse_address_list(&to_expanded);
    if all_to.is_empty() {
        model.status = rimay_localize::t("paloma-status-no-recipient");
        return model;
    }

    // Partir destinatarios: rail (`@suyu` → identidad agora) vs SMTP.
    let mut rail_to: Vec<[u8; 32]> = Vec::new();
    let mut smtp_to: Vec<Address> = Vec::new();
    for a in &all_to {
        match paloma_rail::parse_rail_address(&a.email) {
            Some(id) => rail_to.push(id),
            None => smtp_to.push(a.clone()),
        }
    }

    let want_sign = c.sign;
    let subject = c.subject.text();
    let body = c.body.text();
    let cc = parse_address_list(&c.cc.text());
    let in_reply_to = c.in_reply_to.clone();
    let references = c.references.clone();
    let cuerpos = c.cuerpos.clone(); // lienzos multilienzo que viajan con el mensaje

    let mut sent_any = false;
    let mut signed_smtp = false;
    let mut errs: Vec<String> = Vec::new();

    // --- Rail P2P: un Message nativo sellado por identidad ---
    if !rail_to.is_empty() {
        match model.rail.as_ref() {
            Some(rail) => {
                let msg = Message {
                    id: fresh_rail_id(),
                    from: model.me.clone(),
                    to: all_to.clone(),
                    cc: cc.clone(),
                    bcc: vec![],
                    subject: subject.clone(),
                    date: 0,
                    in_reply_to: in_reply_to.clone(),
                    references: references.clone(),
                    body_text: body.clone(),
                    body_html: None,
                    flags: Flags::default(),
                    signature: paloma_core::SignatureStatus::Unsigned,
                    mailbox: SUYU_MAILBOX.to_string(),
                    cuerpos: cuerpos.clone(),
                };
                for id in &rail_to {
                    match rail.send(*id, &msg) {
                        Ok(()) => sent_any = true,
                        Err(e) => errs.push(format!("rail: {e}")),
                    }
                }
            }
            None => errs.push("rail no disponible".into()),
        }
    }

    // --- SMTP: el resto de los destinatarios ---
    if !smtp_to.is_empty() {
        let mut out = OutgoingMessage {
            from: model.me.clone(),
            to: smtp_to,
            cc,
            bcc: Vec::new(),
            subject,
            body_text: body,
            body_html: None,
            in_reply_to,
            references,
            signature: None,
            cuerpos: cuerpos.clone(),
        };
        // Firma Ed25519 (agora) si se pidió y hay identidad.
        if want_sign {
            if let Some(signer) = model.signer.as_ref() {
                let (pubkey, sig) = signer.sign(&out.canonical_signing_bytes());
                out.signature = Some(MailSignature { pubkey, sig });
                signed_smtp = true;
            }
        }
        match model.backend.send(&out) {
            Ok(_id) => sent_any = true,
            Err(e) => errs.push(format!("smtp: {e}")),
        }
    }

    // --- Resultado ---
    if sent_any && errs.is_empty() {
        model.compose = None;
        model.status = if signed_smtp {
            rimay_localize::t("paloma-status-sent-signed")
        } else if want_sign && !smtp_to_was_empty(&all_to) {
            rimay_localize::t("paloma-status-sent-unsigned-nokey")
        } else if !rail_to.is_empty() {
            rimay_localize::t("paloma-status-rail-sent")
        } else {
            rimay_localize::t("paloma-status-sent")
        };
        if model.selected_mailbox.as_deref() == Some("Sent") {
            model.open_mailbox("Sent");
        }
    } else if sent_any {
        model.compose = None;
        model.status = format!("enviado parcialmente · {}", errs.join("; "));
    } else {
        model.status = format!("error al enviar: {}", errs.join("; "));
    }
    model
}

/// ¿La lista de destinatarios no tenía ninguno SMTP? (todos eran del rail).
fn smtp_to_was_empty(all_to: &[Address]) -> bool {
    all_to.iter().all(|a| paloma_rail::parse_rail_address(&a.email).is_some())
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

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_theme::Theme;
    use paloma_core::MockBackend;
    use std::sync::{Arc, Mutex};

    /// Enlace de rail que graba a quién y qué se le entregó.
    struct RecRail {
        sent: Arc<Mutex<Vec<([u8; 32], Message)>>>,
    }
    impl RailLink for RecRail {
        fn send(&self, to: [u8; 32], msg: &Message) -> Result<(), String> {
            self.sent.lock().unwrap().push((to, msg.clone()));
            Ok(())
        }
        fn my_address(&self) -> String {
            "test@suyu".into()
        }
    }

    #[test]
    fn send_compose_enruta_rail_y_smtp_por_separado() {
        let me = Address::named("Yo", "yo@x.com");
        let mut model = Model::new(Box::new(MockBackend::new(vec![])), me, Theme::dark());
        let grabado = Arc::new(Mutex::new(Vec::new()));
        model.attach_rail(Box::new(RecRail { sent: grabado.clone() }));
        assert!(model.store.is_pinned(SUYU_MAILBOX), "attach_rail fija el buzón Suyu");

        // Un destinatario del rail (@suyu) + uno SMTP.
        let rail_addr = paloma_rail::rail_address(&[9u8; 32]);
        let mut c = Compose::empty();
        c.to.set_text(format!("{rail_addr}, ana@gmail.com"));
        c.subject.set_text("minga");
        c.body.set_text("vení el sábado");
        model.compose = Some(c);

        model = send_compose(model);

        let sent = grabado.lock().unwrap();
        assert_eq!(sent.len(), 1, "exactamente una entrega por el rail");
        assert_eq!(sent[0].0, [9u8; 32], "a la identidad correcta");
        assert_eq!(sent[0].1.subject, "minga");
        assert_eq!(sent[0].1.to.len(), 2, "el Message lleva ambos destinatarios");
        // El compositor se cerró → se envió (rail + smtp por el MockBackend).
        assert!(model.compose.is_none());
    }

    #[test]
    fn alias_de_la_libreta_se_resuelve_y_enruta() {
        let me = Address::named("Yo", "yo@x.com");
        let mut model = Model::new(Box::new(MockBackend::new(vec![])), me, Theme::dark());
        let grabado = Arc::new(Mutex::new(Vec::new()));
        model.attach_rail(Box::new(RecRail { sent: grabado.clone() }));

        // Libreta: "Ana" → su dirección del rail.
        let rail_addr = paloma_rail::rail_address(&[9u8; 32]);
        let mut book = paloma_contacts::Contactbook::new();
        book.upsert("Ana", &rail_addr);
        model.set_contacts(book, std::path::PathBuf::from("/tmp/no-existe-igual.json"));

        // Escribo "Ana" (alias), no la dirección.
        let mut c = Compose::empty();
        c.to.set_text("Ana");
        c.subject.set_text("hola");
        model.compose = Some(c);
        model = send_compose(model);

        let sent = grabado.lock().unwrap();
        assert_eq!(sent.len(), 1, "el alias se resolvió y enrutó por el rail");
        assert_eq!(sent[0].0, [9u8; 32]);
    }

    #[test]
    fn confianza_remitente_conocido_por_la_libreta() {
        let me = Address::named("Yo", "yo@x.com");
        let mut model = Model::new(Box::new(MockBackend::new(vec![])), me, Theme::dark());
        let ana_addr = paloma_rail::rail_address(&[9u8; 32]);
        let mut book = paloma_contacts::Contactbook::new();
        book.upsert("Ana", &ana_addr);
        model.set_contacts(book, std::path::PathBuf::from("/tmp/x.json"));

        // Mensaje del rail: from.email = la identidad (dirección del rail).
        let msg = paloma_core::Message {
            id: MessageId("<r@suyu>".into()),
            from: Address::named("Ana", &ana_addr),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "hola".into(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: "qué tal".into(),
            body_html: None,
            flags: Flags::default(),
            signature: paloma_core::SignatureStatus::Verified,
            mailbox: SUYU_MAILBOX.into(),
            cuerpos: vec![],
        };
        assert_eq!(model.sender_trust(&msg), SenderTrust::Direct("Ana".into()), "remitente en la libreta");

        let otro = Address::named("X", "desconocido@rail.suyu");
        let mut ajeno = msg.clone();
        ajeno.from = otro;
        assert_eq!(model.sender_trust(&ajeno), SenderTrust::Unknown, "remitente desconocido");
    }

    #[test]
    fn correo_normal_no_toca_el_rail() {
        let me = Address::named("Yo", "yo@x.com");
        let mut model = Model::new(Box::new(MockBackend::new(vec![])), me, Theme::dark());
        let grabado = Arc::new(Mutex::new(Vec::new()));
        model.attach_rail(Box::new(RecRail { sent: grabado.clone() }));

        let mut c = Compose::empty();
        c.to.set_text("ana@gmail.com");
        c.subject.set_text("hola");
        model.compose = Some(c);
        model = send_compose(model);

        assert!(grabado.lock().unwrap().is_empty(), "un correo normal va por SMTP, no por el rail");
        assert!(model.compose.is_none());
    }
}
