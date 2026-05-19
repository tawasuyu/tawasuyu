//! Identidad multi-key del nodo: separación entre **identity** (master,
//! persistente forever) y **session** (keypair libp2p efímera, rotable).
//!
//! ## Problema que resuelve
//!
//! Hasta Fase 3, el `peer_id` libp2p era la única identidad. Rotar la
//! keypair (por compromiso, por higiene, por cambio de hardware)
//! cambiaba el peer_id, lo que invalidaba todas las allowlists
//! remotas y desconectaba al nodo de la malla. Imposible rotar sin
//! coordinar.
//!
//! ## Modelo
//!
//! Cada nodo tiene **dos** keypairs Ed25519:
//!
//! - **Identity** (master): persistente para siempre. Identifica al
//!   nodo como entidad lógica. Su `peer_id` es lo que va en
//!   allowlists/denylists remotas.
//! - **Session** (operacional): la que libp2p usa para Noise. Puede
//!   rotarse libremente sin coordinar — el nodo emite un
//!   [`SessionCert`] firmado con la identity que prueba "esta session
//!   key pertenece a mí".
//!
//! ## Wire
//!
//! El cert viaja en `Hello.identity_cert: Option<SessionCert>`. El
//! server valida:
//! 1. La session key del cert == public key de `Hello.signature` ==
//!    deriva al peer_id autenticado por Noise (consistencia interna).
//! 2. La firma del cert verifica con la master pubkey declarada.
//! 3. El cert no está expirado.
//! 4. La política (allowlist/denylist) se evalúa contra
//!    `master.to_peer_id()`, NO contra el session peer_id.
//!
//! Sin cert, el server cae al modelo de Fase 3: policy contra session
//! peer_id (compat). Esto permite migración gradual.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use brahman_net::{Keypair, PeerId, PublicKey};
use serde::{Deserialize, Serialize};

/// TTL recomendado para un session cert: 24 horas. Suficiente para
/// que un nodo "viva" un día sin re-emitir; corto enough para que
/// un cert robado no sirva por mucho. Operadores con políticas
/// estrictas pueden bajarlo; con uptime largo, subirlo.
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Identidad lógica del nodo. Wraps la master keypair y emite certs
/// de session firmados.
///
/// **Critical**: la master keypair NUNCA debe filtrarse a la red.
/// Sólo se usa para firmar certs locales y para derivar
/// `master_peer_id`. Ni siquiera el swarm libp2p la ve — ese usa la
/// session keypair.
#[derive(Clone)]
pub struct Identity {
    master: Arc<Keypair>,
}

impl Identity {
    /// Construye una Identity a partir de una keypair existente.
    /// Típicamente cargada desde disco vía `keypair_store::load_or_generate`.
    pub fn from_keypair(master: Keypair) -> Self {
        Self {
            master: Arc::new(master),
        }
    }

    /// Variante para callers que ya tienen la keypair en `Arc`.
    pub fn from_arc(master: Arc<Keypair>) -> Self {
        Self { master }
    }

    /// PeerId derivado de la master pubkey. Ésta es la identidad
    /// "lógica" estable del nodo — lo que va en allowlists/denylists.
    pub fn master_peer_id(&self) -> PeerId {
        self.master.public().to_peer_id()
    }

    /// Emite un [`SessionCert`] firmado: certifica que la session
    /// keypair `session` pertenece a esta identity hasta `now + ttl`.
    pub fn issue_session_cert(
        &self,
        session: &Keypair,
        ttl: Duration,
    ) -> Result<SessionCert, CertError> {
        let now_ms = now_unix_ms();
        let expires_at_ms = now_ms.saturating_add(ttl.as_millis() as u64);
        let session_pubkey = session.public().encode_protobuf();
        let master_pubkey = self.master.public().encode_protobuf();
        let payload = sign_payload(&session_pubkey, expires_at_ms);
        let signature = self
            .master
            .sign(&payload)
            .map_err(|e| CertError::Sign(e.to_string()))?;
        Ok(SessionCert {
            version: SESSION_CERT_VERSION,
            session_pubkey,
            master_pubkey,
            expires_at_ms,
            signature,
        })
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("master_peer_id", &self.master_peer_id())
            .finish()
    }
}

/// Versión del esquema del cert. Bump al cambiar `sign_payload` o
/// el shape de `SessionCert`.
pub const SESSION_CERT_VERSION: u8 = 1;

/// Certificado firmado por la identity que vincula una session key
/// libp2p a la identidad master del nodo, con expiración.
///
/// **Wire**: viaja en `Hello.identity_cert`. Las pubkeys van en
/// formato canónico libp2p (`encode_protobuf`) — mismo encoding que
/// `HelloSignature.public_key`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCert {
    /// Versión del esquema (ver `SESSION_CERT_VERSION`).
    pub version: u8,
    /// Public key de la session libp2p (la que firma el Hello), en
    /// formato libp2p protobuf.
    pub session_pubkey: Vec<u8>,
    /// Public key de la master identity, en formato libp2p protobuf.
    /// El verificador deriva el `master_peer_id` desde acá.
    pub master_pubkey: Vec<u8>,
    /// Expiración en milisegundos desde UNIX_EPOCH. Tras esto, el
    /// cert no es válido y el nodo debe re-emitirse uno nuevo
    /// (rotando o re-firmando la misma session).
    pub expires_at_ms: u64,
    /// Firma Ed25519 del master sobre `sign_payload(session_pubkey, expires_at_ms)`.
    pub signature: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum CertError {
    #[error("versión de cert desconocida: {0} (esperaba {SESSION_CERT_VERSION})")]
    UnknownVersion(u8),
    #[error("decode master_pubkey: {0}")]
    DecodeMaster(String),
    #[error("decode session_pubkey: {0}")]
    DecodeSession(String),
    #[error("firma del cert inválida")]
    InvalidSignature,
    #[error("cert expirado: expires_at_ms={expires}, now_ms={now}")]
    Expired { expires: u64, now: u64 },
    #[error("session_pubkey del cert no coincide con la del Hello.signature")]
    SessionMismatch,
    #[error("error al firmar: {0}")]
    Sign(String),
}

impl SessionCert {
    /// Verifica el cert: versión, firma criptográfica, no expiración.
    /// Devuelve el `(master_peer_id, session_peer_id)` derivados.
    ///
    /// El caller debe además chequear que `session_peer_id` coincide
    /// con el peer_id autenticado por Noise (lo verifica
    /// [`verify_against_session`]).
    pub fn verify(&self) -> Result<(PeerId, PeerId), CertError> {
        if self.version != SESSION_CERT_VERSION {
            return Err(CertError::UnknownVersion(self.version));
        }
        let master_pk = PublicKey::try_decode_protobuf(&self.master_pubkey)
            .map_err(|e| CertError::DecodeMaster(e.to_string()))?;
        let session_pk = PublicKey::try_decode_protobuf(&self.session_pubkey)
            .map_err(|e| CertError::DecodeSession(e.to_string()))?;
        let payload = sign_payload(&self.session_pubkey, self.expires_at_ms);
        if !master_pk.verify(&payload, &self.signature) {
            return Err(CertError::InvalidSignature);
        }
        let now = now_unix_ms();
        if now >= self.expires_at_ms {
            return Err(CertError::Expired {
                expires: self.expires_at_ms,
                now,
            });
        }
        Ok((master_pk.to_peer_id(), session_pk.to_peer_id()))
    }

    /// Verifica el cert Y exige que su `session_pubkey` matchee a
    /// `expected_session_pubkey` (la que firmó el Hello). Esto
    /// previene que un atacante reutilice un cert válido con una
    /// session key distinta.
    ///
    /// Devuelve el `master_peer_id` derivado, que es el que el server
    /// debe usar para evaluar la política de admisión.
    pub fn verify_against_session(
        &self,
        expected_session_pubkey: &[u8],
    ) -> Result<PeerId, CertError> {
        if self.session_pubkey.as_slice() != expected_session_pubkey {
            return Err(CertError::SessionMismatch);
        }
        let (master_peer, _session_peer) = self.verify()?;
        Ok(master_peer)
    }
}

/// Concat canónico de los campos firmados. Cualquier cambio aquí
/// rompe compatibilidad — bump `SESSION_CERT_VERSION`.
fn sign_payload(session_pubkey: &[u8], expires_at_ms: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 4 + session_pubkey.len() + 8);
    buf.push(SESSION_CERT_VERSION);
    buf.extend_from_slice(b"sess");
    buf.extend_from_slice(&(session_pubkey.len() as u32).to_le_bytes());
    buf.extend_from_slice(session_pubkey);
    buf.extend_from_slice(&expires_at_ms.to_le_bytes());
    buf
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify_cert() {
        let master = Keypair::generate_ed25519();
        let session = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let cert = id
            .issue_session_cert(&session, DEFAULT_SESSION_TTL)
            .unwrap();
        let (master_peer, session_peer) = cert.verify().unwrap();
        assert_eq!(master_peer, id.master_peer_id());
        assert_eq!(session_peer, session.public().to_peer_id());
    }

    #[test]
    fn verify_against_session_admits_matching() {
        let master = Keypair::generate_ed25519();
        let session = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let cert = id
            .issue_session_cert(&session, DEFAULT_SESSION_TTL)
            .unwrap();
        let session_pk = session.public().encode_protobuf();
        let master_peer = cert.verify_against_session(&session_pk).unwrap();
        assert_eq!(master_peer, id.master_peer_id());
    }

    #[test]
    fn verify_against_session_rejects_mismatch() {
        let master = Keypair::generate_ed25519();
        let session_a = Keypair::generate_ed25519();
        let session_b = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let cert = id
            .issue_session_cert(&session_a, DEFAULT_SESSION_TTL)
            .unwrap();
        let other_pk = session_b.public().encode_protobuf();
        let err = cert.verify_against_session(&other_pk).unwrap_err();
        assert!(matches!(err, CertError::SessionMismatch), "got {err:?}");
    }

    #[test]
    fn cert_with_zero_ttl_is_expired() {
        let master = Keypair::generate_ed25519();
        let session = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let cert = id
            .issue_session_cert(&session, Duration::from_secs(0))
            .unwrap();
        // Pequeña espera para asegurar que now_ms > expires_at_ms.
        std::thread::sleep(Duration::from_millis(5));
        let err = cert.verify().unwrap_err();
        assert!(matches!(err, CertError::Expired { .. }), "got {err:?}");
    }

    #[test]
    fn tampered_signature_rejected() {
        let master = Keypair::generate_ed25519();
        let session = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let mut cert = id
            .issue_session_cert(&session, DEFAULT_SESSION_TTL)
            .unwrap();
        if let Some(b) = cert.signature.last_mut() {
            *b ^= 0x01;
        }
        let err = cert.verify().unwrap_err();
        assert!(matches!(err, CertError::InvalidSignature), "got {err:?}");
    }

    #[test]
    fn tampered_expires_at_rejected() {
        // Si alguien extiende el expires_at sin re-firmar, la firma
        // no cuadra → InvalidSignature.
        let master = Keypair::generate_ed25519();
        let session = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let mut cert = id
            .issue_session_cert(&session, DEFAULT_SESSION_TTL)
            .unwrap();
        cert.expires_at_ms = cert.expires_at_ms.saturating_add(1_000_000);
        let err = cert.verify().unwrap_err();
        assert!(matches!(err, CertError::InvalidSignature), "got {err:?}");
    }

    #[test]
    fn unknown_version_rejected() {
        let master = Keypair::generate_ed25519();
        let session = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let mut cert = id
            .issue_session_cert(&session, DEFAULT_SESSION_TTL)
            .unwrap();
        cert.version = 99;
        let err = cert.verify().unwrap_err();
        assert!(matches!(err, CertError::UnknownVersion(99)), "got {err:?}");
    }

    #[test]
    fn rotated_session_with_same_master_yields_same_master_peer_id() {
        // La propiedad fundamental: rotar la session key NO cambia el
        // master_peer_id derivado del cert.
        let master = Keypair::generate_ed25519();
        let id = Identity::from_keypair(master);
        let original_master_peer = id.master_peer_id();

        let session1 = Keypair::generate_ed25519();
        let cert1 = id
            .issue_session_cert(&session1, DEFAULT_SESSION_TTL)
            .unwrap();
        let (master_from_cert1, _) = cert1.verify().unwrap();

        // Rotar: nueva session keypair, mismo master.
        let session2 = Keypair::generate_ed25519();
        let cert2 = id
            .issue_session_cert(&session2, DEFAULT_SESSION_TTL)
            .unwrap();
        let (master_from_cert2, _) = cert2.verify().unwrap();

        assert_eq!(master_from_cert1, original_master_peer);
        assert_eq!(master_from_cert2, original_master_peer);
        assert_eq!(
            master_from_cert1, master_from_cert2,
            "rotar session NO debe cambiar el master_peer_id"
        );
    }
}
