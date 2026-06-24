//! El enlace al **rail soberano** P2P del binario `paloma` (Eje 3.B).
//!
//! Implementa el trait `RailLink` de `paloma-llimphi` sobre `paloma-rail`:
//! sella el `Message` con la identidad Ed25519 del usuario y lo entrega por un
//! transporte P2P. La identidad es la misma seed que firma el correo SMTP (ver
//! `identity`) — un solo "yo" para toda la soberanía.
//!
//! ## Transporte (por env, de más a menos capaz)
//!
//! - `PALOMA_RAIL_P2P=1` → **libp2p** (`Libp2pRail`): discovery por DHT +
//!   NAT traversal (relay/dcutr) de `card-net`. `PALOMA_RAIL_BIND` = multiaddr
//!   (default `/ip4/0.0.0.0/tcp/0`), `PALOMA_RAIL_PEERS` = multiaddrs a dialear
//!   (rendezvous/relay/contactos). Se anuncia bajo su identidad.
//! - `PALOMA_RAIL_BIND=host:port` (sin P2P) → **TCP directo** (`TcpRail`),
//!   ruteado por identidad; `PALOMA_RAIL_PEERS` = `host:port,…`.
//! - sin nada → **loopback** (enviarte a vos mismo entrega a "Suyu").
//!
//! En todos los casos, enviarte a tu propia dirección hace loopback en proceso,
//! y un hilo abre/verifica cada sobre entrante y despacha `Msg::RailReceived`.

use std::sync::mpsc::Receiver;

use agora_core::Keypair;
use paloma_core::{Address, Message};
use paloma_llimphi::{Handle, Msg, RailLink};
use paloma_rail::{MockTransport, RailEnvelope, RailId, RailTransport};
use paloma_rail_net::{Libp2pRail, TcpRail};

pub struct RailHost {
    keypair: Keypair,
    me: RailId,
    transport: Box<dyn RailTransport>,
    handle: Handle<Msg>,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|s| !s.trim().is_empty())
}

impl RailHost {
    /// Crea el enlace desde la seed de identidad y el `Handle` de la app, eligiendo
    /// el transporte por entorno (ver doc del módulo) y arrancando la recepción.
    pub fn new(seed: [u8; 32], handle: Handle<Msg>) -> Self {
        let keypair = Keypair::from_seed(seed);
        let me = keypair.public_key();

        let (transport, rx): (Box<dyn RailTransport>, Option<Receiver<RailEnvelope>>) =
            if env("PALOMA_RAIL_P2P").is_some() {
                Self::build_libp2p(seed, me)
            } else if let Some(bind) = env("PALOMA_RAIL_BIND") {
                Self::build_tcp(&bind, me)
            } else {
                (Box::new(MockTransport::new()), None)
            };

        // Recepción unificada: abre/verifica cada sobre y lo despacha. Estampa la
        // dirección del remitente = su identidad (para responder/guardar por rail).
        if let Some(rx) = rx {
            let h = handle.clone();
            std::thread::spawn(move || {
                for envelope in rx {
                    match paloma_rail::open(&envelope, me) {
                        Ok(mut msg) => {
                            let name = msg.from.name.clone();
                            msg.from = Address { name, email: paloma_rail::rail_address(&envelope.from) };
                            h.dispatch(Msg::RailReceived { msg, avales: envelope.avales.clone() });
                        }
                        Err(e) => eprintln!("paloma · sobre del rail rechazado: {e}"),
                    }
                }
            });
        }

        Self { keypair, me, transport, handle }
    }

    fn build_libp2p(seed: [u8; 32], me: RailId) -> (Box<dyn RailTransport>, Option<Receiver<RailEnvelope>>) {
        match Libp2pRail::new(seed, me) {
            Ok((p2p, rx)) => {
                let bind = env("PALOMA_RAIL_BIND").unwrap_or_else(|| "/ip4/0.0.0.0/tcp/0".to_string());
                match p2p.listen(&bind) {
                    Ok(addr) => eprintln!("paloma · rail libp2p escuchando: {addr}"),
                    Err(e) => eprintln!("paloma · rail libp2p listen falló: {e}"),
                }
                if let Some(peers) = env("PALOMA_RAIL_PEERS") {
                    for addr in peers.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        match p2p.dial(addr) {
                            Ok(()) => eprintln!("  → dial {addr}"),
                            Err(e) => eprintln!("  ✗ {addr}: {e}"),
                        }
                    }
                }
                p2p.announce();
                (Box::new(p2p), Some(rx))
            }
            Err(e) => {
                eprintln!("paloma · rail libp2p falló ({e}); usando loopback");
                (Box::new(MockTransport::new()), None)
            }
        }
    }

    fn build_tcp(bind: &str, me: RailId) -> (Box<dyn RailTransport>, Option<Receiver<RailEnvelope>>) {
        match TcpRail::escuchar(bind, me) {
            Ok((tcp, rx)) => {
                eprintln!("paloma · rail TCP escuchando en {}", tcp.direccion_local());
                if let Some(peers) = env("PALOMA_RAIL_PEERS") {
                    for addr in peers.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        match tcp.conectar(addr) {
                            Ok(()) => eprintln!("  → conectado a {addr}"),
                            Err(e) => eprintln!("  ✗ {addr}: {e}"),
                        }
                    }
                }
                (Box::new(tcp), Some(rx))
            }
            Err(e) => {
                eprintln!("paloma · rail TCP falló ({e}); usando loopback");
                (Box::new(MockTransport::new()), None)
            }
        }
    }

    /// La dirección del rail de este usuario (`<hex>@rail.suyu`), para logging.
    pub fn address(&self) -> String {
        paloma_rail::rail_address(&self.me)
    }
}

impl RailLink for RailHost {
    fn send(&self, to: RailId, msg: &Message, avales: &[Vec<u8>]) -> Result<(), String> {
        let env = paloma_rail::seal(&self.keypair, to, msg, avales.to_vec()).map_err(|e| e.to_string())?;
        if to == self.me {
            // Loopback: entrega local inmediata (te escribís a vos mismo).
            let recibido = paloma_rail::open(&env, self.me).map_err(|e| e.to_string())?;
            self.handle.dispatch(Msg::RailReceived { msg: recibido, avales: env.avales });
            Ok(())
        } else {
            self.transport.send(to, &env).map_err(|e| e.to_string())
        }
    }

    fn my_address(&self) -> String {
        paloma_rail::rail_address(&self.me)
    }
}
