//! `sandokan-vigilante` — el **lazo vivo** de la automatización por métrica.
//!
//! La capa 2 (`sandokan-monitor-core::reglas`) es pura: dado un snapshot y un
//! `dt`, dice qué disparó. Falta quien la **corra**: pollear el `Engine`, pasarle
//! el tiempo y aplicar los disparos. Eso es el [`Vigilante`] —el equivalente,
//! del lado del control, del coordinador de idle que pata armó sobre `energia`—.
//!
//! Cierra el lazo end-to-end por el **mismo contrato** (SDD §6/§8): cada vuelta
//! `observe(engine) → MotorMetrico::evaluar → aplicar(&engine)`. Las reglas son
//! **hot-swappables** ([`Vigilante::armar`]): una intención `pacha` arma su set
//! al entrar y lo desarma al salir, así una intención condiciona servicios
//! mientras está enfocada (capa 4) en vez de sólo fijar prioridades estáticas.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use sandokan_core::Engine;
use sandokan_monitor_core::observe;
use sandokan_monitor_core::reglas::{aplicar, Disparo, MotorMetrico, ReglaMetrica};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Corre las reglas de métrica contra un `Engine` en un lazo de poll. El motor
/// (con su estado de rachas) vive tras un `Mutex` para poder **reemplazar el
/// set de reglas en caliente** sin recrear el Vigilante.
pub struct Vigilante {
    engine: Arc<dyn Engine>,
    motor: Mutex<MotorMetrico>,
    intervalo: Duration,
}

impl Vigilante {
    /// Vigilante sobre `engine`, evaluando `reglas` cada `intervalo`.
    pub fn new(engine: Arc<dyn Engine>, reglas: Vec<ReglaMetrica>, intervalo: Duration) -> Self {
        Self {
            engine,
            motor: Mutex::new(MotorMetrico::new(reglas)),
            intervalo,
        }
    }

    /// Reemplaza el set de reglas activo (descarta el estado de rachas previo).
    /// Es el gancho de la capa 4: al cambiar de intención `pacha`, se arma el
    /// set de la intención entrante. Pasar `vec![]` desarma todo.
    pub async fn armar(&self, reglas: Vec<ReglaMetrica>) {
        *self.motor.lock().await = MotorMetrico::new(reglas);
    }

    /// Una vuelta del lazo: pollea el Engine, evalúa avanzando `intervalo` de
    /// tiempo y aplica cada disparo por el contrato. Devuelve los disparos que
    /// se ejecutaron (vacío si el `observe` falló o nada cruzó su umbral) —
    /// útil para tests y para que el caller los muestre/loguee. Un `observe`
    /// caído NO mata el lazo: se loguea y la vuelta queda vacía.
    pub async fn tick(&self) -> Vec<Disparo> {
        let snap = match observe(self.engine.as_ref()).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "vigilante: observe falló, salteo la vuelta");
                return Vec::new();
            }
        };
        let disparos = {
            let mut motor = self.motor.lock().await;
            motor.evaluar(&snap, self.intervalo)
        };
        for d in &disparos {
            match aplicar(d, self.engine.as_ref()).await {
                Ok(()) => debug!(regla = %d.regla, unidad = %d.label, "vigilante: disparo aplicado"),
                Err(e) => warn!(regla = %d.regla, unidad = %d.label, error = %e, "vigilante: aplicar falló"),
            }
        }
        disparos
    }

    /// Corre el lazo indefinidamente, una `tick` por `intervalo`. El caller lo
    /// spawnea (`tokio::spawn`) y lo detiene abortando la tarea; el Vigilante no
    /// guarda estado externo que haya que limpiar.
    pub async fn correr(self: Arc<Self>) {
        let mut tic = tokio::time::interval(self.intervalo);
        // La primera marca es inmediata; saltarla evita un poll en t=0 con dt=0.
        tic.tick().await;
        loop {
            tic.tick().await;
            self.tick().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sandokan_core::{EngineError, ExecHandle, Intent, TelemetryFrame};
    use sandokan_lifecycle::LifecycleState;
    use sandokan_monitor_core::reglas::{AccionControl, Condicion};
    use std::sync::Mutex as StdMutex;
    use std::time::SystemTime;
    use ulid::Ulid;

    /// Engine que reporta una unidad fija y graba los verbos de control que le
    /// llegan. Así el test cierra el lazo: la unidad «caliente» dispara una
    /// regla y verificamos que el verbo viajó por el contrato.
    struct MockEngine {
        id: Ulid,
        cpu: f64,
        aplicados: StdMutex<Vec<String>>,
    }

    #[async_trait]
    impl Engine for MockEngine {
        async fn run(&self, _i: Intent) -> Result<ExecHandle, EngineError> {
            unreachable!()
        }
        async fn stop(&self, id: Ulid, _g: Duration) -> Result<(), EngineError> {
            self.aplicados.lock().unwrap().push(format!("stop {id}"));
            Ok(())
        }
        async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
            Ok(vec![ExecHandle {
                card_id: self.id,
                label: "hog".into(),
                started_at: SystemTime::UNIX_EPOCH,
            }])
        }
        async fn status(&self, _id: Ulid) -> Result<LifecycleState, EngineError> {
            Ok(LifecycleState::Running)
        }
        async fn telemetry(&self, id: Ulid) -> Result<TelemetryFrame, EngineError> {
            Ok(TelemetryFrame {
                card_id: id,
                at: SystemTime::UNIX_EPOCH,
                mem_bytes: 1024,
                nproc: 1,
                cpu_pct: self.cpu,
                restarts: 0,
            })
        }
        async fn set_cpu_weight(&self, p: String, w: u32) -> Result<(), EngineError> {
            self.aplicados.lock().unwrap().push(format!("weight {p}={w}"));
            Ok(())
        }
        async fn freeze(&self, p: String, f: bool) -> Result<(), EngineError> {
            self.aplicados.lock().unwrap().push(format!("freeze {p}={f}"));
            Ok(())
        }
    }

    fn regla_stop_si_cpu(durante: Duration) -> ReglaMetrica {
        ReglaMetrica {
            id: "matar-hog".into(),
            cuando: Condicion::CpuPctMin(80.0),
            durante,
            entonces: AccionControl::Detener { grace_ms: 0 },
        }
    }

    #[tokio::test]
    async fn tick_cierra_el_lazo_observe_evaluar_aplicar() {
        let eng = Arc::new(MockEngine { id: Ulid::new(), cpu: 95.0, aplicados: StdMutex::new(vec![]) });
        // durante=0 → dispara en la primera vuelta.
        let v = Vigilante::new(eng.clone(), vec![regla_stop_si_cpu(Duration::ZERO)], Duration::from_secs(1));
        let d = v.tick().await;
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].regla, "matar-hog");
        // El verbo viajó por el contrato hasta el Engine.
        assert_eq!(eng.aplicados.lock().unwrap().as_slice(), &["stop ".to_string() + &eng.id.to_string()]);
    }

    #[tokio::test]
    async fn cpu_baja_no_dispara() {
        let eng = Arc::new(MockEngine { id: Ulid::new(), cpu: 5.0, aplicados: StdMutex::new(vec![]) });
        let v = Vigilante::new(eng.clone(), vec![regla_stop_si_cpu(Duration::ZERO)], Duration::from_secs(1));
        assert!(v.tick().await.is_empty());
        assert!(eng.aplicados.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn armar_desarma_las_reglas() {
        let eng = Arc::new(MockEngine { id: Ulid::new(), cpu: 95.0, aplicados: StdMutex::new(vec![]) });
        let v = Vigilante::new(eng.clone(), vec![regla_stop_si_cpu(Duration::ZERO)], Duration::from_secs(1));
        v.armar(vec![]).await; // desarmar
        assert!(v.tick().await.is_empty(), "sin reglas no debe disparar nada");
        // Re-armar y ahora sí dispara.
        v.armar(vec![regla_stop_si_cpu(Duration::ZERO)]).await;
        assert_eq!(v.tick().await.len(), 1);
    }
}
