use std::sync::{Arc, Mutex};

use paloma_core::{Account, Flags, MailBackend, Mailbox, MailError, Message, MessageId, OutgoingMessage};

use crate::imap_client::{ImapClient, DEFAULT_FETCH_LIMIT};
use crate::secret::Secret;
use crate::smtp;

/// Fuente de **token OAuth2 fresco**: una función que devuelve un `access_token`
/// vigente, renovándolo si venció. La inyecta el anfitrión (`paloma-app`, sobre
/// `paloma-oauth::valid_access_token`); este crate no sabe de proveedores OAuth,
/// sólo le pide un token cada vez que va a autenticar. `Send + Sync` porque se
/// llama desde el envío SMTP y desde la reconexión IMAP.
pub type TokenSource = Arc<dyn Fn() -> Result<String, String> + Send + Sync>;

/// Cómo se autentica la cuenta: con un secreto fijo (contraseña) o con un token
/// OAuth2 **refrescable** a mitad de sesión.
enum Auth {
    /// Contraseña/app-password fija para SMTP (IMAP ya se autenticó al conectar).
    /// No se refresca ni se reconecta sola (las contraseñas no vencen).
    Static { smtp: Secret },
    /// Token OAuth2 renovable: cada envío SMTP y cada reconexión IMAP piden un
    /// token fresco a la fuente (que renueva si venció).
    OAuth(TokenSource),
}

/// Implementación real del [`MailBackend`]: IMAP para entrada (sesión viva tras
/// un `Mutex`, porque el trait es `&self` pero IMAP es stateful) y SMTP para
/// salida (sin conexión persistente; cada envío abre/cierra).
///
/// Con OAuth2 el token vence (~1 h): el backend pide uno fresco a la
/// [`TokenSource`] **en cada envío SMTP** y, si una operación IMAP falla (la
/// sesión pudo caer al vencer el token), **reconecta con un token nuevo y
/// reintenta una vez**. Así el correo sigue andando a mitad de sesión sin que el
/// usuario tenga que reautorizar.
pub struct NetBackend {
    account: Account,
    auth: Auth,
    imap: Mutex<ImapClient>,
    /// Límite de fetch vigente, recordado para re-aplicarlo al reconectar.
    fetch_limit: Mutex<Option<usize>>,
}

impl NetBackend {
    /// Conecta IMAP con contraseña y guarda el secreto SMTP para reusarlo por
    /// envío. (Camino clásico, sin OAuth: no reconecta solo.)
    pub fn connect(account: Account, imap_secret: &Secret, smtp_secret: &Secret) -> Result<Self, MailError> {
        let imap = ImapClient::connect(&account.imap, imap_secret)?;
        Ok(Self {
            account,
            auth: Auth::Static { smtp: smtp_secret.clone() },
            imap: Mutex::new(imap),
            fetch_limit: Mutex::new(Some(DEFAULT_FETCH_LIMIT)),
        })
    }

    /// Conecta IMAP con **OAuth2**, tomando el primer token de la fuente (que ya
    /// renueva si el guardado venció) y guardándola para refrescar a mitad de
    /// sesión (SMTP por envío + reconexión IMAP).
    pub fn connect_oauth(account: Account, token: TokenSource) -> Result<Self, MailError> {
        let access = token().map_err(MailError::Transport)?;
        let imap = ImapClient::connect(&account.imap, &Secret::OAuth2(access))?;
        Ok(Self {
            account,
            auth: Auth::OAuth(token),
            imap: Mutex::new(imap),
            fetch_limit: Mutex::new(Some(DEFAULT_FETCH_LIMIT)),
        })
    }

    /// La cuenta que sirve este backend.
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Ajusta cuántos mensajes recientes traer por buzón (`None` = todos). Se
    /// recuerda para re-aplicarlo si la sesión IMAP se reconecta.
    pub fn set_fetch_limit(&self, limit: Option<usize>) {
        *self.fetch_limit.lock().unwrap() = limit;
        self.imap.lock().unwrap().set_fetch_limit(limit);
    }

    /// `true` si la cuenta usa OAuth2 (y por tanto puede refrescar/reconectar).
    fn is_oauth(&self) -> bool {
        matches!(self.auth, Auth::OAuth(_))
    }

    /// El secreto a usar **ahora** para autenticar (IMAP o SMTP): contraseña fija,
    /// o un token OAuth2 fresco (la fuente lo renueva si venció).
    fn fresh_secret(&self) -> Result<Secret, MailError> {
        secret_for_auth(&self.auth)
    }

    /// Reconecta la sesión IMAP con un secreto fresco y re-aplica el fetch limit.
    fn reconnect_imap(&self) -> Result<(), MailError> {
        let secret = self.fresh_secret()?;
        let mut client = ImapClient::connect(&self.account.imap, &secret)?;
        client.set_fetch_limit(*self.fetch_limit.lock().unwrap());
        *self.imap.lock().unwrap() = client;
        Ok(())
    }

    /// Corre una operación IMAP; si falla porque **la sesión murió** —token
    /// vencido (`Auth`) o conexión perdida (`Disconnected`)— **y** la cuenta es
    /// OAuth, reconecta con un token fresco y reintenta una vez. Los fallos
    /// lógicos (buzón inexistente, parseo) y de transporte que no tiran la sesión
    /// suben tal cual: reconectar no los arregla, así que no se reconecta de más.
    fn imap_op<T>(
        &self,
        mut op: impl FnMut(&mut ImapClient) -> Result<T, MailError>,
    ) -> Result<T, MailError> {
        let first = {
            let mut guard = self.imap.lock().unwrap();
            op(&mut guard)
        };
        match first {
            Err(MailError::Auth | MailError::Disconnected(_)) if self.is_oauth() => {
                self.reconnect_imap()?;
                let mut guard = self.imap.lock().unwrap();
                op(&mut guard)
            }
            other => other,
        }
    }
}

/// El secreto vigente para un [`Auth`]: contraseña fija, o un token OAuth2
/// **fresco** pedido a la fuente (que renueva si venció). Es la pieza que hace el
/// refresco a mitad de sesión: se llama por envío SMTP y al reconectar IMAP, así
/// el token nunca queda «congelado» en el backend.
fn secret_for_auth(auth: &Auth) -> Result<Secret, MailError> {
    match auth {
        Auth::Static { smtp } => Ok(smtp.clone()),
        Auth::OAuth(src) => src().map(Secret::OAuth2).map_err(MailError::Transport),
    }
}

impl MailBackend for NetBackend {
    fn list_mailboxes(&self) -> Result<Vec<Mailbox>, MailError> {
        self.imap_op(|c| c.list_mailboxes())
    }

    fn fetch_messages(&self, mailbox: &str) -> Result<Vec<Message>, MailError> {
        self.imap_op(|c| c.fetch_messages(mailbox))
    }

    fn send(&self, msg: &OutgoingMessage) -> Result<MessageId, MailError> {
        // Token fresco por envío: con OAuth, si venció lo renueva acá mismo.
        let secret = self.fresh_secret()?;
        smtp::send(&self.account.smtp, &secret, msg)
    }

    fn set_flags(&self, mailbox: &str, id: &MessageId, flags: Flags) -> Result<(), MailError> {
        self.imap_op(|c| c.set_flags_by_message_id(mailbox, id, flags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    #[test]
    fn oauth_pide_token_fresco_cada_vez() {
        // Cada consulta devuelve un token distinto: simula que la fuente renovó.
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let src: TokenSource = Arc::new(move || Ok(format!("tok{}", n2.fetch_add(1, SeqCst))));
        let auth = Auth::OAuth(src);
        // El backend NO cachea: por envío/reconexión pide uno nuevo.
        assert!(matches!(secret_for_auth(&auth), Ok(Secret::OAuth2(t)) if t == "tok0"));
        assert!(matches!(secret_for_auth(&auth), Ok(Secret::OAuth2(t)) if t == "tok1"));
        assert_eq!(n.load(SeqCst), 2, "se consulta la fuente en cada uso");
    }

    #[test]
    fn oauth_propaga_error_de_token() {
        // Si no se puede refrescar (sin refresh_token, red caída…), el error sube.
        let src: TokenSource = Arc::new(|| Err("token vencido sin refresh".into()));
        assert!(matches!(secret_for_auth(&Auth::OAuth(src)), Err(MailError::Transport(_))));
    }

    #[test]
    fn password_devuelve_siempre_el_mismo_secreto() {
        let auth = Auth::Static { smtp: Secret::Password("pw".into()) };
        assert!(matches!(secret_for_auth(&auth), Ok(Secret::Password(t)) if t == "pw"));
        assert!(matches!(secret_for_auth(&auth), Ok(Secret::Password(t)) if t == "pw"));
    }
}
