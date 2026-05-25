//! Cuotas de recursos + chequeo de breaches.

use serde::{Deserialize, Serialize};

/// Acción a tomar cuando una cuota se excede.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum QuotaAction {
    /// No hacer nada (accounting puro).
    #[default]
    None,
    /// Sólo loggear el breach.
    Log,
    /// Terminar la entidad supervisada.
    Kill,
}

/// Límites declarativos de recursos. `None` = sin límite para ese recurso.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceQuota {
    pub mem_bytes: Option<u64>,
    pub nproc: Option<u32>,
    /// Porcentaje de CPU (100.0 = 1 core saturado).
    pub cpu_pct: Option<f64>,
}

/// Uso de recursos medido en un instante.
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    pub mem_bytes: u64,
    pub nproc: u32,
    pub cpu_pct: f64,
}

/// Un recurso que excedió su límite.
#[derive(Debug, Clone, PartialEq)]
pub struct Breach {
    pub resource: &'static str,
    pub used: f64,
    pub limit: f64,
}

/// Resultado de chequear `ResourceUsage` contra `ResourceQuota`.
#[derive(Debug, Clone, Default)]
pub struct QuotaReport {
    pub breaches: Vec<Breach>,
}

impl QuotaReport {
    /// `true` si no hay ningún breach.
    pub fn ok(&self) -> bool {
        self.breaches.is_empty()
    }
}

/// Compara uso contra cuota y reporta cada recurso excedido.
pub fn check_quota(usage: &ResourceUsage, quota: &ResourceQuota) -> QuotaReport {
    let mut breaches = Vec::new();
    if let Some(limit) = quota.mem_bytes {
        if usage.mem_bytes > limit {
            breaches.push(Breach {
                resource: "mem_bytes",
                used: usage.mem_bytes as f64,
                limit: limit as f64,
            });
        }
    }
    if let Some(limit) = quota.nproc {
        if usage.nproc > limit {
            breaches.push(Breach {
                resource: "nproc",
                used: usage.nproc as f64,
                limit: limit as f64,
            });
        }
    }
    if let Some(limit) = quota.cpu_pct {
        if usage.cpu_pct > limit {
            breaches.push(Breach {
                resource: "cpu_pct",
                used: usage.cpu_pct,
                limit,
            });
        }
    }
    QuotaReport { breaches }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_limits_is_ok() {
        let usage = ResourceUsage { mem_bytes: 100, nproc: 2, cpu_pct: 50.0 };
        let quota = ResourceQuota {
            mem_bytes: Some(200), nproc: Some(4), cpu_pct: Some(90.0),
        };
        assert!(check_quota(&usage, &quota).ok());
    }

    #[test]
    fn detects_mem_breach() {
        let usage = ResourceUsage { mem_bytes: 300, nproc: 1, cpu_pct: 0.0 };
        let quota = ResourceQuota { mem_bytes: Some(200), ..Default::default() };
        let report = check_quota(&usage, &quota);
        assert!(!report.ok());
        assert_eq!(report.breaches[0].resource, "mem_bytes");
    }

    #[test]
    fn no_limit_means_no_breach() {
        let usage = ResourceUsage { mem_bytes: u64::MAX, nproc: 9999, cpu_pct: 999.0 };
        assert!(check_quota(&usage, &ResourceQuota::default()).ok());
    }

    #[test]
    fn multiple_breaches_reported() {
        let usage = ResourceUsage { mem_bytes: 300, nproc: 10, cpu_pct: 200.0 };
        let quota = ResourceQuota {
            mem_bytes: Some(100), nproc: Some(2), cpu_pct: Some(100.0),
        };
        assert_eq!(check_quota(&usage, &quota).breaches.len(), 3);
    }
}
