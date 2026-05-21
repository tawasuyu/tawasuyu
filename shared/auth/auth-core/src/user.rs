//! Resolución de la identidad de un usuario del sistema.

use std::path::PathBuf;

use crate::AuthError;

/// Identidad de un usuario en el sistema: lo que el compositor necesita
/// para arrancar una sesión — fijar uid/gid, `cd` al home, ejecutar el
/// shell o la sesión de escritorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    /// Nombre de login.
    pub name: String,
    /// User ID.
    pub uid: u32,
    /// Group ID primario.
    pub gid: u32,
    /// Directorio personal.
    pub home: PathBuf,
    /// Shell de login.
    pub shell: PathBuf,
}

impl UserInfo {
    /// Identidad sintética para tests y para cajas donde el usuario no
    /// está en `/etc/passwd`. **No** representa a un usuario real del SO
    /// — no usar para fijar privilegios de un proceso real.
    pub fn synthetic(name: &str) -> Self {
        Self {
            name: name.to_string(),
            uid: 1000,
            gid: 1000,
            home: PathBuf::from(format!("/home/{name}")),
            shell: PathBuf::from("/bin/sh"),
        }
    }
}

/// Resuelve un usuario por nombre vía `getpwnam`. `Err` si no existe o
/// si la consulta a `/etc/passwd` (o NSS) falla.
pub fn resolve_user(name: &str) -> Result<UserInfo, AuthError> {
    match nix::unistd::User::from_name(name) {
        Ok(Some(u)) => Ok(UserInfo {
            name: u.name,
            uid: u.uid.as_raw(),
            gid: u.gid.as_raw(),
            home: u.dir,
            shell: u.shell,
        }),
        Ok(None) => Err(AuthError::UnresolvedUser(name.to_string())),
        Err(e) => Err(AuthError::Pam(format!("getpwnam({name}): {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_root() {
        // root (uid 0) existe en todo sistema Unix.
        let info = resolve_user("root").expect("root debe existir");
        assert_eq!(info.uid, 0);
        assert_eq!(info.name, "root");
    }

    #[test]
    fn unknown_user_errs() {
        let r = resolve_user("usuario-que-no-existe-xyzzy");
        assert!(matches!(r, Err(AuthError::UnresolvedUser(_))));
    }

    #[test]
    fn synthetic_has_home_under_slash_home() {
        let info = UserInfo::synthetic("prueba");
        assert_eq!(info.home, PathBuf::from("/home/prueba"));
        assert_eq!(info.uid, 1000);
    }
}
