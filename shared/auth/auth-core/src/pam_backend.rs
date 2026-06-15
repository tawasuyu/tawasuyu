//! Backend PAM del contrato [`Authenticator`](crate::Authenticator).
//!
//! El módulo se llama `pam_backend` (no `pam`) para no chocar con el
//! crate externo `pam`, del que depende.

use pam::{Client, PamError, PamReturnCode};

use crate::{resolve_user, AuthError, Authenticator, UserInfo};

/// Servicio PAM por defecto del escritorio mirada. Resuelve a
/// `/etc/pam.d/mirada` — ver el archivo `data/mirada` de este crate.
pub const DEFAULT_SERVICE: &str = "mirada";

/// Autentica contra PAM: el mismo subsistema de `login`/`sudo`. Honra
/// `/etc/pam.d/<service>` — módulos, 2FA, llaves FIDO2, `pam_faillock`,
/// lo que el administrador configure ahí, sin que `brahman-auth` lo sepa.
#[derive(Debug, Clone)]
pub struct PamAuthenticator {
    service: String,
}

impl PamAuthenticator {
    /// Autenticador para un servicio PAM concreto (`/etc/pam.d/<service>`).
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Autenticador para el servicio por defecto del escritorio,
    /// [`DEFAULT_SERVICE`].
    pub fn mirada() -> Self {
        Self::new(DEFAULT_SERVICE)
    }

    /// Nombre del servicio PAM que usa este autenticador.
    pub fn service(&self) -> &str {
        &self.service
    }
}

impl Default for PamAuthenticator {
    fn default() -> Self {
        Self::mirada()
    }
}

impl Authenticator for PamAuthenticator {
    fn authenticate(&self, username: &str, secret: &str) -> Result<UserInfo, AuthError> {
        // Un handle PAM nuevo por intento: PAM es stateful por
        // transacción y reusar el handle entre intentos es un bug. El
        // `Client` cierra la transacción (`pam_end`) en su `Drop`.
        let mut client = Client::with_password(&self.service)
            .map_err(|e| AuthError::Pam(format!("pam_start({}): {e}", self.service)))?;
        client.conversation_mut().set_credentials(username, secret);

        // `authenticate()` del crate hace pam_authenticate + pam_acct_mgmt:
        // cubre credenciales Y estado de la cuenta en un solo paso.
        client.authenticate().map_err(map_pam_error)?;

        // Credenciales válidas: resolvemos la identidad del sistema.
        resolve_user(username)
    }
}

/// Traduce un error de PAM a la taxonomía gruesa de [`AuthError`].
fn map_pam_error(err: PamError) -> AuthError {
    match err.0 {
        // Credenciales: el greeter debe dejar reintentar.
        PamReturnCode::Auth_Err
        | PamReturnCode::User_Unknown
        | PamReturnCode::Cred_Insufficient
        | PamReturnCode::MaxTries => AuthError::BadCredentials,

        // Cuenta válida pero vetada o que requiere una acción.
        PamReturnCode::Acct_Expired => AuthError::AccountUnavailable("la cuenta expiró".into()),
        PamReturnCode::Cred_Expired => {
            AuthError::AccountUnavailable("las credenciales expiraron".into())
        }
        PamReturnCode::AuthTok_Expired => {
            AuthError::AccountUnavailable("la contraseña expiró".into())
        }
        PamReturnCode::New_Authtok_Reqd => {
            AuthError::AccountUnavailable("requiere cambiar la contraseña".into())
        }
        PamReturnCode::Perm_Denied => {
            AuthError::AccountUnavailable("acceso denegado por política".into())
        }

        // Todo lo demás: fallo de infraestructura PAM.
        other => AuthError::Pam(format!("{other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirada_uses_default_service() {
        assert_eq!(PamAuthenticator::mirada().service(), DEFAULT_SERVICE);
        assert_eq!(PamAuthenticator::default().service(), "mirada");
    }

    #[test]
    fn custom_service_name() {
        assert_eq!(PamAuthenticator::new("login").service(), "login");
    }

    #[test]
    fn unknown_service_fails_gracefully() {
        // Sin `/etc/pam.d/<servicio>` PAM cae a `other` (deny). Debe
        // devolver un `AuthError`, nunca paniquear.
        let auth = PamAuthenticator::new("brahman-auth-servicio-inexistente-xyz");
        assert!(
            auth.authenticate("root", "contraseña-cualquiera").is_err(),
            "un servicio inexistente debe fallar limpio"
        );
    }
}
