//! energia — juicio de sólo-lectura sobre si conviene **suspender** el sistema.
//!
//! El plano de control sabe qué unidades corren y cuánta CPU queman. Antes de
//! que el escritorio suspenda (o apague) por inactividad, alguien tiene que
//! poder decir «esperá, hay trabajo importante en curso» — sin los workarounds
//! toscos tipo `caffeine`. Esta es esa voz, del lado del control: **pura**,
//! sobre el [`MonitorSnapshot`](crate::MonitorSnapshot) que el monitor ya
//! produce. No mira `/proc` ni el reloj; el coordinador (pata) le pasa el
//! snapshot y combina este veredicto con su propia señal de sistema.

use crate::MonitorSnapshot;
use sandokan_lifecycle::LifecycleState;

/// Política de veto a la suspensión: qué cuenta como «ocupado».
#[derive(Debug, Clone)]
pub struct PoliticaVeto {
    /// Una unidad corriendo a ≥ este %CPU veta la suspensión (100.0 = 1 core
    /// saturado, igual que [`TelemetryFrame::cpu_pct`](sandokan_core::TelemetryFrame)).
    pub cpu_ocupada_pct: f64,
    /// Subcadenas de label cuyas unidades, si corren, vetan **siempre** — el
    /// equivalente declarativo de «mantener despierto»: backups, descargas,
    /// transcodificación… que importan aunque no quemen CPU en el instante
    /// medido (esperan red/disco). Vacío = sin keep-awake por label.
    pub etiquetas_despiertas: Vec<String>,
}

impl Default for PoliticaVeto {
    fn default() -> Self {
        // 25% de un core: una unidad realmente trabajando, sin disparar por el
        // ruido de un daemon que hace tic-tac.
        Self {
            cpu_ocupada_pct: 25.0,
            etiquetas_despiertas: Vec::new(),
        }
    }
}

/// Por qué una unidad bloquea la suspensión (para mostrarle al usuario el
/// motivo en vez de cortarle el trabajo en silencio).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bloqueo {
    pub unidad: String,
    pub razon: String,
}

/// Veredicto del plano de control: ¿se puede suspender? Si no, quién lo impide.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VeredictoSuspension {
    /// `true` si ninguna unidad viva veta la suspensión.
    pub permite: bool,
    /// Las unidades que la bloquean (vacío si `permite`).
    pub bloqueos: Vec<Bloqueo>,
}

/// Evalúa el snapshot contra la política. Sólo pesan unidades **corriendo**:
/// una unidad parada/aparcada/terminada no defiende su sueño.
pub fn evaluar(snap: &MonitorSnapshot, pol: &PoliticaVeto) -> VeredictoSuspension {
    let mut bloqueos = Vec::new();
    for u in &snap.units {
        if !matches!(u.state, LifecycleState::Running) {
            continue;
        }
        // Keep-awake declarativo por label: gana aunque la CPU esté tranquila.
        if pol
            .etiquetas_despiertas
            .iter()
            .any(|e| !e.is_empty() && u.label.contains(e.as_str()))
        {
            bloqueos.push(Bloqueo {
                unidad: u.label.clone(),
                razon: "marcada para mantenerse despierta".into(),
            });
            continue;
        }
        // Ocupación medida por CPU.
        if let Some(t) = &u.telemetry {
            if t.cpu_pct >= pol.cpu_ocupada_pct {
                bloqueos.push(Bloqueo {
                    unidad: u.label.clone(),
                    razon: format!("ocupada ({:.0}% CPU)", t.cpu_pct),
                });
            }
        }
    }
    VeredictoSuspension {
        permite: bloqueos.is_empty(),
        bloqueos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UnitObservation;
    use sandokan_core::TelemetryFrame;
    use std::time::SystemTime;
    use ulid::Ulid;

    fn unidad(label: &str, state: LifecycleState, cpu: Option<f64>) -> UnitObservation {
        let card_id = Ulid::new();
        UnitObservation {
            card_id,
            label: label.into(),
            state,
            telemetry: cpu.map(|cpu_pct| TelemetryFrame {
                card_id,
                at: SystemTime::UNIX_EPOCH,
                mem_bytes: 1024,
                nproc: 1,
                cpu_pct,
                restarts: 0,
            }),
            restarts: 0,
        }
    }

    fn snap(units: Vec<UnitObservation>) -> MonitorSnapshot {
        MonitorSnapshot { units }
    }

    #[test]
    fn sistema_ocioso_permite_suspender() {
        let s = snap(vec![
            unidad("paloma", LifecycleState::Running, Some(0.5)),
            unidad("reloj", LifecycleState::Running, Some(2.0)),
        ]);
        let v = evaluar(&s, &PoliticaVeto::default());
        assert!(v.permite);
        assert!(v.bloqueos.is_empty());
    }

    #[test]
    fn unidad_ocupada_veta_con_motivo() {
        let s = snap(vec![
            unidad("reloj", LifecycleState::Running, Some(1.0)),
            unidad("compilar", LifecycleState::Running, Some(180.0)),
        ]);
        let v = evaluar(&s, &PoliticaVeto::default());
        assert!(!v.permite);
        assert_eq!(v.bloqueos.len(), 1);
        assert_eq!(v.bloqueos[0].unidad, "compilar");
        assert!(v.bloqueos[0].razon.contains("CPU"));
    }

    #[test]
    fn keep_awake_por_label_veta_aunque_cpu_baja() {
        let pol = PoliticaVeto {
            etiquetas_despiertas: vec!["backup".into()],
            ..Default::default()
        };
        let s = snap(vec![unidad("backup-nocturno", LifecycleState::Running, Some(0.1))]);
        let v = evaluar(&s, &pol);
        assert!(!v.permite);
        assert_eq!(v.bloqueos[0].unidad, "backup-nocturno");
        assert!(v.bloqueos[0].razon.contains("despierta"));
    }

    #[test]
    fn unidad_ocupada_pero_no_corriendo_no_veta() {
        // Una unidad que ya salió no defiende su sueño aunque su última
        // telemetría fuera alta.
        let s = snap(vec![unidad(
            "compilar",
            LifecycleState::Exited { code: 0 },
            Some(200.0),
        )]);
        let v = evaluar(&s, &PoliticaVeto::default());
        assert!(v.permite);
    }

    #[test]
    fn sin_telemetria_no_se_asume_ocupada() {
        // Si el Engine no pudo dar telemetría, no inventamos ocupación (pero
        // tampoco bloquea: degrada a «permite» por esa unidad).
        let s = snap(vec![unidad("misteriosa", LifecycleState::Running, None)]);
        let v = evaluar(&s, &PoliticaVeto::default());
        assert!(v.permite);
    }
}
