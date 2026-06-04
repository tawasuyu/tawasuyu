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
    state: LifecycleState,
    /// Conteo + política de restart. El tracker se incrementa en cada
    /// transición a un estado terminal de fallo (`Exited{code != 0}`,
    /// `Killed`, `Failed{..}`). v1 NO auto-restartea — sólo cuenta;
    /// `telemetry()` expone el contador para que un orquestador externo
    /// (sandokan-monitor, supervisor) decida actuar.
    tracker: RestartTracker,
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
/// las operaciones que toquen pids consultan [`is_virtual_pid`] primero.
const VIRTUAL_PID: i32 = 0;

fn is_virtual_pid(pid: i32) -> bool {
    pid == VIRTUAL_PID
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
                    pid: VIRTUAL_PID,
                    state: LifecycleState::Running,
                    tracker: default_tracker(),
                },
            );
            return Ok(handle);
        }

        // `Payload::Wasm` exige delegar a un runtime distinto (futuro
        // `ente-wasm`); por ahora marcamos no-soportado en vez de
        // pretender encarnarlo y fallar con un mensaje de exec.
        if matches!(intent.card.payload, Payload::Wasm { .. }) {
            return Err(EngineError::UnsupportedPayload {
                kind: "Wasm".into(),
            });
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
                pid: outcome.pid.as_raw(),
                state: LifecycleState::Running,
                tracker: default_tracker(),
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
        // Entidades Virtual no tienen proceso: marcar Killed y salir.
        if is_virtual_pid(pid) {
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
            // Entidades Virtual (sin pid real) no reapean: viven en
            // Running hasta que un stop() las marque Killed.
            if !ent.state.is_terminal() && !is_virtual_pid(ent.pid) {
                if let Some(new_state) = reap(ent.pid) {
                    if es_fallo(&new_state) {
                        ent.tracker.on_failure();
                    }
                    ent.state = new_state;
                }
            }
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
        if !ent.state.is_terminal() && !is_virtual_pid(ent.pid) {
            if let Some(new_state) = reap(ent.pid) {
                if es_fallo(&new_state) {
                    ent.tracker.on_failure();
                }
                ent.state = new_state;
            }
        }
        Ok(ent.state.clone())
    }

    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError> {
        // Lock una sola vez: necesitamos el pid Y el conteo del tracker.
        // Cualquier reap pendiente se ejecuta acá también para que el
        // restarts reportado refleje el último estado conocido.
        let (pid, restarts) = {
            let mut reg = self.registry.lock().expect("registry lock");
            let ent = reg
                .get_mut(&card_id)
                .ok_or(EngineError::NotFound(card_id))?;
            if !ent.state.is_terminal() && !is_virtual_pid(ent.pid) {
                if let Some(new_state) = reap(ent.pid) {
                    if es_fallo(&new_state) {
                        ent.tracker.on_failure();
                    }
                    ent.state = new_state;
                }
            }
            (ent.pid, ent.tracker.count())
        };
        // Para entidades Virtual no hay /proc/0 — devolvemos 0s en vez
        // de leer un path inexistente; el resto del frame mantiene su
        // semántica (timestamp + tracker).
        let (mem_bytes, nproc) = if is_virtual_pid(pid) {
            (0, 0)
        } else {
            (proc::read_mem_bytes(pid), proc::read_thread_count(pid))
        };
        Ok(TelemetryFrame {
            card_id,
            at: SystemTime::now(),
            mem_bytes,
            nproc,
            // v1: CPU% requiere dos samples espaciados — pendiente.
            cpu_pct: 0.0,
            restarts,
        })
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

    fn wasm_card(label: &str) -> card_core::Card {
        let mut c = card_core::Card::new(label);
        c.payload = card_core::Payload::Wasm {
            module_sha256: [0u8; 32],
            entry: "main".into(),
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

    /// Una Card `Wasm` se rechaza con error tipado, no se intenta
    /// encarnar como si fuera Native (que daría un error de exec
    /// confuso). El caller distingue "no soportado" de "fallé".
    #[tokio::test]
    async fn wasm_card_es_unsupported_payload() {
        let e = LocalEngine::new();
        let card = wasm_card("modulo-wasm");
        let err = e.run(Intent::new(card)).await.unwrap_err();
        match err {
            EngineError::UnsupportedPayload { kind } => assert_eq!(kind, "Wasm"),
            other => panic!("esperaba UnsupportedPayload, fue {other:?}"),
        }
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
