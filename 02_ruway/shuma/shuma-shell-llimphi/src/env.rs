//! Detección de engines disponibles, source por defecto y path de askpass.
//!
//! Este módulo no depende de ningún tipo del chasis — puede ser importado
//! por `types`, `containers` y `persist` sin ciclos.

use shuma_module::Source;

// ─── Detección de binarios ─────────────────────────────────────────

pub(crate) fn binary_disponible(name: &str) -> bool {
    let Some(path_env) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_env) {
        if dir.join(name).exists() {
            return true;
        }
    }
    false
}

/// `true` si el binario `podman` está disponible en `PATH`.
pub(crate) fn podman_disponible() -> bool {
    binary_disponible("podman")
}

/// `true` si el binario `bwrap` (bubblewrap) está disponible en `PATH`.
pub(crate) fn bwrap_disponible() -> bool {
    binary_disponible("bwrap")
}

/// `true` si `unshare` + `chroot` están en `PATH`.
pub(crate) fn unshare_disponible() -> bool {
    binary_disponible("unshare") && binary_disponible("chroot")
}

/// Engine preferido para containers de esta máquina.
/// 1. `unshare` — sin instalar nada extra.
/// 2. `bwrap` — sin config, buen aislamiento.
/// 3. `podman` — fallback OCI completo.
pub(crate) fn engine_preferido() -> Option<&'static str> {
    if unshare_disponible() {
        Some("unshare")
    } else if bwrap_disponible() {
        Some("bwrap")
    } else if podman_disponible() {
        Some("podman")
    } else {
        None
    }
}

// ─── Source por defecto ─────────────────────────────────────────────

/// `Source` por defecto de la tab shell según las env vars del proceso.
pub(crate) fn default_shell_source() -> Source {
    let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let (Some(addr), Some(pub_hex)) = (
        nonempty("SHUMA_REMOTE_TCP_ADDR"),
        nonempty("SHUMA_REMOTE_TCP_PUB"),
    ) {
        return Source::DaemonTcp {
            addr,
            server_pub_hex: pub_hex,
            label: None,
        };
    }
    if let Some(path) = nonempty("SHUMA_REMOTE_SOCKET") {
        return Source::Daemon {
            socket: Some(std::path::PathBuf::from(path)),
            label: None,
        };
    }
    if std::env::var("SHUMA_REMOTE").as_deref() == Ok("1") {
        return Source::Daemon {
            socket: None,
            label: None,
        };
    }
    Source::Local
}

// ─── Askpass ────────────────────────────────────────────────────────

/// Resuelve el path del binario `shuma-askpass`.
pub(crate) fn resolve_askpass_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SHUMA_ASKPASS") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("shuma-askpass");
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }
    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            let cand = dir.join("shuma-askpass");
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    None
}
