//! `wawa-explorer-aoe` — cliente Akasha Over Ether (AoE) sobre raw sockets.
//!
//! Habla AoE (`EtherType 0x88B5`) directo sobre Ethernet de capa 2 — sin IP,
//! sin TCP — con peers Wawa que escuchen en la misma red local. Sirve para:
//!
//! - **Pedir un objeto por su hash BLAKE3** (`solicitar`): difunde
//!   `SolicitarObjeto(id)`, bloquea hasta que llegue `ProveedorObjeto(id, d)`
//!   con `id` coincidente o se agote el timeout. Verifica `blake3(d) == id`
//!   antes de devolver — el protocolo lo exige por contrato.
//! - **Anunciar la raíz local** (`anunciar_raiz`): difunde `AnunciarRaiz(id)`.
//!
//! ## Permisos
//!
//! Los raw sockets (`AF_PACKET`) requieren `CAP_NET_RAW` o root. Para uso
//! cotidiano:
//!
//! ```sh
//! sudo setcap cap_net_raw=eip target/release/wawa-explorer-llimphi
//! ```
//!
//! ## Por qué SOCK_RAW (no SOCK_DGRAM)
//!
//! `akasha::componer_frame` arma el frame Ethernet completo (dst + src +
//! ethertype + payload). Usar `SOCK_RAW` deja al kernel transmitirlo tal
//! cual; `SOCK_DGRAM` haría que el kernel añadiera otra cabecera y
//! tendríamos dos. Misma ruta que usa el kernel de Wawa cuando inyecta
//! frames al cable.

#![deny(unsafe_op_in_unsafe_fn)]

use std::io;
use std::mem::{size_of, zeroed};
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::Duration;

use akasha::{
    analizar_frame, componer_frame, ErrorAkasha, Mac, MensajeAkasha, ObjectId,
    ETHER_TYPE_AKASHA, MAC_BROADCAST,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("AoE: {0:?}")]
    Aoe(ErrorAkasha),
    #[error("interfaz '{0}' no encontrada o sin permiso")]
    InterfazInaccesible(String),
    #[error("nombre de interfaz demasiado largo: {0} (límite del kernel: IFNAMSIZ-1)")]
    NombreInterfazLargo(usize),
    #[error("provider devolvió hash incorrecto: esperado {esperado}, recibido {recibido}")]
    HashIncorrecto { esperado: String, recibido: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Cliente conectado a una interfaz Ethernet específica.
///
/// El descriptor de socket se cierra al hacer `drop` — `Send` pero no `Sync`
/// (el syscall `recvfrom` no es seguro de compartir entre hilos sin lock).
#[derive(Debug)]
pub struct ClienteAoE {
    fd: OwnedFd,
    ifindex: i32,
    my_mac: Mac,
}

/// RAII para el descriptor: cierra en Drop.
#[derive(Debug)]
struct OwnedFd(RawFd);

impl OwnedFd {
    fn raw(&self) -> RawFd {
        self.0
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        // SAFETY: el fd vive solo dentro de OwnedFd; al Drop nadie más lo usa.
        unsafe {
            libc::close(self.0);
        }
    }
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl ClienteAoE {
    /// Crea el cliente atado a la interfaz dada (ej `"eth0"`, `"wlp3s0"`).
    pub fn nuevo(nombre_iface: &str) -> Result<Self> {
        if nombre_iface.len() >= libc::IFNAMSIZ {
            return Err(Error::NombreInterfazLargo(nombre_iface.len()));
        }

        let fd = abrir_socket()?;
        let ifindex = ifindex_de(&fd, nombre_iface)?;
        let my_mac = mac_de(&fd, nombre_iface)?;
        bind_a_interfaz(&fd, ifindex)?;

        Ok(Self { fd, ifindex, my_mac })
    }

    pub fn mac_local(&self) -> Mac {
        self.my_mac
    }

    pub fn ifindex(&self) -> i32 {
        self.ifindex
    }

    /// Difunde `AnunciarRaiz(id)`. No espera respuesta.
    pub fn anunciar_raiz(&self, id: ObjectId) -> Result<()> {
        let frame =
            componer_frame(self.my_mac, MAC_BROADCAST, &MensajeAkasha::AnunciarRaiz(id))
                .map_err(Error::Aoe)?;
        enviar_frame(&self.fd, self.ifindex, MAC_BROADCAST, &frame)
    }

    /// Difunde `SolicitarObjeto(id)` y bloquea hasta recibir
    /// `ProveedorObjeto(id, datos)` con hash coincidente, o hasta `timeout`.
    ///
    /// Verifica `blake3(datos) == id` antes de devolver — si llega un
    /// provider con datos corruptos, los descarta y sigue esperando hasta
    /// que el timeout cumpla. Frames de otros mensajes (anuncios, otras
    /// solicitudes) se ignoran sin tocar el timeout.
    pub fn solicitar(&self, id: ObjectId, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let pedido = componer_frame(
            self.my_mac,
            MAC_BROADCAST,
            &MensajeAkasha::SolicitarObjeto(id),
        )
        .map_err(Error::Aoe)?;
        enviar_frame(&self.fd, self.ifindex, MAC_BROADCAST, &pedido)?;

        setsockopt_rcvtimeo(&self.fd, timeout)?;

        let mut buf = vec![0u8; 65536];
        let inicio = std::time::Instant::now();
        loop {
            let restante = match timeout.checked_sub(inicio.elapsed()) {
                Some(r) if !r.is_zero() => r,
                _ => return Ok(None),
            };
            // Ajustar el timeout del socket al tiempo restante — un peer
            // ruidoso que mande mensajes ajenos no debe extender la espera.
            setsockopt_rcvtimeo(&self.fd, restante)?;

            match recvfrom_frame(&self.fd, &mut buf) {
                Ok(longitud) => {
                    let (_, mensaje) = match analizar_frame(&buf[..longitud]) {
                        Ok(t) => t,
                        Err(_) => continue, // frame ajeno, sigue esperando
                    };
                    if let MensajeAkasha::ProveedorObjeto(provider_id, datos) = mensaje {
                        if provider_id != id {
                            continue; // provider de otro objeto
                        }
                        // Contrato AoE: verificar blake3(datos) == id.
                        let calculado = *blake3::hash(&datos).as_bytes();
                        if calculado != id {
                            // Provider malicioso o corrupto. Descartar y
                            // seguir — quizás haya otro peer con el bueno.
                            continue;
                        }
                        return Ok(Some(datos));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

// =============================================================================
//  Plomería libc — todo el unsafe queda contenido aquí
// =============================================================================

fn abrir_socket() -> Result<OwnedFd> {
    // SAFETY: socket() no toca memoria del programa; devuelve fd o -1.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW,
            (ETHER_TYPE_AKASHA as u16).to_be() as i32,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(OwnedFd(fd))
}

fn ifindex_de(fd: &OwnedFd, nombre: &str) -> Result<i32> {
    // SAFETY: ifreq se zeroea entero antes de tocarse; copiamos el nombre
    // dentro de su buffer con NUL terminador y un len verificado < IFNAMSIZ.
    let mut req: libc::ifreq = unsafe { zeroed() };
    let bytes = nombre.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        req.ifr_name[i] = b as libc::c_char;
    }
    // SAFETY: req contiene un nombre válido NUL-terminated.
    let r = unsafe { libc::ioctl(fd.raw(), libc::SIOCGIFINDEX, &mut req) };
    if r < 0 {
        let err = io::Error::last_os_error();
        if matches!(err.raw_os_error(), Some(libc::ENODEV) | Some(libc::ENOTTY)) {
            return Err(Error::InterfazInaccesible(nombre.to_string()));
        }
        return Err(err.into());
    }
    // SAFETY: ifr_ifindex es válido tras un ioctl SIOCGIFINDEX exitoso.
    let idx = unsafe { req.ifr_ifru.ifru_ifindex };
    Ok(idx)
}

fn mac_de(fd: &OwnedFd, nombre: &str) -> Result<Mac> {
    let mut req: libc::ifreq = unsafe { zeroed() };
    let bytes = nombre.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        req.ifr_name[i] = b as libc::c_char;
    }
    // SAFETY: req con nombre válido; SIOCGIFHWADDR rellena ifr_hwaddr.
    let r = unsafe { libc::ioctl(fd.raw(), libc::SIOCGIFHWADDR, &mut req) };
    if r < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: tras SIOCGIFHWADDR exitoso, ifr_hwaddr.sa_data contiene los 6
    // bytes de la MAC.
    let sa_data = unsafe { req.ifr_ifru.ifru_hwaddr.sa_data };
    let mut mac = [0u8; 6];
    for i in 0..6 {
        mac[i] = sa_data[i] as u8;
    }
    Ok(mac)
}

fn bind_a_interfaz(fd: &OwnedFd, ifindex: i32) -> Result<()> {
    let mut addr: libc::sockaddr_ll = unsafe { zeroed() };
    addr.sll_family = libc::AF_PACKET as u16;
    addr.sll_protocol = (ETHER_TYPE_AKASHA as u16).to_be();
    addr.sll_ifindex = ifindex;
    // SAFETY: sockaddr_ll inicializado, longitud correcta.
    let r = unsafe {
        libc::bind(
            fd.raw(),
            &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
            size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

fn setsockopt_rcvtimeo(fd: &OwnedFd, t: Duration) -> Result<()> {
    let tv = libc::timeval {
        tv_sec: t.as_secs() as libc::time_t,
        tv_usec: t.subsec_micros() as libc::suseconds_t,
    };
    // SAFETY: timeval bien formado, tamaño correcto.
    let r = unsafe {
        libc::setsockopt(
            fd.raw(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const libc::timeval as *const libc::c_void,
            size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

fn enviar_frame(fd: &OwnedFd, ifindex: i32, dst: Mac, frame: &[u8]) -> Result<()> {
    let mut addr: libc::sockaddr_ll = unsafe { zeroed() };
    addr.sll_family = libc::AF_PACKET as u16;
    addr.sll_protocol = (ETHER_TYPE_AKASHA as u16).to_be();
    addr.sll_ifindex = ifindex;
    addr.sll_halen = 6;
    addr.sll_addr[..6].copy_from_slice(&dst);

    // SAFETY: addr inicializado, frame es slice contiguo de bytes.
    let r = unsafe {
        libc::sendto(
            fd.raw(),
            frame.as_ptr() as *const libc::c_void,
            frame.len(),
            0,
            &addr as *const libc::sockaddr_ll as *const libc::sockaddr,
            size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

fn recvfrom_frame(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: buf es slice mutable; le pedimos al kernel copiar hasta buf.len() bytes.
    let n = unsafe {
        libc::recvfrom(
            fd.raw(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El cliente solo se puede crear con CAP_NET_RAW. Test gated por euid
    /// para no fallar en CI sin permisos. Corré con `cargo test -- --ignored`
    /// tras `sudo setcap` o `sudo`.
    #[test]
    #[ignore]
    fn nuevo_con_loopback_funciona_con_caps() {
        let cliente = ClienteAoE::nuevo("lo").expect("requiere CAP_NET_RAW");
        // loopback tiene MAC 00:00:00:00:00:00
        assert_eq!(cliente.mac_local(), [0; 6]);
        assert!(cliente.ifindex() > 0);
    }

    #[test]
    fn nombre_de_interfaz_largo_es_error_de_validacion() {
        let largo = "a".repeat(libc::IFNAMSIZ);
        let err = ClienteAoE::nuevo(&largo).unwrap_err();
        assert!(matches!(err, Error::NombreInterfazLargo(_)), "fue {err:?}");
    }
}
