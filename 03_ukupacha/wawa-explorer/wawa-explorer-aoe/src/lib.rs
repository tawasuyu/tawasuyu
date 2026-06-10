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

use std::collections::{HashMap, HashSet};
use std::io;
use std::mem::{size_of, zeroed};
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

use akasha::{
    analizar_frame, componer_frame, ErrorAkasha, Mac, MensajeAkasha, ObjectId,
    ETHER_TYPE_AKASHA, MAC_BROADCAST,
};
use thiserror::Error;

/// Cuántas veces reenviamos `SolicitarObjeto` dentro del timeout total cuando
/// nadie nos responde — el broadcast de Ethernet no es confiable, una sola
/// solicitud puede caer en el cable sin que ningún peer la vea. Tres intentos
/// equiespaciados dentro del timeout total dan una probabilidad efectiva de
/// recepción de ~99% incluso a pérdida 50% por intento, sin extender el
/// tiempo total que ve el caller.
pub const INTENTOS_SOLICITAR: u32 = 3;

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

/// Qué pasó durante una sesión de [`ClienteAoE::servir`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EstadisticasServir {
    /// Pedidos respondidos con `ProveedorObjeto`.
    pub servidos: u64,
    /// Pedidos de objetos que no teníamos (`id` desconocido) — ruido normal
    /// del cable, no un error.
    pub ignorados: u64,
    /// Objetos grandes (> `MAX_FRAGMENTO_DATOS`) servidos PARTIDOS en varios
    /// `ProveedorFragmento` (Fase 65). El kernel los reensambla y verifica el
    /// hash sobre el objeto entero.
    pub fragmentados: u64,
}

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

    /// Difunde `AnunciarCanal{...}` — la recomendación FIRMADA de release, el
    /// "apt upgrade en un frame de capa-2". No espera respuesta: los peers que
    /// confíen en `autor` pedirán luego el canal, el manifiesto y los bytecodes
    /// por `SolicitarObjeto` — atendelos con [`servir`]. Los campos firmados
    /// los produce `agora_channel::construir_release` (`canal`, `manifiesto`
    /// como `raiz`, `autor`, `timestamp`, `firma_anuncio`).
    pub fn anunciar_canal(
        &self,
        canal: ObjectId,
        raiz: ObjectId,
        autor: akasha::AutorId,
        timestamp: u64,
        firma: akasha::FirmaAkasha,
    ) -> Result<()> {
        let frame = componer_frame(
            self.my_mac,
            MAC_BROADCAST,
            &MensajeAkasha::AnunciarCanal {
                canal,
                raiz,
                autor,
                timestamp,
                firma,
            },
        )
        .map_err(Error::Aoe)?;
        enviar_frame(&self.fd, self.ifindex, MAC_BROADCAST, &frame)
    }

    /// Atiende `SolicitarObjeto` durante `duracion`: por cada pedido cuyo `id`
    /// esté en `objetos`, responde `ProveedorObjeto(id, datos)` UNICAST al
    /// solicitante. Es el lado servidor del pull AoE — lo que permite a una
    /// wawa corriendo absorber el grafo de un release recién publicado.
    ///
    /// OBJETOS GRANDES (Fase 65): un objeto cuyo payload supere
    /// `akasha::MAX_FRAGMENTO_DATOS` (1024 B) se sirve PARTIDO en varios
    /// `ProveedorFragmento`; el kernel los reensambla y verifica el hash sobre
    /// el objeto entero. Así un `.wasm` real (rimay 3.3 KiB, pluma 11 KiB) ya
    /// viaja. Se cuenta en [`EstadisticasServir::fragmentados`].
    ///
    /// Bloquea el hilo durante `duracion`. Devuelve cuántos pedidos sirvió,
    /// ignoró (id desconocido) y omitió (demasiado grande).
    pub fn servir(
        &self,
        objetos: &HashMap<ObjectId, Vec<u8>>,
        duracion: Duration,
    ) -> Result<EstadisticasServir> {
        let inicio = Instant::now();
        let mut buf = vec![0u8; 65536];
        let mut stats = EstadisticasServir::default();

        loop {
            let restante = match duracion.checked_sub(inicio.elapsed()) {
                Some(r) if !r.is_zero() => r,
                _ => break,
            };
            // Despertamos al menos cada 200 ms para reevaluar `duracion`, así
            // un `servir` largo no queda colgado en un recv eterno.
            setsockopt_rcvtimeo(&self.fd, restante.min(Duration::from_millis(200)))?;

            match recvfrom_frame(&self.fd, &mut buf) {
                Ok(longitud) => {
                    let (origen, mensaje) = match analizar_frame(&buf[..longitud]) {
                        Ok(t) => t,
                        Err(_) => continue, // frame ajeno
                    };
                    if let MensajeAkasha::SolicitarObjeto(id) = mensaje {
                        let Some(datos) = objetos.get(&id) else {
                            stats.ignorados += 1;
                            continue;
                        };
                        if datos.len() > akasha::MAX_FRAGMENTO_DATOS {
                            // Objeto grande: partirlo en `ProveedorFragmento`. El
                            // kernel reensambla y verifica el hash sobre el todo.
                            let total = akasha::total_fragmentos(datos.len());
                            for (i, trozo) in datos.chunks(akasha::MAX_FRAGMENTO_DATOS).enumerate()
                            {
                                let frame = componer_frame(
                                    self.my_mac,
                                    origen,
                                    &MensajeAkasha::ProveedorFragmento {
                                        id,
                                        indice: i as u16,
                                        total,
                                        datos: trozo.to_vec(),
                                    },
                                )
                                .map_err(Error::Aoe)?;
                                enviar_frame(&self.fd, self.ifindex, origen, &frame)?;
                            }
                            stats.fragmentados += 1;
                        } else {
                            let frame = componer_frame(
                                self.my_mac,
                                origen,
                                &MensajeAkasha::ProveedorObjeto(id, datos.clone()),
                            )
                            .map_err(Error::Aoe)?;
                            enviar_frame(&self.fd, self.ifindex, origen, &frame)?;
                            stats.servidos += 1;
                        }
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(stats)
    }

    /// Difunde `SolicitarObjeto(id)` y bloquea hasta recibir
    /// `ProveedorObjeto(id, datos)` con hash coincidente, o hasta `timeout`.
    ///
    /// Reintenta automáticamente: el broadcast Ethernet no es confiable, una
    /// sola solicitud puede no llegar a ningún peer. Equiparte `timeout` en
    /// [`INTENTOS_SOLICITAR`] slots; al cumplirse un slot sin respuesta,
    /// reenvía el pedido. El tiempo total que ve el caller no cambia.
    ///
    /// Verifica `blake3(datos) == id` antes de devolver — si llega un
    /// provider con datos corruptos, los descarta y sigue esperando hasta
    /// que el timeout cumpla. Frames de otros mensajes (anuncios, otras
    /// solicitudes) se ignoran sin tocar el timeout. Si un mismo peer envía
    /// múltiples provider-frames mal-hashed para el mismo id, se le marca
    /// como sospechoso y se descartan futuros frames suyos en este intento
    /// (backpressure básico — evita ser secuestrado por un peer ruidoso).
    pub fn solicitar(&self, id: ObjectId, timeout: Duration) -> Result<Option<Vec<u8>>> {
        self.solicitar_con_reintentos(id, timeout, INTENTOS_SOLICITAR)
    }

    /// Variante explícita de [`solicitar`] que toma el número de intentos.
    /// Con `intentos == 1` el comportamiento es one-shot (la semántica
    /// original previa a la fase de robustez).
    pub fn solicitar_con_reintentos(
        &self,
        id: ObjectId,
        timeout: Duration,
        intentos: u32,
    ) -> Result<Option<Vec<u8>>> {
        let intentos = intentos.max(1);
        let presupuesto_por_intento = presupuesto_por_intento(timeout, intentos);
        let inicio_total = Instant::now();
        let mut buf = vec![0u8; 65536];
        // MACs que mandaron payloads corruptos en este pedido — las ignoramos
        // hasta que el caller vuelva a llamar. Se descarta entre intentos
        // porque el peer puede haber recuperado integridad mientras tanto.
        let mut sospechosos: HashSet<Mac> = HashSet::new();

        for intento in 0..intentos {
            // Enviar (o re-enviar) el broadcast del pedido.
            let pedido = componer_frame(
                self.my_mac,
                MAC_BROADCAST,
                &MensajeAkasha::SolicitarObjeto(id),
            )
            .map_err(Error::Aoe)?;
            enviar_frame(&self.fd, self.ifindex, MAC_BROADCAST, &pedido)?;

            // Cada intento dedica al menos `presupuesto_por_intento` a esperar;
            // el último intento absorbe cualquier remanente del timeout total
            // por redondeo entero, así no perdemos milisegundos al caller.
            let limite_intento = if intento + 1 == intentos {
                timeout
            } else {
                presupuesto_por_intento.saturating_mul((intento + 1) as u32)
            };

            loop {
                let elapsed_total = inicio_total.elapsed();
                let restante_total = match timeout.checked_sub(elapsed_total) {
                    Some(r) if !r.is_zero() => r,
                    _ => return Ok(None),
                };
                let restante_intento = match limite_intento.checked_sub(elapsed_total) {
                    Some(r) if !r.is_zero() => r,
                    // Slot del intento agotado: rompemos para reenviar.
                    _ => break,
                };
                // El socket espera el menor de los dos — un peer ruidoso que
                // ocupe el slot no debe extender más allá del total.
                let espera = restante_intento.min(restante_total);
                setsockopt_rcvtimeo(&self.fd, espera)?;

                match recvfrom_frame(&self.fd, &mut buf) {
                    Ok(longitud) => {
                        let (origen, mensaje) = match analizar_frame(&buf[..longitud]) {
                            Ok(t) => t,
                            Err(_) => continue, // frame ajeno
                        };
                        // Backpressure: un peer ya marcado como sospechoso no
                        // gasta más nuestro tiempo en este pedido.
                        if sospechosos.contains(&origen) {
                            continue;
                        }
                        if let MensajeAkasha::ProveedorObjeto(provider_id, datos) = mensaje
                        {
                            if provider_id != id {
                                continue;
                            }
                            let calculado = *blake3::hash(&datos).as_bytes();
                            if calculado != id {
                                // Provider corrupto. Marcamos al emisor para
                                // no atender más payloads suyos en lo que
                                // resta de este `solicitar`.
                                sospechosos.insert(origen);
                                continue;
                            }
                            return Ok(Some(datos));
                        }
                    }
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        // Timeout del recv: si fue del slot del intento, se
                        // detecta arriba al recalcular `restante_intento`.
                        // Si fue del total, la próxima iteración del loop
                        // sale por `restante_total`.
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }

        Ok(None)
    }
}

/// Calcula el budget por intento como `timeout / intentos` redondeado hacia
/// abajo, garantizando al menos 1 ms — un slot de 0 ms convertiría el
/// reintento en busy-loop sin oportunidad real de recibir.
fn presupuesto_por_intento(timeout: Duration, intentos: u32) -> Duration {
    let intentos = intentos.max(1) as u128;
    let per = timeout.as_nanos() / intentos;
    let nanos = per.max(1_000_000); // piso de 1 ms
    Duration::from_nanos(nanos as u64)
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
    // `request` es c_ulong en glibc y c_int en musl: `as _` adapta el tipo por target.
    let r = unsafe { libc::ioctl(fd.raw(), libc::SIOCGIFINDEX as _, &mut req) };
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
    // `request` es c_ulong en glibc y c_int en musl: `as _` adapta el tipo por target.
    let r = unsafe { libc::ioctl(fd.raw(), libc::SIOCGIFHWADDR as _, &mut req) };
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

    #[test]
    fn presupuesto_por_intento_divide_uniforme() {
        let t = Duration::from_millis(300);
        let p = presupuesto_por_intento(t, 3);
        assert_eq!(p, Duration::from_millis(100));
    }

    #[test]
    fn presupuesto_con_intentos_extremos_no_es_cero() {
        // Aun con muchos intentos en poco tiempo, garantizamos un piso de 1 ms
        // para que no se vuelva busy-loop.
        let p = presupuesto_por_intento(Duration::from_micros(100), 10);
        assert!(p >= Duration::from_millis(1), "p fue {p:?}");
    }

    #[test]
    fn presupuesto_con_un_solo_intento_es_el_total() {
        let t = Duration::from_millis(500);
        let p = presupuesto_por_intento(t, 1);
        assert_eq!(p, t);
    }

    #[test]
    fn fragmentar_y_reensamblar_roundtrip_via_akasha() {
        // Verifica end-to-end el contrato de chunking que `servir` (emisor) y
        // el kernel (receptor) comparten: partir como hace `servir` y reensamblar
        // como hace el kernel devuelve EXACTAMENTE el payload original.
        let payload: Vec<u8> = (0..5000u32).map(|i| (i * 31 + 7) as u8).collect();
        let id: ObjectId = *blake3::hash(&payload).as_bytes();
        let total = akasha::total_fragmentos(payload.len());
        assert!(total > 1, "el payload debe requerir varios fragmentos");

        let mut re = akasha::Reensamblador::nuevo();
        let mut completo = None;
        for (i, trozo) in payload.chunks(akasha::MAX_FRAGMENTO_DATOS).enumerate() {
            completo = re.ingerir(id, i as u16, total, trozo);
        }
        let recon = completo.expect("reensamblado completo");
        assert_eq!(recon, payload);
        // El hash del reensamblado casa con el id — lo que el kernel re-verifica.
        assert_eq!(*blake3::hash(&recon).as_bytes(), id);
    }

    #[test]
    fn presupuesto_redondea_hacia_abajo() {
        // 300 ms / 7 intentos = 42.857 ms cada uno. La división entera baja
        // a 42 ms — el último intento se queda con el remanente porque el
        // `limite_intento` del último iteración usa `timeout` directo.
        let p = presupuesto_por_intento(Duration::from_millis(300), 7);
        assert!(p <= Duration::from_millis(43));
    }
}
