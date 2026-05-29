//! ente-resolved-compat: shim de `org.freedesktop.resolve1`.
//!
//! ResolveHostname/ResolveAddress delegan en getaddrinfo/getnameinfo del
//! libc (vía tokio::net::lookup_host). ResolveRecord usa hickory-resolver
//! para emitir queries DNS de tipos arbitrarios (TXT, SRV, MX, etc.) cuyo
//! rdata empaquetamos en bytes wire-format — lo que pide la firma D-Bus
//! `a(iqqay)` del método.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use hickory_proto::serialize::binary::{BinEncodable, BinEncoder};
use hickory_resolver::{
    config::ResolverConfig,
    name_server::TokioConnectionProvider,
    proto::rr::RecordType,
    TokioResolver,
};
use std::net::IpAddr;
use std::sync::OnceLock;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface};

/// Una instancia compartida — el resolver es Send+Sync y cachea respuestas.
/// Lazy init para no fallar si /etc/resolv.conf todavía no existe al boot.
static RESOLVER: OnceLock<TokioResolver> = OnceLock::new();

fn resolver() -> &'static TokioResolver {
    RESOLVER.get_or_init(|| {
        // builder_tokio() lee /etc/resolv.conf. Si no existe (boot temprano),
        // construimos con ResolverConfig::default() (Google/Cloudflare) que
        // al menos permite responder antes de que la red local esté lista.
        match TokioResolver::builder_tokio() {
            Ok(b) => b.build(),
            Err(e) => {
                warn!(?e, "resolv.conf no disponible, usando default");
                TokioResolver::builder_with_config(
                    ResolverConfig::default(),
                    TokioConnectionProvider::default(),
                )
                .build()
            }
        }
    })
}

const BUS_NAME: &str = "org.freedesktop.resolve1";
const OBJ_PATH: &str = "/org/freedesktop/resolve1";

const AF_INET: i32 = 2;
const AF_INET6: i32 = 10;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-resolved-compat: arrancando");
    announce_to_fractal().await;

    let manager = ResolveManager;
    let conn_result = zbus::connection::Builder::system()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, manager));
    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name acquired, sirviendo");
                wait_for_term().await
            }
            Err(e) => { warn!(?e, "build conn falló — modo idle"); wait_for_term().await }
        },
        Err(e) => { warn!(?e, "builder D-Bus falló — modo idle"); wait_for_term().await }
    }
}

struct ResolveManager;

/// Tipo del wire format de `ResolveHostname`. Por entry: (ifindex, family,
/// address-as-bytes). systemd-resolved devuelve hasta 4 bytes para AF_INET
/// y 16 para AF_INET6.
type HostnameAddress = (i32, i32, Vec<u8>);

#[interface(name = "org.freedesktop.resolve1.Manager")]
impl ResolveManager {
    /// Wire signature: `ResolveHostname(in iiusst, out a(iiay)st)` — recibe
    /// (ifindex, name, family, flags), devuelve (addresses, canonical, flags).
    async fn resolve_hostname(
        &self,
        _ifindex: i32,
        name: String,
        family: i32,
        _flags: u64,
    ) -> fdo::Result<(Vec<HostnameAddress>, String, u64)> {
        // tokio::net::lookup_host requiere "host:port"; usamos puerto sentinel.
        let target = format!("{name}:0");
        let addrs = match tokio::net::lookup_host(&target).await {
            Ok(it) => it,
            Err(e) => return Err(fdo::Error::Failed(format!("lookup_host {name}: {e}"))),
        };
        let mut out = Vec::new();
        for sa in addrs {
            let ip = sa.ip();
            let (af, bytes) = match ip {
                IpAddr::V4(v4) => (AF_INET, v4.octets().to_vec()),
                IpAddr::V6(v6) => (AF_INET6, v6.octets().to_vec()),
            };
            // Filtrado por family si el llamador lo pidió específico.
            if family != 0 && family != af { continue; }
            out.push((0i32, af, bytes));
        }
        if out.is_empty() {
            return Err(fdo::Error::Failed(format!("sin resoluciones para {name} (family={family})")));
        }
        info!(%name, family, count = out.len(), "ResolveHostname");
        Ok((out, name, 0))
    }

    /// Wire signature: `ResolveAddress(in iiayt, out a(is)t)` — (ifindex,
    /// family, address, flags) → (names, flags).
    async fn resolve_address(
        &self,
        _ifindex: i32,
        family: i32,
        address: Vec<u8>,
        _flags: u64,
    ) -> fdo::Result<(Vec<(i32, String)>, u64)> {
        let ip = parse_address(family, &address)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("address malformado family={family} bytes={}", address.len())))?;
        // Reverse lookup vía getnameinfo. Usamos std::net::lookup_addr no existe,
        // así que invocamos via libc directamente.
        let name = reverse_lookup(ip)
            .ok_or_else(|| fdo::Error::Failed(format!("sin reverse para {ip}")))?;
        info!(%ip, %name, "ResolveAddress");
        Ok((vec![(0, name)], 0))
    }

    /// Wire signature: `ResolveRecord(in iiqqt, out a(iqqay)t)` — recibe
    /// (ifindex, name, class, type, flags), devuelve (records, flags).
    /// Cada record es (ifindex, class, type, rdata-wire-bytes).
    async fn resolve_record(
        &self,
        _ifindex: i32,
        name: String,
        class: u16,
        type_: u16,
        _flags: u64,
    ) -> fdo::Result<(Vec<(i32, u16, u16, Vec<u8>)>, u64)> {
        let rtype = RecordType::from(type_);
        let lookup = resolver()
            .lookup(name.clone(), rtype)
            .await
            .map_err(|e| fdo::Error::Failed(format!("DNS {name} type={type_}: {e}")))?;
        let mut out = Vec::new();
        for record in lookup.records() {
            let data = match record.data() {
                d => d,
            };
            let mut bytes = Vec::with_capacity(64);
            {
                let mut enc = BinEncoder::new(&mut bytes);
                if let Err(e) = data.emit(&mut enc) {
                    warn!(%name, ?e, "rdata emit falló, record descartado");
                    continue;
                }
            }
            out.push((0i32, class, type_, bytes));
        }
        if out.is_empty() {
            return Err(fdo::Error::Failed(format!(
                "sin registros para {name} type={type_}"
            )));
        }
        info!(%name, type_, count = out.len(), "ResolveRecord");
        Ok((out, 0))
    }
}

fn parse_address(family: i32, bytes: &[u8]) -> Option<IpAddr> {
    match family {
        AF_INET if bytes.len() == 4 => {
            let mut a = [0u8; 4];
            a.copy_from_slice(bytes);
            Some(IpAddr::V4(std::net::Ipv4Addr::from(a)))
        }
        AF_INET6 if bytes.len() == 16 => {
            let mut a = [0u8; 16];
            a.copy_from_slice(bytes);
            Some(IpAddr::V6(std::net::Ipv6Addr::from(a)))
        }
        _ => None,
    }
}

/// getnameinfo(3) wrapper. Devuelve None si no resuelve.
fn reverse_lookup(ip: IpAddr) -> Option<String> {
    use std::os::raw::c_char;
    let mut buf = [0i8; 256];
    let r = match ip {
        IpAddr::V4(v4) => unsafe {
            let octets = v4.octets();
            let mut sin = std::mem::zeroed::<libc::sockaddr_in>();
            sin.sin_family = libc::AF_INET as u16;
            sin.sin_addr = libc::in_addr {
                s_addr: u32::from_ne_bytes(octets),
            };
            libc::getnameinfo(
                &sin as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as u32,
                buf.as_mut_ptr() as *mut c_char, buf.len() as u32,
                std::ptr::null_mut(), 0,
                libc::NI_NAMEREQD,
            )
        },
        IpAddr::V6(v6) => unsafe {
            let octets = v6.octets();
            let mut sin6 = std::mem::zeroed::<libc::sockaddr_in6>();
            sin6.sin6_family = libc::AF_INET6 as u16;
            sin6.sin6_addr.s6_addr.copy_from_slice(&octets);
            libc::getnameinfo(
                &sin6 as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in6>() as u32,
                buf.as_mut_ptr() as *mut c_char, buf.len() as u32,
                std::ptr::null_mut(), 0,
                libc::NI_NAMEREQD,
            )
        },
    };
    if r != 0 { return None; }
    let cs = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    cs.to_str().ok().map(String::from)
}

extern crate libc;

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa3; 16]),
                version: 1,
            }],
        };
        match client.call(req).await {
            Ok(BusResponse::Ok) => info!("Announce → bus interno OK"),
            Ok(other) => warn!(?other, "Announce respuesta inesperada"),
            Err(e) => warn!(?e, "Announce falló"),
        }
    }
}

async fn wait_for_term() -> anyhow::Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => info!("SIGTERM"),
        _ = int_.recv() => info!("SIGINT"),
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_resolved_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
