//! `pacha-llavero` — **caché de secretos de sesión** para pacha (Fase 3).
//!
//! Para descifrar al vuelo el store de dotfiles ([`pacha-dotfiles`]) hace falta
//! la clave desbloqueada *en la sesión*: "un secreto para acceder a los
//! secretos". Este crate es ese eslabón, y NADA más — deliberadamente angosto:
//!
//! * [`Llavero`] — el trait: guardar / recuperar / olvidar un secreto de 32
//!   bytes (la **seed de identidad** desbloqueada, típicamente). Es el **punto
//!   de conmutación de política**: cambiar de backend no toca a quien lo usa.
//! * [`LlaveroKernel`] — backend real sobre el **session keyring del kernel**
//!   (`add_key`/`keyctl`). La vida del secreto la gobierna el kernel: vive
//!   mientras viva la sesión, se evapora al logout, nunca toca disco ni el
//!   espacio de direcciones de userspace de forma legible por otro proceso sin
//!   permiso. Es el análogo de cómo `ssh-agent` retiene llaves.
//! * [`LlaveroMemoria`] — backend en RAM del proceso (tests / headless / opt-out).
//!
//! **Desacople de la cripto a propósito:** este crate no conoce `Cifrador` ni
//! HKDF. Maneja 32 bytes opacos. Quien lo usa (`pacha-manager`) hace
//! `Cifrador::derivar_de_seed(&secreto)` con lo que recupera. Así la elección de
//! backend de desbloqueo (kernel hoy; mañana PAM/TPM/passphrase+Argon2/greeter)
//! NO está cementada: se cambia la impl de [`Llavero`], no la cripto ni el manager.
//!
//! *De dónde sale la seed la PRIMERA vez* (pedir passphrase, abrir
//! `agora-keystore`) es responsabilidad del orquestador, no de acá: este crate
//! sólo retiene lo ya desbloqueado para no re-preguntar en cada conmutación.

#![cfg(unix)]

use thiserror::Error;

/// Un secreto de sesión: 32 bytes (p.ej. la seed Ed25519 de identidad).
pub type Secreto = [u8; 32];

/// Errores del llavero.
#[derive(Debug, Error)]
pub enum LlaveroError {
    #[error("nombre de secreto inválido (NUL interior)")]
    NombreInvalido,
    #[error("keyring del kernel: {0}")]
    Kernel(std::io::Error),
    #[error("secreto de tamaño inesperado: {0} bytes (esperaba 32)")]
    TamanoInesperado(usize),
}

/// Caché de secretos de sesión con backend enchufable. `Send + Sync` para que el
/// manager lo comparta entre tareas.
pub trait Llavero: Send + Sync {
    /// Guarda (o reemplaza) el secreto bajo `nombre`.
    fn guardar(&self, nombre: &str, secreto: &Secreto) -> Result<(), LlaveroError>;
    /// Recupera el secreto si está cacheado; `None` si no.
    fn recuperar(&self, nombre: &str) -> Result<Option<Secreto>, LlaveroError>;
    /// Olvida el secreto (idempotente: no error si no estaba).
    fn olvidar(&self, nombre: &str) -> Result<(), LlaveroError>;
}

// =====================================================================
// LlaveroMemoria — RAM del proceso (tests / headless / opt-out)
// =====================================================================

/// Backend en memoria del proceso. No persiste entre procesos ni sobrevive al
/// fin del programa. Útil en tests y donde no haya keyring del kernel.
#[derive(Default)]
pub struct LlaveroMemoria {
    mapa: std::sync::Mutex<std::collections::HashMap<String, Secreto>>,
}

impl LlaveroMemoria {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Llavero for LlaveroMemoria {
    fn guardar(&self, nombre: &str, secreto: &Secreto) -> Result<(), LlaveroError> {
        self.mapa.lock().unwrap().insert(nombre.to_string(), *secreto);
        Ok(())
    }
    fn recuperar(&self, nombre: &str) -> Result<Option<Secreto>, LlaveroError> {
        Ok(self.mapa.lock().unwrap().get(nombre).copied())
    }
    fn olvidar(&self, nombre: &str) -> Result<(), LlaveroError> {
        self.mapa.lock().unwrap().remove(nombre);
        Ok(())
    }
}

// =====================================================================
// LlaveroKernel — session keyring del kernel Linux
// =====================================================================

/// Backend sobre el **session keyring** del kernel. Cada secreto es una key de
/// tipo `user` cuya descripción es `<prefijo><nombre>`. El kernel le pone la
/// vida (sesión) y los permisos; al logout desaparece. `add_key` sobre una
/// descripción existente **reemplaza** el payload (idempotente).
pub struct LlaveroKernel {
    prefijo: String,
}

impl Default for LlaveroKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl LlaveroKernel {
    /// Prefijo de namespacing `pacha:` (evita choque con otras apps del session
    /// keyring).
    pub fn new() -> Self {
        Self { prefijo: "pacha:".into() }
    }

    /// Con un prefijo de namespacing propio.
    pub fn con_prefijo(prefijo: impl Into<String>) -> Self {
        Self { prefijo: prefijo.into() }
    }

    /// `true` si el session keyring es usable acá (algunos entornos sin sesión
    /// de usuario o con LSM restrictivo lo niegan). Proba con una key efímera.
    pub fn disponible() -> bool {
        let k = LlaveroKernel::con_prefijo("pacha-probe:");
        let probe = [0u8; 32];
        match k.guardar("disponibilidad", &probe) {
            Ok(()) => {
                let _ = k.olvidar("disponibilidad");
                true
            }
            Err(_) => false,
        }
    }

    fn desc(&self, nombre: &str) -> Result<std::ffi::CString, LlaveroError> {
        std::ffi::CString::new(format!("{}{}", self.prefijo, nombre))
            .map_err(|_| LlaveroError::NombreInvalido)
    }
}

// El tipo de key del kernel y el keyring destino.
const TIPO_USER: &[u8] = b"user\0";
const KEY_SPEC_SESSION_KEYRING: libc::c_long = -3;
const KEYCTL_UNLINK: libc::c_long = 9;
const KEYCTL_SEARCH: libc::c_long = 10;
const KEYCTL_READ: libc::c_long = 11;

impl Llavero for LlaveroKernel {
    fn guardar(&self, nombre: &str, secreto: &Secreto) -> Result<(), LlaveroError> {
        let desc = self.desc(nombre)?;
        // SAFETY: add_key con punteros válidos y longitudes correctas; el payload
        // es nuestro &[u8;32] vivo durante la llamada. Devuelve serial o -1/errno.
        let r = unsafe {
            libc::syscall(
                libc::SYS_add_key,
                TIPO_USER.as_ptr(),
                desc.as_ptr(),
                secreto.as_ptr() as *const libc::c_void,
                secreto.len() as libc::size_t,
                KEY_SPEC_SESSION_KEYRING,
            )
        };
        if r < 0 {
            return Err(LlaveroError::Kernel(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn recuperar(&self, nombre: &str) -> Result<Option<Secreto>, LlaveroError> {
        let desc = self.desc(nombre)?;
        // Buscar la key en el session keyring.
        // SAFETY: KEYCTL_SEARCH con punteros válidos; dest keyring 0 (no relink).
        let serial = unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_SEARCH,
                KEY_SPEC_SESSION_KEYRING,
                TIPO_USER.as_ptr(),
                desc.as_ptr(),
                0 as libc::c_long,
            )
        };
        if serial < 0 {
            let e = std::io::Error::last_os_error();
            // ENOKEY = no está cacheada: es un caso normal, no un error.
            if e.raw_os_error() == Some(libc::ENOKEY) {
                return Ok(None);
            }
            return Err(LlaveroError::Kernel(e));
        }
        // Leer el payload. KEYCTL_READ devuelve la longitud real (puede exceder
        // el buffer; acá el secreto es de 32, fijamos buffer de 32).
        let mut buf = [0u8; 32];
        // SAFETY: KEYCTL_READ con la serial recién hallada y un buffer de 32.
        let len = unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_READ,
                serial,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len() as libc::size_t,
            )
        };
        if len < 0 {
            return Err(LlaveroError::Kernel(std::io::Error::last_os_error()));
        }
        if len as usize != buf.len() {
            return Err(LlaveroError::TamanoInesperado(len as usize));
        }
        Ok(Some(buf))
    }

    fn olvidar(&self, nombre: &str) -> Result<(), LlaveroError> {
        let desc = self.desc(nombre)?;
        // SAFETY: buscar y desvincular; ENOKEY = ya no estaba (idempotente).
        let serial = unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_SEARCH,
                KEY_SPEC_SESSION_KEYRING,
                TIPO_USER.as_ptr(),
                desc.as_ptr(),
                0 as libc::c_long,
            )
        };
        if serial < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ENOKEY) {
                return Ok(());
            }
            return Err(LlaveroError::Kernel(e));
        }
        // SAFETY: KEYCTL_UNLINK de la serial respecto del session keyring.
        let r = unsafe {
            libc::syscall(libc::SYS_keyctl, KEYCTL_UNLINK, serial, KEY_SPEC_SESSION_KEYRING)
        };
        if r < 0 {
            return Err(LlaveroError::Kernel(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memoria_round_trip() {
        let ll = LlaveroMemoria::new();
        assert!(ll.recuperar("seed").unwrap().is_none());
        let s = [42u8; 32];
        ll.guardar("seed", &s).unwrap();
        assert_eq!(ll.recuperar("seed").unwrap(), Some(s));
        ll.olvidar("seed").unwrap();
        assert!(ll.recuperar("seed").unwrap().is_none());
        // Olvidar lo ausente no rompe.
        ll.olvidar("seed").unwrap();
    }

    #[test]
    fn kernel_round_trip_en_session_keyring() {
        if !LlaveroKernel::disponible() {
            eprintln!("session keyring no disponible, salteando");
            return;
        }
        // Prefijo único por test para no chocar con corridas paralelas/previas.
        let ll = LlaveroKernel::con_prefijo("pacha-test-llavero:");
        let nombre = "seed-de-prueba";
        let _ = ll.olvidar(nombre);

        assert!(ll.recuperar(nombre).unwrap().is_none(), "no debía estar cacheada");
        let secreto = [7u8; 32];
        ll.guardar(nombre, &secreto).unwrap();
        assert_eq!(ll.recuperar(nombre).unwrap(), Some(secreto), "round-trip por el kernel");

        // Reemplazo idempotente.
        let otro = [9u8; 32];
        ll.guardar(nombre, &otro).unwrap();
        assert_eq!(ll.recuperar(nombre).unwrap(), Some(otro));

        ll.olvidar(nombre).unwrap();
        assert!(ll.recuperar(nombre).unwrap().is_none(), "tras olvidar ya no está");
    }
}
