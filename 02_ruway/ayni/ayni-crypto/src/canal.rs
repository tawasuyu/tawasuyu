//! Canal E2EE 1:1 — confidencialidad sobre el grafo firmado (P2).
//!
//! La firma de un nodo prueba QUIÉN lo escribió; no esconde QUÉ dice. Este
//! módulo añade la confidencialidad: un [`CanalSeguro`] entre dos identidades
//! cifra el claro a un blob AEAD que viaja como [`ayni_core::Carga::Cifrado`].
//! Como ese blob es el contenido firmado y direccionado, la autoría y la
//! integridad siguen siendo públicas y verificables —sólo el contenido queda
//! oculto a quien no es del canal—.
//!
//! Construcción (sólo primitivas auditadas — la regla dura del repo):
//!   * **X25519** acuerda un secreto compartido por Diffie-Hellman entre la
//!     clave privada de uno y la pública del otro. Es simétrico: Alicia·pub(Beto)
//!     == Beto·pub(Alicia). El par X25519 se deriva de la MISMA semilla agora
//!     (HKDF con etiqueta de dominio) — la identidad Ed25519 y la de cifrado son
//!     una sola raíz, sin material de clave extra que distribuir aparte.
//!   * **HKDF-SHA256** estira el secreto DH a la clave del canal.
//!   * **ChaCha20-Poly1305** cifra cada mensaje con un nonce aleatorio (que se
//!     antepone al ciphertext).
//!
//! Es un canal static-static (estilo `crypto_box` de libsodium): da
//! confidencialidad + integridad, NO forward-secrecy ni post-compromise. Esos
//! los aporta MLS (RFC 9420), diferido a una fase posterior; este canal es el
//! seam donde MLS entrará para el caso de grupo.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::ErrorCripto;

/// Etiqueta de dominio para derivar el par X25519 desde la semilla agora —
/// separa este uso de cualquier otro que derive de la misma semilla.
const INFO_X25519: &[u8] = b"ayni-x25519-v1";
/// Etiqueta de dominio para derivar la clave de canal desde el secreto DH.
const INFO_CANAL: &[u8] = b"ayni-canal-1a1-v1";
/// Tamaño del nonce de ChaCha20-Poly1305.
const NONCE_LEN: usize = 12;

/// Deriva la clave privada X25519 de una semilla agora (HKDF-SHA256).
pub(crate) fn secreto_x25519(seed: &[u8; 32]) -> StaticSecret {
    let hk = Hkdf::<Sha256>::new(None, seed);
    let mut okm = [0u8; 32];
    hk.expand(INFO_X25519, &mut okm)
        .expect("HKDF expand 32 bytes nunca falla");
    StaticSecret::from(okm)
}

/// La clave pública X25519 correspondiente a una semilla — lo que un par publica
/// para que otro pueda abrirle un canal.
pub(crate) fn publico_x25519(seed: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&secreto_x25519(seed)).to_bytes()
}

/// Un canal cifrado 1:1 entre dos identidades. Ambos extremos lo derivan por su
/// cuenta —misma clave compartida, mismo `CanalSeguro`— sin intercambiar
/// secretos: sólo hace falta conocer la clave pública X25519 del otro.
pub struct CanalSeguro {
    cifrador: ChaCha20Poly1305,
}

impl CanalSeguro {
    /// Deriva el canal desde MI secreto X25519 y la clave pública X25519 del otro.
    pub(crate) fn derivar(mio: &StaticSecret, su_publico: &[u8; 32]) -> CanalSeguro {
        let compartido = mio.diffie_hellman(&PublicKey::from(*su_publico));
        let hk = Hkdf::<Sha256>::new(None, compartido.as_bytes());
        let mut clave = [0u8; 32];
        hk.expand(INFO_CANAL, &mut clave)
            .expect("HKDF expand 32 bytes nunca falla");
        CanalSeguro {
            cifrador: ChaCha20Poly1305::new((&clave).into()),
        }
    }

    /// Cifra un claro: devuelve `nonce(12) || ciphertext+tag`. El nonce es
    /// aleatorio por mensaje. El resultado es lo que va en `Carga::Cifrado`.
    pub fn cifrar(&self, claro: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self
            .cifrador
            .encrypt(Nonce::from_slice(&nonce), claro)
            .expect("ChaCha20-Poly1305 encrypt no falla con nonce/clave válidos");
        let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        blob
    }

    /// Descifra un blob `nonce || ciphertext`. Falla si el blob es corto, el tag
    /// Poly1305 no valida (manipulación), o la clave es la equivocada.
    pub fn descifrar(&self, blob: &[u8]) -> Result<Vec<u8>, ErrorCripto> {
        if blob.len() < NONCE_LEN {
            return Err(ErrorCripto::CifradoFallo);
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        self.cifrador
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| ErrorCripto::CifradoFallo)
    }
}
