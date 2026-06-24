use std::collections::HashMap;

use crate::backend::MailBackend;
use crate::error::MailError;
use crate::mailbox::Mailbox;
use crate::message::{Flags, Message, MessageId};
use crate::thread::{build_threads, Thread};

/// Caché local de correo en memoria: la vista que el frontend consume. Guarda
/// los buzones y, por buzón, sus mensajes; deriva los hilos a demanda. Se
/// llena desde un [`MailBackend`] (real o mock) y aplica cambios de flags
/// localmente además de delegarlos al backend.
///
/// Es el "modelo de dominio del cliente": agnóstico a quién lo pinta y a
/// quién trae los bytes. La persistencia (BLAKE3 + postcard) y el sync
/// incremental llegan en una fase posterior.
#[derive(Default)]
pub struct MailStore {
    mailboxes: Vec<Mailbox>,
    /// buzón → mensajes (orden indefinido; se ordena al consultar hilos).
    by_mailbox: HashMap<String, Vec<Message>>,
    /// Buzones **locales** (no del backend IMAP): sobreviven a cada
    /// `sync_mailboxes`. El rail soberano usa esto para su buzón "Suyu".
    pinned: Vec<Mailbox>,
}

impl MailStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ordena los buzones por rol y nombre (orden canónico de la UI).
    fn sort_mailboxes(&mut self) {
        self.mailboxes
            .sort_by(|a, b| a.role.sort_key().cmp(&b.role.sort_key()).then(a.name.cmp(&b.name)));
    }

    /// Reincorpora los buzones locales (pinned) que falten tras un sync.
    fn merge_pinned(&mut self) {
        for p in &self.pinned {
            if !self.mailboxes.iter().any(|m| m.name == p.name) {
                self.mailboxes.push(p.clone());
            }
        }
        self.sort_mailboxes();
    }

    /// Fija un buzón **local** que no viene del backend y debe sobrevivir a los
    /// syncs (p. ej. "Suyu", el buzón del rail P2P). Idempotente.
    pub fn pin_mailbox(&mut self, mailbox: Mailbox) {
        if !self.pinned.iter().any(|m| m.name == mailbox.name) {
            self.pinned.push(mailbox);
        }
        self.merge_pinned();
    }

    /// ¿Es `name` un buzón local (pinned)? El frontend lo usa para no intentar
    /// sincronizarlo contra el backend IMAP (que no lo conoce).
    pub fn is_pinned(&self, name: &str) -> bool {
        self.pinned.iter().any(|m| m.name == name)
    }

    /// Agrega un mensaje a un buzón (sin reemplazar los existentes), deduplicado
    /// por `Message-ID`. Para mensajes que llegan de a uno, como los del rail.
    pub fn add_message(&mut self, mailbox: &str, message: Message) {
        let v = self.by_mailbox.entry(mailbox.to_string()).or_default();
        if !v.iter().any(|m| m.id == message.id) {
            v.push(message);
        }
    }

    /// Sincroniza la lista de buzones desde el backend.
    pub fn sync_mailboxes(&mut self, backend: &dyn MailBackend) -> Result<(), MailError> {
        self.mailboxes = backend.list_mailboxes()?;
        self.sort_mailboxes();
        self.merge_pinned();
        Ok(())
    }

    /// Trae (y reemplaza) los mensajes de un buzón desde el backend.
    pub fn sync_messages(&mut self, backend: &dyn MailBackend, mailbox: &str) -> Result<(), MailError> {
        let msgs = backend.fetch_messages(mailbox)?;
        self.by_mailbox.insert(mailbox.to_string(), msgs);
        Ok(())
    }

    /// Inserta mensajes directamente (para tests/demos o para precargar desde
    /// una caché en disco antes de tener red).
    pub fn ingest(&mut self, mailbox: &str, messages: Vec<Message>) {
        self.by_mailbox.insert(mailbox.to_string(), messages);
    }

    /// Fija la lista de buzones directamente, ordenándola por rol (igual que
    /// `sync_mailboxes`). Para precargar desde la caché en disco cuando todavía
    /// no hubo —o falló— el sync de red.
    pub fn ingest_mailboxes(&mut self, mailboxes: Vec<Mailbox>) {
        self.mailboxes = mailboxes;
        self.sort_mailboxes();
        self.merge_pinned();
    }

    /// Los buzones conocidos, ya ordenados por rol.
    pub fn mailboxes(&self) -> &[Mailbox] {
        &self.mailboxes
    }

    /// Los mensajes de un buzón (vacío si no se sincronizó).
    pub fn messages(&self, mailbox: &str) -> &[Message] {
        self.by_mailbox.get(mailbox).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Los hilos de un buzón, recientes primero. Oculta los mensajes marcados
    /// como borrados (`\Deleted`): siguen en la caché hasta el expunge, pero no
    /// aparecen en la bandeja.
    pub fn threads(&self, mailbox: &str) -> Vec<Thread> {
        let visible: Vec<Message> =
            self.messages(mailbox).iter().filter(|m| !m.flags.deleted).cloned().collect();
        build_threads(&visible)
    }

    /// Busca un mensaje por id en cualquier buzón.
    pub fn message(&self, id: &MessageId) -> Option<&Message> {
        self.by_mailbox.values().flatten().find(|m| &m.id == id)
    }

    /// Búsqueda de texto sobre **todos** los buzones cacheados. Devuelve los
    /// mensajes que matchean todos los términos de `query`, mejor puntuados y
    /// más recientes primero. Consulta vacía → sin resultados.
    pub fn search(&self, query: &str) -> Vec<&Message> {
        let terms = crate::search::terms(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(i32, &Message)> = self
            .by_mailbox
            .values()
            .flatten()
            .filter_map(|m| crate::search::score(m, &terms).map(|s| (s, m)))
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.date.cmp(&a.1.date)));
        hits.into_iter().map(|(_, m)| m).collect()
    }

    /// Cantidad de mensajes sin leer en un buzón.
    pub fn unread_count(&self, mailbox: &str) -> usize {
        self.messages(mailbox).iter().filter(|m| m.is_unread()).count()
    }

    /// Marca un mensaje como leído, local y en el backend. No-op si el mensaje
    /// no está en la caché.
    pub fn mark_seen(&mut self, backend: &dyn MailBackend, mailbox: &str, id: &MessageId) -> Result<(), MailError> {
        if let Some(msgs) = self.by_mailbox.get_mut(mailbox) {
            if let Some(m) = msgs.iter_mut().find(|m| &m.id == id) {
                m.flags.seen = true;
                let flags = m.flags;
                return backend.set_flags(mailbox, id, flags);
            }
        }
        Ok(())
    }

    /// Aplica flags arbitrarios local + backend.
    pub fn set_flags(
        &mut self,
        backend: &dyn MailBackend,
        mailbox: &str,
        id: &MessageId,
        flags: Flags,
    ) -> Result<(), MailError> {
        if let Some(msgs) = self.by_mailbox.get_mut(mailbox) {
            if let Some(m) = msgs.iter_mut().find(|m| &m.id == id) {
                m.flags = flags;
            }
        }
        backend.set_flags(mailbox, id, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;
    use crate::backend::MockBackend;
    use crate::message::{MessageId, SignatureStatus};

    fn msg(id: &str, seen: bool, date: i64, irt: Option<&str>) -> Message {
        Message {
            id: MessageId(id.into()),
            from: Address::new("a@x.com"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Hola".into(),
            date,
            in_reply_to: irt.map(|s| MessageId(s.into())),
            references: irt.map(|s| vec![MessageId(s.into())]).unwrap_or_default(),
            body_text: String::new(),
            body_html: None,
            flags: Flags { seen, ..Default::default() },
            signature: SignatureStatus::Unsigned,
            mailbox: "INBOX".into(),
            cuerpos: Vec::new(),
        }
    }

    #[test]
    fn sync_desde_backend_y_cuenta_no_leidos() {
        let backend = MockBackend::new(vec![msg("<1@x>", false, 10, None), msg("<2@x>", true, 20, None)]);
        let mut store = MailStore::new();
        store.sync_mailboxes(&backend).unwrap();
        store.sync_messages(&backend, "INBOX").unwrap();
        assert_eq!(store.messages("INBOX").len(), 2);
        assert_eq!(store.unread_count("INBOX"), 1);
        // Buzones ordenados: INBOX (Inbox) antes que Sent.
        assert_eq!(store.mailboxes()[0].name, "INBOX");
    }

    #[test]
    fn buzon_pinned_sobrevive_al_sync_y_acepta_mensajes() {
        use crate::mailbox::Mailbox;
        let backend = MockBackend::new(vec![msg("<1@x>", false, 10, None)]);
        let mut store = MailStore::new();
        store.pin_mailbox(Mailbox::new("Suyu"));
        assert!(store.is_pinned("Suyu"));
        // Un sync del backend (que NO conoce "Suyu") no debe borrarlo.
        store.sync_mailboxes(&backend).unwrap();
        assert!(store.mailboxes().iter().any(|m| m.name == "Suyu"));
        // add_message agrega sin reemplazar y deduplica por id.
        store.add_message("Suyu", msg("<r1@x>", false, 5, None));
        store.add_message("Suyu", msg("<r1@x>", false, 5, None)); // dup
        store.add_message("Suyu", msg("<r2@x>", false, 6, None));
        assert_eq!(store.messages("Suyu").len(), 2);
    }

    #[test]
    fn threads_agrupa_la_cadena() {
        let mut store = MailStore::new();
        store.ingest("INBOX", vec![msg("<1@x>", true, 10, None), msg("<2@x>", false, 20, Some("<1@x>"))]);
        let threads = store.threads("INBOX");
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].unread, 1);
    }

    #[test]
    fn mark_seen_actualiza_local_y_backend() {
        let backend = MockBackend::new(vec![msg("<1@x>", false, 10, None)]);
        let mut store = MailStore::new();
        store.sync_messages(&backend, "INBOX").unwrap();
        assert_eq!(store.unread_count("INBOX"), 1);
        store.mark_seen(&backend, "INBOX", &MessageId("<1@x>".into())).unwrap();
        assert_eq!(store.unread_count("INBOX"), 0);
        // Persistió en el backend también.
        assert!(backend.fetch_messages("INBOX").unwrap()[0].flags.seen);
    }

    #[test]
    fn search_cruza_buzones_y_ordena() {
        let mut store = MailStore::new();
        let mut a = msg("<1@x>", true, 10, None);
        a.subject = "Factura de mayo".into();
        let mut b = msg("<2@x>", true, 30, None);
        b.subject = "Otra cosa".into();
        b.body_text = "te paso la factura adjunta".into();
        b.mailbox = "Sent".into();
        store.ingest("INBOX", vec![a]);
        store.ingest("Sent", vec![b]);
        let hits = store.search("factura");
        assert_eq!(hits.len(), 2);
        // El match en asunto (peso mayor) va primero pese a ser más viejo.
        assert_eq!(hits[0].id.0, "<1@x>");
        assert!(store.search("inexistente").is_empty());
        assert!(store.search("").is_empty());
    }

    #[test]
    fn message_busca_por_id() {
        let mut store = MailStore::new();
        store.ingest("INBOX", vec![msg("<1@x>", true, 10, None)]);
        assert!(store.message(&MessageId("<1@x>".into())).is_some());
        assert!(store.message(&MessageId("<nope@x>".into())).is_none());
    }
}
