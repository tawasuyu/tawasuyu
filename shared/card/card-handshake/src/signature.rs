//! Firma y verificación del payload del `Hello` para trust remoto.
//!
//! Usa la identidad Ed25519 de libp2p (la misma keypair que el peer
//! presenta al swarm vía Noise). Esto ancla la identidad criptográfica
//! del Ente a la identidad de transporte: si Noise autenticó al
//! `peer_id` X, sólo X puede firmar Cards válidas para esa conexión.
//!
//! ## Payload firmado
//!
//! Bytes postcard de la tupla `(WireCard, Option<WitInterface>)`. Se
//! eligió postcard porque ya es el wire format del resto del protocolo:
//! mismo determinismo, sin convertir a otro formato sólo para firmar.
//!
//! Cualquier campo que entre al payload firmado en el futuro debe
//! añadirse al final de la tupla (postcard es position-dependent), o
//! bumpearse el [`SIGNATURE_VERSION`] para distinguir esquemas.

use brahman_card::{WireCard, WitInterface};
use brahman_net::{Keypair, PeerId, PublicKey};

use crate::messages::HelloSignature;

/// Versión del esquema de payload firmado. Si cambia el shape de
/// `(WireCard, Option<WitInterface>)` o cómo se serializa, bump este
/// número y el verificador rechaza firmas antiguas.
pub const SIGNATURE_VERSION: u8 = 1;

/// Errores de verificación de firma.
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("public_key inválida (libp2p decode protobuf): {0}")]
    DecodeKey(String),
    #[error("encode del payload falló: {0}")]
    EncodePayload(String),
    #[error("firma rechazada: bytes inválidos para la public_key")]
    Invalid,
    #[error("peer_id de la firma ({signer}) no coincide con el peer libp2p autenticado ({expected})")]
    PeerMismatch { signer: PeerId, expected: PeerId },
    #[error("firma del Hello faltante (requerida para conexión remota libp2p)")]
    Missing,
    #[error("firma del Hello inesperada en path local sin trust remoto")]
    Unexpected,
}

/// Construye los bytes canónicos a firmar/verificar para un Hello.
/// Postcard determinístico de `(version, WireCard, Option<WitInterface>)`.
fn payload_bytes(card: &WireCard, wit: &Option<WitInterface>) -> Result<Vec<u8>, SignatureError> {
    let tup = (SIGNATURE_VERSION, card, wit);
    postcard::to_allocvec(&tup).map_err(|e| SignatureError::EncodePayload(e.to_string()))
}

/// Firma `(card, wit)` con la `keypair`. La public key derivada de
/// `keypair` debe coincidir con la identidad libp2p del peer cuando
/// el verificador la chequee.
pub fn sign_hello(
    keypair: &Keypair,
    card: &WireCard,
    wit: &Option<WitInterface>,
) -> Result<HelloSignature, SignatureError> {
    let bytes = payload_bytes(card, wit)?;
    let signature_bytes = keypair
        .sign(&bytes)
        .map_err(|e| SignatureError::EncodePayload(e.to_string()))?;
    Ok(HelloSignature {
        public_key: keypair.public().encode_protobuf(),
        signature: signature_bytes,
    })
}

/// Verifica que `sig` es una firma válida sobre `(card, wit)` y que
/// la public key declarada coincide con `expected_peer` (la identidad
/// libp2p autenticada por Noise).
///
/// Devuelve `Ok(())` si todo cuadra; si no, el error concreto.
pub fn verify_hello(
    sig: &HelloSignature,
    card: &WireCard,
    wit: &Option<WitInterface>,
    expected_peer: PeerId,
) -> Result<(), SignatureError> {
    let public_key = PublicKey::try_decode_protobuf(&sig.public_key)
        .map_err(|e| SignatureError::DecodeKey(e.to_string()))?;
    let signer_peer = public_key.to_peer_id();
    if signer_peer != expected_peer {
        return Err(SignatureError::PeerMismatch {
            signer: signer_peer,
            expected: expected_peer,
        });
    }
    let bytes = payload_bytes(card, wit)?;
    if !public_key.verify(&bytes, &sig.signature) {
        return Err(SignatureError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use brahman_card::Card;

    fn sample_card() -> WireCard {
        Card::new("test.signed").into()
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let kp = Keypair::generate_ed25519();
        let peer = kp.public().to_peer_id();
        let card = sample_card();
        let wit = None;
        let sig = sign_hello(&kp, &card, &wit).unwrap();
        verify_hello(&sig, &card, &wit, peer).expect("firma propia debe verificar");
    }

    #[test]
    fn verify_rejects_wrong_peer() {
        let kp = Keypair::generate_ed25519();
        let other = Keypair::generate_ed25519().public().to_peer_id();
        let card = sample_card();
        let wit = None;
        let sig = sign_hello(&kp, &card, &wit).unwrap();
        let err = verify_hello(&sig, &card, &wit, other).unwrap_err();
        assert!(matches!(err, SignatureError::PeerMismatch { .. }), "got {err:?}");
    }

    #[test]
    fn verify_rejects_tampered_card() {
        let kp = Keypair::generate_ed25519();
        let peer = kp.public().to_peer_id();
        let original = sample_card();
        let wit = None;
        let sig = sign_hello(&kp, &original, &wit).unwrap();

        // Verificamos contra una Card distinta (mismo shape, distinto label).
        let tampered: WireCard = Card::new("test.tampered").into();
        let err = verify_hello(&sig, &tampered, &wit, peer).unwrap_err();
        assert!(matches!(err, SignatureError::Invalid), "got {err:?}");
    }

    #[test]
    fn verify_rejects_corrupted_signature() {
        let kp = Keypair::generate_ed25519();
        let peer = kp.public().to_peer_id();
        let card = sample_card();
        let wit = None;
        let mut sig = sign_hello(&kp, &card, &wit).unwrap();
        // Flip un bit de la firma.
        if let Some(b) = sig.signature.last_mut() {
            *b ^= 0x01;
        }
        let err = verify_hello(&sig, &card, &wit, peer).unwrap_err();
        assert!(matches!(err, SignatureError::Invalid), "got {err:?}");
    }
}
