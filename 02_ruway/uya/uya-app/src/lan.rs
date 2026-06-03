// =============================================================================
//  uya-app::lan — descubrimiento LAN por baliza UDP multicast (zero-config).
// -----------------------------------------------------------------------------
//  El mDNS de libp2p resultó poco fiable entre instancias (sobre todo en la
//  misma máquina), así que uya trae su propio descubrimiento de sala en la LAN:
//  cada nodo emite cada 2 s una baliza multicast `uya1\t<sala>\t<puerto>\t<peerid>`
//  y escucha las ajenas. Al recibir una de MI sala, reconstruye la multiaddr
//  dialable usando la **IP de ORIGEN del datagrama** (así funciona entre
//  máquinas aunque la dirección anunciada del par sea loopback) y la disca.
//
//  Es room-aware (filtra por nombre de sala), anda en la misma máquina (gracias
//  a `IP_MULTICAST_LOOP` + `SO_REUSEPORT`) y entre máquinas, y no depende del
//  DHT ni del mDNS. Convive con `Enlace::unir_sala` (DHT, para WAN/bootstrap):
//  ambos alimentan la misma malla, deduplicada.
// =============================================================================

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use crate::Enlace;

/// Grupo multicast administrativamente acotado (239.0.0.0/8 = sólo LAN) y puerto
/// propios de uya.
const GRUPO: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 99);
const PUERTO: u16 = 7799;
const MAGIA: &str = "uya1";

/// Arranca la baliza LAN para una sala: un hilo que anuncia y otro que escucha y
/// disca a los pares de la misma sala que aparezcan.
pub fn iniciar_baliza_lan(enlace: Arc<Enlace>, sala: String) {
    let depurar = std::env::var("UYA_DEBUG").is_ok();

    // Mi puerto TCP y PeerId salen de mi propia dirección dialable.
    let Some((puerto, peerid)) = parsear_dialable(enlace.direccion_local()) else {
        eprintln!("uya: baliza LAN: no pude parsear mi dirección dialable");
        return;
    };

    let sock = match socket_multicast() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("uya: baliza LAN desactivada ({e})");
            return;
        }
    };
    if depurar {
        eprintln!("uya: baliza LAN activa en {GRUPO}:{PUERTO} (sala '{sala}')");
    }

    // Emisor: anuncia cada 2 s.
    {
        let sock = match sock.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("uya: baliza LAN: no pude clonar el socket ({e})");
                return;
            }
        };
        let mensaje = format!("{MAGIA}\t{sala}\t{puerto}\t{peerid}");
        std::thread::Builder::new()
            .name("uya-baliza-tx".into())
            .spawn(move || {
                let destino = SocketAddr::new(GRUPO.into(), PUERTO);
                loop {
                    let _ = sock.send_to(mensaje.as_bytes(), destino);
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
                let (n, src) = match sock.recv_from(&mut buf) {
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
                // Otra sala, o mi propia baliza (loopback): ignorar.
                if r != sala || pid == peerid {
                    continue;
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

/// Socket UDP unido al grupo multicast, con reuse (para varias instancias en la
/// misma máquina) y loopback (para que se vean entre sí en un solo host).
fn socket_multicast() -> std::io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    sock.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), PUERTO).into())?;
    sock.set_multicast_loop_v4(true)?;
    sock.join_multicast_v4(&GRUPO, &Ipv4Addr::UNSPECIFIED)?;
    Ok(sock.into())
}
