//! El transporte, unificado — TCP directo o minga (libp2p P2P), tras una sola
//! fachada. Ambos implementan el trait [`Transporte`] de `ayni-sync` y entregan
//! el MISMO `Receiver<EventoRed>`; este `enum` sólo elige cuál al arrancar y
//! reconcilia las pocas operaciones que no están en el trait (`conectar`,
//! `num_peers`, `direccion_local`) bajo una firma común, para que la app no
//! sepa —ni le importe— sobre qué cable viaja.

use std::sync::mpsc::Receiver;

use ayni_minga::EnlaceMinga;
use ayni_sync::{EnlaceTcp, ErrorSync, EventoRed, PeerId, Sobre, Transporte};

/// Qué transporte usar. La elige el operador (env/arg); cambiarla NO toca la
/// lógica de la app — es el sentido del trait `Transporte`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    /// TCP LAN directo (`EnlaceTcp`). Bind tipo `127.0.0.1:7700`.
    Tcp,
    /// P2P sobre libp2p (`EnlaceMinga`). Bind tipo `/ip4/127.0.0.1/tcp/0`.
    Minga,
}

impl Tipo {
    /// Interpreta un nombre (`"tcp"`/`"minga"`); por defecto, TCP.
    pub fn desde_nombre(s: &str) -> Tipo {
        match s.trim().to_lowercase().as_str() {
            "minga" | "libp2p" | "p2p" => Tipo::Minga,
            _ => Tipo::Tcp,
        }
    }

    /// Un bind por defecto razonable para este transporte.
    pub fn bind_por_defecto(self) -> &'static str {
        match self {
            Tipo::Tcp => "127.0.0.1:7700",
            Tipo::Minga => "/ip4/127.0.0.1/tcp/0",
        }
    }
}

/// El enlace abierto — uno de los dos transportes concretos.
pub enum Enlace {
    Tcp(EnlaceTcp),
    Minga(EnlaceMinga),
}

impl Enlace {
    /// Abre el transporte pedido en `bind`, devolviendo el enlace (lado de
    /// envío) y el `Receiver<EventoRed>` (lado de recepción) — la misma forma
    /// para ambos.
    pub fn abrir(tipo: Tipo, bind: &str) -> Result<(Enlace, Receiver<EventoRed>), String> {
        match tipo {
            Tipo::Tcp => EnlaceTcp::escuchar(bind)
                .map(|(e, rx)| (Enlace::Tcp(e), rx))
                .map_err(|e| e.to_string()),
            Tipo::Minga => EnlaceMinga::escuchar(bind)
                .map(|(e, rx)| (Enlace::Minga(e), rx))
                .map_err(|e| e.to_string()),
        }
    }

    /// Conéctate a un peer. El formato de `addr` depende del transporte
    /// (`ip:puerto` para TCP, multiaddr para minga).
    pub fn conectar(&self, addr: &str) -> Result<(), String> {
        match self {
            Enlace::Tcp(e) => e.conectar(addr).map_err(|e| e.to_string()),
            Enlace::Minga(e) => e.conectar(addr).map_err(|e| e.to_string()),
        }
    }

    /// La dirección local en la que se escucha — para mostrarla y compartirla.
    pub fn direccion_local(&self) -> String {
        match self {
            Enlace::Tcp(e) => e.direccion_local().to_string(),
            Enlace::Minga(e) => e.direccion_local().to_string(),
        }
    }

    /// Cuántos peers conectados (TCP lo sabe con exactitud; minga aún no expone
    /// el conteo, así que devuelve 0 — la presencia real la dan los `EventoRed`).
    pub fn num_peers(&self) -> usize {
        match self {
            Enlace::Tcp(e) => e.num_peers(),
            Enlace::Minga(_) => 0,
        }
    }

    /// Nombre del transporte, para la UI.
    pub fn etiqueta(&self) -> &'static str {
        match self {
            Enlace::Tcp(_) => "TCP",
            Enlace::Minga(_) => "minga·p2p",
        }
    }
}

// La fachada implementa el propio trait `Transporte`: difundir/enviar despachan
// al concreto. Así `Nucleo` opera sobre `Enlace` exactamente como sobre
// cualquier `Transporte`.
impl Transporte for Enlace {
    fn difundir(&self, sobre: &Sobre) -> Result<(), ErrorSync> {
        match self {
            Enlace::Tcp(e) => e.difundir(sobre),
            Enlace::Minga(e) => e.difundir(sobre),
        }
    }

    fn enviar(&self, peer: &PeerId, sobre: &Sobre) -> Result<(), ErrorSync> {
        match self {
            Enlace::Tcp(e) => e.enviar(peer, sobre),
            Enlace::Minga(e) => e.enviar(peer, sobre),
        }
    }
}
