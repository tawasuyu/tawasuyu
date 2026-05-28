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
use std::time::Duration;

use asistente_puente::{
    construir_frame, construir_frame_error, construir_frame_firma, construir_prompt_usuario,
    leer_request_firma, traducir_propuesta_llm, InterpretacionLlm, PROMPT_SISTEMA_WAWA,
};
use format::{
    leer_cabecera_cable, Contexto, TipoCable, ETHERTYPE_ASISTENTE, TAM_CABECERA_CABLE,
    TIPO_OBJETO_CONFIGURACION, TIPO_OBJETO_CUADERNO,
};
use pluma_llm_core::{ChatClient, ChatRequest};

/// Cota dura del payload de una consulta. Frames Ethernet sin VLAN
/// permiten hasta 1500 bytes de payload; nuestra cabecera del cable
/// (12 B) deja unos 1488 para el prompt. Mas que suficiente.
const RX_MAX: usize = 2048;

const MAX_TOKENS_RESPUESTA: u32 = 500;

/// Fase 60 v4 :: tiempo maximo que el puente espera la decision del
/// operador (y/N) por stdin. Alineado con el del `wawactl daemon-firma`
/// para que el operador encuentre la misma cadencia entre ambos.
const TIMEOUT_FIRMA: Duration = Duration::from_secs(30);

/// Fase 60 v4 :: configuracion de firma del puente. Si `correr` recibe
/// `None`, cualquier RequestFirma se rechaza con un Error explicito.
pub struct ConfigFirma {
    pub clave: ed25519_compact::SecretKey,
    pub slot: u8,
    pub log_path: String,
}

/// Fase 60 v4 :: carga la clave Ed25519 desde un archivo. Mismo formato
/// que `wawactl daemon-firma`: 32 B (seed) o 64 B (SecretKey expandida).
/// Mantenemos el contrato cruzado para que el operador pueda compartir
/// el mismo `.sk` entre ambos demonios sin recablear.
pub fn cargar_clave_privada(path: &str) -> Result<ed25519_compact::SecretKey, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("no pude leer --firma-clave {path}: {e}"))?;
    match bytes.len() {
        32 => {
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(seed));
            Ok(kp.sk)
        }
        64 => ed25519_compact::SecretKey::from_slice(&bytes)
            .map_err(|e| format!("SecretKey invalida en {path}: {e}")),
        n => Err(format!(
            "la clave debe traer 32 (seed) o 64 (SecretKey) bytes; {path} trae {n}"
        )),
    }
}

/// Punto de entrada del modo Akasha. Bloquea hasta que el socket se
/// cierra o un error fatal lo tumba.
pub fn correr(
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
    iface: &str,
    firma: Option<&ConfigFirma>,
) -> Result<(), String> {
    let ifindex = resolver_ifindex(iface)?;
    let fd = abrir_socket(ifindex)?;
    eprintln!(
        "asistente-puente: AF_PACKET SOCK_DGRAM bindeado a {iface} (ifindex={ifindex}), \
         escuchando EtherType 0x{:04X}",
        ETHERTYPE_ASISTENTE,
    );
    match firma {
        Some(c) => eprintln!(
            "asistente-puente: firma habilitada, slot {} del AGORA_AUTH_RING; \
             audit a {}",
            c.slot, c.log_path,
        ),
        None => eprintln!(
            "asistente-puente: firma deshabilitada (sin --firma-clave) — las propuestas hash \
             rebotaran con Error"
        ),
    }
    eprintln!("asistente-puente: Ctrl-C para terminar");
    bucle(fd, llm, rt, ifindex, firma)
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
    firma: Option<&ConfigFirma>,
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
        let payload = &frame[TAM_CABECERA_CABLE..];
        let respuesta = match tipo {
            TipoCable::Consulta => atender_consulta(llm, rt, id, payload),
            TipoCable::RequestFirma => atender_request_firma(rt, id, payload, firma),
            // Otras variantes (Propuesta*, Error, Firma) son ecos de
            // respuestas previas en el broadcast; ignorar.
            _ => continue,
        };
        if let Err(e) = enviar_broadcast(&fd, ifindex, &respuesta) {
            eprintln!("asistente-puente: sendto fallo: {e}");
        } else {
            eprintln!(
                "asistente-puente: envie respuesta id={id} ({} B)",
                respuesta.len(),
            );
        }
    }
}

/// Maneja una `Consulta`: el LLM responde con un `MensajeAsistente`,
/// el puente lo empaqueta como un frame del cable.
fn atender_consulta(
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
    id: u64,
    payload: &[u8],
) -> Vec<u8> {
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
    construir_frame(id, &interp)
}

/// Fase 60 v4 :: maneja un `RequestFirma`. Si el puente no tiene
/// `--firma-clave`, responde con `Error("sin clave")`. Si el tipo de
/// objeto es desconocido, responde con `Error("tipo de objeto invalido")`.
/// En caso normal, prompt interactivo + firma Ed25519 + Firma sobre el
/// cable.
fn atender_request_firma(
    rt: &tokio::runtime::Runtime,
    id: u64,
    payload: &[u8],
    firma: Option<&ConfigFirma>,
) -> Vec<u8> {
    let Some(conf) = firma else {
        eprintln!("asistente-puente: RequestFirma id={id} pero el puente no tiene clave");
        return construir_frame_error(
            id,
            "PUENTE SIN CLAVE: levanta --firma-clave para autorizar firmas",
        );
    };
    let Some((tipo_obj, hash)) = leer_request_firma(payload) else {
        eprintln!(
            "asistente-puente: RequestFirma id={id} con payload invalido ({} B)",
            payload.len(),
        );
        return construir_frame_error(id, "REQUESTFIRMA INVALIDO");
    };
    let etiqueta = match tipo_obj {
        TIPO_OBJETO_CUADERNO => "CUADERNO/MANIFIESTO",
        TIPO_OBJETO_CONFIGURACION => "CONFIGURACION",
        _ => "DESCONOCIDO",
    };
    let hash_hex = hex_de_hash(&hash);
    eprintln!(
        "asistente-puente: RequestFirma id={id} tipo={etiqueta} hash={hash_hex}"
    );
    // Prompt + decision (bloqueante con timeout). Reusa el runtime
    // tokio para los timers; la lectura de stdin va en spawn_blocking.
    let autorizada = rt.block_on(confirmar_con_operador(etiqueta, &hash_hex));
    if !autorizada {
        escribir_auditoria(&conf.log_path, "FIRMA_RECHAZADA", etiqueta, conf.slot, &hash_hex);
        eprintln!("asistente-puente: firma rechazada o timeout — devuelvo Error");
        return construir_frame_error(id, "FIRMA RECHAZADA POR EL OPERADOR");
    }
    let sig = conf.clave.sign(hash, None);
    let mut firma_bytes = [0u8; 64];
    firma_bytes.copy_from_slice(sig.as_ref());
    escribir_auditoria(&conf.log_path, "FIRMA_EMITIDA", etiqueta, conf.slot, &hash_hex);
    eprintln!(
        "asistente-puente: FIRMA_EMITIDA tipo={etiqueta} slot={} (id={id})",
        conf.slot
    );
    construir_frame_firma(id, conf.slot, &firma_bytes)
}

/// Hex-encode de 32 bytes a 64 chars ASCII (para prompt y log).
fn hex_de_hash(hash: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in hash {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Prompt interactivo al operador. Igual UX que `wawactl daemon-firma`:
/// imprime el HASH + tipo, espera `y` / `N` por stdin con 30 s de timeout.
async fn confirmar_con_operador(etiqueta: &str, hash_hex: &str) -> bool {
    use std::io::Write;
    eprintln!();
    eprintln!("================================================================");
    eprintln!("  SOLICITUD DE FIRMA DE {etiqueta} (asistente.wasm)");
    eprintln!("  HASH: {hash_hex}");
    eprintln!("  Autorizar firma en el metal? [y/N]  (timeout 30 s)");
    eprintln!("================================================================");
    let _ = std::io::stderr().flush();

    let respuesta = tokio::task::spawn_blocking(|| {
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
        buf.trim().to_string()
    });
    match tokio::time::timeout(TIMEOUT_FIRMA, respuesta).await {
        Ok(Ok(s)) => s.eq_ignore_ascii_case("y"),
        Ok(Err(_)) => false,
        Err(_) => {
            eprintln!("asistente-puente: timeout — sin respuesta del operador");
            false
        }
    }
}

/// Append a `log_path` de una entrada de auditoria. Espejo del formato
/// de `wawactl daemon-firma` para que un grep cruzado tenga sentido.
/// Errores de I/O se imprimen a stderr pero NO interrumpen el lazo.
fn escribir_auditoria(log_path: &str, accion: &str, tipo: &str, slot: u8, hash_hex: &str) {
    use std::io::Write;
    let ts = chrono::Utc::now().to_rfc3339();
    let linea = format!(
        "[{ts}] | ORIGEN: asistente-puente | ACCION: {accion} | TIPO: {tipo} | SLOT: {slot} | HASH: {hash_hex}\n"
    );
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(linea.as_bytes()) {
                eprintln!("asistente-puente: no pude escribir audit log: {e}");
            }
        }
        Err(e) => eprintln!("asistente-puente: no pude abrir audit log {log_path}: {e}"),
    }
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
