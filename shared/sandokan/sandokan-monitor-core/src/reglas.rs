//! reglas — disparadores por **métrica** sobre el plano de control (SDD §8
//! capa 2).
//!
//! El cerebro de `arje-brain-rules` reacciona a **eventos** (nació/murió un
//! Ente, llegó un invoke). Esta es la otra mitad de la automatización: reaccionar
//! a **estado sostenido** —«chasqui lleva 30 s sobre 80 % de CPU», «hay una
//! unidad con 5 restarts»— que un stream de eventos no captura. Sigue el patrón
//! canónico de [`energia`](crate::energia): evaluación **pura** sobre el
//! [`MonitorSnapshot`](crate::MonitorSnapshot) que el monitor ya produce; ni
//! mira `/proc` ni el reloj (el caller pasa el `dt` entre polls).
//!
//! El resultado es un [`Disparo`]: qué unidad, qué regla y qué [`AccionControl`]
//! ejecutar. La acción se aplica por el **mismo contrato** que todo lo demás
//! ([`aplicar`] → `Engine::{stop,set_cpu_weight,freeze}`, los verbos que la capa
//! 1 cableó), así «lo que observás» y «lo que se hace» son la misma fuente de
//! verdad —sin abrir un canal paralelo (SDD §6/§8)—.

use crate::{MonitorSnapshot, UnitObservation};
use sandokan_core::{Engine, EngineError};
use sandokan_lifecycle::LifecycleState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use ulid::Ulid;

/// Predicado **instantáneo** sobre una unidad observada. Las condiciones de
/// métrica leen la telemetría del frame; si una unidad no tiene telemetría, las
/// de CPU/memoria dan `false` (no inventamos ocupación, igual que `energia`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condicion {
    /// `cpu_pct >= v` (100.0 = 1 core saturado).
    CpuPctMin(f64),
    /// `mem_bytes >= v` (RSS).
    MemBytesMin(u64),
    /// `restarts >= v` (el supervisor la reinició al menos `v` veces).
    RestartsMin(u32),
    /// El label de la unidad contiene esta subcadena (para acotar una regla a
    /// una familia de unidades: «las que se llaman *build*…»).
    EtiquetaContiene(String),
    /// AND: matchea si **todas** las sub-condiciones matchean. Vacío = matchea
    /// siempre (útil como «cualquier unidad corriendo»).
    Todas(Vec<Condicion>),
}

impl Condicion {
    /// `true` si la unidad satisface la condición **en este instante**.
    pub fn evalua(&self, u: &UnitObservation) -> bool {
        match self {
            Condicion::CpuPctMin(v) => u.telemetry.as_ref().is_some_and(|t| t.cpu_pct >= *v),
            Condicion::MemBytesMin(v) => u.telemetry.as_ref().is_some_and(|t| t.mem_bytes >= *v),
            Condicion::RestartsMin(v) => u.restarts >= *v,
            Condicion::EtiquetaContiene(s) => !s.is_empty() && u.label.contains(s.as_str()),
            Condicion::Todas(cs) => cs.iter().all(|c| c.evalua(u)),
        }
    }
}

/// Qué hacer cuando una regla dispara. Cada variante mapea a un verbo del
/// contrato [`Engine`] (capa 1). `Detener` apunta a la **unidad que disparó**
/// (por su `card_id`); `Priorizar`/`Congelar` nombran un cgroup explícito —el
/// slice a re-pesar/congelar, típicamente el de un contexto `pacha`— porque el
/// snapshot no carga el cgroup de cada unidad.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AccionControl {
    /// → `Engine::stop(card_id, grace)`. Detiene la unidad que disparó.
    Detener {
        #[serde(default)]
        grace_ms: u64,
    },
    /// → `Engine::set_cpu_weight(cgroup_path, weight)`.
    Priorizar { cgroup_path: String, weight: u32 },
    /// → `Engine::freeze(cgroup_path, frozen)`.
    Congelar {
        cgroup_path: String,
        #[serde(default)]
        frozen: bool,
    },
}

/// Una regla de métrica: cuando `cuando` se cumple **sostenidamente** por
/// `durante`, ejecutá `entonces`. `durante == 0` = instantánea (dispara en el
/// primer poll que la cumple).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReglaMetrica {
    pub id: String,
    pub cuando: Condicion,
    /// Tiempo que la condición debe mantenerse continua antes de disparar.
    #[serde(default)]
    pub durante: Duration,
    pub entonces: AccionControl,
}

/// Lo que una regla produjo al dispararse: la unidad culpable + la acción.
#[derive(Debug, Clone, PartialEq)]
pub struct Disparo {
    pub card_id: Ulid,
    pub label: String,
    pub regla: String,
    pub accion: AccionControl,
}

/// Racha de una (regla, unidad): cuánto lleva la condición continua y si ya
/// disparó (debounce — no re-dispara hasta que la condición caiga).
#[derive(Debug, Default, Clone, Copy)]
struct Racha {
    sostenido: Duration,
    disparado: bool,
}

/// Motor **con estado**: acumula cuánto lleva cada condición continua a lo
/// largo de polls sucesivos para soportar «sostenido por N s». El estado es
/// sólo las rachas; las reglas son inmutables tras construirlo.
#[derive(Debug, Default)]
pub struct MotorMetrico {
    reglas: Vec<ReglaMetrica>,
    rachas: HashMap<(String, Ulid), Racha>,
}

impl MotorMetrico {
    pub fn new(reglas: Vec<ReglaMetrica>) -> Self {
        Self { reglas, rachas: HashMap::new() }
    }

    /// Evalúa el snapshot, avanzando las rachas en `dt` (el tiempo desde el
    /// poll anterior). Devuelve los disparos cuya condición acaba de cruzar su
    /// `durante`. Sólo pesan unidades **corriendo** (una parada/terminada no
    /// dispara). La racha se resetea cuando la condición cae o la unidad
    /// desaparece — así el debounce no se queda pegado.
    pub fn evaluar(&mut self, snap: &MonitorSnapshot, dt: Duration) -> Vec<Disparo> {
        let mut disparos = Vec::new();
        // Qué (regla, unidad) siguen vivas-y-matcheando este poll; el resto se
        // purga al final para no acumular rachas de unidades idas.
        let mut vigentes: std::collections::HashSet<(String, Ulid)> = std::collections::HashSet::new();

        for u in &snap.units {
            if !matches!(u.state, LifecycleState::Running) {
                continue;
            }
            for r in &self.reglas {
                if !r.cuando.evalua(u) {
                    continue;
                }
                let key = (r.id.clone(), u.card_id);
                vigentes.insert(key.clone());
                let racha = self.rachas.entry(key).or_default();
                racha.sostenido = racha.sostenido.saturating_add(dt);
                if racha.sostenido >= r.durante && !racha.disparado {
                    racha.disparado = true;
                    disparos.push(Disparo {
                        card_id: u.card_id,
                        label: u.label.clone(),
                        regla: r.id.clone(),
                        accion: r.entonces.clone(),
                    });
                }
            }
        }

        // Reset: toda racha que no quedó vigente (condición cayó o unidad ida).
        self.rachas.retain(|k, _| vigentes.contains(k));
        disparos
    }
}

/// Aplica un disparo por el contrato `Engine` — el mismo que observa y controla
/// todo lo demás. Es el puente de la capa 2 a los verbos que cableó la capa 1.
pub async fn aplicar(d: &Disparo, engine: &dyn Engine) -> Result<(), EngineError> {
    match &d.accion {
        AccionControl::Detener { grace_ms } => {
            engine.stop(d.card_id, Duration::from_millis(*grace_ms)).await
        }
        AccionControl::Priorizar { cgroup_path, weight } => {
            engine.set_cpu_weight(cgroup_path.clone(), *weight).await
        }
        AccionControl::Congelar { cgroup_path, frozen } => {
            engine.freeze(cgroup_path.clone(), *frozen).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandokan_core::TelemetryFrame;
    use std::time::SystemTime;

    fn unidad(label: &str, state: LifecycleState, cpu: Option<f64>, restarts: u32) -> UnitObservation {
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
                restarts,
            }),
            restarts,
        }
    }

    fn snap(units: Vec<UnitObservation>) -> MonitorSnapshot {
        MonitorSnapshot { units }
    }

    #[test]
    fn condicion_cpu_y_etiqueta() {
        let u = unidad("build-grande", LifecycleState::Running, Some(90.0), 0);
        assert!(Condicion::CpuPctMin(80.0).evalua(&u));
        assert!(!Condicion::CpuPctMin(95.0).evalua(&u));
        assert!(Condicion::EtiquetaContiene("build".into()).evalua(&u));
        let y = Condicion::Todas(vec![
            Condicion::CpuPctMin(80.0),
            Condicion::EtiquetaContiene("build".into()),
        ]);
        assert!(y.evalua(&u));
    }

    #[test]
    fn sin_telemetria_no_dispara_cpu() {
        let u = unidad("misteriosa", LifecycleState::Running, None, 0);
        assert!(!Condicion::CpuPctMin(0.0).evalua(&u));
    }

    fn regla_cpu_sostenida(durante: Duration) -> ReglaMetrica {
        ReglaMetrica {
            id: "cpu-alta".into(),
            cuando: Condicion::CpuPctMin(80.0),
            durante,
            entonces: AccionControl::Priorizar { cgroup_path: "pacha/fondo".into(), weight: 10 },
        }
    }

    #[test]
    fn sostenido_no_dispara_antes_de_durante() {
        let mut m = MotorMetrico::new(vec![regla_cpu_sostenida(Duration::from_secs(30))]);
        let s = snap(vec![unidad("hog", LifecycleState::Running, Some(95.0), 0)]);
        // 10 s + 10 s = 20 s < 30 s → nada todavía.
        assert!(m.evaluar(&s, Duration::from_secs(10)).is_empty());
        assert!(m.evaluar(&s, Duration::from_secs(10)).is_empty());
        // +15 s = 35 s ≥ 30 s → dispara.
        let d = m.evaluar(&s, Duration::from_secs(15));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].regla, "cpu-alta");
        assert!(matches!(d[0].accion, AccionControl::Priorizar { weight: 10, .. }));
    }

    #[test]
    fn dispara_una_sola_vez_debounce() {
        let mut m = MotorMetrico::new(vec![regla_cpu_sostenida(Duration::ZERO)]);
        let s = snap(vec![unidad("hog", LifecycleState::Running, Some(95.0), 0)]);
        assert_eq!(m.evaluar(&s, Duration::from_secs(1)).len(), 1); // cruza
        assert!(m.evaluar(&s, Duration::from_secs(1)).is_empty()); // debounce
        assert!(m.evaluar(&s, Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn condicion_falsa_resetea_la_racha() {
        let mut m = MotorMetrico::new(vec![regla_cpu_sostenida(Duration::from_secs(30))]);
        let id = Ulid::new();
        let alta = MonitorSnapshot {
            units: vec![UnitObservation {
                card_id: id, label: "hog".into(), state: LifecycleState::Running,
                telemetry: Some(TelemetryFrame { card_id: id, at: SystemTime::UNIX_EPOCH, mem_bytes: 1, nproc: 1, cpu_pct: 95.0, restarts: 0 }),
                restarts: 0,
            }],
        };
        let baja = MonitorSnapshot {
            units: vec![UnitObservation {
                card_id: id, label: "hog".into(), state: LifecycleState::Running,
                telemetry: Some(TelemetryFrame { card_id: id, at: SystemTime::UNIX_EPOCH, mem_bytes: 1, nproc: 1, cpu_pct: 5.0, restarts: 0 }),
                restarts: 0,
            }],
        };
        assert!(m.evaluar(&alta, Duration::from_secs(20)).is_empty()); // 20 s
        assert!(m.evaluar(&baja, Duration::from_secs(20)).is_empty()); // cae → reset
        assert!(m.evaluar(&alta, Duration::from_secs(20)).is_empty()); // 20 s de nuevo, no 40
        let d = m.evaluar(&alta, Duration::from_secs(15)); // 35 s ≥ 30 → dispara
        assert_eq!(d.len(), 1);
    }

    // --- aplicar: el disparo viaja por el contrato Engine ---
    use async_trait::async_trait;
    use sandokan_core::{ExecHandle, Intent};
    use tokio::sync::mpsc::{self, UnboundedSender};

    #[derive(Debug, PartialEq, Eq)]
    enum Llamada { Stop(Ulid), Weight(String, u32), Freeze(String, bool) }

    struct MockEngine { tx: UnboundedSender<Llamada> }

    #[async_trait]
    impl Engine for MockEngine {
        async fn run(&self, _i: Intent) -> Result<ExecHandle, EngineError> { unreachable!() }
        async fn stop(&self, id: Ulid, _g: Duration) -> Result<(), EngineError> {
            self.tx.send(Llamada::Stop(id)).unwrap(); Ok(())
        }
        async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> { Ok(vec![]) }
        async fn status(&self, _id: Ulid) -> Result<LifecycleState, EngineError> { Ok(LifecycleState::Running) }
        async fn telemetry(&self, id: Ulid) -> Result<TelemetryFrame, EngineError> { Err(EngineError::NotFound(id)) }
        async fn set_cpu_weight(&self, p: String, w: u32) -> Result<(), EngineError> {
            self.tx.send(Llamada::Weight(p, w)).unwrap(); Ok(())
        }
        async fn freeze(&self, p: String, f: bool) -> Result<(), EngineError> {
            self.tx.send(Llamada::Freeze(p, f)).unwrap(); Ok(())
        }
    }

    #[tokio::test]
    async fn aplicar_enruta_cada_accion_al_verbo() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine = MockEngine { tx };
        let id = Ulid::new();
        aplicar(&Disparo { card_id: id, label: "x".into(), regla: "r".into(), accion: AccionControl::Detener { grace_ms: 0 } }, &engine).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), Llamada::Stop(id));
        aplicar(&Disparo { card_id: id, label: "x".into(), regla: "r".into(), accion: AccionControl::Priorizar { cgroup_path: "s".into(), weight: 7 } }, &engine).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), Llamada::Weight("s".into(), 7));
        aplicar(&Disparo { card_id: id, label: "x".into(), regla: "r".into(), accion: AccionControl::Congelar { cgroup_path: "s".into(), frozen: true } }, &engine).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), Llamada::Freeze("s".into(), true));
    }
}
