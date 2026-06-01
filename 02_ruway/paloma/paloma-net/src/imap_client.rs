//! Cliente IMAP síncrono (sobre `imap` + `native-tls`).
//!
//! Trae buzones y mensajes y actualiza flags. Síncrono a propósito: encaja
//! con el patrón del resto de la suite (igual que `ureq` en puriy). Por ahora
//! sólo TLS implícito (puerto 993); STARTTLS/plain se rechazan con un error
//! claro y quedan para una sub-fase.

use std::net::TcpStream;

use imap::types::Flag as ImapFlag;
use imap::Session;
use native_tls::TlsStream;
use paloma_core::{Flags, Mailbox, MailError, Message, MessageId, Security, ServerConfig};

use crate::mime;

/// Sesión IMAP autenticada contra un servidor.
pub struct ImapClient {
    session: Session<TlsStream<TcpStream>>,
}

impl ImapClient {
    /// Conecta y hace login. La contraseña la provee el caller (a futuro,
    /// desde el proveedor de credenciales de la suite).
    pub fn connect(cfg: &ServerConfig, password: &str) -> Result<Self, MailError> {
        if !matches!(cfg.security, Security::Tls) {
            return Err(MailError::Transport(
                "paloma-net: por ahora sólo IMAP sobre TLS implícito (993)".into(),
            ));
        }
        let tls = native_tls::TlsConnector::builder()
            .build()
            .map_err(|e| MailError::Transport(e.to_string()))?;
        let client = imap::connect((cfg.host.as_str(), cfg.port), cfg.host.as_str(), &tls)
            .map_err(|e| MailError::Transport(e.to_string()))?;
        let session = client
            .login(&cfg.username, password)
            .map_err(|(_e, _client)| MailError::Auth)?;
        Ok(Self { session })
    }

    /// Lista los buzones (`LIST "" "*"`).
    pub fn list_mailboxes(&mut self) -> Result<Vec<Mailbox>, MailError> {
        let names = self.session.list(Some(""), Some("*")).map_err(map_err)?;
        Ok(names.iter().map(|n| Mailbox::new(n.name())).collect())
    }

    /// Trae todos los mensajes de un buzón (`FETCH 1:* (UID FLAGS RFC822)`) y
    /// los parsea al modelo nativo. (Limitar a los últimos N queda para la
    /// fase de sync incremental.)
    pub fn fetch_messages(&mut self, mailbox: &str) -> Result<Vec<Message>, MailError> {
        self.session.select(mailbox).map_err(map_err)?;
        let fetches = self.session.fetch("1:*", "(UID FLAGS RFC822)").map_err(map_err)?;
        let mut out = Vec::new();
        for f in fetches.iter() {
            let Some(body) = f.body().or_else(|| f.text()) else {
                continue;
            };
            let flags = flags_from(f.flags());
            if let Ok(m) = mime::parse_message(body, mailbox, flags) {
                out.push(m);
            }
        }
        Ok(out)
    }

    /// Reemplaza los flags de un mensaje, ubicándolo por su `Message-ID`
    /// (`UID SEARCH HEADER MESSAGE-ID …` → `UID STORE`).
    pub fn set_flags_by_message_id(
        &mut self,
        mailbox: &str,
        id: &MessageId,
        flags: Flags,
    ) -> Result<(), MailError> {
        self.session.select(mailbox).map_err(map_err)?;
        let inner = id.0.trim_matches(|c| c == '<' || c == '>');
        let uids = self
            .session
            .uid_search(format!("HEADER MESSAGE-ID <{inner}>"))
            .map_err(map_err)?;
        let Some(&uid) = uids.iter().next() else {
            return Err(MailError::UnknownMessage(id.0.clone()));
        };
        self.session
            .uid_store(uid.to_string(), format!("FLAGS ({})", imap_flag_string(flags)))
            .map_err(map_err)?;
        Ok(())
    }

    /// Cierra la sesión limpiamente.
    pub fn logout(&mut self) -> Result<(), MailError> {
        self.session.logout().map_err(map_err)
    }
}

fn map_err(e: imap::Error) -> MailError {
    MailError::Transport(e.to_string())
}

/// Traduce los flags IMAP del servidor a nuestro [`Flags`].
fn flags_from(flags: &[ImapFlag]) -> Flags {
    let mut f = Flags::default();
    for fl in flags {
        match fl {
            ImapFlag::Seen => f.seen = true,
            ImapFlag::Answered => f.answered = true,
            ImapFlag::Flagged => f.flagged = true,
            ImapFlag::Draft => f.draft = true,
            ImapFlag::Deleted => f.deleted = true,
            _ => {}
        }
    }
    f
}

/// Construye la lista de flags IMAP (`\Seen \Flagged …`) para un `STORE`.
fn imap_flag_string(f: Flags) -> String {
    let mut v: Vec<&str> = Vec::new();
    if f.seen {
        v.push("\\Seen");
    }
    if f.answered {
        v.push("\\Answered");
    }
    if f.flagged {
        v.push("\\Flagged");
    }
    if f.draft {
        v.push("\\Draft");
    }
    if f.deleted {
        v.push("\\Deleted");
    }
    v.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_string_arma_la_lista() {
        let f = Flags { seen: true, flagged: true, ..Default::default() };
        assert_eq!(imap_flag_string(f), "\\Seen \\Flagged");
        assert_eq!(imap_flag_string(Flags::default()), "");
    }
}
