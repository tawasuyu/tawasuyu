//! Cliente IMAP síncrono (sobre `imap` + `native-tls`).
//!
//! Trae buzones y mensajes y actualiza flags. Síncrono a propósito: encaja
//! con el patrón del resto de la suite (igual que `ureq` en puriy). Soporta los
//! tres modos de transporte: **TLS implícito** (993), **STARTTLS** (143→TLS) y
//! **plano** (143, sólo redes de confianza/pruebas). STARTTLS y TLS terminan en
//! el mismo tipo de stream cifrado; el plano vive en una variante aparte del
//! enum [`ImapSession`].
//!
//! El fetch trae **los últimos N** mensajes del buzón (no todo el histórico):
//! se calcula el rango a partir del `EXISTS` que devuelve el `SELECT`. `N` es
//! configurable (default [`DEFAULT_FETCH_LIMIT`]); `None` trae todo.

use std::io::{Read, Write};
use std::net::TcpStream;

use imap::types::Flag as ImapFlag;
use imap::Session;
use native_tls::TlsStream;
use paloma_core::{Flags, Mailbox, MailError, Message, MessageId, Security, ServerConfig};

use crate::mime;
use crate::secret::Secret;

/// Cuántos mensajes recientes traer por buzón si no se configura otra cosa.
pub const DEFAULT_FETCH_LIMIT: usize = 200;

/// Autenticador `XOAUTH2` para el `imap` crate: responde la cadena SASL Bearer
/// (`user=…\x01auth=Bearer …\x01\x01`), que el cliente codifica en base64.
struct XOAuth2 {
    user: String,
    token: String,
}

impl imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> Self::Response {
        Secret::xoauth2_sasl(&self.user, &self.token)
    }
}

/// Sesión IMAP autenticada, cifrada (TLS/STARTTLS) o en claro (plano).
enum ImapSession {
    Tls(Session<TlsStream<TcpStream>>),
    Plain(Session<TcpStream>),
}

/// Cliente IMAP: una sesión + el límite de fetch.
pub struct ImapClient {
    session: ImapSession,
    /// Cuántos mensajes recientes traer por buzón; `None` = todos.
    fetch_limit: Option<usize>,
}

impl ImapClient {
    /// Conecta y autentica según `cfg.security`, con el secreto que provee el
    /// caller: contraseña (`LOGIN`) o token OAuth2 (`AUTHENTICATE XOAUTH2`).
    pub fn connect(cfg: &ServerConfig, secret: &Secret) -> Result<Self, MailError> {
        let session = match cfg.security {
            Security::Tls => {
                let tls = build_tls()?;
                let client = imap::connect((cfg.host.as_str(), cfg.port), cfg.host.as_str(), &tls)
                    .map_err(|e| MailError::Transport(e.to_string()))?;
                ImapSession::Tls(auth(client, cfg, secret)?)
            }
            Security::StartTls => {
                let tls = build_tls()?;
                let client =
                    imap::connect_starttls((cfg.host.as_str(), cfg.port), cfg.host.as_str(), &tls)
                        .map_err(|e| MailError::Transport(e.to_string()))?;
                ImapSession::Tls(auth(client, cfg, secret)?)
            }
            Security::Plain => {
                let tcp = TcpStream::connect((cfg.host.as_str(), cfg.port))
                    .map_err(|e| MailError::Transport(e.to_string()))?;
                let mut client = imap::Client::new(tcp);
                client.read_greeting().map_err(|e| MailError::Transport(e.to_string()))?;
                ImapSession::Plain(auth(client, cfg, secret)?)
            }
        };
        Ok(Self { session, fetch_limit: Some(DEFAULT_FETCH_LIMIT) })
    }

    /// Ajusta cuántos mensajes recientes traer por buzón (`None` = todos).
    pub fn set_fetch_limit(&mut self, limit: Option<usize>) {
        self.fetch_limit = limit;
    }

    /// Lista los buzones (`LIST "" "*"`).
    pub fn list_mailboxes(&mut self) -> Result<Vec<Mailbox>, MailError> {
        match &mut self.session {
            ImapSession::Tls(s) => list_on(s),
            ImapSession::Plain(s) => list_on(s),
        }
    }

    /// Trae los últimos N mensajes de un buzón y los parsea al modelo nativo.
    pub fn fetch_messages(&mut self, mailbox: &str) -> Result<Vec<Message>, MailError> {
        let limit = self.fetch_limit;
        match &mut self.session {
            ImapSession::Tls(s) => fetch_on(s, mailbox, limit),
            ImapSession::Plain(s) => fetch_on(s, mailbox, limit),
        }
    }

    /// Reemplaza los flags de un mensaje, ubicándolo por su `Message-ID`.
    pub fn set_flags_by_message_id(
        &mut self,
        mailbox: &str,
        id: &MessageId,
        flags: Flags,
    ) -> Result<(), MailError> {
        match &mut self.session {
            ImapSession::Tls(s) => set_flags_on(s, mailbox, id, flags),
            ImapSession::Plain(s) => set_flags_on(s, mailbox, id, flags),
        }
    }

    /// Cierra la sesión limpiamente.
    pub fn logout(&mut self) -> Result<(), MailError> {
        match &mut self.session {
            ImapSession::Tls(s) => s.logout().map_err(map_err),
            ImapSession::Plain(s) => s.logout().map_err(map_err),
        }
    }
}

fn build_tls() -> Result<native_tls::TlsConnector, MailError> {
    native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| MailError::Transport(e.to_string()))
}

/// Autenticación genérica sobre cualquier `Client<T>`, mapeando el rechazo a
/// `Auth`. Con contraseña usa `LOGIN`; con token OAuth2, `AUTHENTICATE XOAUTH2`.
fn auth<T: Read + Write>(
    client: imap::Client<T>,
    cfg: &ServerConfig,
    secret: &Secret,
) -> Result<Session<T>, MailError> {
    match secret {
        Secret::Password(pw) => {
            client.login(&cfg.username, pw).map_err(|(_e, _client)| MailError::Auth)
        }
        Secret::OAuth2(token) => {
            let authr = XOAuth2 { user: cfg.username.clone(), token: token.clone() };
            client.authenticate("XOAUTH2", &authr).map_err(|(_e, _client)| MailError::Auth)
        }
    }
}

/// `LIST "" "*"` → buzones nativos. Genérico sobre el tipo de stream.
fn list_on<T: Read + Write>(s: &mut Session<T>) -> Result<Vec<Mailbox>, MailError> {
    let names = s.list(Some(""), Some("*")).map_err(map_err)?;
    Ok(names.iter().map(|n| Mailbox::new(n.name())).collect())
}

/// `SELECT` + `FETCH` de los últimos `limit` mensajes (o todos si `None`).
fn fetch_on<T: Read + Write>(
    s: &mut Session<T>,
    mailbox: &str,
    limit: Option<usize>,
) -> Result<Vec<Message>, MailError> {
    let meta = s.select(mailbox).map_err(map_err)?;
    let total = meta.exists;
    if total == 0 {
        return Ok(Vec::new());
    }
    // Rango de números de secuencia: los últimos `limit`, hasta el final.
    let start = match limit {
        Some(n) if (n as u32) < total => total - n as u32 + 1,
        _ => 1,
    };
    let range = format!("{start}:*");
    let fetches = s.fetch(range, "(UID FLAGS RFC822)").map_err(map_err)?;
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

/// `UID SEARCH HEADER MESSAGE-ID …` → `UID STORE FLAGS`.
fn set_flags_on<T: Read + Write>(
    s: &mut Session<T>,
    mailbox: &str,
    id: &MessageId,
    flags: Flags,
) -> Result<(), MailError> {
    s.select(mailbox).map_err(map_err)?;
    let inner = id.0.trim_matches(|c| c == '<' || c == '>');
    let uids = s.uid_search(format!("HEADER MESSAGE-ID <{inner}>")).map_err(map_err)?;
    let Some(&uid) = uids.iter().next() else {
        return Err(MailError::UnknownMessage(id.0.clone()));
    };
    s.uid_store(uid.to_string(), format!("FLAGS ({})", imap_flag_string(flags)))
        .map_err(map_err)?;
    Ok(())
}

fn map_err(e: imap::Error) -> MailError {
    if is_auth_error(&e) {
        MailError::Auth
    } else if is_connection_lost(&e) {
        MailError::Disconnected(e.to_string())
    } else {
        MailError::Transport(e.to_string())
    }
}

/// `true` si el error indica que la **sesión IMAP se cayó** (socket cerrado,
/// server dropeó la conexión): reconectar y reintentar tiene sentido. Lo
/// distinguimos de los fallos lógicos (`No`/`Bad` por buzón inexistente, parseo)
/// donde reabrir la sesión no arregla nada. Un error de IO sobre el stream deja
/// la sesión inservible (es stateful), así que cuenta como conexión perdida.
fn is_connection_lost(e: &imap::Error) -> bool {
    matches!(e, imap::Error::ConnectionLost | imap::Error::Io(_))
}

/// `true` si el error IMAP es un **rechazo de autenticación** (credencial/token
/// inválido o vencido) y no un fallo de red/lógico. Es el único caso donde
/// reconectar con un token OAuth fresco ayuda — el resto (`ConnectionLost`, IO,
/// buzón inexistente, parseo) no se arregla reautenticando. Se detecta por la
/// respuesta `NO`/`BAD` con un código de auth (RFC 5530 `AUTHENTICATIONFAILED`/
/// `AUTHORIZATIONFAILED`) o marcadores equivalentes de token vencido.
fn is_auth_error(e: &imap::Error) -> bool {
    let msg = match e {
        imap::Error::No(s) | imap::Error::Bad(s) => s,
        _ => return false,
    };
    let up = msg.to_ascii_uppercase();
    const MARKERS: &[&str] = &[
        "AUTHENTICATIONFAILED",
        "AUTHORIZATIONFAILED",
        "AUTHENTICATION FAILED",
        "INVALID CREDENTIALS",
        "INVALID SASL",
        "TOKEN",   // "invalid token" / "token has expired"
        "EXPIRED",
    ];
    MARKERS.iter().any(|m| up.contains(m))
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

    #[test]
    fn detecta_rechazo_de_autenticacion() {
        // Respuestas NO/BAD con código de auth → reconectar con token fresco ayuda.
        assert!(is_auth_error(&imap::Error::No(
            "[AUTHENTICATIONFAILED] Invalid credentials (Failure)".into()
        )));
        assert!(is_auth_error(&imap::Error::No("[UNAVAILABLE] token has expired".into())));
        assert!(is_auth_error(&imap::Error::Bad("Invalid SASL argument".into())));
        assert!(matches!(map_err(imap::Error::No("[AUTHENTICATIONFAILED] x".into())), MailError::Auth));
    }

    #[test]
    fn distingue_conexion_perdida_de_fallos_logicos() {
        // Conexión caída → Disconnected (reconectar ayuda).
        assert!(is_connection_lost(&imap::Error::ConnectionLost));
        assert!(is_connection_lost(&imap::Error::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "pipe",
        ))));
        assert!(matches!(map_err(imap::Error::ConnectionLost), MailError::Disconnected(_)));
        // Fallos lógicos → Transport (reconectar no los arregla).
        assert!(!is_connection_lost(&imap::Error::No("[NONEXISTENT] Unknown Mailbox".into())));
        assert!(!is_auth_error(&imap::Error::No("[NONEXISTENT] Unknown Mailbox".into())));
        assert!(matches!(
            map_err(imap::Error::No("[NONEXISTENT] Unknown Mailbox".into())),
            MailError::Transport(_)
        ));
        // Un NO de auth no es "conexión perdida" (va por la rama Auth).
        assert!(!is_connection_lost(&imap::Error::No("[AUTHENTICATIONFAILED] x".into())));
    }
}
