//! Resolución y creación de cgroups v2 para el hijo.

use crate::error::IncarnateError;
use card_core::{CgroupSpec, ResourceLimits};
use std::path::{Path, PathBuf};

/// Cgroup actual del proceso que llama. Lo usamos como prefijo para paths
/// declarados relativos en `CgroupSpec.path`.
pub fn current_cgroup() -> Option<String> {
    let s = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|s| s.trim().to_string())
}

/// Resuelve un path declarado contra la jerarquía real.
pub fn resolve_cgroup_path(spec_path: &str) -> String {
    if spec_path.is_empty() {
        return String::new();
    }
    if spec_path.starts_with('/') {
        return spec_path.to_string();
    }
    let trimmed = spec_path.trim_start_matches('/');
    if let Some(cg) = current_cgroup() {
        let base = if cg == "/" {
            String::new()
        } else {
            cg.trim_end_matches('/').to_string()
        };
        format!("{base}/{trimmed}")
    } else {
        format!("/{trimmed}")
    }
}

/// Crea el cgroup declarado y aplica weights. Devuelve el path absoluto
/// resultante bajo `/sys/fs/cgroup`.
pub fn ensure_cgroup(spec: &CgroupSpec) -> Result<PathBuf, IncarnateError> {
    let rel = resolve_cgroup_path(&spec.path);
    if rel.is_empty() {
        return Err(IncarnateError::CgroupNotWritable {
            path: PathBuf::from("(empty)"),
        });
    }
    let abs = PathBuf::from(format!("/sys/fs/cgroup{}", rel));
    std::fs::create_dir_all(&abs).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => IncarnateError::CgroupNotWritable { path: abs.clone() },
        _ => IncarnateError::Io(e),
    })?;
    if let Some(w) = spec.cpu_weight {
        let _ = std::fs::write(abs.join("cpu.weight"), format!("{w}\n"));
    }
    if let Some(w) = spec.io_weight {
        // io.weight requiere "default <n>" en cgroup v2.
        let _ = std::fs::write(abs.join("io.weight"), format!("default {w}\n"));
    }
    Ok(abs)
}

/// Escribe `memory.max` y `pids.max` al cgroup según `rlimits`. Falla
/// silenciosamente si los archivos no son escribibles (cgroup no
/// delegated). El kernel hace OOM kill cuando `memory.max` se excede,
/// y bloquea forks cuando `pids.max` se alcanza.
///
/// `memory.max` acepta `max` o un número en bytes. `pids.max` igual.
pub fn apply_rlimits_to_cgroup(cgroup_abs: &Path, rlimits: &ResourceLimits) -> Vec<String> {
    let mut applied = Vec::new();
    if let Some(mem) = rlimits.mem_bytes {
        let path = cgroup_abs.join("memory.max");
        match std::fs::write(&path, format!("{mem}\n")) {
            Ok(_) => applied.push(format!("memory.max={mem}")),
            Err(e) => tracing::warn!(?e, path = %path.display(), "memory.max write failed"),
        }
    }
    if let Some(np) = rlimits.nproc {
        let path = cgroup_abs.join("pids.max");
        match std::fs::write(&path, format!("{np}\n")) {
            Ok(_) => applied.push(format!("pids.max={np}")),
            Err(e) => tracing::warn!(?e, path = %path.display(), "pids.max write failed"),
        }
    }
    applied
}

/// Mueve `pid` a `cgroup_abs/cgroup.procs`.
pub fn move_to_cgroup(cgroup_abs: &Path, pid: nix::unistd::Pid) -> Result<(), IncarnateError> {
    let procs = cgroup_abs.join("cgroup.procs");
    std::fs::write(&procs, format!("{}\n", pid.as_raw())).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => IncarnateError::CgroupNotWritable {
            path: procs.clone(),
        },
        _ => IncarnateError::Io(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_passthrough() {
        assert_eq!(resolve_cgroup_path("/foo/bar"), "/foo/bar");
    }

    #[test]
    fn empty_returns_empty() {
        assert_eq!(resolve_cgroup_path(""), "");
    }

    #[test]
    fn relative_path_prefixed() {
        let r = resolve_cgroup_path("shuma/ws-1");
        assert!(r.ends_with("/shuma/ws-1") || r == "/shuma/ws-1");
    }
}
