use std::sync::Mutex;

use paloma_core::{Account, Flags, MailBackend, Mailbox, MailError, Message, MessageId, OutgoingMessage};

use crate::imap_client::ImapClient;
use crate::secret::Secret;
use crate::smtp;

/// Implementación real del [`MailBackend`]: IMAP para entrada (sesión viva tras
/// un `Mutex`, porque el trait es `&self` pero IMAP es stateful) y SMTP para
/// salida (sin conexión persistente; cada envío abre/cierra). El secreto SMTP
/// (contraseña o token OAuth2) se guarda para reusarlo por envío.
pub struct NetBackend {
    account: Account,
    smtp_secret: Secret,
    imap: Mutex<ImapClient>,
}

impl NetBackend {
    /// Conecta y autentica IMAP de una vez; guarda lo necesario para SMTP. Cada
    /// servidor recibe su propio secreto (contraseña o token OAuth2).
    pub fn connect(account: Account, imap_secret: &Secret, smtp_secret: &Secret) -> Result<Self, MailError> {
        let imap = ImapClient::connect(&account.imap, imap_secret)?;
        Ok(Self {
            account,
            smtp_secret: smtp_secret.clone(),
            imap: Mutex::new(imap),
        })
    }

    /// La cuenta que sirve este backend.
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Ajusta cuántos mensajes recientes traer por buzón (`None` = todos).
    /// Por defecto, [`crate::imap_client::DEFAULT_FETCH_LIMIT`].
    pub fn set_fetch_limit(&self, limit: Option<usize>) {
        self.imap.lock().unwrap().set_fetch_limit(limit);
    }
}

impl MailBackend for NetBackend {
    fn list_mailboxes(&self) -> Result<Vec<Mailbox>, MailError> {
        self.imap.lock().unwrap().list_mailboxes()
    }

    fn fetch_messages(&self, mailbox: &str) -> Result<Vec<Message>, MailError> {
        self.imap.lock().unwrap().fetch_messages(mailbox)
    }

    fn send(&self, msg: &OutgoingMessage) -> Result<MessageId, MailError> {
        smtp::send(&self.account.smtp, &self.smtp_secret, msg)
    }

    fn set_flags(&self, mailbox: &str, id: &MessageId, flags: Flags) -> Result<(), MailError> {
        self.imap.lock().unwrap().set_flags_by_message_id(mailbox, id, flags)
    }
}
