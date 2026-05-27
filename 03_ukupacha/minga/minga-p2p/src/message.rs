//! Mensajes del protocolo de sincronización (versión recursiva sobre
//! la estructura del MST).
//!
//! El protocolo es simétrico — ambos peers ejecutan el mismo rol y
//! emiten los mismos mensajes — y consta de seis tipos:
//!
//! 1. `Hello { root_subtree_hash }` anuncia el hash Merkle del MST raíz
//!    del emisor. Si ambos hashes coinciden, los dos repos son idénticos
//!    y la sincronización termina sin un solo byte adicional.
//!
//! 2. `ProbeReq { subtree_hash }` solicita la **estructura** (level +
//!    keys + child_hashes) de un subárbol previamente anunciado por el
//!    otro peer. Es lo que permite descender el árbol del peer paso a
//!    paso, podando ramas idénticas por igualdad de hash.
//!
//! 3. `ProbeRes { subtree_hash, probe }` responde con el `NodeProbe`,
//!    o `None` si el subárbol era el vacío. Cada subárbol que el peer
//!    no reconoce dispara un `ProbeReq` recursivo; cuando el peer ya
//!    tiene un subárbol con el mismo hash, la rama se poda.
//!
//! 4. `Fetch { hash }` y `Deliver { hash, stored }` mueven los nodos
//!    propiamente dichos. El receptor del `Deliver` **verifica
//!    criptográficamente** que `hash_stored(stored) == hash` antes de
//!    insertar — un peer malicioso no puede colar un `StoredNode`
//!    distinto bajo un hash anunciado.
//!
//! 5. `Done` cierra el lado del emisor: ya recibió el `Hello` del otro,
//!    no tiene probes ni fetches pendientes. Cuando ambos `Done`s han
//!    cruzado, la sesión termina con ambos repos convergentes.

use minga_core::{Attestation, ContentHash, Did, NodeProbe, Retraction, Signature, StoredNode};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Message {
    /// Reto de session-handshake: 32 bytes aleatorios. Cada peer envía
    /// uno al inicio. El otro lado lo incrustará en el payload del
    /// `Hello` que firme con su llave privada — así un `Hello`
    /// capturado en una sesión no puede replayearse en otra (que
    /// tendrá un nonce distinto).
    Challenge {
        nonce: [u8; 32],
    },

    /// Saludo autenticado anti-replay: el emisor presenta su DID, el
    /// hash del subárbol raíz de su MST, y una firma sobre el payload
    /// `(peer_did || root_subtree_hash || nonce_recibido_del_peer)`.
    /// El receptor reconstruye el payload con su PROPIO nonce (el que
    /// envió en su Challenge) y verifica con la llave pública del
    /// peer. Sin Challenge previo no hay Hello válido posible.
    Hello {
        peer_did: Did,
        root_subtree_hash: ContentHash,
        signature: Signature,
    },
    ProbeReq {
        subtree_hash: ContentHash,
    },
    ProbeRes {
        subtree_hash: ContentHash,
        probe: Option<NodeProbe>,
    },
    Fetch {
        hash: ContentHash,
    },
    Deliver {
        hash: ContentHash,
        stored: StoredNode,
    },
    /// Empuje de atestaciones: el emisor entrega al peer las pruebas
    /// criptográficas de autoría que conoce. Cada `Attestation` es
    /// auto-verificable (firma + autor + contenido), así que el
    /// receptor puede validar y mezclar sin confiar en la palabra del
    /// remitente. Se envían tras el `Hello` autenticado para que el
    /// peer verifique la identidad del remitente antes de procesarlas.
    AttestPush {
        attestations: Vec<Attestation>,
    },
    /// Empuje de retracciones: contraparte negativa de `AttestPush`.
    /// Cada `Retraction` es auto-verificable (firma sobre
    /// `RETRACTION_DOMAIN ++ content_hash`), así que el receptor las
    /// valida igual que las atestaciones — sin necesidad de confiar
    /// en el remitente más allá de su firma.
    RetractPush {
        retractions: Vec<Retraction>,
    },
    Done,
}

impl Message {
    /// Codifica el mensaje a bytes vía postcard. Diseñado para
    /// transferir sobre cualquier transporte que mueva `Vec<u8>`.
    /// Postcard es compacto, sin overhead de schema runtime.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("postcard encoding cannot fail for our types")
    }

    /// Decodifica bytes a un `Message`. `Err` si los bytes son
    /// malformados o no representan un `Message` válido.
    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}
