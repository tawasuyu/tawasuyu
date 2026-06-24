//! La identidad firmante del binario `paloma` (Eje 3: soberanía).
//!
//! Implementa el trait `MailSigner` de `paloma-llimphi` sobre una `Keypair`
//! Ed25519 de `agora`. La seed (32 bytes) se persiste en el dir de config
//! (`~/.config/paloma/identity.seed`, permisos 0600) y se genera con un CSPRNG
//! la primera vez. Firma síncrona, sin runtime.
//!
//! Nota: hoy la seed se guarda en claro (0600). El paso a `agora-keystore`
//! (seed cifrada con passphrase) queda para cuando paloma tenga UI de
//! desbloqueo — ver LEEME · Pendiente.

use std::io::Write;
use std::path::{Path, PathBuf};

use agora_core::Keypair;
use paloma_llimphi::MailSigner;

pub struct AgoraSigner {
    kp: Keypair,
}

impl MailSigner for AgoraSigner {
    fn sign(&self, canonical: &[u8]) -> ([u8; 32], [u8; 64]) {
        (self.kp.public_key(), self.kp.sign(canonical))
    }
}

impl AgoraSigner {
    /// Construye el firmante desde una seed de 32 bytes.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { kp: Keypair::from_seed(seed) }
    }

    /// Clave pública (para logging / mostrar la identidad).
    pub fn public_key(&self) -> [u8; 32] {
        self.kp.public_key()
    }
}

/// Carga la seed de identidad desde `<config_dir>/identity.seed`, o la crea
/// (CSPRNG) si no existe. La comparten el firmante (`AgoraSigner`) y el rail
/// (`RailHost`) — una sola identidad para todo. `None` si no se puede resolver
/// el dir o escribir la seed.
pub fn load_or_create_seed(config_dir: Option<PathBuf>) -> Option<[u8; 32]> {
    let dir = config_dir?;
    let path = dir.join("identity.seed");
    match std::fs::read(&path) {
        Ok(b) if b.len() == 32 => {
            let mut s = [0u8; 32];
            s.copy_from_slice(&b);
            Some(s)
        }
        _ => {
            let mut s = [0u8; 32];
            use rand::RngCore;
            rand::rngs::OsRng.fill_bytes(&mut s);
            std::fs::create_dir_all(&dir).ok()?;
            write_private(&path, &s)?;
            Some(s)
        }
    }
}

/// Escribe la seed con permisos 0600 (sólo el dueño lee/escribe).
fn write_private(path: &Path, seed: &[u8; 32]) -> Option<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    f.write_all(seed).ok()?;
    Some(())
}
