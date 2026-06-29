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

/// Path absoluto bajo `/sys/fs/cgroup` de un cgroup declarado (mismo
/// resolución que `CgroupSpec.path`: relativo → bajo el cgroup actual).
/// Error si el path declarado es vacío.
fn cgroup_abs(path: &str) -> Result<PathBuf, IncarnateError> {
    let rel = resolve_cgroup_path(path);
    if rel.is_empty() {
        return Err(IncarnateError::CgroupNotWritable { path: PathBuf::from("(empty)") });
    }
    Ok(PathBuf::from(format!("/sys/fs/cgroup{rel}")))
}

/// Escribe un archivo de control de un cgroup, mapeando los errores típicos
/// (sin permiso / inexistente) a `CgroupNotWritable` para que el caller
/// pueda distinguir "cgroup no delegado" de un IO genérico.
fn write_cgroup_file(file: &Path, content: &str) -> Result<(), IncarnateError> {
    std::fs::write(file, content).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound => {
            IncarnateError::CgroupNotWritable { path: file.to_path_buf() }
        }
        _ => IncarnateError::Io(e),
    })
}

/// Reescribe `cpu.weight` de un cgroup **ya existente** dado su dir absoluto.
/// Es el reweight en caliente: deprioritizar/priorizar todo un subárbol (el
/// slice de un contexto `pacha`) sin reencarnar nada. Rango cgroup v2:
/// 1..=10000 (100 = neutro).
pub fn set_cpu_weight_at(cgroup_abs: &Path, weight: u32) -> Result<(), IncarnateError> {
    write_cgroup_file(&cgroup_abs.join("cpu.weight"), &format!("{weight}\n"))
}

/// Congela (`true`) o descongela (`false`) un cgroup vía el freezer v2
/// (`cgroup.freeze`), dado su dir absoluto. Es **jerárquico**: gobierna todo
/// el subárbol → equivale a un SIGSTOP de grupo conservando la RAM.
pub fn set_frozen_at(cgroup_abs: &Path, frozen: bool) -> Result<(), IncarnateError> {
    write_cgroup_file(&cgroup_abs.join("cgroup.freeze"), freeze_value(frozen))
}

/// El valor que `cgroup.freeze` espera: `"1\n"` congela, `"0\n"` descongela.
pub fn freeze_value(frozen: bool) -> &'static str {
    if frozen { "1\n" } else { "0\n" }
}

/// Reweight en caliente por path declarado (`CgroupSpec.path`-style). Resuelve
/// y delega en [`set_cpu_weight_at`].
pub fn set_cpu_weight(path: &str, weight: u32) -> Result<(), IncarnateError> {
    set_cpu_weight_at(&cgroup_abs(path)?, weight)
}

/// Freeze/unfreeze por path declarado. Resuelve y delega en [`set_frozen_at`].
pub fn set_frozen(path: &str, frozen: bool) -> Result<(), IncarnateError> {
    set_frozen_at(&cgroup_abs(path)?, frozen)
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

    #[test]
    fn freeze_value_es_1_o_0() {
        assert_eq!(freeze_value(true), "1\n");
        assert_eq!(freeze_value(false), "0\n");
    }

    #[test]
    fn set_cpu_weight_at_escribe_el_archivo() {
        let dir = std::env::temp_dir().join(format!("pacha-cg-w-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        set_cpu_weight_at(&dir, 4321).unwrap();
        let got = std::fs::read_to_string(dir.join("cpu.weight")).unwrap();
        assert_eq!(got, "4321\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_frozen_at_alterna_freeze() {
        let dir = std::env::temp_dir().join(format!("pacha-cg-f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        set_frozen_at(&dir, true).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("cgroup.freeze")).unwrap(), "1\n");
        set_frozen_at(&dir, false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("cgroup.freeze")).unwrap(), "0\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_a_path_inexistente_es_cgroup_not_writable() {
        let bogus = Path::new("/sys/fs/cgroup/__pacha_no_existe__/cpu.weight");
        let err = write_cgroup_file(bogus, "100\n").unwrap_err();
        assert!(matches!(err, IncarnateError::CgroupNotWritable { .. }));
    }
}
