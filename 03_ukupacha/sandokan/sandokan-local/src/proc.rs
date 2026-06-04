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

/// Ticks de CPU consumidos por el proceso desde su nacimiento: suma de
/// `utime` (user) + `stime` (kernel) leídos de `/proc/<pid>/stat`. Es la
/// magnitud que `telemetry()` compara entre dos samples espaciados para
/// obtener `cpu_pct`. `None` si el proceso ya no existe o el parse falla
/// (un PID transitorio que muere entre el `read_to_string` y el siguiente
/// snapshot vale 0 también — el caller debe interpretar `None` como
/// "no medible ahora", no como bug).
pub fn read_cpu_ticks(pid: i32) -> Option<u64> {
    let path = format!("/proc/{pid}/stat");
    let content = std::fs::read_to_string(&path).ok()?;
    // Formato: "pid (comm) state ppid pgrp ..."  — comm puede contener
    // espacios y paréntesis, así que partimos en el último `)` y
    // procesamos los campos POSTERIORES por posición.
    let close_paren = content.rfind(')')?;
    let after = content[close_paren + 1..].trim_start();
    let mut it = after.split_ascii_whitespace();
    // Tras el `)` los campos están 1-indexed empezando en `state` = 3.
    // utime es el campo 14 → posición (14 - 3) = 11; stime, 15 → 12.
    let utime: u64 = it.nth(11)?.parse().ok()?;
    let stime: u64 = it.next()?.parse().ok()?;
    Some(utime + stime)
}

/// Frecuencia del reloj de procesos (`USER_HZ` / `CLK_TCK`). Casi
/// universalmente 100 en Linux, pero lo consultamos a `sysconf` para
/// portabilidad y para descartar configuraciones no estándar.
pub fn clock_ticks_per_second() -> u64 {
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 {
        v as u64
    } else {
        100
    }
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
