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
use sandokan_monitor_core::reglas::{
    aplicar, aplicar_sistema, evaluar_sistema, Disparo, EstadoSistema, MotorMetrico, MotorTiempo,
    ReglaMetrica, ReglaSistema, ReglaTiempo,
};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Corre las reglas de métrica contra un `Engine` en un lazo de poll. El motor
/// (con su estado de rachas) vive tras un `Mutex` para poder **reemplazar el
/// set de reglas en caliente** sin recrear el Vigilante. Lleva además un set de
/// **reglas de sistema** (energía/red/idle), evaluadas contra un `EstadoSistema`
/// que el caller provee en [`tick_sistema`](Vigilante::tick_sistema) —el I/O de
/// sensar batería/red vive en el borde, como en `energia`—.
pub struct Vigilante {
    engine: Arc<dyn Engine>,
    motor: Mutex<MotorMetrico>,
    sistema: Mutex<Vec<ReglaSistema>>,
    /// Motor de reglas de **tiempo** (cron). Como el de métrica, tiene estado
    /// (acumuladores/edge) y se reemplaza en caliente con [`armar_tiempo`](Self::armar_tiempo).
    tiempo: Mutex<MotorTiempo>,
    intervalo: Duration,
}

impl Vigilante {
    /// Vigilante sobre `engine`, evaluando `reglas` cada `intervalo`. Las reglas
    /// de sistema arrancan vacías (se arman con [`armar_sistema`](Self::armar_sistema)).
    pub fn new(engine: Arc<dyn Engine>, reglas: Vec<ReglaMetrica>, intervalo: Duration) -> Self {
        Self {
            engine,
            motor: Mutex::new(MotorMetrico::new(reglas)),
            sistema: Mutex::new(Vec::new()),
            tiempo: Mutex::new(MotorTiempo::default()),
            intervalo,
        }
    }

    /// Reemplaza el set de reglas de métrica activo (descarta el estado de
    /// rachas previo). Es el gancho de la capa 4: al cambiar de intención
    /// `pacha`, se arma el set de la intención entrante. `vec![]` desarma todo.
    pub async fn armar(&self, reglas: Vec<ReglaMetrica>) {
        *self.motor.lock().await = MotorMetrico::new(reglas);
    }

    /// Reemplaza el set de **reglas de sistema** activo (energía/red/idle).
    /// También parte del armado por intención. `vec![]` desarma.
    pub async fn armar_sistema(&self, reglas: Vec<ReglaSistema>) {
        *self.sistema.lock().await = reglas;
    }

    /// Reemplaza el set de **reglas de tiempo** activo (cron). Parte del armado
    /// por intención. `vec![]` desarma (y descarta acumuladores/edge previos).
    pub async fn armar_tiempo(&self, reglas: Vec<ReglaTiempo>) {
        *self.tiempo.lock().await = MotorTiempo::new(reglas);
    }

    /// Evalúa las reglas de **tiempo** dado el `minuto` del día local (`0..1440`)
    /// que el caller sensó del reloj, avanzando `intervalo` de `dt`, y aplica los
    /// disparos por el contrato. Devuelve los ids aplicados. Separado de
    /// [`tick`](Self::tick) porque su entrada es el reloj del borde, no el snapshot.
    pub async fn tick_tiempo(&self, minuto: u16) -> Vec<String> {
        let disparos = {
            let mut motor = self.tiempo.lock().await;
            motor.evaluar(minuto, self.intervalo)
        };
        let mut aplicadas = Vec::with_capacity(disparos.len());
        for (id, accion) in disparos {
            match aplicar_sistema(&accion, self.engine.as_ref()).await {
                Ok(()) => {
                    debug!(regla = %id, "vigilante: regla de tiempo aplicada");
                    aplicadas.push(id);
                }
                Err(e) => warn!(regla = %id, error = %e, "vigilante: aplicar_tiempo falló"),
            }
        }
        aplicadas
    }

    /// Evalúa las reglas de **sistema** contra `estado` (que el caller sensó:
    /// batería/red/idle) y aplica las que matchean por el contrato. Devuelve los
    /// ids de regla aplicados. Separado de [`tick`](Self::tick) porque su entrada
    /// es I/O del borde, no el snapshot del Engine.
    pub async fn tick_sistema(&self, estado: &EstadoSistema) -> Vec<String> {
        let reglas = self.sistema.lock().await;
        let matched = evaluar_sistema(estado, &reglas);
        let mut aplicadas = Vec::with_capacity(matched.len());
        for (id, accion) in matched {
            match aplicar_sistema(accion, self.engine.as_ref()).await {
                Ok(()) => {
                    debug!(regla = %id, "vigilante: regla de sistema aplicada");
                    aplicadas.push(id.to_string());
                }
                Err(e) => warn!(regla = %id, error = %e, "vigilante: aplicar_sistema falló"),
            }
        }
        aplicadas
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
    async fn tick_sistema_aplica_regla_de_energia() {
        use sandokan_monitor_core::reglas::{EstadoSistema, ReglaSistema, CondicionSistema};
        let eng = Arc::new(MockEngine { id: Ulid::new(), cpu: 5.0, aplicados: StdMutex::new(vec![]) });
        let v = Vigilante::new(eng.clone(), vec![], Duration::from_secs(1));
        v.armar_sistema(vec![ReglaSistema {
            id: "ahorro".into(),
            cuando: CondicionSistema::EnBateria,
            entonces: AccionControl::Congelar { cgroup_path: "pacha/secundario".into(), frozen: true },
        }]).await;
        // En AC: no dispara.
        let ac = EstadoSistema { en_bateria: false, ..Default::default() };
        assert!(v.tick_sistema(&ac).await.is_empty());
        // A batería: congela el slice secundario.
        let bat = EstadoSistema { en_bateria: true, ..Default::default() };
        assert_eq!(v.tick_sistema(&bat).await, vec!["ahorro".to_string()]);
        assert_eq!(eng.aplicados.lock().unwrap().as_slice(), &["freeze pacha/secundario=true".to_string()]);
    }

    #[tokio::test]
    async fn tick_tiempo_dispara_regla_diaria_por_el_contrato() {
        use sandokan_monitor_core::reglas::{Horario, ReglaTiempo};
        let eng = Arc::new(MockEngine { id: Ulid::new(), cpu: 5.0, aplicados: StdMutex::new(vec![]) });
        let v = Vigilante::new(eng.clone(), vec![], Duration::from_secs(1));
        let objetivo = 2 * 60; // 02:00
        v.armar_tiempo(vec![ReglaTiempo {
            id: "nocturna".into(),
            horario: Horario::DiariaA { minuto_del_dia: objetivo },
            entonces: AccionControl::Congelar { cgroup_path: "pacha/pesado".into(), frozen: true },
        }]).await;
        // Antes del minuto: nada.
        assert!(v.tick_tiempo(objetivo - 1).await.is_empty());
        // Al entrar: dispara y congela por el contrato.
        assert_eq!(v.tick_tiempo(objetivo).await, vec!["nocturna".to_string()]);
        assert_eq!(eng.aplicados.lock().unwrap().as_slice(), &["freeze pacha/pesado=true".to_string()]);
        // Mismo minuto: no re-dispara.
        assert!(v.tick_tiempo(objetivo).await.is_empty());
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
