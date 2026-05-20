//! `brahman-dht` — capa de discovery typed sobre el Kademlia compartido.
//!
//! `brahman-net` corre un único Kademlia para todo el ecosistema. Este
//! crate le pone arriba un esquema de claves namespaced ([`DhtKey`]):
//! `minga` publica bloques de código, `brahman-card-discovery` publica
//! Cards, `agorapura` publica Personas — todo sobre la misma malla sin
//! colisión, porque cada clave lleva un byte de `kind`.
//!
//! El modelo es de **provider records**: un nodo `announce`-a que provee
//! una clave; otros `find`-an quién la provee y abren un stream directo.

#![forbid(unsafe_code)]

pub mod key;

pub use key::{DhtKey, RecordKind, DHT_KEY_LEN};

use brahman_net::BrahmanNet;
use libp2p::PeerId;
use std::sync::Arc;

/// Discovery typed sobre `brahman-net`.
#[derive(Clone)]
pub struct Dht {
    net: Arc<BrahmanNet>,
}

impl Dht {
    /// Crea la capa DHT sobre un nodo `brahman-net` ya inicializado.
    pub fn new(net: Arc<BrahmanNet>) -> Self {
        Self { net }
    }

    /// Anuncia que este nodo provee `key`. El registro de provider se
    /// renueva solo mientras el nodo siga vivo en la malla.
    pub fn announce(&self, key: &DhtKey) {
        self.net.start_providing(&key.to_bytes());
    }

    /// Retira el anuncio de `key`.
    pub fn withdraw(&self, key: &DhtKey) {
        self.net.stop_providing(&key.to_bytes());
    }

    /// Busca los peers que proveen `key`.
    pub async fn find(&self, key: &DhtKey) -> Vec<PeerId> {
        self.net.find_providers(&key.to_bytes()).await
    }

    /// El nodo `brahman-net` subyacente (para abrir streams a un provider).
    pub fn net(&self) -> &Arc<BrahmanNet> {
        &self.net
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn announce_find_withdraw_on_a_live_node() {
        // Smoke: un nodo solo. `find` de una clave que nadie provee
        // devuelve vacío; announce/withdraw no panickean.
        let net = Arc::new(BrahmanNet::new().expect("nodo libp2p"));
        let dht = Dht::new(net);
        let key = DhtKey::card("modulo-inexistente");
        dht.announce(&key);
        dht.withdraw(&key);
        let found = dht.find(&DhtKey::card("nadie-lo-provee")).await;
        assert!(found.is_empty());
    }
}
