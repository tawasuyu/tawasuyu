//! Modo `--akasha <iface>` del puente — bind a un socket AF_PACKET
//! SOCK_DGRAM filtrado por `ETHERTYPE_ASISTENTE`, recvfrom en bucle, y
//! sendto al broadcast con la propuesta empaquetada.
//!
//! Usar SOCK_DGRAM (en lugar de SOCK_RAW) significa que el kernel
//! gestiona la cabecera Ethernet por nosotros: el `recvfrom` entrega solo
//! el payload + un `sockaddr_ll` con la MAC origen; el `sendto` toma el
//! payload y el sockaddr_ll destino, y el kernel construye la cabecera.
//! Eso evita tener que tocar Ethernet a mano — y, según las capabilities
//! de Linux, SOCK_DGRAM con AF_PACKET puede no requerir `cap_net_raw`
//! aunque sí necesita `cap_net_bind_service` (cf. `man 7 packet`). El
//! invocante decide si correr el binario con sudo o con setcap.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;

use asistente_puente::{
    construir_frame, construir_prompt_usuario, traducir_propuesta_llm, InterpretacionLlm,
    PROMPT_SISTEMA_WAWA,
};
use format::{leer_cabecera_cable, Contexto, TipoCable, ETHERTYPE_ASISTENTE, TAM_CABECERA_CABLE};
use pluma_llm_core::{ChatClient, ChatRequest};

/// Cota dura del payload de una consulta. Frames Ethernet sin VLAN
/// permiten hasta 1500 bytes de payload; nuestra cabecera del cable
/// (12 B) deja unos 1488 para el prompt. Mas que suficiente.
const RX_MAX: usize = 2048;

const MAX_TOKENS_RESPUESTA: u32 = 500;

/// Punto de entrada del modo Akasha. Bloquea hasta que el socket se
/// cierra o un error fatal lo tumba.
pub fn correr(
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
    iface: &str,
) -> Result<(), String> {
    let ifindex = resolver_ifindex(iface)?;
    let fd = abrir_socket(ifindex)?;
    eprintln!(
        "asistente-puente: AF_PACKET SOCK_DGRAM bindeado a {iface} (ifindex={ifindex}), \
         escuchando EtherType 0x{:04X}",
        ETHERTYPE_ASISTENTE,
    );
    eprintln!("asistente-puente: Ctrl-C para terminar");
    bucle(fd, llm, rt, ifindex)
}

/// Resuelve el indice numerico de una interfaz por nombre via
/// `if_nametoindex(3)`. Devuelve un mensaje claro si el nombre no
/// existe en el host.
fn resolver_ifindex(iface: &str) -> Result<i32, String> {
    let c_name = std::ffi::CString::new(iface)
        .map_err(|_| format!("nombre de interfaz '{iface}' contiene NUL"))?;
    let idx = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if idx == 0 {
        return Err(format!(
            "if_nametoindex('{iface}') fallo (¿interfaz inexistente? errno={})",
            io::Error::last_os_error()
        ));
    }
    Ok(idx as i32)
}

/// Crea y bindea un `AF_PACKET SOCK_DGRAM` filtrado por
/// `ETHERTYPE_ASISTENTE`. El `OwnedFd` devuelto se cierra al drop.
fn abrir_socket(ifindex: i32) -> Result<OwnedFd, String> {
    let proto = (ETHERTYPE_ASISTENTE as i32).to_be();
    let raw = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM, proto) };
    if raw < 0 {
        return Err(format!(
            "socket(AF_PACKET, SOCK_DGRAM) fallo: {} (¿faltan privilegios?, ver setcap cap_net_raw)",
            io::Error::last_os_error()
        ));
    }
    // SEGURIDAD: el descriptor recien se acaba de crear, no esta
    // duplicado, y vive en el frame; lo movemos a OwnedFd para que el
    // close se haga en Drop.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };

    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (ETHERTYPE_ASISTENTE as u16).to_be();
    sll.sll_ifindex = ifindex;
    let ret = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&sll as *const libc::sockaddr_ll) as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        return Err(format!("bind sockaddr_ll fallo: {}", io::Error::last_os_error()));
    }
    Ok(fd)
}

fn bucle(
    fd: OwnedFd,
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
    ifindex: i32,
) -> Result<(), String> {
    let mut buf = [0u8; RX_MAX];
    loop {
        let n = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("recv fallo: {err}"));
        }
        let frame = &buf[..n as usize];
        if frame.len() < TAM_CABECERA_CABLE {
            // Demasiado corto — el kernel ya filtro por EtherType, asi
            // que esto solo pasaria con un paquete deliberadamente raro.
            continue;
        }
        let Some((tipo, id)) = leer_cabecera_cable(frame) else {
            continue;
        };
        if tipo != TipoCable::Consulta {
            // Otra cosa que el cable trajo — Propuesta/Error suelen ser
            // ecos de respuestas previas en el broadcast; ignorar.
            continue;
        }
        let payload = &frame[TAM_CABECERA_CABLE..];
        let prompt = match std::str::from_utf8(payload) {
            Ok(s) => s.to_string(),
            Err(_) => String::from_utf8_lossy(payload).into_owned(),
        };
        eprintln!(
            "asistente-puente: recibi Consulta id={id} prompt={:?} ({} B)",
            prompt,
            payload.len()
        );
        let interp = consultar_llm(llm, rt, &prompt);
        // Empaquetar la propuesta y reenviarla por el mismo socket al
        // broadcast — la app `asistente.wasm` filtra por id.
        let respuesta = construir_frame(id, &interp);
        if let Err(e) = enviar_broadcast(&fd, ifindex, &respuesta) {
            eprintln!("asistente-puente: sendto fallo: {e}");
        } else {
            eprintln!(
                "asistente-puente: envie {tipo_resp:?} id={id} ({n} B)",
                tipo_resp = empaqueta_tipo(&interp),
                n = respuesta.len(),
            );
        }
    }
}

/// Resumen del tipo de respuesta para el log — no toca el cable.
fn empaqueta_tipo(interp: &InterpretacionLlm) -> TipoCable {
    let (tipo, _) = asistente_puente::empaquetar_cable(interp);
    tipo
}

fn consultar_llm(
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
    prompt: &str,
) -> InterpretacionLlm {
    // V3 minimo: el contexto va vacio porque la app `asistente.wasm`
    // todavia no nos lo envia. Cuando v4 sume `Contexto` al payload,
    // este caller lo deserializa y lo pasa aqui.
    let ctx = Contexto::default();
    let user = construir_prompt_usuario(&ctx, prompt);
    let req = ChatRequest::una_vuelta(user, MAX_TOKENS_RESPUESTA).con_sistema(PROMPT_SISTEMA_WAWA);
    let resp = rt.block_on(llm.complete(&req));
    match resp {
        Ok(r) => traducir_propuesta_llm(&r.content),
        Err(e) => InterpretacionLlm::Error(format!("transporte LLM: {e}")),
    }
}

/// Envia un payload al broadcast (FF:FF:FF:FF:FF:FF) sobre la interfaz
/// bindeada. AF_PACKET SOCK_DGRAM se encarga de pegar la cabecera
/// Ethernet con la MAC origen del socket.
fn enviar_broadcast(fd: &OwnedFd, ifindex: i32, payload: &[u8]) -> Result<(), String> {
    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (ETHERTYPE_ASISTENTE as u16).to_be();
    sll.sll_ifindex = ifindex;
    sll.sll_halen = 6;
    sll.sll_addr[..6].copy_from_slice(&[0xFFu8; 6]);
    let ret = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            payload.as_ptr() as *const libc::c_void,
            payload.len(),
            0,
            (&sll as *const libc::sockaddr_ll) as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        Err(format!("sendto: {}", io::Error::last_os_error()))
    } else {
        Ok(())
    }
}
