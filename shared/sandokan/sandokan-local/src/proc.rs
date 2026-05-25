//! Lectura de `/proc/<pid>/` para telemetría. Sin invocar binarios
//! externos (`ps`, `free`): syscalls + lectura directa de procfs.

/// RSS en bytes desde `/proc/<pid>/status` (línea `VmRSS:`).
/// Devuelve 0 si el proceso ya no existe o la línea falta.
pub fn read_mem_bytes(pid: i32) -> u64 {
    let path = format!("/proc/{pid}/status");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 0;
    };
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            // format: "VmRSS:\t   1234 kB"
            let kb: u64 = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            return kb * 1024;
        }
    }
    0
}

/// Número de threads desde `/proc/<pid>/status` (línea `Threads:`).
/// Devuelve 0 si el proceso ya no existe.
pub fn read_thread_count(pid: i32) -> u32 {
    let path = format!("/proc/{pid}/status");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 0;
    };
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Threads:") {
            return rest.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// `true` si `/proc/<pid>` existe (el proceso, vivo o zombie, está presente).
pub fn proc_exists(pid: i32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_has_positive_rss_and_threads() {
        let me = std::process::id() as i32;
        assert!(read_mem_bytes(me) > 0, "el propio proceso debe tener RSS");
        assert!(read_thread_count(me) >= 1);
        assert!(proc_exists(me));
    }

    #[test]
    fn nonexistent_pid_is_zero() {
        // PID improbable de existir.
        assert_eq!(read_mem_bytes(2_000_000_000), 0);
        assert!(!proc_exists(2_000_000_000));
    }
}
