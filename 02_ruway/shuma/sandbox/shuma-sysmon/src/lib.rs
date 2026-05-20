//! `shuma-sysmon` — muestreo de CPU y memoria para los monitores del shell.
//!
//! Lee `/proc/stat` y `/proc/meminfo`, calcula el porcentaje de uso de
//! CPU (delta entre dos muestras) y de memoria, y mantiene un historial
//! corto para dibujar la curva del monitor.
//!
//! El parseo de `/proc` está separado del cálculo: las funciones puras
//! [`parse_cpu_stat`] y [`parse_meminfo`] se prueban con texto fijo, y
//! [`SystemSampler::sample`] sólo añade la lectura de archivos. Así la
//! lógica es testeable sin depender del sistema y el crate es agnóstico
//! de cualquier frontend.

#![forbid(unsafe_code)]

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Acumuladores de CPU de `/proc/stat` — tiempo ocupado y total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuStat {
    pub busy: u64,
    pub total: u64,
}

/// Memoria de `/proc/meminfo`, en kibibytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemStat {
    pub total_kb: u64,
    pub available_kb: u64,
}

/// Parsea la línea agregada `cpu` de `/proc/stat`. El uso instantáneo no
/// se puede sacar de una sola muestra — hace falta el delta entre dos.
pub fn parse_cpu_stat(text: &str) -> Option<CpuStat> {
    let line = text.lines().find(|l| {
        l.starts_with("cpu") && l[3..].starts_with(char::is_whitespace)
    })?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1) // la etiqueta "cpu"
        .filter_map(|f| f.parse().ok())
        .collect();
    if fields.len() < 4 {
        return None;
    }
    // Campos: user nice system idle iowait irq softirq steal …
    let total: u64 = fields.iter().sum();
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0); // idle + iowait
    Some(CpuStat { busy: total.saturating_sub(idle), total })
}

/// Parsea `MemTotal` y `MemAvailable` de `/proc/meminfo`.
pub fn parse_meminfo(text: &str) -> Option<MemStat> {
    let field = |key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    Some(MemStat {
        total_kb: field("MemTotal:")?,
        available_kb: field("MemAvailable:")?,
    })
}

/// Un historial circular de valores `f32` para dibujar una curva.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    samples: VecDeque<f32>,
    capacity: usize,
}

impl History {
    /// Historial vacío con capacidad para `capacity` muestras.
    pub fn new(capacity: usize) -> Self {
        Self { samples: VecDeque::with_capacity(capacity.max(1)), capacity: capacity.max(1) }
    }

    /// Añade una muestra; descarta la más antigua si se llena.
    pub fn push(&mut self, value: f32) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    /// Muestras de la más antigua a la más reciente.
    pub fn values(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }

    /// Muestra más reciente.
    pub fn last(&self) -> Option<f32> {
        self.samples.back().copied()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Una lectura del estado del sistema en un instante.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Uso de CPU, `0.0..=100.0`.
    pub cpu_percent: f32,
    /// Uso de memoria, `0.0..=100.0`.
    pub mem_percent: f32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    /// `false` si no se pudo leer `/proc` (p. ej. fuera de Linux).
    pub valid: bool,
}

impl Snapshot {
    fn invalid() -> Self {
        Self { cpu_percent: 0.0, mem_percent: 0.0, mem_used_mb: 0, mem_total_mb: 0, valid: false }
    }
}

/// Muestreador del sistema: guarda la muestra de CPU anterior (para el
/// delta) y el historial de ambas curvas.
#[derive(Debug, Clone)]
pub struct SystemSampler {
    prev_cpu: Option<CpuStat>,
    cpu_history: History,
    mem_history: History,
}

impl SystemSampler {
    /// Crea un muestreador cuyas curvas guardan `history` muestras.
    pub fn new(history: usize) -> Self {
        Self {
            prev_cpu: None,
            cpu_history: History::new(history),
            mem_history: History::new(history),
        }
    }

    /// Calcula un `Snapshot` a partir del texto de `/proc/stat` y
    /// `/proc/meminfo`. Es la parte pura — `sample` sólo le añade la
    /// lectura de archivos.
    pub fn sample_from(&mut self, stat_text: &str, meminfo_text: &str) -> Snapshot {
        let (Some(cpu), Some(mem)) =
            (parse_cpu_stat(stat_text), parse_meminfo(meminfo_text))
        else {
            return Snapshot::invalid();
        };

        // El uso de CPU es el delta de ocupación entre dos muestras.
        let cpu_percent = match self.prev_cpu {
            Some(prev) => {
                let total_delta = cpu.total.saturating_sub(prev.total);
                let busy_delta = cpu.busy.saturating_sub(prev.busy);
                if total_delta == 0 {
                    self.cpu_history.last().unwrap_or(0.0)
                } else {
                    (busy_delta as f32 / total_delta as f32 * 100.0).clamp(0.0, 100.0)
                }
            }
            None => 0.0, // primera muestra: aún no hay delta
        };
        self.prev_cpu = Some(cpu);

        let used_kb = mem.total_kb.saturating_sub(mem.available_kb);
        let mem_percent = if mem.total_kb == 0 {
            0.0
        } else {
            (used_kb as f32 / mem.total_kb as f32 * 100.0).clamp(0.0, 100.0)
        };

        self.cpu_history.push(cpu_percent);
        self.mem_history.push(mem_percent);

        Snapshot {
            cpu_percent,
            mem_percent,
            mem_used_mb: used_kb / 1024,
            mem_total_mb: mem.total_kb / 1024,
            valid: true,
        }
    }

    /// Lee `/proc` y produce un `Snapshot`. Fuera de Linux, o si `/proc`
    /// no está disponible, devuelve un snapshot `valid: false`.
    pub fn sample(&mut self) -> Snapshot {
        let stat = std::fs::read_to_string("/proc/stat");
        let meminfo = std::fs::read_to_string("/proc/meminfo");
        match (stat, meminfo) {
            (Ok(s), Ok(m)) => self.sample_from(&s, &m),
            _ => Snapshot::invalid(),
        }
    }

    /// Historial de uso de CPU (curva del monitor).
    pub fn cpu_history(&self) -> &History {
        &self.cpu_history
    }

    /// Historial de uso de memoria.
    pub fn mem_history(&self) -> &History {
        &self.mem_history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT_1: &str = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0\n";
    // 100 jiffies de ocupación más que STAT_1, 100 de inactividad más.
    const STAT_2: &str = "cpu  150 0 100 850 100 0 0 0 0 0\ncpu0 75 0 50 425 50 0 0\n";
    const MEMINFO: &str =
        "MemTotal:       16000000 kB\nMemFree: 2000000 kB\nMemAvailable:    4000000 kB\n";

    #[test]
    fn parses_cpu_aggregate_line() {
        let c = parse_cpu_stat(STAT_1).unwrap();
        // total = 100+0+50+800+50 = 1000; idle = 800+50 = 850; busy = 150.
        assert_eq!(c.total, 1000);
        assert_eq!(c.busy, 150);
    }

    #[test]
    fn parses_meminfo() {
        let m = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(m.total_kb, 16_000_000);
        assert_eq!(m.available_kb, 4_000_000);
    }

    #[test]
    fn rejects_malformed_proc_text() {
        assert!(parse_cpu_stat("garbage").is_none());
        assert!(parse_meminfo("MemTotal: only").is_none());
    }

    #[test]
    fn first_sample_has_zero_cpu_then_delta() {
        let mut s = SystemSampler::new(60);
        let first = s.sample_from(STAT_1, MEMINFO);
        assert_eq!(first.cpu_percent, 0.0); // sin muestra previa
        assert!(first.valid);

        let second = s.sample_from(STAT_2, MEMINFO);
        // total_delta: 1200-1000=200; busy_delta: STAT_2 busy = 150+0+100=250
        // total2 = 150+0+100+850+100 = 1200; idle2 = 950; busy2 = 250.
        // busy_delta = 250-150 = 100; cpu% = 100/200 = 50%.
        assert!((second.cpu_percent - 50.0).abs() < 0.01);
    }

    #[test]
    fn memory_percent_uses_available() {
        let mut s = SystemSampler::new(60);
        let snap = s.sample_from(STAT_1, MEMINFO);
        // used = 16000000 - 4000000 = 12000000 kB → 75%.
        assert!((snap.mem_percent - 75.0).abs() < 0.01);
        assert_eq!(snap.mem_total_mb, 16_000_000 / 1024);
    }

    #[test]
    fn invalid_proc_yields_invalid_snapshot() {
        let mut s = SystemSampler::new(60);
        let snap = s.sample_from("nonsense", "nonsense");
        assert!(!snap.valid);
    }

    #[test]
    fn history_fills_both_curves() {
        let mut s = SystemSampler::new(60);
        s.sample_from(STAT_1, MEMINFO);
        s.sample_from(STAT_2, MEMINFO);
        assert_eq!(s.cpu_history().len(), 2);
        assert_eq!(s.mem_history().len(), 2);
    }

    #[test]
    fn history_is_a_bounded_ring() {
        let mut h = History::new(3);
        for v in [1.0, 2.0, 3.0, 4.0, 5.0] {
            h.push(v);
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.values(), vec![3.0, 4.0, 5.0]); // las 3 más recientes
        assert_eq!(h.last(), Some(5.0));
    }
}
