//! Persistencia de la keypair Ed25519 de identidad libp2p de Arje.
//!
//! El `peer_id` que Arje presenta en la malla `brahman-net` deriva de
//! esta keypair. Si se regenera en cada arranque, el peer_id cambia
//! y los nodos remotos pierden la referencia. Persistir el secret a
//! disco (32 bytes raw, permisos 0o600) garantiza identidad estable.
//!
//! ## Path
//!
//! Por orden de precedencia:
//! 1. `BRAHMAN_KEYPAIR_PATH` env var (override explícito).
//! 2. Si PID 1 / root: `/var/lib/brahman/init-keypair.bin`.
//! 3. Si dev mode: `$XDG_DATA_HOME/brahman/init-keypair.bin`, fallback
//!    a `$HOME/.local/share/brahman/init-keypair.bin`, último recurso
//!    `/tmp/brahman-init-keypair.bin` (sin persistencia útil pero al
//!    menos no rompe en CI minimalista).
//!
//! ## Formato
//!
//! 32 bytes raw del secret Ed25519. Sin header, sin metadata. La
//! public key se deriva determinísticamente al cargar. Esto evita
//! depender de un schema de serialización (postcard, json) que
//! pudiera bumpear y romper compat de identidad.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use card_net::Keypair;

/// Tamaño exacto del secret Ed25519.
const SECRET_LEN: usize = 32;

/// Carga la keypair desde `path` si existe, o genera una nueva,
/// la persiste y la devuelve. Devuelve también si fue cargada (true)
/// o generada (false), para logging.
pub fn load_or_generate(path: &Path) -> Result<(Keypair, bool)> {
    if path.exists() {
        let bytes = std::fs::read(path)
            .with_context(|| format!("leer keypair de {}", path.display()))?;
        if bytes.len() != SECRET_LEN {
            bail!(
                "keypair en {} tiene {} bytes, esperaba {}",
                path.display(),
                bytes.len(),
                SECRET_LEN
            );
        }
        let mut secret = [0u8; SECRET_LEN];
        secret.copy_from_slice(&bytes);
        let kp = Keypair::ed25519_from_bytes(secret)
            .with_context(|| format!("decodificar keypair en {}", path.display()))?;
        Ok((kp, true))
    } else {
        let kp = Keypair::generate_ed25519();
        save(path, &kp).context("persistir keypair recién generada")?;
        Ok((kp, false))
    }
}

/// Persiste el secret de `keypair` a `path`. Crea directorios padres,
/// escribe atómico (vía rename), y aplica permisos 0o600 (sólo dueño).
fn save(path: &Path, keypair: &Keypair) -> Result<()> {
    let secret = extract_secret_bytes(keypair)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("crear dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, secret).with_context(|| format!("write tmp {}", tmp.display()))?;
    apply_owner_only_perms(&tmp).context("permisos 0o600 en tmp")?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

fn extract_secret_bytes(keypair: &Keypair) -> Result<[u8; SECRET_LEN]> {
    // libp2p::Keypair no expone secret() directo; pasamos
    // por la variante ed25519. Solo Ed25519 soportado en brahman-net,
    // así que el unwrap es seguro tras with_keypair.
    let ed = keypair
        .clone()
        .try_into_ed25519()
        .map_err(|_| anyhow::anyhow!("la keypair no es Ed25519 (no debería pasar)"))?;
    let bytes = ed.secret();
    let raw: &[u8] = bytes.as_ref();
    if raw.len() != SECRET_LEN {
        bail!("ed25519 secret no es {} bytes", SECRET_LEN);
    }
    let mut out = [0u8; SECRET_LEN];
    out.copy_from_slice(raw);
    Ok(out)
}

#[cfg(unix)]
fn apply_owner_only_perms(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn apply_owner_only_perms(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Resuelve el path del keystore según convención (env > root path >
/// XDG > HOME > tmp).
pub fn default_path(dev_mode: bool) -> PathBuf {
    if let Ok(p) = std::env::var("BRAHMAN_KEYPAIR_PATH") {
        return PathBuf::from(p);
    }

    if !dev_mode {
        // PID 1: paths del sistema. /var/lib es el lugar canónico
        // para state persistente de servicios root.
        return PathBuf::from("/var/lib/brahman/init-keypair.bin");
    }

    // Dev mode: paths de usuario.
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("brahman").join("init-keypair.bin");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("brahman")
            .join("init-keypair.bin");
    }
    PathBuf::from("/tmp/brahman-init-keypair.bin")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generate_persist_and_reload_yields_same_peer_id() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("identity.bin");
        let (kp1, loaded) = load_or_generate(&path).unwrap();
        assert!(!loaded, "primera vez debe generar");
        let peer1 = kp1.public().to_peer_id();

        let (kp2, loaded) = load_or_generate(&path).unwrap();
        assert!(loaded, "segunda vez debe cargar");
        let peer2 = kp2.public().to_peer_id();

        assert_eq!(peer1, peer2, "peer_id estable across reloads");
    }

    #[test]
    fn rejects_corrupted_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.bin");
        std::fs::write(&path, b"too short").unwrap();
        assert!(load_or_generate(&path).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn persisted_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("perm.bin");
        let _ = load_or_generate(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "permisos del keypair file deben ser 0o600 (solo dueño), got {:o}",
            mode & 0o777
        );
    }

    #[test]
    fn default_path_honors_env() {
        std::env::set_var("BRAHMAN_KEYPAIR_PATH", "/custom/path.bin");
        assert_eq!(default_path(false), PathBuf::from("/custom/path.bin"));
        assert_eq!(default_path(true), PathBuf::from("/custom/path.bin"));
        std::env::remove_var("BRAHMAN_KEYPAIR_PATH");
    }
}
