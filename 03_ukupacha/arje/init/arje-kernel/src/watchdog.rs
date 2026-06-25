//! Watchdog de hardware para PID 1. Si el bucle primordial se cuelga y deja
//! de "acariciar" el watchdog, el kernel **reinicia la máquina** en vez de
//! dejarla muerta para siempre. Mejor un reboot que un hang eterno sin
//! consola — un PID 1 atorado es irrecuperable sin esto.
//!
//! Protocolo del watchdog de Linux (`/dev/watchdog`):
//!   - **abrir** el device ARRANCA la cuenta regresiva.
//!   - hay que **acariciarlo** (escribir cualquier byte ≠ `'V'`) antes de que
//!     expire, o el kernel reinicia.
//!   - **cierre mágico**: escribir `'V'` antes de `close` lo DESACTIVA — así un
//!     shutdown limpio no dispara un reboot. (No funciona si el driver se
//!     compiló con `CONFIG_WATCHDOG_NOWAYOUT=y`; ahí el reboot es inevitable
//!     una vez armado — por eso sólo se arma en PID 1 real.)
//!
//! Best-effort de punta a punta: si no hay `/dev/watchdog` (kernel sin
//! watchdog ni `softdog`), `arm` devuelve `None` y el arranque sigue sin
//! watchdog. Nunca es un error duro.

use libc::c_int;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use tracing::{info, warn};

// ioctls del API de watchdog (`linux/watchdog.h`, base `'W'`). En x86_64 y
// aarch64 (los targets de arje) la codificación `_IOC` es la asm-generic.
const WD_IOCTL_BASE: u8 = b'W';
nix::ioctl_readwrite!(wdioc_settimeout, WD_IOCTL_BASE, 6, c_int);
nix::ioctl_read!(wdioc_gettimeout, WD_IOCTL_BASE, 7, c_int);
nix::ioctl_read!(wdioc_setoptions, WD_IOCTL_BASE, 4, c_int);

const WDIOS_DISABLECARD: c_int = 0x0001;
const WDIOS_ENABLECARD: c_int = 0x0002;

/// Intervalo de acariciado para un timeout dado: un tercio (margen 3×),
/// mínimo 1 s. Función libre para poder testearla sin un device real.
fn pet_interval_for(timeout_secs: u32) -> Duration {
    Duration::from_secs((timeout_secs / 3).max(1) as u64)
}

/// Watchdog armado. Mientras viva y se acaricie, el kernel no reinicia.
/// Al `disarm`/drop intenta el cierre mágico para no reiniciar en shutdown
/// limpio.
pub struct Watchdog {
    file: File,
    /// Timeout efectivo (lo que el driver aceptó, puede diferir del pedido).
    timeout_secs: u32,
}

impl Watchdog {
    /// Abre y arma el watchdog con `timeout_secs` (clamp del driver aparte).
    /// `None` si no hay device de watchdog — el arranque sigue sin él.
    pub fn arm(timeout_secs: u32) -> Option<Watchdog> {
        let (file, path) = ["/dev/watchdog", "/dev/watchdog0"]
            .into_iter()
            .find_map(|p| OpenOptions::new().write(true).open(p).ok().map(|f| (f, p)))?;
        let fd = file.as_raw_fd();

        // Pedir el timeout deseado (best-effort: algunos drivers son fijos).
        let mut to: c_int = timeout_secs as c_int;
        // SAFETY: `fd` es válido (recién abierto); el ioctl escribe/lee un int.
        unsafe {
            let _ = wdioc_settimeout(fd, &mut to);
        }
        // Leer el timeout efectivo para calcular el intervalo de acariciado.
        let mut actual: c_int = 0;
        // SAFETY: idem; el ioctl rellena un int.
        let eff = if unsafe { wdioc_gettimeout(fd, &mut actual) }.is_ok() && actual > 0 {
            actual as u32
        } else {
            timeout_secs
        };
        // Asegurar que la tarjeta quede habilitada (best-effort).
        let mut enable: c_int = WDIOS_ENABLECARD;
        // SAFETY: idem.
        unsafe {
            let _ = wdioc_setoptions(fd, &mut enable);
        }

        info!(path, timeout_secs = eff, "watchdog de hardware armado");
        Some(Watchdog { file, timeout_secs: eff })
    }

    /// Cada cuánto hay que acariciar: un tercio del timeout (margen 3×),
    /// mínimo 1 s. El bucle primordial late a este ritmo.
    pub fn pet_interval(&self) -> Duration {
        pet_interval_for(self.timeout_secs)
    }

    /// Acaricia el watchdog (keepalive por escritura de un byte ≠ `'V'`).
    /// Devuelve `Err` si el device murió — el llamador puede desistir.
    pub fn pet(&mut self) -> std::io::Result<()> {
        self.file.write_all(b"\0")
    }

    /// Desarma con cierre mágico (`'V'` + drop). Para un shutdown limpio:
    /// evita que soltar el device reinicie la máquina. Best-effort.
    pub fn disarm(mut self) {
        // 'V' marca "voy a cerrar a propósito" para drivers sin NOWAYOUT.
        if self.file.write_all(b"V").is_ok() {
            info!("watchdog desarmado (cierre mágico)");
        } else {
            warn!("no se pudo escribir el cierre mágico del watchdog — el driver puede reiniciar al soltar");
        }
        // Best-effort: también pedir DISABLECARD por ioctl.
        let mut disable: c_int = WDIOS_DISABLECARD;
        // SAFETY: `fd` válido mientras `self.file` viva (hasta el drop de abajo).
        unsafe {
            let _ = wdioc_setoptions(self.file.as_raw_fd(), &mut disable);
        }
        // `self.file` se cierra acá al salir de scope.
    }
}

#[cfg(test)]
mod tests {
    use super::pet_interval_for;
    use std::time::Duration;

    #[test]
    fn intervalo_es_un_tercio_del_timeout() {
        assert_eq!(pet_interval_for(30), Duration::from_secs(10));
        assert_eq!(pet_interval_for(60), Duration::from_secs(20));
    }

    #[test]
    fn intervalo_nunca_baja_de_un_segundo() {
        // timeout chico (1 ó 2 s) → 1/3 redondea a 0; el clamp lo sube a 1 s.
        assert_eq!(pet_interval_for(1), Duration::from_secs(1));
        assert_eq!(pet_interval_for(2), Duration::from_secs(1));
        assert_eq!(pet_interval_for(0), Duration::from_secs(1));
    }
}
