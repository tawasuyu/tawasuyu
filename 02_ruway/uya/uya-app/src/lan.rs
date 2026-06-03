// =============================================================================
//  uya-app::lan — descubrimiento LAN por baliza UDP multicast (zero-config).
// -----------------------------------------------------------------------------
//  El mDNS de libp2p resultó poco fiable entre instancias, así que uya trae su
//  propio descubrimiento de sala en la LAN: cada nodo emite cada 2 s una baliza
//  multicast `uya1\t<sala>\t<puerto>\t<peerid>` y escucha las ajenas. Al recibir
//  una de MI sala, reconstruye la multiaddr dialable usando la **IP de ORIGEN
//  del datagrama** (así funciona entre máquinas aunque la dirección anunciada
//  del par sea loopback) y la disca.
//
//  Robusto a desktops con varias interfaces (wifi + eth + docker + VPN): se une
//  al grupo y emite por TODAS las interfaces IPv4, no sólo la default — que es
//  la causa típica de que el multicast "no se vea" en una máquina real.
//
//  Room-aware (filtra por nombre de sala), anda misma-máquina (`IP_MULTICAST_LOOP`
//  + `SO_REUSEPORT`) y entre máquinas, y no depende del DHT ni del mDNS. Convive
//  con `Enlace::unir_sala` (DHT, para WAN/bootstrap): ambos alimentan la malla.
// =============================================================================

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::Enlace;

/// Grupo multicast administrativamente acotado (239.0.0.0/8 = sólo LAN) y puerto
/// propios de uya.
const GRUPO: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
const PUERTO: u16 = 7799;
const MAGIA: &str = "uya1";

/// Arranca la baliza LAN para una sala: un hilo que anuncia (por todas las
/// interfaces) y otro que escucha y disca a los pares de la misma sala.
pub fn iniciar_baliza_lan(enlace: Arc<Enlace>, sala: String) {
    let depurar = std::env::var("UYA_DEBUG").is_ok();

    let Some((puerto, peerid)) = parsear_dialable(enlace.direccion_local()) else {
        eprintln!("uya: baliza LAN: no pude parsear mi dirección dialable");
        return;
    };

    let ifaces = interfaces_ipv4();
    if depurar {
        eprintln!("uya: baliza LAN: interfaces {ifaces:?}");
    }

    // Socket receptor: unido al grupo en todas las interfaces.
    let rx = match socket_receptor(&ifaces) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("uya: baliza LAN desactivada (rx: {e})");
            return;
        }
    };
    // Socket emisor: rota la interfaz de salida en cada ronda.
    let tx = match socket_emisor() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("uya: baliza LAN desactivada (tx: {e})");
            return;
        }
    };
    if depurar {
        eprintln!("uya: baliza LAN activa en {GRUPO}:{PUERTO} (sala '{sala}')");
    }

    // Emisor: anuncia cada 2 s por cada interfaz IPv4.
    {
        let mensaje = format!("{MAGIA}\t{sala}\t{puerto}\t{peerid}");
        let ifaces = ifaces.clone();
        std::thread::Builder::new()
            .name("uya-baliza-tx".into())
            .spawn(move || {
                let destino: SockAddr = SocketAddr::new(GRUPO.into(), PUERTO).into();
                loop {
                    for ip in &ifaces {
                        let _ = tx.set_multicast_if_v4(ip);
                        let _ = tx.send_to(mensaje.as_bytes(), &destino);
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("uya: spawn baliza tx");
    }

    // Receptor: escucha balizas y disca las de mi sala (no las mías).
    std::thread::Builder::new()
        .name("uya-baliza-rx".into())
        .spawn(move || {
            let mut buf = [0u8; 512];
            loop {
                let (n, src) = match rx.recv_from(&mut buf) {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let Ok(txt) = std::str::from_utf8(&buf[..n]) else {
                    continue;
                };
                let mut it = txt.split('\t');
                if it.next() != Some(MAGIA) {
                    continue;
                }
                let (Some(r), Some(pu), Some(pid)) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                if r != sala || pid == peerid {
                    continue; // otra sala o mi propia baliza
                }
                let IpAddr::V4(ip) = src.ip() else {
                    continue;
                };
                let dir = format!("/ip4/{ip}/tcp/{pu}/p2p/{pid}");
                if depurar {
                    eprintln!("uya: baliza LAN → {dir}");
                }
                enlace.conectar(&dir);
            }
        })
        .expect("uya: spawn baliza rx");
}

/// Direcciones IPv4 no-loopback de las interfaces locales (para unirse/emitir el
/// multicast por todas). Si no se puede enumerar, cae a `UNSPECIFIED` (default).
fn interfaces_ipv4() -> Vec<Ipv4Addr> {
    let mut v = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for i in ifaces {
            if i.is_loopback() {
                continue;
            }
            if let IpAddr::V4(ip) = i.ip() {
                if !v.contains(&ip) {
                    v.push(ip);
                }
            }
        }
    }
    if v.is_empty() {
        v.push(Ipv4Addr::UNSPECIFIED);
    }
    v
}

/// Extrae `(puerto_tcp, peerid)` de una multiaddr `…/tcp/<puerto>/p2p/<peerid>`.
fn parsear_dialable(addr: &str) -> Option<(String, String)> {
    let partes: Vec<&str> = addr.split('/').collect();
    let mut puerto = None;
    let mut peerid = None;
    for par in partes.windows(2) {
        match par[0] {
            "tcp" => puerto = Some(par[1].to_string()),
            "p2p" => peerid = Some(par[1].to_string()),
            _ => {}
        }
    }
    Some((puerto?, peerid?))
}

/// Socket receptor: bind a `*:PUERTO` con reuse, loopback, y unido al grupo en
/// la interfaz default + cada interfaz IPv4.
fn socket_receptor(ifaces: &[Ipv4Addr]) -> std::io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    sock.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), PUERTO).into())?;
    sock.set_multicast_loop_v4(true)?;
    // Default como respaldo + cada interfaz (errores por-interfaz no son fatales).
    let _ = sock.join_multicast_v4(&GRUPO, &Ipv4Addr::UNSPECIFIED);
    for ip in ifaces {
        let _ = sock.join_multicast_v4(&GRUPO, ip);
    }
    Ok(sock.into())
}

/// Socket emisor: efímero, con loopback, cuya interfaz de salida se rota antes
/// de cada `send_to` para cubrir todas las NIC.
fn socket_emisor() -> std::io::Result<Socket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    sock.set_multicast_loop_v4(true)?;
    sock.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0).into())?;
    Ok(sock)
}
