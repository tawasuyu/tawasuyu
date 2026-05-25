//! Identidad self-sovereign basada en Ed25519.
//!
//! Cada peer (y cada autor humano o agente IA) se identifica por un
//! `Did` — el bytestring de su clave pública Ed25519. La clave privada
//! vive en su `Keypair` y nunca sale del nodo. Firmar un mensaje con la
//! `Keypair` produce una `Signature` que cualquiera con el `Did` puede
//! verificar — la atribución es irrefutable bajo el modelo
//! criptográfico estándar (asumiendo que la clave privada no fugó).
//!
//! El esquema es deliberadamente minimalista: no hay rotación de
//! claves, ni revocación, ni metadatos en el DID. Esas capas (DID
//! Documents, métodos `did:web`/`did:ion`, claves de firma versus de
//! cifrado, etc.) se construyen encima cuando la complejidad del
//! producto lo justifique. Por ahora, el `Did` ES la clave pública.

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use argon2::Argon2;
use ed25519_dalek::{
    Signature as Ed25519Sig, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH,
    SIGNATURE_LENGTH,
};
use rand::rngs::OsRng;
use rand::RngCore;

/// Cabecera del formato de keypair cifrado en disco.
const KEYPAIR_MAGIC: &[u8; 8] = b"MINGAKEY";
const KEYPAIR_VERSION: u8 = 1;
const ARGON2_SALT_LEN: usize = 16;
const AES_NONCE_LEN: usize = 12;
const KEYPAIR_HEADER_LEN: usize = 8 + 1 + ARGON2_SALT_LEN + AES_NONCE_LEN;

#[derive(Debug, thiserror::Error)]
pub enum KeypairCryptoError {
    #[error("formato inválido: faltan magic / versión / longitud")]
    InvalidFormat,

    #[error("passphrase incorrecta o cifrado manipulado")]
    DecryptFailed,

    #[error("argon2: {0}")]
    Argon2(String),
}

/// Decentralized Identifier: 32 bytes de la clave pública Ed25519.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Did(pub [u8; SECRET_KEY_LENGTH]);

impl Did {
    pub fn as_bytes(&self) -> &[u8; SECRET_KEY_LENGTH] {
        &self.0
    }

    /// Verifica que `sig` sea una firma válida sobre `msg` producida
    /// con la llave privada correspondiente a este DID. Devuelve
    /// `false` ante cualquier irregularidad: bytes de DID que no son
    /// un punto válido en la curva, firma malformada, mensaje que no
    /// coincide.
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(&self.0) else {
            return false;
        };
        let ed_sig = Ed25519Sig::from_bytes(&sig.0);
        vk.verify(msg, &ed_sig).is_ok()
    }
}

impl std::fmt::Display for Did {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "did:key:")?;
        for b in &self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Signature(
    #[serde(with = "serde_big_array::BigArray")] pub [u8; SIGNATURE_LENGTH],
);

impl Signature {
    pub fn as_bytes(&self) -> &[u8; SIGNATURE_LENGTH] {
        &self.0
    }
}

/// Llave criptográfica completa: priva (para firmar) + pública (para
/// que otros verifiquen). Por convención llamamos `Did` al lado público
/// expuesto al mundo, pero el `Keypair` mantiene ambos lados juntos.
#[derive(Clone)]
pub struct Keypair {
    signing: SigningKey,
}

impl Keypair {
    /// Genera un nuevo `Keypair` usando aleatoriedad del sistema
    /// operativo (`/dev/urandom` en Unix, `BCryptGenRandom` en
    /// Windows). Para producción.
    pub fn generate() -> Self {
        let mut seed = [0u8; SECRET_KEY_LENGTH];
        OsRng.fill_bytes(&mut seed);
        Self::from_seed(&seed)
    }

    /// Reconstruye un `Keypair` desde una semilla de 32 bytes. Misma
    /// semilla → mismo `Keypair` (mismo `Did`, mismas firmas). Útil
    /// para tests reproducibles y para escenarios donde la semilla
    /// proviene de otra fuente determinista (HKDF, BIP39, etc.).
    pub fn from_seed(seed: &[u8; SECRET_KEY_LENGTH]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    pub fn did(&self) -> Did {
        Did(self.signing.verifying_key().to_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.signing.sign(msg).to_bytes())
    }

    /// Cifra la parte privada del keypair con una passphrase humana.
    /// Esquema:
    ///
    /// 1. Genera un salt aleatorio de 16 bytes y un nonce de 12 bytes.
    /// 2. Deriva una clave AES-256 desde la passphrase vía Argon2id
    ///    (parámetros por defecto OWASP).
    /// 3. Cifra los 32 bytes de la clave secreta con AES-256-GCM
    ///    (autenticado: integrity built-in).
    /// 4. Compone el blob:
    ///    `MAGIC(8) || VERSION(1) || SALT(16) || NONCE(12) || CIPHERTEXT+TAG(48)`.
    ///
    /// Total: 85 bytes. La passphrase nunca se almacena; quien no la
    /// conozca no puede recuperar la identidad.
    pub fn encrypt(&self, passphrase: &str) -> Result<Vec<u8>, KeypairCryptoError> {
        let mut salt = [0u8; ARGON2_SALT_LEN];
        let mut nonce_bytes = [0u8; AES_NONCE_LEN];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce_bytes);

        let aes_key = derive_aes_key(passphrase, &salt)?;

        let cipher = Aes256Gcm::new_from_slice(&aes_key)
            .map_err(|_| KeypairCryptoError::DecryptFailed)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let secret_bytes = self.signing.to_bytes();
        let ciphertext = cipher
            .encrypt(nonce, secret_bytes.as_ref())
            .map_err(|_| KeypairCryptoError::DecryptFailed)?;

        let mut out = Vec::with_capacity(KEYPAIR_HEADER_LEN + ciphertext.len());
        out.extend_from_slice(KEYPAIR_MAGIC);
        out.push(KEYPAIR_VERSION);
        out.extend_from_slice(&salt);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Descifra un keypair cifrado con `encrypt`. Falla con
    /// `DecryptFailed` si la passphrase es incorrecta **o** si los
    /// bytes han sido manipulados (AES-GCM detecta ambas vías).
    pub fn decrypt(bytes: &[u8], passphrase: &str) -> Result<Self, KeypairCryptoError> {
        if bytes.len() < KEYPAIR_HEADER_LEN {
            return Err(KeypairCryptoError::InvalidFormat);
        }
        if &bytes[..8] != KEYPAIR_MAGIC {
            return Err(KeypairCryptoError::InvalidFormat);
        }
        if bytes[8] != KEYPAIR_VERSION {
            return Err(KeypairCryptoError::InvalidFormat);
        }

        let salt = &bytes[9..9 + ARGON2_SALT_LEN];
        let nonce_bytes = &bytes[9 + ARGON2_SALT_LEN..KEYPAIR_HEADER_LEN];
        let ciphertext = &bytes[KEYPAIR_HEADER_LEN..];

        let aes_key = derive_aes_key(passphrase, salt)?;
        let cipher = Aes256Gcm::new_from_slice(&aes_key)
            .map_err(|_| KeypairCryptoError::DecryptFailed)?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| KeypairCryptoError::DecryptFailed)?;

        if plaintext.len() != SECRET_KEY_LENGTH {
            return Err(KeypairCryptoError::InvalidFormat);
        }
        let mut seed = [0u8; SECRET_KEY_LENGTH];
        seed.copy_from_slice(&plaintext);
        Ok(Self::from_seed(&seed))
    }
}

fn derive_aes_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], KeypairCryptoError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| KeypairCryptoError::Argon2(e.to_string()))?;
    Ok(key)
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Nunca exponemos la parte privada en debug. Solo el DID.
        write!(f, "Keypair {{ did: {} }}", self.did())
    }
}
