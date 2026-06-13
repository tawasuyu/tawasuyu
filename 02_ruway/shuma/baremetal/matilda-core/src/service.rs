//! `Service` — la especificación declarativa de un servicio systemd.
//!
//! Como el resto del core, es sólo el *deseo*: qué unidad debe estar
//! habilitada (arrancar al boot) y/o activa (corriendo ahora). Ejecutar
//! `systemctl` es trabajo de capas superiores; aquí el servicio es un dato
//! comparable (`PartialEq`) para que el plan detecte cambios.

use serde::{Deserialize, Serialize};

/// El estado deseado de un servicio systemd administrado por matilda.
/// Clave única: `unit` (`sshd.service`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// Nombre de la unidad — `sshd.service`. Se normaliza con sufijo
    /// `.service` si no trae un sufijo de tipo systemd.
    pub unit: String,
    /// Debe arrancar en el boot (`systemctl enable`).
    pub enabled: bool,
    /// Debe estar corriendo ahora (`systemctl start`).
    pub active: bool,
}

impl Service {
    /// Servicio mínimo: habilitado **y** activo (el caso normal — "que
    /// este servicio esté prendido y arranque solo").
    pub fn new(unit: impl Into<String>) -> Self {
        Self {
            unit: normalize_unit(unit.into()),
            enabled: true,
            active: true,
        }
    }

    /// Fija si debe arrancar al boot (encadenable).
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Fija si debe estar corriendo ahora (encadenable).
    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

/// Agrega `.service` si la unidad no trae ya un sufijo de tipo systemd —
/// así `Service::new("sshd")` y `Service::new("sshd.service")` son lo mismo.
fn normalize_unit(unit: String) -> String {
    const SUFFIXES: [&str; 7] = [
        ".service", ".socket", ".timer", ".target", ".mount", ".path", ".slice",
    ];
    if SUFFIXES.iter().any(|s| unit.ends_with(s)) {
        unit
    } else {
        format!("{unit}.service")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_normaliza_la_unidad_y_default_on() {
        let s = Service::new("sshd");
        assert_eq!(s.unit, "sshd.service");
        assert!(s.enabled && s.active);
        // Una unidad con sufijo explícito se respeta.
        assert_eq!(Service::new("redis.socket").unit, "redis.socket");
    }

    #[test]
    fn builders_y_equality() {
        let a = Service::new("nginx").with_active(false);
        let b = Service::new("nginx.service").with_active(false);
        assert_eq!(a, b);
        assert!(!a.active && a.enabled);
    }
}
