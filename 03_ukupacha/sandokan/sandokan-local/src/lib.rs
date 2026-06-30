//! sandokan-local — `LocalEngine`, orquestación in-process.
//!
//! Primera implementación del trait [`Engine`]: encarna Cards en el
//! mismo host vía `arje-incarnate`, mantiene un registro de las
//! entidades activas y hace reaping perezoso (waitpid WNOHANG en cada
//! consulta) para detectar salidas sin un task de fondo dedicado.
//!
//! `DaemonEngine` y `RemoteEngine` (transportes) se construirán sobre
//! este mismo contrato en crates separados.

mod interactive;
mod proc;

pub use interactive::{Attachment, EngineSnapshot, SessionSnapshot};
pub use sandokan_core::PtySize;

use arje_incarnate::{Incarnator, IncarnatorConfig};
use arje_wasm::WasmHandle;
use async_trait::async_trait;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use sandokan_core::{Engine, EngineError, ExecHandle, Intent, TelemetryFrame};
use sandokan_lifecycle::{Backoff, LifecycleState, RestartPolicy, RestartTracker};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use ulid::Ulid;

/// Una entidad encarnada y su estado conocido.
struct Entity {
    handle: ExecHandle,
    pid: i32,
    /// El intent con que se encarnó, retenido para poder **reiniciarla** sin
    /// que el caller vuelva a traer la Card (`restart` = stop + run del mismo
    /// intent). Es el único estado nuevo que `restart` necesita.
    intent: Intent,
    state: LifecycleState,
    /// Conteo + política de restart. El tracker se incrementa en cada
    /// transición a un estado terminal de fallo (`Exited{code != 0}`,
    /// `Killed`, `Failed{..}`). v1 NO auto-restartea — sólo cuenta;
    /// `telemetry()` expone el contador para que un orquestador externo
    /// (sandokan-monitor, supervisor) decida actuar.
    tracker: RestartTracker,
    /// Último sample CPU `(instante, ticks)` para calcular `cpu_pct` en
    /// `telemetry`. El primer sample sólo siembra el cache y devuelve
    /// 0.0 %; del segundo en adelante hay delta vs wall-clock.
    last_cpu_sample: Option<(Instant, u64)>,
    /// Handle del Ente Wasm si esta entidad es `Payload::Wasm`. `None`
    /// para procesos reales (se cosechan por `pid`) y para Virtual. Su
    /// `finished`/`exit_code` reemplazan al `reap` de proceso: un Wasm no
    /// tiene PID, su terminación se observa por este handle.
    wasm: Option<WasmHandle>,
}

/// Política y backoff por defecto del tracker en `LocalEngine`. Cuenta
/// fallos sin tope (`max_restarts = 0` = infinito) y mantiene un backoff
/// que el orquestador puede consultar si decide hacer respawn manual.
fn default_tracker() -> RestartTracker {
    RestartTracker::new(
        RestartPolicy {
            on_failure: true,
            max_restarts: 0,
        },
        Backoff::new(Duration::from_millis(100), Duration::from_secs(30)),
    )
}

/// Calcula `cpu_pct` por delta vs el último sample en `cache`. Devuelve
/// 0.0 en el primer sample (sin baseline contra el que comparar) y de
/// ahí en adelante el % real con respecto a un solo core. Sube por
/// encima de 100% para procesos multi-thread quemando varios cores —
/// es la convención clásica (top, htop). `cache` se actualiza al sample
/// vigente para que la próxima llamada compare contra éste.
fn cpu_pct_desde_ultimo_sample(pid: i32, cache: &mut Option<(Instant, u64)>) -> f64 {
    let ticks = match proc::read_cpu_ticks(pid) {
        Some(t) => t,
        None => return 0.0,
    };
    let ahora = Instant::now();
    let pct = match *cache {
        Some((prev_t, prev_ticks)) => {
            let dt = ahora.duration_since(prev_t).as_secs_f64();
            if dt > 0.0 && ticks >= prev_ticks {
                let dticks = (ticks - prev_ticks) as f64;
                let clk = proc::clock_ticks_per_second() as f64;
                (dticks / clk / dt) * 100.0
            } else {
                0.0
            }
        }
        None => 0.0,
    };
    *cache = Some((ahora, ticks));
    pct
}

/// `true` si el estado terminal indica una salida anómala — eso es lo
/// que cuenta para el tracker. Exit code 0 (limpio) o estados no
/// terminales devuelven `false`.
fn es_fallo(state: &LifecycleState) -> bool {
    match state {
        LifecycleState::Exited { code } => *code != 0,
        LifecycleState::Killed | LifecycleState::Failed { .. } => true,
        _ => false,
    }
}

/// Orquestador in-process. Encarna Cards localmente y trackea su lifecycle.
pub struct LocalEngine {
    base_cfg: IncarnatorConfig,
    registry: Mutex<HashMap<Ulid, Entity>>,
    /// Sesiones interactivas vivas (PTY retenido) indexadas por card_id.
    /// Vacío para entidades no interactivas (`run`).
    sessions: Mutex<interactive::SessionMap>,
    /// Directorio donde vive un socket por sesión interactiva
    /// (`<run_dir>/<card_id>.sock`). El front (shuma) **siempre** se conecta
    /// a ese path por card_id, sin asumir quién atiende detrás — hoy el
    /// engine in-process (Model 1), mañana un holder por sesión (Model 2).
    run_dir: PathBuf,
    /// Si está, el engine auto-persiste su set de sesiones interactivas acá
    /// tras cada alta/baja (re-hidratación al reiniciar el daemon).
    snapshot_path: Option<PathBuf>,
}

/// Directorio por defecto de los sockets de sesión: `$SANDOKAN_RUN_DIR`, o
/// `$XDG_RUNTIME_DIR/sandokan`, o `/tmp/sandokan-<uid>` como último recurso.
fn default_run_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SANDOKAN_RUN_DIR") {
        return PathBuf::from(d);
    }
    if let Ok(x) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(x).join("sandokan");
    }
    PathBuf::from(format!("/tmp/sandokan-{}", nix::unistd::getuid().as_raw()))
}

impl LocalEngine {
    /// Crea un engine con configuración de incarnación por defecto.
    pub fn new() -> Self {
        Self::with_config(IncarnatorConfig::default())
    }

    /// Crea un engine con una `IncarnatorConfig` explícita (bus socket,
    /// env extra, strict_caps).
    pub fn with_config(cfg: IncarnatorConfig) -> Self {
        Self::with_run_dir(cfg, default_run_dir())
    }

    /// Como [`Self::with_config`] pero con un `run_dir` explícito para los
    /// sockets de sesión. Útil para tests (dir temporal) y para correr varios
    /// engines aislados.
    /// Conveniencia: engine con `run_dir` explícito y config de incarnación
    /// por defecto. Para el binario daemon, que no quiere depender de
    /// arje-incarnate sólo por `IncarnatorConfig::default()`.
    pub fn in_dir(run_dir: PathBuf) -> Self {
        Self::with_run_dir(IncarnatorConfig::default(), run_dir)
    }

    pub fn with_run_dir(cfg: IncarnatorConfig, run_dir: PathBuf) -> Self {
        Self {
            base_cfg: cfg,
            registry: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            run_dir,
            snapshot_path: None,
        }
    }

    /// Fija el archivo de snapshot: el engine auto-persiste el set de sesiones
    /// interactivas tras cada alta/baja. El daemon lo combina con
    /// `restore_snapshot` al arrancar para re-hidratar (Model 1).
    pub fn with_snapshot_path(mut self, path: PathBuf) -> Self {
        self.snapshot_path = Some(path);
        self
    }

    /// Marca el estado de una entidad (best-effort; ignora lock envenenado).
    fn mark(&self, card_id: Ulid, state: LifecycleState) {
        if let Ok(mut reg) = self.registry.lock() {
            if let Some(ent) = reg.get_mut(&card_id) {
                ent.state = state;
            }
        }
    }
}

impl Default for LocalEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Reaping no-bloqueante de un pid. `Some(estado terminal)` si la entidad
/// transicionó; `None` si sigue viva.
fn reap(pid: i32) -> Option<LifecycleState> {
    match waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::Exited(_, code)) => Some(LifecycleState::Exited { code }),
        Ok(WaitStatus::Signaled(_, _, _)) => Some(LifecycleState::Killed),
        Ok(WaitStatus::StillAlive) => None,
        // Stopped / Continued / PtraceEvent: la entidad sigue presente.
        Ok(_) => None,
        Err(nix::errno::Errno::ECHILD) => {
            // No es (ya) hijo reapable. Si procfs no lo tiene, terminó.
            if proc::proc_exists(pid) {
                None
            } else {
                Some(LifecycleState::Exited { code: -1 })
            }
        }
        Err(_) => None,
    }
}

/// PID sentinel para entidades `Virtual` — nodos lógicos del grafo sin
/// proceso real detrás. `0` no es nunca un PID válido en Linux (lo
/// ocupa el scheduler swapper-task) y `nix::Pid::from_raw(0)` también
/// tiene semántica especial en `waitpid`/`kill`. Para esquivar ambos,
/// las operaciones que toquen pids consultan [`is_synthetic_pid`] primero.
const VIRTUAL_PID: i32 = 0;

/// PID sentinel para Entes `Wasm` — cuerpo de cómputo que corre en un
/// hilo wasmi, sin proceso ni PID del kernel. Negativo para no colisionar
/// jamás con un PID real (siempre > 0) ni con el sentinel Virtual (`0`).
const WASM_PID: i32 = -2;

/// `true` para cualquier PID que **no** corresponde a un proceso real del
/// kernel: Virtual (`0`) o Wasm (`< 0`). Esas entidades nunca se cosechan
/// con `waitpid` ni se leen de `/proc` — su estado se resuelve de otra
/// forma (Virtual: vive hasta `stop`; Wasm: por su [`WasmHandle`]).
fn is_synthetic_pid(pid: i32) -> bool {
    pid <= 0
}

/// Refresca el estado de una entidad in-place. Para un proceso real:
/// `reap` no-bloqueante. Para un Ente Wasm: consulta su `WasmHandle`
/// (terminó → `Exited{code}`, contando el fallo en el tracker si el código
/// no es 0). No-op para entidades ya terminales o Virtual (sin cuerpo).
/// Es el punto único que antes estaba duplicado en cada loop de consulta.
fn refresh_entity_state(ent: &mut Entity) {
    if ent.state.is_terminal() {
        return;
    }
    if let Some(h) = &ent.wasm {
        if h.is_finished() {
            let new_state = LifecycleState::Exited {
                code: h.exit_code(),
            };
            if es_fallo(&new_state) {
                ent.tracker.on_failure();
            }
            ent.state = new_state;
        }
        return;
    }
    if !is_synthetic_pid(ent.pid) {
        if let Some(new_state) = reap(ent.pid) {
            if es_fallo(&new_state) {
                ent.tracker.on_failure();
            }
            ent.state = new_state;
        }
    }
}

#[async_trait]
impl Engine for LocalEngine {
    async fn run(&self, intent: Intent) -> Result<ExecHandle, EngineError> {
        use card_core::Payload;
        let card_id = intent.card_id();
        let label = intent.card.label.clone();

        // Atajo para `Payload::Virtual`: registramos la entidad sin
        // tocar el incarnator. Vive como Running indefinidamente y el
        // operador la cierra con stop()/drop_session — ningún reap
        // entra porque su pid es el sentinel `VIRTUAL_PID`.
        if matches!(intent.card.payload, Payload::Virtual) {
            let handle = ExecHandle {
                card_id,
                label,
                started_at: SystemTime::now(),
            };
            let mut reg = self.registry.lock().expect("registry lock");
            reg.insert(
                card_id,
                Entity {
                    handle: handle.clone(),
                    intent: intent.clone(),
                    pid: VIRTUAL_PID,
                    state: LifecycleState::Running,
                    tracker: default_tracker(),
                    last_cpu_sample: None,
                    wasm: None,
                },
            );
            return Ok(handle);
        }

        // `Payload::Wasm`: resolvemos el módulo por su sha256 desde el CAS
        // (arje-cas) y lo encarnamos en wasmi (arje-wasm), que lo corre en
        // un hilo dedicado. No hay PID — el `WasmHandle` reporta cuándo el
        // `entry` retornó y con qué código. Se trata como Ente sintético
        // (`WASM_PID`): no se cosecha por `waitpid` ni se lee de `/proc`.
        if let Payload::Wasm { module_sha256, entry } = &intent.card.payload {
            let bytes = arje_cas::resolve(module_sha256)
                .map_err(|e| EngineError::Incarnate(format!("resolver módulo wasm del CAS: {e}")))?;
            let wasm = arje_wasm::incarnate_wasm(&intent.card, bytes, entry.clone())
                .map_err(|e| EngineError::Incarnate(format!("encarnar wasm: {e}")))?;
            let handle = ExecHandle {
                card_id,
                label,
                started_at: SystemTime::now(),
            };
            let mut reg = self.registry.lock().expect("registry lock");
            reg.insert(
                card_id,
                Entity {
                    handle: handle.clone(),
                    intent: intent.clone(),
                    pid: WASM_PID,
                    state: LifecycleState::Running,
                    tracker: default_tracker(),
                    last_cpu_sample: None,
                    wasm: Some(wasm),
                },
            );
            return Ok(handle);
        }

        // El env del contexto se mergea sobre el del engine base.
        let mut cfg = self.base_cfg.clone();
        cfg.extra_env.extend(intent.context.env.clone());

        // NOTA v1: `IsolationLevel` es advisory. None/Standard encarnan
        // según `Card.soma`; Sealed (rootfs aislado vía pivot_root +
        // OverlayFS) queda reservado para cuando el Intent transporte una
        // spec de rootfs — `arje-incarnate` ya expone los builders.
        let incarnator = Incarnator::new(cfg);
        let outcome = incarnator
            .incarnate(&intent.card)
            .map_err(|e| EngineError::Incarnate(e.to_string()))?;

        let handle = ExecHandle {
            card_id,
            label,
            started_at: SystemTime::now(),
        };
        let mut reg = self.registry.lock().expect("registry lock");
        reg.insert(
            card_id,
            Entity {
                handle: handle.clone(),
                intent: intent.clone(),
                pid: outcome.pid.as_raw(),
                state: LifecycleState::Running,
                tracker: default_tracker(),
                last_cpu_sample: None,
                wasm: None,
            },
        );
        Ok(handle)
    }

    async fn stop(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
        let pid = {
            let reg = self.registry.lock().expect("registry lock");
            reg.get(&card_id)
                .map(|e| e.pid)
                .ok_or(EngineError::NotFound(card_id))?
        };
        // Entidades sintéticas (Virtual/Wasm) no tienen proceso: marcar
        // Killed y salir. Para Wasm es best-effort: wasmi 1.0 sin fuel/epoch
        // no es interrumpible, así que el hilo corre hasta que su `entry`
        // retorne; el registro deja de reportarla viva igual.
        if is_synthetic_pid(pid) {
            self.mark(card_id, LifecycleState::Killed);
            self.drop_session(card_id);
            return Ok(());
        }
        let npid = Pid::from_raw(pid);

        // SIGTERM + período de gracia: damos chance a un cierre ordenado.
        if grace > Duration::ZERO {
            let _ = kill(npid, Signal::SIGTERM);
            let deadline = Instant::now() + grace;
            loop {
                if reap(pid).is_some() {
                    self.mark(card_id, LifecycleState::Killed);
                    self.drop_session(card_id);
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        // SIGKILL + reap bloqueante de lo que haya quedado.
        let _ = kill(npid, Signal::SIGKILL);
        let _ = waitpid(npid, None);
        self.mark(card_id, LifecycleState::Killed);
        self.drop_session(card_id);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
        let mut reg = self.registry.lock().expect("registry lock");
        let mut out = Vec::new();
        for ent in reg.values_mut() {
            // Entidades Virtual viven en Running hasta stop(); procesos
            // reales se cosechan por pid; Entes Wasm por su handle.
            refresh_entity_state(ent);
            if !ent.state.is_terminal() {
                out.push(ent.handle.clone());
            }
        }
        Ok(out)
    }

    async fn status(&self, card_id: Ulid) -> Result<LifecycleState, EngineError> {
        let mut reg = self.registry.lock().expect("registry lock");
        let ent = reg
            .get_mut(&card_id)
            .ok_or(EngineError::NotFound(card_id))?;
        refresh_entity_state(ent);
        Ok(ent.state.clone())
    }

    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError> {
        // Lock una sola vez: leemos pid + tracker count + samples CPU,
        // y de paso ejecutamos cualquier reap pendiente para que
        // `restarts` refleje la última transición conocida.
        let (pid, restarts, cpu_pct) = {
            let mut reg = self.registry.lock().expect("registry lock");
            let ent = reg
                .get_mut(&card_id)
                .ok_or(EngineError::NotFound(card_id))?;
            refresh_entity_state(ent);
            // Sample CPU + cómputo del % desde el último sample. Entidades
            // sintéticas (Virtual/Wasm) y procesos ya terminales devuelven
            // 0.0 sin tocar /proc.
            let cpu_pct = if is_synthetic_pid(ent.pid) || ent.state.is_terminal() {
                0.0
            } else {
                cpu_pct_desde_ultimo_sample(ent.pid, &mut ent.last_cpu_sample)
            };
            (ent.pid, ent.tracker.count(), cpu_pct)
        };
        let (mem_bytes, nproc) = if is_synthetic_pid(pid) {
            (0, 0)
        } else {
            (proc::read_mem_bytes(pid), proc::read_thread_count(pid))
        };
        Ok(TelemetryFrame {
            card_id,
            at: SystemTime::now(),
            mem_bytes,
            nproc,
            cpu_pct,
            restarts,
        })
    }

    async fn set_cpu_weight(&self, cgroup_path: String, weight: u32) -> Result<(), EngineError> {
        arje_incarnate::cgroup::set_cpu_weight(&cgroup_path, weight)
            .map_err(|e| EngineError::Cgroup(e.to_string()))
    }

    async fn freeze(&self, cgroup_path: String, frozen: bool) -> Result<(), EngineError> {
        arje_incarnate::cgroup::set_frozen(&cgroup_path, frozen)
            .map_err(|e| EngineError::Cgroup(e.to_string()))
    }

    async fn restart(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
        // Capturamos el intent ANTES de parar (stop suelta el registro de la
        // entidad). Reiniciar es stop→run del mismo intent: misma Card, mismo
        // card_id (Card.id es estable), entidad fresca en Running. El tracker
        // de restart arranca de cero — es un reinicio deliberado, no un fallo
        // contado (esos los cuenta `on_failure` en el reap).
        let intent = {
            let reg = self.registry.lock().expect("registry lock");
            reg.get(&card_id)
                .map(|e| e.intent.clone())
                .ok_or(EngineError::NotFound(card_id))?
        };
        self.stop(card_id, grace).await?;
        self.run(intent).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_engine_lists_nothing() {
        let e = LocalEngine::new();
        assert!(e.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_card_is_not_found() {
        let e = LocalEngine::new();
        let id = Ulid::new();
        assert!(matches!(e.status(id).await, Err(EngineError::NotFound(_))));
        assert!(matches!(
            e.stop(id, Duration::ZERO).await,
            Err(EngineError::NotFound(_))
        ));
        assert!(matches!(
            e.telemetry(id).await,
            Err(EngineError::NotFound(_))
        ));
    }

    /// Card que corre `/bin/sh -c "<cmd>"` sin namespaces. Sin aislamiento
    /// a propósito: el path fork+exec de `arje-incarnate` es confiable bajo
    /// el runtime de test (la correctitud de los namespaces la cubren los
    /// tests de `arje-incarnate`; acá probamos el contrato del Engine).
    fn sh_card(label: &str, cmd: &str) -> card_core::Card {
        use card_core::{NamespaceSet, Payload};
        let mut c = card_core::Card::new(label);
        c.payload = Payload::Native {
            exec: "/bin/sh".into(),
            argv: vec!["-c".into(), cmd.into()],
            envp: vec![],
        };
        c.soma.namespaces = NamespaceSet::default();
        c
    }

    #[tokio::test]
    async fn runs_tracks_and_kills_a_real_process() {
        let e = LocalEngine::new();
        let card = sh_card("sandbox-sleep", "sleep 30");
        let id = card.id;
        let handle = e.run(Intent::new(card)).await.expect("run");
        assert_eq!(handle.card_id, id);

        // Aparece como activa y en Running.
        assert_eq!(e.list().await.unwrap().len(), 1);
        assert_eq!(e.status(id).await.unwrap(), LifecycleState::Running);

        // Telemetría real: el proceso ocupa memoria (RSS leído de /proc).
        let t = e.telemetry(id).await.unwrap();
        assert!(t.mem_bytes > 0, "esperaba RSS > 0, fue {}", t.mem_bytes);
        assert!(t.nproc >= 1);

        // Stop inmediato (SIGKILL). Tras reapear, no queda activa.
        e.stop(id, Duration::ZERO).await.expect("stop");
        assert!(e.list().await.unwrap().is_empty());
        assert!(e.status(id).await.unwrap().is_terminal());
    }

    #[tokio::test]
    async fn restart_recicla_la_unidad_con_el_mismo_id() {
        let e = LocalEngine::new();
        let card = sh_card("sandbox-restart", "sleep 30");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run");
        let pid_inicial = e.telemetry(id).await.unwrap().card_id; // (id estable)
        assert_eq!(pid_inicial, id);
        assert_eq!(e.status(id).await.unwrap(), LifecycleState::Running);

        // Reiniciar: la para y la vuelve a encarnar con el mismo card_id.
        e.restart(id, Duration::ZERO).await.expect("restart");

        // Sigue viva, con el mismo id, y es un proceso nuevo (RSS > 0).
        assert_eq!(e.status(id).await.unwrap(), LifecycleState::Running);
        assert_eq!(e.list().await.unwrap().len(), 1);
        assert!(e.telemetry(id).await.unwrap().mem_bytes > 0);

        e.stop(id, Duration::ZERO).await.expect("stop");
        assert!(e.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restart_de_inexistente_es_not_found() {
        let e = LocalEngine::new();
        assert!(matches!(
            e.restart(Ulid::new(), Duration::ZERO).await,
            Err(EngineError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn graceful_stop_terminates_via_sigterm() {
        let e = LocalEngine::new();
        // `sleep` termina con la acción default de SIGTERM, así que el
        // período de gracia lo cierra sin llegar al SIGKILL.
        let card = sh_card("sandbox-grace", "sleep 30");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run");
        assert_eq!(e.list().await.unwrap().len(), 1);

        e.stop(id, Duration::from_secs(2)).await.expect("stop");
        assert!(e.status(id).await.unwrap().is_terminal());
        assert!(e.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn pid_namespace_isolates_the_child_as_pid_1() {
        use card_core::{NamespaceSet, Payload};
        // En un PID namespace propio, el primer proceso se ve como PID 1.
        // `test $$ -eq 1` sale 0 sólo si el aislamiento se aplicó. Requiere
        // user namespace para que un uid no privilegiado pueda crear el
        // resto (funciona acá: unprivileged_userns_clone=1).
        let e = LocalEngine::new();
        let mut c = card_core::Card::new("isolated");
        c.payload = Payload::Native {
            exec: "/bin/sh".into(),
            argv: vec!["-c".into(), "test $$ -eq 1".into()],
            envp: vec![],
        };
        c.soma.namespaces = NamespaceSet {
            user: true,
            pid: true,
            mount: true,
            uts: true,
            ipc: true,
            net: false,
            cgroup: false,
        };
        let id = c.id;
        e.run(Intent::new(c)).await.expect("run isolated");
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(
            e.status(id).await.unwrap(),
            LifecycleState::Exited { code: 0 },
            "el hijo debería ser PID 1 dentro de su pidns"
        );
    }

    #[tokio::test]
    async fn process_that_exits_on_its_own_leaves_active_set() {
        let e = LocalEngine::new();
        let card = sh_card("sandbox-quick", "true");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run");
        // Damos un instante a que salga y reapeamos vía list/status.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let st = e.status(id).await.unwrap();
        assert!(st.is_terminal(), "esperaba terminal, fue {st:?}");
        assert!(e.list().await.unwrap().is_empty());
    }

    /// Una salida anómala (exit code != 0 por su cuenta) hace que la
    /// transición a estado terminal incremente el tracker exactamente
    /// una vez. El TelemetryFrame deja de mentir `restarts: 0`.
    #[tokio::test]
    async fn telemetry_cuenta_restarts_en_salida_anomala() {
        let e = LocalEngine::new();
        let card = sh_card("sandbox-fallo", "exit 1");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run");

        // Polling hasta que telemetry vea la transición y suba el contador.
        let deadline = Instant::now() + Duration::from_secs(2);
        let restarts = loop {
            let frame = e.telemetry(id).await.expect("telemetry");
            if frame.restarts > 0 {
                break frame.restarts;
            }
            if Instant::now() >= deadline {
                panic!(
                    "el contador de restarts no se incrementó dentro del timeout: {frame:?}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(restarts, 1, "una sola salida anómala = 1 restart");

        // Una consulta más NO debe re-incrementar: ya estamos en terminal,
        // el reap no vuelve a ejecutarse para esa entidad.
        let frame = e.telemetry(id).await.expect("telemetry segunda");
        assert_eq!(
            frame.restarts, 1,
            "tras llegar a terminal el contador no debe re-incrementar",
        );
    }

    // -----------------------------------------------------------------
    //  RunCard arbitraria — Virtual + Wasm
    // -----------------------------------------------------------------

    fn virtual_card(label: &str) -> card_core::Card {
        let mut c = card_core::Card::new(label);
        c.payload = card_core::Payload::Virtual;
        c
    }

    fn wasm_card(label: &str, module_sha256: [u8; 32], entry: &str) -> card_core::Card {
        let mut c = card_core::Card::new(label);
        c.payload = card_core::Payload::Wasm {
            module_sha256,
            entry: entry.into(),
        };
        c
    }

    /// Una Card `Virtual` se acepta sin incarnator: aparece en list/status
    /// como Running con pid sentinel. stop() la marca Killed sin tocar
    /// señales (no hay proceso real).
    #[tokio::test]
    async fn virtual_card_corre_sin_proceso_y_se_para_limpio() {
        let e = LocalEngine::new();
        let card = virtual_card("nodo-logico");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run virtual");

        assert_eq!(e.list().await.unwrap().len(), 1);
        assert_eq!(e.status(id).await.unwrap(), LifecycleState::Running);

        // Telemetry: no hay /proc/0 que leer, pero el frame se devuelve
        // bien (mem y nproc en 0, sin panic).
        let t = e.telemetry(id).await.expect("telemetry virtual");
        assert_eq!(t.mem_bytes, 0);
        assert_eq!(t.nproc, 0);
        assert_eq!(t.restarts, 0);

        e.stop(id, Duration::ZERO).await.expect("stop virtual");
        assert!(e.list().await.unwrap().is_empty());
        assert_eq!(e.status(id).await.unwrap(), LifecycleState::Killed);
    }

    /// Encarnación real de `Payload::Wasm`: el módulo demo (`_start` →
    /// `ente.log` + `ente.exit(0)`) se guarda en el CAS, se referencia por
    /// su sha256, y el `LocalEngine` lo corre en wasmi. El Ente no tiene
    /// PID — su terminación se observa por el `WasmHandle`: `status()`
    /// transiciona de `Running` a `Exited{0}` cuando el hilo termina.
    /// Cubre también el caso de sha ausente → `EngineError::Incarnate`.
    ///
    /// Un solo test (no dos) a propósito: ambos manipulan el env global
    /// `ENTE_CAS_ROOT`; partirlo en dos tests los haría correr en paralelo
    /// y pisarse la raíz del CAS.
    #[tokio::test]
    async fn wasm_card_corre_en_cas_y_termina_limpio() {
        // CAS hermético en tempdir (override por env del default XDG).
        let cas_dir = tempfile::tempdir().expect("tempdir CAS");
        std::env::set_var("ENTE_CAS_ROOT", cas_dir.path());

        let e = LocalEngine::new();

        // 1) sha que no está en el CAS → error de encarnación tipado.
        let ausente = wasm_card("wasm-ausente", [7u8; 32], "_start");
        match e.run(Intent::new(ausente)).await.unwrap_err() {
            EngineError::Incarnate(_) => {}
            other => panic!("esperaba Incarnate por sha ausente, fue {other:?}"),
        }

        // 2) módulo demo al CAS → corre y termina con exit(0).
        let bytes = arje_wasm::demo_module_bytes().expect("demo wasm compila");
        let sha = arje_cas::store(&bytes).expect("store demo en CAS");
        let card = wasm_card("modulo-wasm", sha, "_start");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run wasm");

        // El hilo wasmi corre async; poll hasta terminal (limpio en < 5s).
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match e.status(id).await.expect("status wasm") {
                LifecycleState::Exited { code } => {
                    assert_eq!(code, 0, "ente.exit(0) ⇒ Exited{{0}}");
                    break;
                }
                LifecycleState::Running => {}
                other => panic!("estado inesperado: {other:?}"),
            }
            if Instant::now() >= deadline {
                panic!("el ente wasm no terminó en 5s");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Ya terminal: list() no lo reporta vivo; telemetry no toca /proc.
        assert!(e.list().await.unwrap().is_empty());
        let t = e.telemetry(id).await.expect("telemetry wasm");
        assert_eq!(t.mem_bytes, 0);
        assert_eq!(t.nproc, 0);
    }

    /// Telemetry calcula CPU% por delta entre dos samples. El primero
    /// devuelve 0.0 (no hay baseline); el segundo —tras un ratito de
    /// `sh -c "while true; do :; done"`— marca > 0%. No verificamos un
    /// valor exacto porque depende del kernel, pero sí que sea > 0.
    #[tokio::test]
    async fn telemetry_cpu_pct_se_calcula_entre_dos_samples() {
        let e = LocalEngine::new();
        // Bucle activo de shell quemando CPU.
        let card = sh_card("sandbox-cpu", "while true; do :; done");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run busy");

        // Primer sample: siembra el cache → cpu_pct == 0.
        let first = e.telemetry(id).await.expect("first telemetry");
        assert_eq!(first.cpu_pct, 0.0, "primer sample debe sembrar el cache");

        // Esperamos > 0.2s para que el delta sea medible aunque sysconf
        // dé CLK_TCK = 100 (10 ms de granularidad).
        tokio::time::sleep(Duration::from_millis(250)).await;

        let second = e.telemetry(id).await.expect("second telemetry");
        assert!(
            second.cpu_pct > 0.0,
            "esperaba cpu_pct > 0 con bucle activo, fue {}",
            second.cpu_pct,
        );

        e.stop(id, Duration::ZERO).await.ok();
    }

    /// Una salida limpia (exit 0) NO es un fallo: el contador queda en 0.
    #[tokio::test]
    async fn telemetry_no_cuenta_restart_en_exit_limpio() {
        let e = LocalEngine::new();
        let card = sh_card("sandbox-ok", "exit 0");
        let id = card.id;
        e.run(Intent::new(card)).await.expect("run");

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if e.status(id).await.unwrap().is_terminal() {
                break;
            }
            if Instant::now() >= deadline {
                panic!("el proceso no terminó a tiempo");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let frame = e.telemetry(id).await.expect("telemetry");
        assert_eq!(frame.restarts, 0, "exit 0 no es fallo");
    }
}
