//! Descubrimiento de pares khipu en la LAN por baliza UDP.
//!
//! Un cuaderno que publica emite cada pocos segundos una [`Beacon`] —su
//! clave pública, el puerto TCP donde sirve y un nombre— por broadcast de
//! la LAN y por loopback. Quien quiere recibir escucha una ventana corta,
//! junta los pares vistos y le jala el cuaderno al primero (con
//! [`crate::net::fetch`]), sin tener que saber la IP de antemano.
//!
//! `std::net` puro, best-effort: redes que bloquean broadcast no
//! descubren nada, y ahí sigue valiendo apuntar a un par por dirección
//! explícita. El descubrimiento no afloja la seguridad: la baliza sólo
//! dice "dónde", el sobre que llega por TCP se verifica igual con
//! [`crate::open`].

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Puerto UDP estándar donde se emiten y escuchan las balizas.
pub const PUERTO_BALIZA: u16 = 7701;

/// Prefijo que marca un datagrama como baliza khipu — descarta tráfico
/// UDP ajeno antes de intentar parsearlo.
const MAGIA: [u8; 4] = *b"KHPU";

/// Lo que un cuaderno anuncia de sí mismo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beacon {
    /// Clave pública Ed25519 del cuaderno (identidad de quien publica).
    pub author: [u8; 32],
    /// Puerto TCP donde ese cuaderno sirve el sobre (para `fetch`).
    pub port: u16,
    /// Nombre legible para mostrar en una lista de pares.
    pub name: String,
}

impl Beacon {
    /// Serializa la baliza al cable: `MAGIA` seguido del postcard.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIA.to_vec();
        if let Ok(body) = postcard::to_allocvec(self) {
            out.extend_from_slice(&body);
        }
        out
    }

    /// Parsea un datagrama. `None` si no lleva la magia o no decodifica —
    /// así un paquete UDP cualquiera no nos hace ruido.
    pub fn decode(bytes: &[u8]) -> Option<Beacon> {
        let body = bytes.strip_prefix(&MAGIA)?;
        postcard::from_bytes(body).ok()
    }
}

/// Un par visto en la red: dónde jalarle el cuaderno y qué anunció.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerVisto {
    /// Dirección TCP de `fetch`: la IP de origen del datagrama + el
    /// `port` de la baliza. El que anuncia no necesita conocer su propia
    /// IP — la deduce quien recibe.
    pub fetch_addr: SocketAddr,
    pub beacon: Beacon,
}

/// Emite la baliza una vez: a broadcast de la LAN y a loopback (para que
/// dos cuadernos en la misma máquina se vean). Best-effort — los errores
/// de envío de cada destino se ignoran; sólo falla si no se pudo ni armar
/// el socket emisor.
pub fn anunciar(beacon: &Beacon) -> std::io::Result<()> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    let bytes = beacon.encode();
    let _ = sock.send_to(&bytes, (Ipv4Addr::BROADCAST, PUERTO_BALIZA));
    let _ = sock.send_to(&bytes, (Ipv4Addr::LOCALHOST, PUERTO_BALIZA));
    Ok(())
}

/// Escucha balizas en un socket ya bindeado durante `ventana`, devolviendo
/// los pares vistos (deduplicados por dirección de fetch). Bloqueante.
/// Separar el socket del bind permite testear sobre un puerto efímero.
pub fn escuchar_en(sock: &UdpSocket, ventana: Duration) -> Vec<PeerVisto> {
    let _ = sock.set_read_timeout(Some(Duration::from_millis(150)));
    let fin = Instant::now() + ventana;
    let mut vistos: Vec<PeerVisto> = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < fin {
        match sock.recv_from(&mut buf) {
            Ok((n, origen)) => {
                if let Some(beacon) = Beacon::decode(&buf[..n]) {
                    let fetch_addr = SocketAddr::new(origen.ip(), beacon.port);
                    if !vistos.iter().any(|p| p.fetch_addr == fetch_addr) {
                        vistos.push(PeerVisto { fetch_addr, beacon });
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
    }
    vistos
}

/// Bindea el socket de escucha en el puerto estándar de balizas.
pub fn bind_balizas() -> std::io::Result<UdpSocket> {
    UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PUERTO_BALIZA))
}

/// Conveniencia: bindea el puerto estándar y escucha `ventana`.
pub fn descubrir(ventana: Duration) -> std::io::Result<Vec<PeerVisto>> {
    Ok(escuchar_en(&bind_balizas()?, ventana))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beacon() -> Beacon {
        Beacon {
            author: [9u8; 32],
            port: 7700,
            name: "khipu de prueba".into(),
        }
    }

    #[test]
    fn encode_decode_roundtrips() {
        let b = beacon();
        assert_eq!(Beacon::decode(&b.encode()), Some(b));
    }

    #[test]
    fn decode_rejects_non_beacon_traffic() {
        assert_eq!(Beacon::decode(b"hola mundo"), None);
        assert_eq!(Beacon::decode(b"KHP"), None); // magia truncada
        assert_eq!(Beacon::decode(&[]), None);
    }

    #[test]
    fn escuchar_recibe_una_baliza_por_loopback() {
        // Escucha en un puerto efímero (no el estándar, para no chocar con
        // otros tests ni con un khipu corriendo).
        let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let destino = listener.local_addr().unwrap();

        let b = beacon();
        let emisor = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        emisor.send_to(&b.encode(), destino).unwrap();

        let vistos = escuchar_en(&listener, Duration::from_millis(500));
        assert_eq!(vistos.len(), 1);
        // La dirección de fetch combina la IP de origen con el port anunciado.
        assert_eq!(vistos[0].fetch_addr.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(vistos[0].fetch_addr.port(), b.port);
        assert_eq!(vistos[0].beacon, b);
    }

    #[test]
    fn escuchar_deduplica_balizas_repetidas() {
        let listener = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let destino = listener.local_addr().unwrap();

        let b = beacon();
        let emisor = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        // Tres balizas idénticas: un solo par visto.
        for _ in 0..3 {
            emisor.send_to(&b.encode(), destino).unwrap();
        }

        let vistos = escuchar_en(&listener, Duration::from_millis(400));
        assert_eq!(vistos.len(), 1);
    }
}
