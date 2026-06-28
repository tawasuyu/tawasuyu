//! El **secreto** de una cuenta para autenticar: o una contraseña clásica, o un
//! *access token* OAuth2 (`XOAUTH2`). Lo provee el caller (entorno, keystore, o
//! el helper de autorización `paloma-oauth`); este crate sólo lo usa para el
//! login IMAP/SMTP, no lo persiste.

/// El secreto con el que autenticar contra IMAP/SMTP.
#[derive(Clone)]
pub enum Secret {
    /// Contraseña o app-password: IMAP `LOGIN`, SMTP `AUTH PLAIN/LOGIN`.
    Password(String),
    /// `access_token` OAuth2: IMAP/SMTP `AUTH XOAUTH2` (Bearer).
    OAuth2(String),
}

impl Secret {
    /// Arma la cadena SASL `XOAUTH2` para `user`:
    /// `user=<user>\x01auth=Bearer <token>\x01\x01` (sin base64 — el cliente la
    /// codifica). Sólo válida para [`Secret::OAuth2`].
    pub fn xoauth2_sasl(user: &str, token: &str) -> String {
        format!("user={user}\x01auth=Bearer {token}\x01\x01")
    }
}
