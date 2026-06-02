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

pub use interactive::{Attachment, PtySize};

use arje_incarnate::{Incarnator, IncarnatorConfig};
use async_trait::async_trait;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use sandokan_core::{Engine, EngineError, ExecHandle, Intent, TelemetryFrame};
use sandokan_lifecycle::LifecycleState;
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
    pub fn with_run_dir(cfg: IncarnatorConfig, run_dir: PathBuf) -> Self {
        Self {
            base_cfg: cfg,
            registry: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            run_dir,
        }
    }

    /// Path canónico del socket de una sesión interactiva. El front se conecta
    /// **siempre** acá por `card_id`, sin conocer quién atiende detrás.
    pub fn session_socket_path(&self, card_id: Ulid) -> PathBuf {
        self.run_dir.join(format!("{card_id}.sock"))
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

#[async_trait]
impl Engine for LocalEngine {
    async fn run(&self, intent: Intent) -> Result<ExecHandle, EngineError> {
        let card_id = intent.card_id();
        let label = intent.card.label.clone();

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
            if !ent.state.is_terminal() {
                if let Some(new_state) = reap(ent.pid) {
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
        if !ent.state.is_terminal() {
            if let Some(new_state) = reap(ent.pid) {
                ent.state = new_state;
            }
        }
        Ok(ent.state.clone())
    }

    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError> {
        let pid = {
            let reg = self.registry.lock().expect("registry lock");
            reg.get(&card_id)
                .map(|e| e.pid)
                .ok_or(EngineError::NotFound(card_id))?
        };
        Ok(TelemetryFrame {
            card_id,
            at: SystemTime::now(),
            mem_bytes: proc::read_mem_bytes(pid),
            nproc: proc::read_thread_count(pid),
            // v1: CPU% requiere dos samples espaciados — pendiente.
            cpu_pct: 0.0,
            // LocalEngine aún no trackea restarts (lo haría vía
            // sandokan-lifecycle::RestartTracker) — pendiente.
            restarts: 0,
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
}
