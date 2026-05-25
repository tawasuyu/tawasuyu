//! Resource accounting por workspace.
//!
//! Dos fuentes:
//! - **Per-proc** (`/proc/<pid>/status` + `stat`): suma RSS y CPU ticks de
//!   los comandos vivos del workspace. Siempre disponible. Costo: O(N pids).
//! - **Cgroup v2** (`memory.current`, `cpu.stat`): un read por workspace si
//!   `SomaSpec.cgroup.path` está y es leíble. Más preciso (incluye descendants).
//!
//! Si ambos están disponibles, devolvemos el cgroup (más preciso) y dejamos
//! el per-proc como `sample_via_proc`.

use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct WorkspaceStats {
    pub commands_alive: u32,
    pub commands_total: u32,
    /// RSS sumado en bytes. `None` si no se pudo medir.
    pub rss_bytes: Option<u64>,
    /// High-water mark de RSS (peak alguna vez observado). Cgroup v2:
    /// `memory.peak` (≥6.5). Per-proc: suma de `VmHWM` de cada pid.
    pub rss_peak_bytes: Option<u64>,
    /// Tiempo CPU acumulado en microsegundos. `None` si no se pudo medir.
    pub cpu_usec: Option<u64>,
    /// %CPU instantáneo derivado entre dos samples consecutivos. `None`
    /// en el primer sample (no hay baseline). `100.0` = 1 core saturado.
    /// `400.0` con 4 cores activos = la máquina al 100%.
    pub cpu_percent: Option<f32>,
    /// Cores online detectados (sysconf `_SC_NPROCESSORS_ONLN`). Útil
    /// para normalizar `cpu_percent / cpu_cores` → 0..100 absoluto.
    pub cpu_cores: u32,
    /// Fuente del dato: "proc" | "cgroup" | "mixed".
    pub source: String,
    /// Wall-clock uptime del workspace en milisegundos.
    pub uptime_ms: u64,
}

impl WorkspaceStats {
    /// CPU% normalizado al 100% total de la máquina (no por core).
    /// Útil para comparar workspaces independiente del paralelismo.
    pub fn cpu_percent_total(&self) -> Option<f32> {
        self.cpu_percent
            .map(|p| if self.cpu_cores == 0 { p } else { p / self.cpu_cores as f32 })
    }
}

/// Reporte de quotas: comparación entre el accounting real y los
/// `rlimits` declarados en `SomaSpec`. NO hace enforcement automático
/// en v1 — sólo accounting + reporting. El caller decide qué hacer.
#[derive(Debug, Clone, Default)]
pub struct QuotaReport {
    /// Límite de memoria declarado (bytes). None = sin límite.
    pub mem_limit: Option<u64>,
    /// Límite de procesos declarado.
    pub nproc_limit: Option<u32>,
    /// Lista de violaciones detectadas (strings humano-legibles).
    /// Empty = todo dentro de quota.
    pub breaches: Vec<String>,
}

/// Detecta cores online runtime. Cacheado vía OnceLock — el valor no
/// cambia salvo hotplug, que es raro y aceptamos sample stale.
fn online_cores() -> u32 {
    static CACHED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
        if n > 0 { n as u32 } else { 1 }
    })
}

/// Mide stats para un set de PIDs vivos + un path de cgroup opcional.
pub fn measure(
    alive_pids: &[i32],
    cgroup_path: Option<&Path>,
    workspace_started: Instant,
) -> WorkspaceStats {
    let mut rss_proc: u64 = 0;
    let mut rss_peak_proc: u64 = 0;
    let mut cpu_proc: u64 = 0;
    let mut proc_ok = false;
    for &pid in alive_pids {
        if let Some((rss, peak, cpu)) = read_proc_pid(pid) {
            rss_proc += rss;
            rss_peak_proc += peak;
            cpu_proc += cpu;
            proc_ok = true;
        }
    }

    let cgroup = cgroup_path.and_then(read_cgroup_stats);

    let (rss, rss_peak, cpu, source) = match (cgroup, proc_ok) {
        (Some(cg), _) => (Some(cg.rss), cg.rss_peak, Some(cg.cpu_usec), "cgroup".to_string()),
        (None, true) => (
            Some(rss_proc),
            Some(rss_peak_proc),
            Some(cpu_proc),
            "proc".to_string(),
        ),
        (None, false) => (None, None, None, "none".to_string()),
    };

    WorkspaceStats {
        commands_alive: alive_pids.len() as u32,
        commands_total: 0,
        rss_bytes: rss,
        rss_peak_bytes: rss_peak,
        cpu_usec: cpu,
        cpu_percent: None, // El caller lo rellena con el diff vs prev sample.
        cpu_cores: online_cores(),
        source,
        uptime_ms: workspace_started.elapsed().as_millis() as u64,
    }
}

struct CgroupStats {
    rss: u64,
    rss_peak: Option<u64>,
    cpu_usec: u64,
}

/// Lee `(rss_bytes, rss_peak_bytes, cpu_usec)` de `/proc/<pid>/`. None si el proc desapareció.
fn read_proc_pid(pid: i32) -> Option<(u64, u64, u64)> {
    let (rss_kb, hwm_kb) = {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let mut rss = 0u64;
        let mut hwm = 0u64;
        for l in status.lines() {
            if let Some(rest) = l.strip_prefix("VmRSS:") {
                rss = rest
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            } else if let Some(rest) = l.strip_prefix("VmHWM:") {
                hwm = rest
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
        }
        (rss, hwm)
    };
    let cpu_usec = {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // format: pid (comm) state ppid pgrp ... utime stime cutime cstime
        // Cuidado: comm puede tener espacios y paréntesis. Buscamos la última `)`.
        let end_comm = stat.rfind(')')?;
        let after = &stat[end_comm + 1..];
        let fields: Vec<&str> = after.split_whitespace().collect();
        // Tras `)`, índice 0 = state, índice 11 = utime, 12 = stime.
        let utime = fields.get(11).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let stime = fields.get(12).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let ticks = utime + stime;
        // Convertimos ticks → microsegundos. SC_CLK_TCK típicamente 100.
        let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
        ticks * 1_000_000 / clk_tck
    };
    Some((rss_kb * 1024, hwm_kb * 1024, cpu_usec))
}

/// Lee `CgroupStats` del cgroup. None si no existe o no es leíble.
/// `memory.peak` requiere kernel ≥6.5; si falta, `rss_peak` queda None.
fn read_cgroup_stats(cgroup_path: &Path) -> Option<CgroupStats> {
    let mem = std::fs::read_to_string(cgroup_path.join("memory.current"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())?;
    let cpu_stat = std::fs::read_to_string(cgroup_path.join("cpu.stat")).ok()?;
    let cpu_usec = cpu_stat
        .lines()
        .find_map(|l| l.strip_prefix("usage_usec"))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let peak = std::fs::read_to_string(cgroup_path.join("memory.peak"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    Some(CgroupStats {
        rss: mem,
        rss_peak: peak,
        cpu_usec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_with_no_pids_returns_zero() {
        let stats = measure(&[], None, Instant::now());
        assert_eq!(stats.commands_alive, 0);
        assert_eq!(stats.rss_bytes, None);
        assert_eq!(stats.source, "none");
    }

    #[test]
    fn measure_self_pid_returns_data() {
        let me = std::process::id() as i32;
        let stats = measure(&[me], None, Instant::now());
        assert_eq!(stats.commands_alive, 1);
        // Nuestro propio RSS debería ser > 0.
        assert!(stats.rss_bytes.unwrap_or(0) > 0);
        assert_eq!(stats.source, "proc");
    }
}
