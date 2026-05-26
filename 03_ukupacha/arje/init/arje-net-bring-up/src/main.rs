//! `arje-net-bring-up` — oneshot Ente que sube el enlace de la primera
//! interfaz no-loopback que encuentra y termina.
//!
//! Forma parte de la genesis del `arje-host.card.json` como Native payload
//! con `Supervision::OneShot`. Sin él, arje-zero arranca el `display-manager`
//! antes de tener red — la pantalla muestra el greeter pero nada en la red
//! responde. Después de este Ente, los chasquis pueden empezar a hablar.
//!
//! ## Por qué ioctl, no rtnetlink
//!
//! `SIOCSIFFLAGS` es la API mas vieja y mas estable de Linux para tocar
//! `IFF_UP`. Funciona en cualquier kernel desde 2.0, no requiere parsear
//! mensajes netlink ni traer una crate externa, y el binario sale debajo
//! de los 200 KB. La asignación de IP queda en otro Ente (un futuro
//! `arje-dhcp`); este sólo enciende el cable.
//!
//! ## Lo que NO hace
//!
//! - No configura IP/DHCP — el siguiente Ente del fractal lo hace.
//! - No deja la interfaz arriba para siempre — si el cable se desconecta,
//!   el OneShot ya terminó. Cualquier "watchdog de link" va en otro Ente.
//! - No autodetecta wifi — sólo Ethernet (interfaces con `eth*`/`enp*`).
//!   El wifi necesita `wpa_supplicant`, dominio de un Ente aparte.

use std::ffi::CStr;
use std::fs;
use std::io;
use std::mem::zeroed;
use std::os::unix::io::RawFd;
use std::process::ExitCode;

use anyhow::{anyhow, Context};

fn main() -> ExitCode {
    match run() {
        Ok(name) => {
            eprintln!("arje-net-bring-up :: {name} arriba");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("arje-net-bring-up :: ERROR {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<String> {
    let iface = first_ethernet_interface()
        .ok_or_else(|| anyhow!("sin interfaz Ethernet (eth*/enp*) detectable bajo /sys/class/net"))?;

    let fd = abrir_socket_inet().context("abriendo socket AF_INET para ioctl")?;
    let _drop = ScopedFd(fd);

    let flags_actuales =
        leer_flags(fd, &iface).with_context(|| format!("SIOCGIFFLAGS de {iface}"))?;

    if flags_actuales & libc::IFF_UP as i16 != 0 {
        eprintln!("arje-net-bring-up :: {iface} ya esta arriba (flags=0x{flags_actuales:x})");
        return Ok(iface);
    }

    let nuevos = flags_actuales | libc::IFF_UP as i16 | libc::IFF_RUNNING as i16;
    escribir_flags(fd, &iface, nuevos).with_context(|| format!("SIOCSIFFLAGS de {iface}"))?;

    Ok(iface)
}

/// Recorre `/sys/class/net/<name>/type` en orden alfabetico y devuelve el
/// primer Ethernet (`type == 1` = `ARPHRD_ETHER`) que no sea loopback.
/// Ordenar por nombre da resultados deterministas en máquinas con varias
/// NICs — `enp0s3` antes que `enp0s8`, por ejemplo.
fn first_ethernet_interface() -> Option<String> {
    let mut nombres: Vec<String> = fs::read_dir("/sys/class/net")
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n != "lo")
        .collect();
    nombres.sort();
    nombres.into_iter().find(|n| es_ethernet(n))
}

fn es_ethernet(iface: &str) -> bool {
    let path = format!("/sys/class/net/{iface}/type");
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    raw.trim().parse::<u32>().ok() == Some(1) // ARPHRD_ETHER
}

fn abrir_socket_inet() -> io::Result<RawFd> {
    // SAFETY: socket() no toca memoria del programa, devuelve fd o -1.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

struct ScopedFd(RawFd);
impl Drop for ScopedFd {
    fn drop(&mut self) {
        // SAFETY: fd valido durante el scope, cerrado una sola vez.
        unsafe {
            libc::close(self.0);
        }
    }
}

fn copy_iface_name(req: &mut libc::ifreq, name: &str) -> anyhow::Result<()> {
    if name.len() >= libc::IFNAMSIZ {
        return Err(anyhow!(
            "nombre de interfaz '{name}' excede IFNAMSIZ ({})",
            libc::IFNAMSIZ
        ));
    }
    for (i, &b) in name.as_bytes().iter().enumerate() {
        req.ifr_name[i] = b as libc::c_char;
    }
    // Quedan los siguientes bytes en 0 — el caller hizo `zeroed()`, asi que
    // el NUL terminador esta garantizado. Sanity:
    let recovered = unsafe { CStr::from_ptr(req.ifr_name.as_ptr()) }
        .to_str()
        .map_err(|_| anyhow!("nombre de interfaz con UTF-8 invalido"))?;
    if recovered != name {
        return Err(anyhow!("copia del nombre de interfaz no roundtripea"));
    }
    Ok(())
}

fn leer_flags(fd: RawFd, iface: &str) -> anyhow::Result<i16> {
    // SAFETY: ifreq se zeroea entero, el nombre se copia con guard.
    let mut req: libc::ifreq = unsafe { zeroed() };
    copy_iface_name(&mut req, iface)?;
    // SAFETY: req contiene un nombre valido, SIOCGIFFLAGS rellena ifr_flags.
    // SIOCGIFFLAGS es u64 en glibc y c_int en musl — `as _` deja que el
    // compilador elija el tipo correcto según el target.
    let r = unsafe { libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut req) };
    if r < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: tras SIOCGIFFLAGS exitoso, ifr_flags es valido.
    let flags = unsafe { req.ifr_ifru.ifru_flags };
    Ok(flags)
}

fn escribir_flags(fd: RawFd, iface: &str, flags: i16) -> anyhow::Result<()> {
    let mut req: libc::ifreq = unsafe { zeroed() };
    copy_iface_name(&mut req, iface)?;
    req.ifr_ifru.ifru_flags = flags;
    // SAFETY: req inicializado, SIOCSIFFLAGS lee ifr_flags.
    let r = unsafe { libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &req) };
    if r < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_lookup_funciona_para_lo() {
        // Loopback existe en cualquier sistema y NO es ARPHRD_ETHER (type=772).
        assert!(!es_ethernet("lo"));
    }

    #[test]
    fn nombre_demasiado_largo_es_error_validacion() {
        let mut req: libc::ifreq = unsafe { zeroed() };
        let largo = "a".repeat(libc::IFNAMSIZ);
        let err = copy_iface_name(&mut req, &largo).unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("excede IFNAMSIZ"), "fue {s}");
    }

    #[test]
    fn nombre_corto_se_copia_y_roundtripea() {
        let mut req: libc::ifreq = unsafe { zeroed() };
        copy_iface_name(&mut req, "eth0").unwrap();
        let recovered = unsafe { CStr::from_ptr(req.ifr_name.as_ptr()) }
            .to_str()
            .unwrap();
        assert_eq!(recovered, "eth0");
    }
}
