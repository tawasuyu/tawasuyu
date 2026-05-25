//! Identidad del nodo — par X25519 persistente.
//!
//! El par se guarda en `~/.config/shuma/keys/identity.x25519` con
//! permisos `0600`. La pubkey se serializa como hex (64 chars) para
//! que se pueda intercambiar por copy/paste — formato a propósito
//! incompatible con `~/.ssh/authorized_keys` (no es SSH, no usamos
//! ese formato).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Longitud de una clave X25519 (privada o pública).
pub const KEY_LEN: usize = 32;

/// Una clave pública X25519 — 32 bytes. Identifica un nodo
/// indeleblemente para `shuma-link`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(pub [u8; KEY_LEN]);

impl PublicKey {
    pub fn from_bytes(b: [u8; KEY_LEN]) -> Self {
        Self(b)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Formato canónico hex en minúsculas, sin separadores. Coincide
    /// con `hex::encode` y es lo que aparece en `known_peers.txt`.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parsea hex (con o sin prefijo `0x`, case-insensitive). Devuelve
    /// error si la longitud no es exacta.
    pub fn from_hex(s: &str) -> Result<Self, KeypairError> {
        let trimmed = s.trim();
        let body = trimmed.strip_prefix("0x").unwrap_or(trimmed);
        let bytes = hex::decode(body).map_err(|_| KeypairError::InvalidHex)?;
        let arr: [u8; KEY_LEN] = bytes.try_into().map_err(|_| KeypairError::WrongLength)?;
        Ok(Self(arr))
    }
}

/// Par X25519 (privada + pública). La privada **no** debe filtrarse —
/// `Debug` la enmascara y `Drop` no la cero-iza (snow lo hace al
/// crear las sesiones; mientras esté en memoria del proceso es OK).
#[derive(Clone)]
pub struct Keypair {
    private: [u8; KEY_LEN],
    public: PublicKey,
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keypair")
            .field("public", &self.public.to_hex())
            .field("private", &"<oculta>")
            .finish()
    }
}

impl Keypair {
    /// Genera un par nuevo con el RNG del sistema (vía snow, que usa
    /// `rand::rngs::OsRng` por debajo). Es la operación más cara de
    /// este crate (≈ 1 ms).
    pub fn generate() -> Result<Self, KeypairError> {
        let builder = snow::Builder::new(noise_pattern()?);
        let kp = builder.generate_keypair().map_err(KeypairError::Snow)?;
        let private: [u8; KEY_LEN] = kp
            .private
            .as_slice()
            .try_into()
            .map_err(|_| KeypairError::WrongLength)?;
        let public_bytes: [u8; KEY_LEN] = kp
            .public
            .as_slice()
            .try_into()
            .map_err(|_| KeypairError::WrongLength)?;
        Ok(Self { private, public: PublicKey(public_bytes) })
    }

    /// La pubkey — segura de exportar/loggear.
    pub fn public(&self) -> PublicKey {
        self.public
    }

    /// Bytes de la pubkey — `&[u8; 32]`.
    pub fn public_bytes(&self) -> &[u8; KEY_LEN] {
        &self.public.0
    }

    /// Acceso a los bytes privados — `snow::Builder` los consume al
    /// arrancar el handshake. No exportar a logs.
    pub fn private_bytes(&self) -> &[u8; KEY_LEN] {
        &self.private
    }

    /// Carga el par desde `path`. Espera 64 bytes: `[priv][pub]`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, KeypairError> {
        let bytes = fs::read(path.as_ref()).map_err(|e| KeypairError::Io(path.as_ref().to_path_buf(), e))?;
        if bytes.len() != KEY_LEN * 2 {
            return Err(KeypairError::WrongLength);
        }
        let private: [u8; KEY_LEN] = bytes[..KEY_LEN].try_into().unwrap();
        let public: [u8; KEY_LEN] = bytes[KEY_LEN..].try_into().unwrap();
        Ok(Self { private, public: PublicKey(public) })
    }

    /// Guarda el par en `path` (`0600` en Unix). Crea el padre si falta.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), KeypairError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| KeypairError::Io(parent.to_path_buf(), e))?;
        }
        let mut out = Vec::with_capacity(KEY_LEN * 2);
        out.extend_from_slice(&self.private);
        out.extend_from_slice(&self.public.0);
        // Crear con `0600` desde el principio para no exponer la clave
        // privada en una micro-ventana de tiempo. En no-Unix, sólo
        // truncamos.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(path)
                .map_err(|e| KeypairError::Io(path.to_path_buf(), e))?;
            f.write_all(&out)
                .map_err(|e| KeypairError::Io(path.to_path_buf(), e))?;
            f.flush()
                .map_err(|e| KeypairError::Io(path.to_path_buf(), e))?;
        }
        #[cfg(not(unix))]
        {
            fs::write(path, &out).map_err(|e| KeypairError::Io(path.to_path_buf(), e))?;
        }
        Ok(())
    }

    /// Conveniencia: ruta canónica `~/.config/shuma/keys/identity.x25519`.
    /// `None` si el SO no expone un directorio de configuración.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("keys").join("identity.x25519"))
    }

    /// Carga `path` si existe, o genera uno nuevo, lo guarda y lo
    /// devuelve. El patrón típico de un daemon al arrancar.
    pub fn load_or_generate(path: impl AsRef<Path>) -> Result<Self, KeypairError> {
        if path.as_ref().exists() {
            Self::load(path)
        } else {
            let kp = Self::generate()?;
            kp.save(&path)?;
            Ok(kp)
        }
    }
}

/// Patrón Noise que usa todo el crate: XK con X25519 + ChaCha20Poly1305
/// + BLAKE2s. El nombre lo entiende `snow::Builder::new`.
pub(crate) fn noise_pattern() -> Result<snow::params::NoiseParams, KeypairError> {
    "Noise_XK_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .map_err(|_| KeypairError::BadPattern)
}

/// Errores del módulo identity.
#[derive(Debug, Error)]
pub enum KeypairError {
    #[error("IO en {}: {}", .0.display(), .1)]
    Io(PathBuf, std::io::Error),
    #[error("snow: {0}")]
    Snow(snow::Error),
    #[error("longitud de clave incorrecta — esperaba {KEY_LEN} bytes")]
    WrongLength,
    #[error("hex inválido en la clave")]
    InvalidHex,
    #[error("patrón Noise mal formado")]
    BadPattern,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generate_yields_different_keypairs() {
        let a = Keypair::generate().unwrap();
        let b = Keypair::generate().unwrap();
        assert_ne!(a.public(), b.public());
    }

    #[test]
    fn save_and_load_round_trip() {
        let d = tempdir().unwrap();
        let path = d.path().join("id.x25519");
        let kp = Keypair::generate().unwrap();
        kp.save(&path).unwrap();
        let back = Keypair::load(&path).unwrap();
        assert_eq!(kp.public(), back.public());
        assert_eq!(kp.private_bytes(), back.private_bytes());
    }

    #[test]
    fn saved_file_is_owner_only_on_unix() {
        let d = tempdir().unwrap();
        let path = d.path().join("id.x25519");
        Keypair::generate().unwrap().save(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&path).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "keypair file no es 0600: {:o}", mode);
        }
    }

    #[test]
    fn load_or_generate_is_idempotent() {
        let d = tempdir().unwrap();
        let path = d.path().join("id.x25519");
        let a = Keypair::load_or_generate(&path).unwrap();
        let b = Keypair::load_or_generate(&path).unwrap();
        assert_eq!(a.public(), b.public());
    }

    #[test]
    fn public_key_hex_round_trip() {
        let kp = Keypair::generate().unwrap();
        let h = kp.public().to_hex();
        assert_eq!(h.len(), 64);
        let back = PublicKey::from_hex(&h).unwrap();
        assert_eq!(back, kp.public());
        // Con prefijo 0x también funciona.
        let back2 = PublicKey::from_hex(&format!("0x{h}")).unwrap();
        assert_eq!(back2, kp.public());
    }

    #[test]
    fn invalid_hex_returns_error() {
        assert!(PublicKey::from_hex("xy").is_err());
        assert!(PublicKey::from_hex("0xZZ").is_err());
        // Largo incorrecto.
        assert!(PublicKey::from_hex("0001").is_err());
    }

    #[test]
    fn malformed_file_fails_load() {
        let d = tempdir().unwrap();
        let p = d.path().join("bad");
        std::fs::write(&p, b"too-short").unwrap();
        assert!(matches!(Keypair::load(&p), Err(KeypairError::WrongLength)));
    }
}
