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
    aplicar_accion(&d.accion, Some(d.card_id), engine).await
}

/// Enruta una `AccionControl` al verbo del contrato. `card_id` es la unidad que
/// disparó (la necesita `Detener`); las reglas de **sistema** no tienen unidad
/// culpable, así que pasan `None` —`Detener` sin unidad es `Unsupported` (una
/// regla global no sabe a quién parar; usá `Priorizar`/`Congelar` sobre un
/// slice)—.
async fn aplicar_accion(
    accion: &AccionControl,
    card_id: Option<Ulid>,
    engine: &dyn Engine,
) -> Result<(), EngineError> {
    match accion {
        AccionControl::Detener { grace_ms } => match card_id {
            Some(id) => engine.stop(id, Duration::from_millis(*grace_ms)).await,
            None => Err(EngineError::Unsupported(
                "Detener sin unidad: una regla de sistema debe usar Priorizar/Congelar sobre un slice".into(),
            )),
        },
        AccionControl::Priorizar { cgroup_path, weight } => {
            engine.set_cpu_weight(cgroup_path.clone(), *weight).await
        }
        AccionControl::Congelar { cgroup_path, frozen } => {
            engine.freeze(cgroup_path.clone(), *frozen).await
        }
    }
}

// =====================================================================
// Reglas de SISTEMA: disparadores por estado global (no por-unidad)
// =====================================================================

/// Señales **globales** del sistema, las que no caben en el snapshot por-unidad:
/// energía, red, inactividad. Las provee el consumidor (lee `/sys`, upower, el
/// idle del compositor…) — el evaluador es **puro**, igual que `energia` recibe
/// `en_bateria` ya resuelto.
#[derive(Debug, Clone, Default)]
pub struct EstadoSistema {
    /// `true` si corre a batería (no en AC). Un escritorio siempre `false`.
    pub en_bateria: bool,
    /// Carga de batería 0–100, si hay batería.
    pub bateria_pct: Option<u8>,
    /// `true` si hay red utilizable.
    pub red: bool,
    /// Tiempo desde la última interacción del usuario.
    pub idle: Duration,
}

/// Predicado sobre el [`EstadoSistema`]. Recursivo (Todas/Cualquiera) para
/// componer («a batería **y** ocioso 10 min»).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CondicionSistema {
    /// Corriendo a batería.
    EnBateria,
    /// Enchufado a corriente (AC).
    EnCorriente,
    /// `bateria_pct <= v` (si hay batería; sin batería no matchea).
    BateriaMenorQue(u8),
    /// No hay red utilizable.
    SinRed,
    /// `idle >= v` (el usuario lleva ese rato sin tocar nada).
    IdleMayorQue(Duration),
    /// AND.
    Todas(Vec<CondicionSistema>),
    /// OR.
    Cualquiera(Vec<CondicionSistema>),
}

impl CondicionSistema {
    /// `true` si el estado satisface la condición.
    pub fn evalua(&self, e: &EstadoSistema) -> bool {
        match self {
            CondicionSistema::EnBateria => e.en_bateria,
            CondicionSistema::EnCorriente => !e.en_bateria,
            CondicionSistema::BateriaMenorQue(v) => e.bateria_pct.is_some_and(|p| p <= *v),
            CondicionSistema::SinRed => !e.red,
            CondicionSistema::IdleMayorQue(d) => e.idle >= *d,
            CondicionSistema::Todas(cs) => cs.iter().all(|c| c.evalua(e)),
            CondicionSistema::Cualquiera(cs) => cs.iter().any(|c| c.evalua(e)),
        }
    }
}

/// Regla global: cuando el sistema entra en `cuando`, ejecutá `entonces`. La
/// acción opera sobre un slice (`Priorizar`/`Congelar`); `Detener` no aplica
/// (no hay unidad culpable). Instantánea por naturaleza —el estado del sistema
/// ya es un nivel, no un evento—, sin `durante`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReglaSistema {
    pub id: String,
    pub cuando: CondicionSistema,
    pub entonces: AccionControl,
}

/// Evalúa las reglas de sistema contra el estado. Devuelve `(id, acción)` por
/// cada regla que matchea. **Pura** — el caller decide cuándo pollear el estado
/// y aplica con [`aplicar_sistema`]. Sin debounce: el caller aplica lo que sale
/// (los verbos son idempotentes — re-escribir el mismo peso/freeze no daña).
pub fn evaluar_sistema<'a>(
    e: &EstadoSistema,
    reglas: &'a [ReglaSistema],
) -> Vec<(&'a str, &'a AccionControl)> {
    reglas
        .iter()
        .filter(|r| r.cuando.evalua(e))
        .map(|r| (r.id.as_str(), &r.entonces))
        .collect()
}

/// Aplica una acción de regla de **sistema** (sin unidad) por el contrato.
pub async fn aplicar_sistema(accion: &AccionControl, engine: &dyn Engine) -> Result<(), EngineError> {
    aplicar_accion(accion, None, engine).await
}

// =====================================================================
// Reglas de TIEMPO: disparadores por horario (cron)
// =====================================================================

/// Horario de una regla de tiempo. Como el resto de `reglas`, el evaluador es
/// **puro**: el caller pasa el minuto-del-día local (`0..1440`) y el `dt` entre
/// polls — ni mira el reloj ni resuelve zona horaria, igual que las reglas de
/// métrica reciben el `dt` ya calculado. Es el «cron» del plano de control.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Horario {
    /// Cada `intervalo`, repetido — acumula el `dt` de cada poll y dispara al
    /// cruzarlo, restándolo para preservar la fase. `Duration::ZERO` no dispara.
    CadaIntervalo(Duration),
    /// Una vez al día al `minuto_del_dia` local (`0..1440`; p.ej. `3*60+30` =
    /// 03:30). Edge-triggered: dispara al **entrar** en ese minuto, una sola vez
    /// por visita (no re-dispara en polls sucesivos del mismo minuto).
    DiariaA { minuto_del_dia: u16 },
}

/// Regla de tiempo: cuando llega su `horario`, ejecutá `entonces`. Sin unidad
/// culpable (como las de sistema) — la acción opera sobre un slice
/// (`Priorizar`/`Congelar`); `Detener` no aplica.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReglaTiempo {
    pub id: String,
    pub horario: Horario,
    pub entonces: AccionControl,
}

/// Motor **con estado** de reglas de tiempo: acumula el `dt` de cada
/// `CadaIntervalo` y recuerda el último minuto visto para el edge-trigger de
/// `DiariaA`. Las reglas son inmutables tras construirlo; sólo el estado cambia.
#[derive(Debug, Default)]
pub struct MotorTiempo {
    reglas: Vec<ReglaTiempo>,
    acumulado: HashMap<String, Duration>,
    ultimo_minuto: Option<u16>,
}

impl MotorTiempo {
    pub fn new(reglas: Vec<ReglaTiempo>) -> Self {
        Self { reglas, acumulado: HashMap::new(), ultimo_minuto: None }
    }

    /// Evalúa las reglas dado el minuto-del-día local `minuto` (`0..1440`) y el
    /// `dt` transcurrido desde el poll anterior. Devuelve `(id, acción)` por cada
    /// disparo. Sin unidad culpable → aplicá cada una con [`aplicar_sistema`].
    pub fn evaluar(&mut self, minuto: u16, dt: Duration) -> Vec<(String, AccionControl)> {
        let mut out = Vec::new();
        for i in 0..self.reglas.len() {
            let id = self.reglas[i].id.clone();
            let horario = self.reglas[i].horario; // Copy → cierra el préstamo
            match horario {
                Horario::CadaIntervalo(iv) => {
                    let acc = self.acumulado.entry(id.clone()).or_default();
                    *acc = acc.saturating_add(dt);
                    if iv > Duration::ZERO && *acc >= iv {
                        // Restar el intervalo preserva la fase; si el poll fue
                        // más largo que un intervalo entero, no acumulamos una
                        // ráfaga: disparamos una vez y truncamos a < iv.
                        *acc = acc.checked_sub(iv).unwrap_or_default();
                        if *acc >= iv {
                            *acc = Duration::ZERO;
                        }
                        out.push((id.clone(), self.reglas[i].entonces.clone()));
                    }
                }
                Horario::DiariaA { minuto_del_dia } => {
                    // Dispara al ENTRAR en el minuto objetivo (no si ya estábamos
                    // en él el poll anterior) → una vez por visita diaria.
                    if minuto == minuto_del_dia && self.ultimo_minuto != Some(minuto_del_dia) {
                        out.push((id, self.reglas[i].entonces.clone()));
                    }
                }
            }
        }
        self.ultimo_minuto = Some(minuto);
        out
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

    #[test]
    fn condicion_sistema_bateria_y_idle() {
        let e = EstadoSistema { en_bateria: true, bateria_pct: Some(15), red: false, idle: Duration::from_secs(600) };
        assert!(CondicionSistema::EnBateria.evalua(&e));
        assert!(!CondicionSistema::EnCorriente.evalua(&e));
        assert!(CondicionSistema::BateriaMenorQue(20).evalua(&e));
        assert!(!CondicionSistema::BateriaMenorQue(10).evalua(&e));
        assert!(CondicionSistema::SinRed.evalua(&e));
        assert!(CondicionSistema::IdleMayorQue(Duration::from_secs(300)).evalua(&e));
        // Compuesta: a batería Y ocioso → cierto.
        let y = CondicionSistema::Todas(vec![
            CondicionSistema::EnBateria,
            CondicionSistema::IdleMayorQue(Duration::from_secs(300)),
        ]);
        assert!(y.evalua(&e));
    }

    #[test]
    fn bateria_sin_dato_no_matchea() {
        let e = EstadoSistema { en_bateria: false, bateria_pct: None, red: true, idle: Duration::ZERO };
        assert!(!CondicionSistema::BateriaMenorQue(50).evalua(&e));
        assert!(CondicionSistema::EnCorriente.evalua(&e));
    }

    #[test]
    fn evaluar_sistema_filtra_las_que_matchean() {
        let reglas = vec![
            ReglaSistema {
                id: "ahorro".into(),
                cuando: CondicionSistema::EnBateria,
                entonces: AccionControl::Congelar { cgroup_path: "pacha/secundario".into(), frozen: true },
            },
            ReglaSistema {
                id: "solo-sin-red".into(),
                cuando: CondicionSistema::SinRed,
                entonces: AccionControl::Priorizar { cgroup_path: "x".into(), weight: 1 },
            },
        ];
        let e = EstadoSistema { en_bateria: true, bateria_pct: Some(80), red: true, idle: Duration::ZERO };
        let m = evaluar_sistema(&e, &reglas);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].0, "ahorro");
    }

    #[tokio::test]
    async fn aplicar_sistema_enruta_congelar_y_detener_sin_unidad_falla() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine = MockEngine { tx };
        aplicar_sistema(&AccionControl::Congelar { cgroup_path: "s".into(), frozen: true }, &engine).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), Llamada::Freeze("s".into(), true));
        // Detener sin unidad (regla de sistema) es Unsupported — no hay a quién parar.
        let err = aplicar_sistema(&AccionControl::Detener { grace_ms: 0 }, &engine).await;
        assert!(matches!(err, Err(sandokan_core::EngineError::Unsupported(_))));
    }

    // --- reglas de TIEMPO (cron) ---
    fn regla_cada(id: &str, iv: Duration) -> ReglaTiempo {
        ReglaTiempo {
            id: id.into(),
            horario: Horario::CadaIntervalo(iv),
            entonces: AccionControl::Priorizar { cgroup_path: "pacha/fondo".into(), weight: 5 },
        }
    }

    #[test]
    fn cada_intervalo_dispara_al_cruzar_y_preserva_fase() {
        let mut m = MotorTiempo::new(vec![regla_cada("backup", Duration::from_secs(60))]);
        // 20 + 20 = 40 s < 60 → nada.
        assert!(m.evaluar(0, Duration::from_secs(20)).is_empty());
        assert!(m.evaluar(0, Duration::from_secs(20)).is_empty());
        // +25 = 65 ≥ 60 → dispara una vez; sobra 5 s de fase.
        let d = m.evaluar(0, Duration::from_secs(25));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, "backup");
        // 5 (fase) + 50 = 55 < 60 → todavía no.
        assert!(m.evaluar(0, Duration::from_secs(50)).is_empty());
        // +10 = 65 ≥ 60 → segundo disparo (repite).
        assert_eq!(m.evaluar(0, Duration::from_secs(10)).len(), 1);
    }

    #[test]
    fn cada_intervalo_poll_largo_no_hace_rafaga() {
        // Un poll gigante (5×) dispara UNA vez, no cinco.
        let mut m = MotorTiempo::new(vec![regla_cada("x", Duration::from_secs(60))]);
        let d = m.evaluar(0, Duration::from_secs(300));
        assert_eq!(d.len(), 1, "un solo disparo aunque el poll cubra 5 intervalos");
    }

    #[test]
    fn diaria_dispara_al_entrar_en_el_minuto_una_vez_por_dia() {
        // 03:30 = 210 minutos del día.
        let objetivo = 3 * 60 + 30;
        let mut m = MotorTiempo::new(vec![ReglaTiempo {
            id: "nocturna".into(),
            horario: Horario::DiariaA { minuto_del_dia: objetivo },
            entonces: AccionControl::Congelar { cgroup_path: "pacha/pesado".into(), frozen: true },
        }]);
        let dt = Duration::from_secs(2);
        // Antes del minuto: nada.
        assert!(m.evaluar(objetivo - 1, dt).is_empty());
        // Entra al minuto objetivo → dispara.
        let d = m.evaluar(objetivo, dt);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, "nocturna");
        // Sigue en el mismo minuto (polls sucesivos) → no re-dispara.
        assert!(m.evaluar(objetivo, dt).is_empty());
        // Sale del minuto y al otro día vuelve a entrar → dispara de nuevo.
        assert!(m.evaluar(objetivo + 1, dt).is_empty());
        assert!(m.evaluar(objetivo - 1, dt).is_empty()); // otro día, acercándose
        assert_eq!(m.evaluar(objetivo, dt).len(), 1, "vuelve a disparar al día siguiente");
    }

    #[tokio::test]
    async fn regla_de_tiempo_se_aplica_por_el_contrato() {
        // Un disparo de tiempo viaja por el mismo Engine que todo lo demás.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine = MockEngine { tx };
        let mut m = MotorTiempo::new(vec![regla_cada("y", Duration::from_secs(1))]);
        let disparos = m.evaluar(0, Duration::from_secs(2));
        assert_eq!(disparos.len(), 1);
        aplicar_sistema(&disparos[0].1, &engine).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), Llamada::Weight("pacha/fondo".into(), 5));
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
