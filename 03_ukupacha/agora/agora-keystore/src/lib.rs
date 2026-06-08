//! `agora-keystore` — almacén cifrado de seeds Ed25519.
//!
//! Una identidad de ágora es una `Keypair` derivada de una seed de 32
//! bytes. Esa seed es lo único que hace falta perpetuar entre arranques.
//! Persistirla en claro permitiría a cualquier proceso que lea el
//! archivo suplantar al dueño — por eso aquí va cifrada con
//! ChaCha20-Poly1305 bajo una clave Argon2id derivada de la passphrase
//! del usuario.
//!
//! ## Estructura en disco
//!
//! ```text
//! magic(8) || version(4 LE) || salt(16) || nonce(12) || ciphertext(48)
//! ```
//!
//! - `magic = b"agorakey"`.
//! - `version = 1` hoy.
//! - `salt` random por archivo, alimenta Argon2id.
//! - `nonce` random fresco por cifrado.
//! - `ciphertext` = `encrypt(key, nonce, seed[32])` = `seed_cifrado(32) || tag(16)`.
//!
//! El tag Poly1305 hace indistinguibles "passphrase incorrecta" y
//! "archivo manipulado" — para el atacante es la misma falla, lo que es
//! deliberado.
//!
//! ## Lo que NO hace
//!
//! - No genera seeds.
//! - No decide política de desbloqueo.
//! - No zeroea la seed descifrada en memoria del caller — esa es su
//!   responsabilidad mientras la usa.

#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aead::Aead;
use agora_core::IdentityId;
use argon2::Argon2;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use rand::RngCore;
use thiserror::Error;

const MAGIC: &[u8; 8] = b"agorakey";
const VERSION: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// `seed (32) || Poly1305 tag (16)`.
const CIPHERTEXT_LEN: usize = 32 + 16;
/// Largo total de un blob en disco.
pub const BLOB_LEN: usize = 8 + 4 + SALT_LEN + NONCE_LEN + CIPHERTEXT_LEN;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("magic inválido — el archivo no parece un keystore de agora")]
    BadMagic,

    #[error("versión de esquema desconocida: {found} (esta build soporta {VERSION})")]
    SchemaDesconocida { found: u32 },

    #[error(
        "largo de blob inválido: {found} bytes (se esperaba exactamente {})",
        BLOB_LEN
    )]
    BlobLenInvalido { found: usize },

    #[error("no se pudo derivar la clave desde la passphrase: {0}")]
    Kdf(String),

    #[error("autenticación fallida — passphrase incorrecta o archivo manipulado")]
    AuthFailed,

    #[error("no se pudo resolver el directorio de datos del usuario")]
    DirNoResuelto,

    #[error("identidad no encontrada en el keystore: {0}")]
    NoEncontrada(IdentityId),
}

pub type Result<T> = std::result::Result<T, Error>;

// =============================================================================
//  Cifrado / descifrado puro
// =============================================================================

/// Blob serializado listo para escribir a disco. Forma opaca al caller:
/// pasar `as_bytes()` / `from_bytes()` para cruzar la frontera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSeed {
    bytes: [u8; BLOB_LEN],
}

impl EncryptedSeed {
    /// Bytes crudos del blob (88 bytes exactos).
    pub fn as_bytes(&self) -> &[u8; BLOB_LEN] {
        &self.bytes
    }

    /// Toma ownership de los bytes para serializar en disco.
    pub fn into_bytes(self) -> [u8; BLOB_LEN] {
        self.bytes
    }

    /// Reconstruye desde bytes. Verifica magic y versión; no toca cripto.
    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() != BLOB_LEN {
            return Err(Error::BlobLenInvalido { found: b.len() });
        }
        if &b[0..8] != MAGIC {
            return Err(Error::BadMagic);
        }
        let mut v = [0u8; 4];
        v.copy_from_slice(&b[8..12]);
        let version = u32::from_le_bytes(v);
        if version != VERSION {
            return Err(Error::SchemaDesconocida { found: version });
        }
        let mut bytes = [0u8; BLOB_LEN];
        bytes.copy_from_slice(b);
        Ok(Self { bytes })
    }
}

/// Deriva una clave de 32 bytes a partir de una passphrase y un salt
/// fijo (16 bytes) usando Argon2id con los parámetros default de la
/// versión 0.5 del crate.
fn derive_key(passphrase: &str, salt: &[u8; SALT_LEN]) -> Result<[u8; 32]> {
    let argon2 = Argon2::default();
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| Error::Kdf(e.to_string()))?;
    Ok(key)
}

/// Cifra una seed con la passphrase dada. Genera salt y nonce frescos
/// por llamada — dos cifrados de la misma seed con la misma passphrase
/// producen blobs distintos (deseado).
pub fn encrypt_seed(seed: &[u8; 32], passphrase: &str) -> Result<EncryptedSeed> {
    let mut rng = rand::thread_rng();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut nonce);

    let key = derive_key(passphrase, &salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), seed.as_slice())
        .map_err(|_| Error::AuthFailed)?;
    debug_assert_eq!(ciphertext.len(), CIPHERTEXT_LEN);

    let mut bytes = [0u8; BLOB_LEN];
    bytes[0..8].copy_from_slice(MAGIC);
    bytes[8..12].copy_from_slice(&VERSION.to_le_bytes());
    bytes[12..12 + SALT_LEN].copy_from_slice(&salt);
    bytes[12 + SALT_LEN..12 + SALT_LEN + NONCE_LEN].copy_from_slice(&nonce);
    bytes[12 + SALT_LEN + NONCE_LEN..].copy_from_slice(&ciphertext);
    Ok(EncryptedSeed { bytes })
}

/// Descifra un blob con la passphrase. Devuelve [`Error::AuthFailed`]
/// indistinguible entre passphrase incorrecta y archivo manipulado.
pub fn decrypt_seed(blob: &EncryptedSeed, passphrase: &str) -> Result<[u8; 32]> {
    let b = &blob.bytes;
    if &b[0..8] != MAGIC {
        return Err(Error::BadMagic);
    }

    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&b[12..12 + SALT_LEN]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&b[12 + SALT_LEN..12 + SALT_LEN + NONCE_LEN]);
    let ciphertext = &b[12 + SALT_LEN + NONCE_LEN..];

    let key = derive_key(passphrase, &salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext)
        .map_err(|_| Error::AuthFailed)?;
    if plaintext.len() != 32 {
        return Err(Error::AuthFailed);
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&plaintext);
    Ok(seed)
}

// =============================================================================
//  Keystore — directorio con un archivo por identidad
// =============================================================================

/// Almacén local cifrado: un directorio con `<hex(IdentityId)>.key` por
/// identidad. Crea el directorio al abrirse si no existe.
#[derive(Debug, Clone)]
pub struct Keystore {
    dir: PathBuf,
}

impl Keystore {
    /// Abre el keystore en `~/.local/share/agora/keys/` (XDG en Linux,
    /// equivalente en macOS/Windows vía `directories`).
    pub fn open_default() -> Result<Self> {
        let proj = directories::ProjectDirs::from("net", "tawasuyu", "agora")
            .ok_or(Error::DirNoResuelto)?;
        let dir = proj.data_dir().join("keys");
        Self::open(dir)
    }

    /// Abre el keystore en `dir`, creándolo si no existe.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Directorio que respalda el keystore.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    fn file_for(&self, id: IdentityId) -> PathBuf {
        let mut name = String::with_capacity(32 * 2 + 4);
        for b in id.as_bytes() {
            use std::fmt::Write;
            let _ = write!(name, "{b:02x}");
        }
        name.push_str(".key");
        self.dir.join(name)
    }

    /// `true` si hay un archivo para esa identidad.
    pub fn exists(&self, id: IdentityId) -> bool {
        self.file_for(id).exists()
    }

    /// Lista todas las identidades guardadas, leyendo nombres de archivo
    /// `*.key`. Devuelve ordenado por bytes del id para que la salida
    /// sea estable entre lecturas. Entradas con nombre no reconocido se
    /// ignoran en silencio — el directorio puede contener archivos
    /// vecinos sin que list() rompa.
    pub fn list(&self) -> Result<Vec<IdentityId>> {
        let mut ids = Vec::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(Error::Io(e)),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".key") else { continue };
            if let Some(id) = id_from_hex(stem) {
                ids.push(id);
            }
        }
        ids.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        Ok(ids)
    }

    /// Guarda (o sobreescribe) la seed cifrada para `id`. Atómico vía
    /// tmp + rename — un crash no deja un archivo a medio escribir.
    pub fn save(&self, id: IdentityId, seed: &[u8; 32], passphrase: &str) -> Result<()> {
        let blob = encrypt_seed(seed, passphrase)?;
        let final_path = self.file_for(id);
        let tmp = with_ext(&final_path, "tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(blob.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &final_path)?;
        Ok(())
    }

    /// Lee y descifra la seed de `id`. Errores:
    /// - [`Error::NoEncontrada`] si no hay archivo;
    /// - [`Error::AuthFailed`] si la passphrase no descifra.
    pub fn load(&self, id: IdentityId, passphrase: &str) -> Result<[u8; 32]> {
        let path = self.file_for(id);
        if !path.exists() {
            return Err(Error::NoEncontrada(id));
        }
        let mut f = File::open(&path)?;
        let mut buf = Vec::with_capacity(BLOB_LEN);
        f.read_to_end(&mut buf)?;
        let blob = EncryptedSeed::from_bytes(&buf)?;
        decrypt_seed(&blob, passphrase)
    }

    /// Borra el archivo de `id`. No-op si no existe.
    pub fn remove(&self, id: IdentityId) -> Result<()> {
        let path = self.file_for(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

fn with_ext(p: &Path, ext: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

/// Reconstruye un `IdentityId` desde su forma hex de 64 chars. Devuelve
/// `None` si el largo no coincide o si algún chunk no es hex válido.
fn id_from_hex(hex: &str) -> Option<IdentityId> {
    if hex.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        bytes[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(IdentityId::from_bytes(bytes))
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::Keypair;

    fn seed_de(s: u8) -> [u8; 32] {
        [s; 32]
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let seed = seed_de(42);
        let blob = encrypt_seed(&seed, "passphrase fuerte").unwrap();
        let back = decrypt_seed(&blob, "passphrase fuerte").unwrap();
        assert_eq!(back, seed);
    }

    #[test]
    fn wrong_passphrase_falla() {
        let blob = encrypt_seed(&seed_de(7), "real").unwrap();
        assert!(matches!(
            decrypt_seed(&blob, "falsa"),
            Err(Error::AuthFailed)
        ));
    }

    #[test]
    fn ciphertext_manipulado_falla() {
        let mut blob_bytes = encrypt_seed(&seed_de(7), "real").unwrap().into_bytes();
        // Flipea un bit del ciphertext — el Poly1305 tag debería detectarlo.
        let i = 12 + SALT_LEN + NONCE_LEN + 5;
        blob_bytes[i] ^= 0x01;
        let blob = EncryptedSeed::from_bytes(&blob_bytes).unwrap();
        assert!(matches!(decrypt_seed(&blob, "real"), Err(Error::AuthFailed)));
    }

    #[test]
    fn salt_manipulado_falla() {
        let mut blob_bytes = encrypt_seed(&seed_de(7), "real").unwrap().into_bytes();
        // Cambiar el salt cambia la clave derivada → desencriptado falla.
        blob_bytes[12] ^= 0xFF;
        let blob = EncryptedSeed::from_bytes(&blob_bytes).unwrap();
        assert!(matches!(decrypt_seed(&blob, "real"), Err(Error::AuthFailed)));
    }

    #[test]
    fn magic_invalido_se_detecta() {
        let mut bytes = [0u8; BLOB_LEN];
        bytes[0..8].copy_from_slice(b"NOPENOPE");
        assert!(matches!(
            EncryptedSeed::from_bytes(&bytes),
            Err(Error::BadMagic)
        ));
    }

    #[test]
    fn version_desconocida_se_detecta() {
        let blob_bytes = encrypt_seed(&seed_de(1), "x").unwrap().into_bytes();
        let mut bytes = blob_bytes;
        bytes[8..12].copy_from_slice(&999u32.to_le_bytes());
        assert!(matches!(
            EncryptedSeed::from_bytes(&bytes),
            Err(Error::SchemaDesconocida { found: 999 })
        ));
    }

    #[test]
    fn largo_invalido_se_detecta() {
        let bytes = vec![0u8; BLOB_LEN - 5];
        assert!(matches!(
            EncryptedSeed::from_bytes(&bytes),
            Err(Error::BlobLenInvalido { .. })
        ));
    }

    #[test]
    fn salt_y_nonce_frescos_por_cifrado() {
        // Dos cifrados de la misma seed con la misma passphrase deben
        // producir blobs distintos byte a byte (salt + nonce random).
        let a = encrypt_seed(&seed_de(1), "x").unwrap();
        let b = encrypt_seed(&seed_de(1), "x").unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
        // Pero ambos descifran al mismo plaintext.
        assert_eq!(decrypt_seed(&a, "x").unwrap(), decrypt_seed(&b, "x").unwrap());
    }

    #[test]
    fn keystore_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let kp = Keypair::from_seed(seed_de(9));
        ks.save(kp.identity_id(), &seed_de(9), "secreto").unwrap();
        assert!(ks.exists(kp.identity_id()));
        let back = ks.load(kp.identity_id(), "secreto").unwrap();
        assert_eq!(back, seed_de(9));
    }

    #[test]
    fn keystore_load_passphrase_incorrecta() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let kp = Keypair::from_seed(seed_de(9));
        ks.save(kp.identity_id(), &seed_de(9), "real").unwrap();
        assert!(matches!(
            ks.load(kp.identity_id(), "falsa"),
            Err(Error::AuthFailed)
        ));
    }

    #[test]
    fn keystore_load_identidad_ausente() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let id = Keypair::from_seed(seed_de(9)).identity_id();
        assert!(matches!(ks.load(id, "x"), Err(Error::NoEncontrada(_))));
    }

    #[test]
    fn keystore_list_ordenado_y_completo() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let ids: Vec<_> = (0..4u8)
            .map(|i| {
                let kp = Keypair::from_seed(seed_de(i + 1));
                ks.save(kp.identity_id(), &seed_de(i + 1), "p").unwrap();
                kp.identity_id()
            })
            .collect();
        let listed = ks.list().unwrap();
        assert_eq!(listed.len(), ids.len());
        let mut expected = ids.clone();
        expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(listed, expected);
    }

    #[test]
    fn keystore_remove_es_idempotente() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let kp = Keypair::from_seed(seed_de(9));
        ks.save(kp.identity_id(), &seed_de(9), "p").unwrap();
        ks.remove(kp.identity_id()).unwrap();
        assert!(!ks.exists(kp.identity_id()));
        // Remover dos veces no error.
        ks.remove(kp.identity_id()).unwrap();
    }

    #[test]
    fn keystore_save_sobreescribe() {
        let dir = tempfile::tempdir().unwrap();
        let ks = Keystore::open(dir.path()).unwrap();
        let kp = Keypair::from_seed(seed_de(9));
        ks.save(kp.identity_id(), &seed_de(9), "uno").unwrap();
        ks.save(kp.identity_id(), &seed_de(9), "dos").unwrap();
        assert!(ks.load(kp.identity_id(), "uno").is_err());
        assert_eq!(ks.load(kp.identity_id(), "dos").unwrap(), seed_de(9));
    }
}
