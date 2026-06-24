use std::collections::HashMap;
use std::sync::Mutex;

use crate::address::Address;
use crate::error::MailError;
use crate::mailbox::Mailbox;
use crate::message::{Flags, Message, MessageId, SignatureStatus};

/// Un mensaje a **enviar**: lo que el frontend de redacción arma y el
/// transporte SMTP serializa a RFC 5322. Separado de [`Message`] porque al
/// componer todavía no hay `Message-ID` asignado por el servidor ni flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    /// Si es una respuesta, el `Message-ID` al que contesta (alimenta
    /// `In-Reply-To`/`References` en el header).
    pub in_reply_to: Option<MessageId>,
    pub references: Vec<MessageId>,
    /// Firma Ed25519 a adjuntar (si el usuario pidió firmar). La calcula la
    /// capa de firma (`paloma-sign`) sobre los [`crate::canonical_signing_bytes`];
    /// el transporte la emite como headers `X-Paloma-*`.
    pub signature: Option<crate::MailSignature>,
    /// Lienzos multilienzo (Eje 4): versiones del cuerpo en otros idiomas/tonos
    /// que viajan con el mensaje. El transporte las emite (header `X-Paloma-Cuerpos`
    /// en SMTP; nativo en el rail).
    pub cuerpos: Vec<crate::MailCuerpo>,
}

impl OutgoingMessage {
    /// Bytes canónicos a firmar para este saliente. Espejan los que el receptor
    /// recomputará para verificar. Ver [`crate::canonical_signing_bytes`].
    pub fn canonical_signing_bytes(&self) -> Vec<u8> {
        let to: Vec<String> = self.to.iter().map(|a| a.email.clone()).collect();
        crate::canonical_signing_bytes(&self.from.email, &to, &self.subject, &self.body_text)
    }

    /// Arma una respuesta a `original` desde `from`, con el asunto `Re:` y la
    /// cadena de `References` extendida. El cuerpo lo completa el redactor.
    pub fn reply_to(original: &Message, from: Address) -> Self {
        let mut references = original.references.clone();
        references.push(original.id.clone());
        OutgoingMessage {
            from,
            to: vec![original.from.clone()],
            cc: vec![],
            bcc: vec![],
            subject: original.reply_subject(),
            body_text: String::new(),
            body_html: None,
            in_reply_to: Some(original.id.clone()),
            references,
            signature: None,
            cuerpos: Vec::new(),
        }
    }

    /// Arma un **reenvío** de `original` desde `from`: asunto `Fwd:` y el cuerpo
    /// prellenado con el mensaje citado (cabecera + texto). El destinatario lo
    /// completa el redactor. No hilea (un forward abre una conversación nueva).
    pub fn forward(original: &Message, from: Address) -> Self {
        let quoted = format!(
            "\n\n----- Mensaje reenviado -----\nDe: {}\nAsunto: {}\n\n{}",
            original.from,
            original.subject,
            original.display_body(),
        );
        OutgoingMessage {
            from,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: forward_subject(&original.subject),
            body_text: quoted,
            body_html: None,
            in_reply_to: None,
            references: vec![],
            signature: None,
            cuerpos: Vec::new(),
        }
    }
}

/// El asunto de un reenvío: `Fwd: <asunto>` sin duplicar el prefijo.
fn forward_subject(subject: &str) -> String {
    let base = subject.trim();
    let lower = base.to_ascii_lowercase();
    if lower.starts_with("fwd:") || lower.starts_with("fw:") {
        base.to_string()
    } else {
        format!("Fwd: {base}")
    }
}

/// El transporte de correo, agnóstico al protocolo. El puente real (IMAP para
/// entrada, SMTP para salida) implementa este trait; los frontends y el store
/// hablan sólo con él, así que cambiar de backend (o usar el mock) no toca la
/// UI. Síncrono a propósito: encaja con el hilo del compositor de Llimphi
/// (mismo patrón que el `fetch` de puriy con `ureq`); un backend real puede
/// hacer el trabajo pesado en su propio hilo y exponer resultados acá.
pub trait MailBackend {
    /// Lista los buzones/carpetas de la cuenta.
    fn list_mailboxes(&self) -> Result<Vec<Mailbox>, MailError>;

    /// Trae los mensajes de un buzón (orden indefinido; el store ordena).
    fn fetch_messages(&self, mailbox: &str) -> Result<Vec<Message>, MailError>;

    /// Envía un mensaje. Devuelve el `Message-ID` asignado.
    fn send(&self, msg: &OutgoingMessage) -> Result<MessageId, MailError>;

    /// Actualiza los flags de un mensaje (leído, destacado, borrado…).
    fn set_flags(&self, mailbox: &str, id: &MessageId, flags: Flags) -> Result<(), MailError>;
}

/// Backend en memoria para tests y demos: arranca con buzones/mensajes
/// precargados, registra los envíos en una bandeja `Sent` y aplica flags.
/// Permite ejercitar toda la UI sin red.
pub struct MockBackend {
    mailboxes: Vec<Mailbox>,
    /// buzón → mensajes.
    messages: Mutex<HashMap<String, Vec<Message>>>,
    /// Contador para generar `Message-ID`s de los envíos.
    sent_counter: Mutex<u64>,
}

impl MockBackend {
    /// Crea un mock con un INBOX poblado por `inbox` y un `Sent` vacío.
    pub fn new(inbox: Vec<Message>) -> Self {
        let mut messages = HashMap::new();
        messages.insert("INBOX".to_string(), inbox);
        messages.insert("Sent".to_string(), Vec::new());
        Self {
            mailboxes: vec![Mailbox::new("INBOX"), Mailbox::new("Sent")],
            messages: Mutex::new(messages),
            sent_counter: Mutex::new(0),
        }
    }
}

impl MailBackend for MockBackend {
    fn list_mailboxes(&self) -> Result<Vec<Mailbox>, MailError> {
        Ok(self.mailboxes.clone())
    }

    fn fetch_messages(&self, mailbox: &str) -> Result<Vec<Message>, MailError> {
        self.messages
            .lock()
            .unwrap()
            .get(mailbox)
            .cloned()
            .ok_or_else(|| MailError::UnknownMailbox(mailbox.to_string()))
    }

    fn send(&self, msg: &OutgoingMessage) -> Result<MessageId, MailError> {
        let mut counter = self.sent_counter.lock().unwrap();
        *counter += 1;
        let id = MessageId(format!("<sent-{}@paloma.local>", *counter));
        let stored = Message {
            id: id.clone(),
            from: msg.from.clone(),
            to: msg.to.clone(),
            cc: msg.cc.clone(),
            bcc: msg.bcc.clone(),
            subject: msg.subject.clone(),
            date: 0,
            in_reply_to: msg.in_reply_to.clone(),
            references: msg.references.clone(),
            body_text: msg.body_text.clone(),
            body_html: msg.body_html.clone(),
            flags: Flags { seen: true, ..Default::default() },
            signature: SignatureStatus::Unsigned,
            mailbox: "Sent".to_string(),
            cuerpos: msg.cuerpos.clone(),
            signer: None,
        };
        self.messages.lock().unwrap().entry("Sent".to_string()).or_default().push(stored);
        Ok(id)
    }

    fn set_flags(&self, mailbox: &str, id: &MessageId, flags: Flags) -> Result<(), MailError> {
        let mut all = self.messages.lock().unwrap();
        let box_msgs = all
            .get_mut(mailbox)
            .ok_or_else(|| MailError::UnknownMailbox(mailbox.to_string()))?;
        let m = box_msgs
            .iter_mut()
            .find(|m| &m.id == id)
            .ok_or_else(|| MailError::UnknownMessage(id.0.clone()))?;
        m.flags = flags;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str) -> Message {
        Message {
            id: MessageId(id.into()),
            from: Address::new("a@x.com"),
            to: vec![Address::new("yo@x.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Hola".into(),
            date: 10,
            in_reply_to: None,
            references: vec![],
            body_text: "cuerpo".into(),
            body_html: None,
            flags: Flags::default(),
            signature: SignatureStatus::Unsigned,
            mailbox: "INBOX".into(),
            cuerpos: Vec::new(),
            signer: None,
        }
    }

    #[test]
    fn mock_lista_y_trae() {
        let b = MockBackend::new(vec![msg("<1@x>")]);
        assert_eq!(b.list_mailboxes().unwrap().len(), 2);
        assert_eq!(b.fetch_messages("INBOX").unwrap().len(), 1);
        assert!(matches!(b.fetch_messages("Nope"), Err(MailError::UnknownMailbox(_))));
    }

    #[test]
    fn mock_send_aterriza_en_sent() {
        let b = MockBackend::new(vec![]);
        let out = OutgoingMessage {
            from: Address::new("yo@x.com"),
            to: vec![Address::new("a@x.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Test".into(),
            body_text: "hola".into(),
            body_html: None,
            in_reply_to: None,
            references: vec![],
            signature: None,
            cuerpos: Vec::new(),
        };
        let id = b.send(&out).unwrap();
        let sent = b.fetch_messages("Sent").unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].id, id);
        assert!(sent[0].flags.seen);
    }

    #[test]
    fn reply_to_extiende_references_y_re() {
        let original = msg("<1@x>");
        let r = OutgoingMessage::reply_to(&original, Address::new("yo@x.com"));
        assert_eq!(r.subject, "Re: Hola");
        assert_eq!(r.to, vec![Address::new("a@x.com")]);
        assert_eq!(r.in_reply_to, Some(MessageId("<1@x>".into())));
        assert_eq!(r.references, vec![MessageId("<1@x>".into())]);
    }

    #[test]
    fn forward_prefija_fwd_y_cita_el_cuerpo() {
        let original = msg("<1@x>");
        let f = OutgoingMessage::forward(&original, Address::new("yo@x.com"));
        assert_eq!(f.subject, "Fwd: Hola");
        assert!(f.to.is_empty(), "el reenvío no asume destinatario");
        assert!(f.in_reply_to.is_none(), "un reenvío no hilea");
        assert!(f.body_text.contains("Mensaje reenviado"));
        assert!(f.body_text.contains("cuerpo"));
        // No duplica el prefijo.
        let mut twice = original.clone();
        twice.subject = "Fwd: Hola".into();
        assert_eq!(OutgoingMessage::forward(&twice, Address::new("yo@x.com")).subject, "Fwd: Hola");
    }

    #[test]
    fn set_flags_marca_leido() {
        let b = MockBackend::new(vec![msg("<1@x>")]);
        b.set_flags("INBOX", &MessageId("<1@x>".into()), Flags { seen: true, ..Default::default() })
            .unwrap();
        assert!(b.fetch_messages("INBOX").unwrap()[0].flags.seen);
    }
}
