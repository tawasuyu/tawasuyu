//! El enlace al **rail soberano** P2P del binario `paloma` (Eje 3.B).
//!
//! Implementa el trait `RailLink` de `paloma-llimphi` sobre `paloma-rail`:
//! sella el `Message` con la identidad Ed25519 del usuario y lo entrega por un
//! transporte P2P. La identidad es la misma seed que firma el correo SMTP (ver
//! `identity`) — un solo "yo" para toda la soberanía.
//!
//! ## Estado del transporte
//!
//! Hoy usa [`MockTransport`] con **loopback**: enviarte a vos mismo
//! (`<tu-hex>@rail.suyu`) entrega el sobre en el acto a tu buzón "Suyu" —
//! ejercita el rail completo (sellar → abrir → verificar → recibir) dentro de la
//! app, sin red. Los envíos a otra identidad se encolan: la entrega remota llega
//! cuando se enchufe el transporte real (chasqui / canal akasha), que implementa
//! el mismo trait y dispara `Msg::RailReceived` desde su loop de recepción.

use agora_core::Keypair;
use paloma_core::Message;
use paloma_llimphi::{Handle, Msg, RailLink};
use paloma_rail::{MockTransport, RailId, RailTransport};

pub struct RailHost {
    keypair: Keypair,
    me: RailId,
    transport: MockTransport,
    handle: Handle<Msg>,
}

impl RailHost {
    /// Crea el enlace desde la seed de identidad y el `Handle` de la app (para
    /// despachar los mensajes recibidos al bucle de UI).
    pub fn new(seed: [u8; 32], handle: Handle<Msg>) -> Self {
        let keypair = Keypair::from_seed(seed);
        let me = keypair.public_key();
        Self { keypair, me, transport: MockTransport::new(), handle }
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
            // Loopback: entrega local inmediata (demo del rail en proceso).
            let recibido = paloma_rail::open(&env, self.me).map_err(|e| e.to_string())?;
            self.handle.dispatch(Msg::RailReceived(recibido));
            Ok(())
        } else {
            // Sin transporte de red, el sobre queda encolado; un peer remoto lo
            // recibirá cuando se enchufe el transporte chasqui.
            self.transport.send(to, &env).map_err(|e| e.to_string())
        }
    }

    fn my_address(&self) -> String {
        paloma_rail::rail_address(&self.me)
    }
}
