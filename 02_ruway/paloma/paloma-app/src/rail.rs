//! El enlace al **rail soberano** P2P del binario `paloma` (Eje 3.B).
//!
//! Implementa el trait `RailLink` de `paloma-llimphi` sobre `paloma-rail`:
//! sella el `Message` con la identidad Ed25519 del usuario y lo entrega por un
//! transporte P2P. La identidad es la misma seed que firma el correo SMTP (ver
//! `identity`) — un solo "yo" para toda la soberanía.
//!
//! ## Transporte
//!
//! Con `PALOMA_RAIL_BIND` (p. ej. `0.0.0.0:7710`) usa el **transporte TCP real**
//! (`paloma-rail-net::TcpRail`): escucha, se conecta a los peers de
//! `PALOMA_RAIL_PEERS` (lista `host:port` separada por comas), y un hilo drena
//! los sobres entrantes → `open`/verifica → `Msg::RailReceived`. Sin esa env,
//! cae a `MockTransport` (sólo loopback: enviarte a vos mismo entrega a "Suyu").
//! En ambos casos, enviarte a tu propia dirección hace loopback en proceso.

use agora_core::Keypair;
use paloma_core::Message;
use paloma_llimphi::{Handle, Msg, RailLink};
use paloma_rail::{MockTransport, RailId, RailTransport};
use paloma_rail_net::TcpRail;

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
    /// Crea el enlace desde la seed de identidad y el `Handle` de la app. Si hay
    /// `PALOMA_RAIL_BIND`, levanta el transporte TCP real y el hilo de recepción.
    pub fn new(seed: [u8; 32], handle: Handle<Msg>) -> Self {
        let keypair = Keypair::from_seed(seed);
        let me = keypair.public_key();

        let transport: Box<dyn RailTransport> = match env("PALOMA_RAIL_BIND") {
            Some(bind) => match TcpRail::escuchar(&bind, me) {
                Ok((tcp, rx)) => {
                    eprintln!("paloma · rail TCP escuchando en {}", tcp.direccion_local());
                    // Conectar a los peers conocidos.
                    if let Some(peers) = env("PALOMA_RAIL_PEERS") {
                        for addr in peers.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                            match tcp.conectar(addr) {
                                Ok(()) => eprintln!("  → conectado a {addr}"),
                                Err(e) => eprintln!("  ✗ {addr}: {e}"),
                            }
                        }
                    }
                    // Hilo de recepción: abre/verifica cada sobre y lo despacha.
                    let h = handle.clone();
                    std::thread::spawn(move || {
                        for envelope in rx {
                            match paloma_rail::open(&envelope, me) {
                                Ok(msg) => h.dispatch(Msg::RailReceived(msg)),
                                Err(e) => eprintln!("paloma · sobre del rail rechazado: {e}"),
                            }
                        }
                    });
                    Box::new(tcp)
                }
                Err(e) => {
                    eprintln!("paloma · rail TCP falló ({e}); usando loopback");
                    Box::new(MockTransport::new())
                }
            },
            None => Box::new(MockTransport::new()),
        };

        Self { keypair, me, transport, handle }
    }

    /// La dirección del rail de este usuario (`<hex>@rail.suyu`), para logging.
    pub fn address(&self) -> String {
        paloma_rail::rail_address(&self.me)
    }
}

impl RailLink for RailHost {
    fn send(&self, to: RailId, msg: &Message) -> Result<(), String> {
        let env = paloma_rail::seal(&self.keypair, to, msg).map_err(|e| e.to_string())?;
        if to == self.me {
            // Loopback: entrega local inmediata (te escribís a vos mismo).
            let recibido = paloma_rail::open(&env, self.me).map_err(|e| e.to_string())?;
            self.handle.dispatch(Msg::RailReceived(recibido));
            Ok(())
        } else {
            self.transport.send(to, &env).map_err(|e| e.to_string())
        }
    }

    fn my_address(&self) -> String {
        paloma_rail::rail_address(&self.me)
    }
}
