//! Identidad del cuaderno, guardada cifrada con [`agora_keystore`].
//!
//! La clave privada (la semilla Ed25519 de 32 bytes) nunca queda en claro
//! en disco: vive cifrada bajo una passphrase (Argon2id + ChaCha20-Poly1305).
//! [`unlock`] la descifra a pedido. Quien tenga acceso de lectura al disco
//! no obtiene la identidad sin la passphrase — el endurecimiento frente al
//! `identidad.seed` en claro de las primeras versiones.
//!
//! Migración: si encuentra una semilla legacy en claro, la cifra dentro
//! del keystore y borra el archivo en claro, conservando la identidad.

use std::path::Path;

use agora_core::Keypair;
use agora_keystore::Keystore;
use rand::RngCore;

/// Falla al desbloquear o crear la identidad.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("keystore: {0}")]
    Keystore(String),
    #[error("semilla legacy inválida (no son 32 bytes)")]
    SemillaLegacy,
}

/// Desbloquea —o crea— la identidad del cuaderno guardada cifrada en
/// `keys_dir`, descifrándola con `passphrase`. Orden de resolución:
///
/// 1. Si el keystore ya tiene una identidad → la descifra. Passphrase
///    incorrecta ⇒ [`IdentityError::Keystore`] (no se distingue de otros
///    fallos del keystore a propósito: no filtramos si existe o no).
/// 2. Si hay una semilla legacy en claro en `legacy_seed` → la **migra**:
///    la cifra en el keystore con `passphrase`, borra el archivo en claro
///    y devuelve esa identidad.
/// 3. Si no hay nada → genera una identidad nueva (semilla de `OsRng`),
///    la guarda cifrada y la devuelve.
pub fn unlock(
    keys_dir: &Path,
    legacy_seed: Option<&Path>,
    passphrase: &str,
) -> Result<Keypair, IdentityError> {
    let ks = Keystore::open(keys_dir).map_err(|e| IdentityError::Keystore(e.to_string()))?;
    let ids = ks.list().map_err(|e| IdentityError::Keystore(e.to_string()))?;

    // 1. Identidad ya guardada: descifrar.
    if let Some(id) = ids.first().copied() {
        let seed = ks
            .load(id, passphrase)
            .map_err(|e| IdentityError::Keystore(e.to_string()))?;
        return Ok(Keypair::from_seed(seed));
    }

    // 2. Migrar una semilla legacy en claro, si la hay.
    if let Some(path) = legacy_seed {
        if let Ok(bytes) = std::fs::read(path) {
            let seed =
                <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| IdentityError::SemillaLegacy)?;
            let kp = Keypair::from_seed(seed);
            ks.save(kp.identity_id(), &seed, passphrase)
                .map_err(|e| IdentityError::Keystore(e.to_string()))?;
            let _ = std::fs::remove_file(path); // ya está cifrada; fuera el claro
            return Ok(kp);
        }
    }

    // 3. Identidad nueva.
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let kp = Keypair::from_seed(seed);
    ks.save(kp.identity_id(), &seed, passphrase)
        .map_err(|e| IdentityError::Keystore(e.to_string()))?;
    Ok(kp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directorio temporal único por nombre de test (evita tempfile como
    /// dependencia y las colisiones entre tests del mismo proceso).
    fn temp_dir(etiqueta: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("khipu-id-{}-{}", std::process::id(), etiqueta));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn crear_y_desbloquear_da_la_misma_identidad() {
        let dir = temp_dir("crear");
        let a = unlock(&dir, None, "secreta").unwrap();
        let b = unlock(&dir, None, "secreta").unwrap();
        assert_eq!(a.public_key(), b.public_key());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn passphrase_incorrecta_falla() {
        let dir = temp_dir("malpass");
        let _ = unlock(&dir, None, "correcta").unwrap();
        assert!(unlock(&dir, None, "incorrecta").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migra_semilla_legacy_en_claro() {
        let base = temp_dir("migra");
        std::fs::create_dir_all(&base).unwrap();
        let legacy = base.join("identidad.seed");
        let semilla = [7u8; 32];
        std::fs::write(&legacy, semilla).unwrap();

        let keys = base.join("keys");
        let kp = unlock(&keys, Some(&legacy), "secreta").unwrap();
        // Conserva la identidad de la semilla legacy.
        assert_eq!(kp.public_key(), Keypair::from_seed(semilla).public_key());
        // El archivo en claro fue borrado.
        assert!(!legacy.exists());
        // Y ahora se descifra del keystore, sin legacy.
        let kp2 = unlock(&keys, None, "secreta").unwrap();
        assert_eq!(kp2.public_key(), kp.public_key());

        let _ = std::fs::remove_dir_all(&base);
    }
}
