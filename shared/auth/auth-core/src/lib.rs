//! `brahman-auth` — autenticación del escritorio.
//!
//! Contrato [`Authenticator`] agnóstico del backend, con dos
//! implementaciones:
//!
//! - [`PamAuthenticator`] — el camino real: verifica contra PAM
//!   (`/etc/pam.d/<servicio>`), el mismo subsistema que usan `login`,
//!   `sudo` y los gestores de login clásicos. Hereda lo que el
//!   administrador configure ahí (2FA, llaves FIDO2, `pam_faillock`…)
//!   sin que `brahman-auth` tenga que saberlo.
//! - [`MockAuthenticator`] — credenciales fijas en memoria, para tests
//!   y para iterar el greeter en cajas sin PAM configurado.
//!
//! Lo consume el greeter de mirada: el usuario teclea su
//! contraseña, el greeter llama a [`Authenticator::authenticate`], y en
//! éxito recibe un [`UserInfo`] con uid/gid/home/shell — lo que el
//! compositor necesita para arrancar la sesión.

mod accion;
mod autologin;
mod pam_backend;
mod ticket;
mod user;

pub use accion::{ShellAction, CANCEL_TAG, UNLOCK_TAG};
pub use autologin::{AutologinCfg, SecretosPolitica};
pub use pam_backend::{PamAuthenticator, DEFAULT_SERVICE};
pub use ticket::{SessionTicket, TICKET_TAG};
pub use user::{resolve_user, UserInfo};

use std::collections::HashMap;

/// Por qué falló una autenticación. Variantes deliberadamente gruesas:
/// el greeter sólo necesita saber si conviene reintentar (problema de
/// credenciales) o si la cuenta está vetada — y nunca debe poder
/// distinguir "usuario inexistente" de "contraseña errada".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// Usuario o contraseña incorrectos. El greeter deja reintentar sin
    /// revelar cuál de los dos falló.
    #[error("usuario o contraseña incorrectos")]
    BadCredentials,

    /// Las credenciales son válidas pero la cuenta está deshabilitada,
    /// expirada o requiere una acción (cambio de contraseña).
    #[error("la cuenta no está disponible: {0}")]
    AccountUnavailable(String),

    /// Fallo del subsistema PAM no atribuible a las credenciales
    /// (servicio mal configurado, módulo roto, etc.).
    #[error("fallo de PAM: {0}")]
    Pam(String),

    /// No se pudo resolver la identidad del usuario en el sistema tras
    /// una autenticación válida (caso raro: `/etc/passwd` inconsistente).
    #[error("no se pudo resolver el usuario «{0}» en el sistema")]
    UnresolvedUser(String),
}

/// Verifica credenciales y, en éxito, entrega la identidad del sistema.
///
/// `&self`: cada llamada es un intento de login independiente. Las
/// implementaciones crean su propio estado por intento — PAM exige un
/// handle nuevo por transacción, reusarlo entre intentos es un bug.
pub trait Authenticator {
    fn authenticate(&self, username: &str, secret: &str) -> Result<UserInfo, AuthError>;
}

/// Autenticador de credenciales fijas en memoria. No toca PAM: sirve
/// para tests y para iterar el greeter en cajas headless sin
/// `/etc/pam.d` configurado.
#[derive(Debug, Default, Clone)]
pub struct MockAuthenticator {
    creds: HashMap<String, String>,
}

impl MockAuthenticator {
    /// Crea un autenticador sin usuarios: todo intento falla con
    /// [`AuthError::BadCredentials`] hasta registrar alguno.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra un par usuario/secreto aceptado. Encadenable.
    pub fn with_user(mut self, username: &str, secret: &str) -> Self {
        self.creds.insert(username.to_string(), secret.to_string());
        self
    }
}

impl Authenticator for MockAuthenticator {
    fn authenticate(&self, username: &str, secret: &str) -> Result<UserInfo, AuthError> {
        // Mismo error para usuario inexistente y para contraseña errada:
        // no filtra la existencia de cuentas.
        match self.creds.get(username) {
            Some(expected) if expected == secret => {
                // Si el usuario existe en el SO, info real; sino,
                // sintética (suficiente para tests y dev headless).
                Ok(resolve_user(username).unwrap_or_else(|_| UserInfo::synthetic(username)))
            }
            _ => Err(AuthError::BadCredentials),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_accepts_registered_user() {
        let auth = MockAuthenticator::new().with_user("sergio", "clave");
        let info = auth.authenticate("sergio", "clave").expect("debe pasar");
        assert_eq!(info.name, "sergio");
    }

    #[test]
    fn mock_rejects_wrong_password() {
        let auth = MockAuthenticator::new().with_user("sergio", "clave");
        assert_eq!(
            auth.authenticate("sergio", "mala"),
            Err(AuthError::BadCredentials)
        );
    }

    #[test]
    fn mock_unknown_user_indistinguishable_from_wrong_password() {
        let auth = MockAuthenticator::new().with_user("sergio", "clave");
        assert_eq!(
            auth.authenticate("nadie", "x"),
            Err(AuthError::BadCredentials)
        );
    }

    #[test]
    fn empty_mock_rejects_everything() {
        assert!(MockAuthenticator::new().authenticate("root", "").is_err());
    }

    #[test]
    fn auth_error_is_displayable() {
        assert!(!AuthError::BadCredentials.to_string().is_empty());
        assert!(AuthError::Pam("x".into()).to_string().contains("PAM"));
    }
}
