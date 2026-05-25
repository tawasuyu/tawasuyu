//! `ente-incarnate` — rutina extraída del Init para encarnar Cards en
//! procesos aislados (clone(2) + namespaces + cgroup + rlimits + cpu affinity).
//!
//! El núcleo histórico vivía en `ente-soma` con globals dependientes de PID 1.
//! Este crate elimina esos globals: se construye un [`Incarnator`] por
//! supervisor (Init, shuma, etc.), cada uno con su propio bus socket y su
//! propia política de capacidades.
//!
//! ## Limitaciones que NO desaparecen al extraer
//!
//! 1. `mount/pid/net/uts/ipc/cgroup` namespaces requieren `CAP_SYS_ADMIN`
//!    o estar combinados con `CLONE_NEWUSER` en el mismo `clone(2)`.
//! 2. `user` namespace puede estar bloqueado por
//!    `kernel.unprivileged_userns_clone=0` o por LSM (apparmor/selinux).
//! 3. cgroups v2 requieren delegación (sistemas modernos: systemd
//!    `Delegate=yes`). Sin delegación, escribir en `/sys/fs/cgroup` falla.
//! 4. El primer proceso de un PID namespace es PID 1 *de ese ns*; si muere
//!    el kernel mata el namespace entero.
//!
//! [`CapabilitySet::detect`] reporta lo que está disponible runtime;
//! [`Incarnator::dry_run`] valida un [`Card`] antes de ejecutar.

#![doc(html_no_source)]

pub mod caps;
pub mod cgroup;
pub mod child;
pub mod env;
pub mod error;
pub mod namespaced;
pub mod plain;
pub mod pre_exec;

pub use card_core::Card;
pub use caps::{CapabilitySet, CgroupStatus, NsKind, UserNsStatus};
pub use env::{EnvSpec, ENV_BUS_SOCK, ENV_ENTE_ID};
pub use error::{Degradation, IncarnateError};
pub use pre_exec::{ChildPreExec, ChildSetup};

use std::os::fd::RawFd;

/// Redirección declarativa de stdio del hijo. Cada `Some(fd)` se `dup2`-ea
/// como stdin/stdout/stderr en el hijo.
///
/// **Contrato de ownership**: el caller transfiere ownership de los FDs al
/// `Incarnator` (igual que pasaría a `Command::stdio(Stdio::from_raw_fd)`).
/// `Incarnator` se encarga de cerrarlos en el padre tras `incarnate` (path
/// namespaced) o de dejar que `std::process::Command` los absorba (path
/// plain). **No los cierres en el caller** — habría doble-close.
///
/// Útil para conectar pipes entre procesos del pipeline de shuma sin
/// romper la regla async-signal-safe del callback de clone(2).
#[derive(Debug, Clone, Copy, Default)]
pub struct ChildStdio {
    pub stdin_fd: Option<RawFd>,
    pub stdout_fd: Option<RawFd>,
    pub stderr_fd: Option<RawFd>,
}

impl ChildStdio {
    pub fn is_some(&self) -> bool {
        self.stdin_fd.is_some() || self.stdout_fd.is_some() || self.stderr_fd.is_some()
    }
}

use nix::unistd::Pid;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct IncarnatorConfig {
    /// Path del Unix socket del bus interno (se inyecta como `ENTE_BUS_SOCK`).
    /// `None` = no inyectar.
    pub bus_sock: Option<PathBuf>,

    /// Inyectar `NOTIFY_SOCKET` (legacy sd_notify). Default `None`.
    /// `ente-zero` lo pasa = `Some("/run/systemd/notify")`.
    pub notify_socket: Option<PathBuf>,

    /// Vars adicionales que el caller fuerza en cada hijo.
    pub extra_env: Vec<(String, String)>,

    /// Si `true`, falta de capacidades aborta `incarnate()` con error.
    /// Si `false`, se reportan como `Degradation` y la encarnación continúa
    /// con menos aislamiento (semántica histórica del Init).
    pub strict_caps: bool,
}

pub struct Incarnator {
    cfg: IncarnatorConfig,
    caps: CapabilitySet,
}

#[derive(Debug, Clone)]
pub struct IncarnateOutcome {
    pub pid: Pid,
    pub degradations: Vec<Degradation>,
}

#[derive(Debug, Default, Clone)]
pub struct ValidationReport {
    pub will_work: bool,
    pub blocking: Vec<String>,
    pub warnings: Vec<String>,
}

impl Incarnator {
    pub fn new(cfg: IncarnatorConfig) -> Self {
        Self {
            caps: CapabilitySet::detect(),
            cfg,
        }
    }

    /// Constructor para testing/inyección de capacidades pre-calculadas.
    pub fn with_caps(cfg: IncarnatorConfig, caps: CapabilitySet) -> Self {
        Self { cfg, caps }
    }

    pub fn capabilities(&self) -> &CapabilitySet {
        &self.caps
    }

    pub fn config(&self) -> &IncarnatorConfig {
        &self.cfg
    }

    /// Valida una Card sin ejecutar nada. Útil para que el caller (shuma,
    /// admin, tests) sepa de antemano si va a poder encarnar tal cual o si
    /// va a tener que aflojar el SomaSpec.
    pub fn dry_run(&self, card: &Card) -> ValidationReport {
        let mut r = ValidationReport {
            will_work: true,
            ..Default::default()
        };
        let ns = &card.soma.namespaces;

        // Si user_ns está pedido, evaluar su disponibilidad.
        if ns.user {
            match self.caps.user_ns {
                UserNsStatus::DisabledBySysctl => {
                    r.blocking.push(
                        "user namespace requested but kernel.unprivileged_userns_clone=0".into(),
                    );
                    r.will_work = false;
                }
                UserNsStatus::RestrictedByLsm => {
                    r.blocking.push(
                        "user namespace restricted by LSM (apparmor/selinux)".into(),
                    );
                    r.will_work = false;
                }
                _ => {}
            }
        }

        // El resto de namespaces necesitan CAP_SYS_ADMIN o user ns.
        let needs_priv = [
            (ns.mount, NsKind::Mount),
            (ns.pid, NsKind::Pid),
            (ns.net, NsKind::Net),
            (ns.uts, NsKind::Uts),
            (ns.ipc, NsKind::Ipc),
            (ns.cgroup, NsKind::Cgroup),
        ];
        for (wanted, kind) in needs_priv {
            if wanted && !self.caps.can_create_ns(kind) {
                r.blocking.push(format!(
                    "{} namespace requires CAP_SYS_ADMIN or user ns (neither available)",
                    kind.name()
                ));
                r.will_work = false;
            }
        }

        // Cgroup: si el card pide path, chequear que tengamos delegación.
        if !card.soma.cgroup.path.is_empty() && !self.caps.cgroup_delegated {
            r.warnings.push(format!(
                "cgroup `{}` requested but our cgroup is not writable (delegation missing)",
                card.soma.cgroup.path
            ));
        }

        // Payload ejecutable.
        use card_core::Payload;
        if !matches!(card.payload, Payload::Native { .. } | Payload::Legacy { .. }) {
            r.blocking
                .push("payload is not Native/Legacy (use ente-wasm for Wasm)".into());
            r.will_work = false;
        }

        r
    }

    /// Encarna la Card. Si `strict_caps`, valida primero y aborta ante
    /// blocking. Si no, ejecuta y deja que las degradaciones se acumulen.
    pub fn incarnate(&self, card: &Card) -> Result<IncarnateOutcome, IncarnateError> {
        self.incarnate_with(card, ChildStdio::default())
    }

    /// Variante con redirección de stdio declarativa. Útil para conectar
    /// pipes entre procesos (caso: pipeline aislado).
    pub fn incarnate_with(
        &self,
        card: &Card,
        stdio: ChildStdio,
    ) -> Result<IncarnateOutcome, IncarnateError> {
        self.incarnate_full(card, stdio, ChildSetup::default())
    }

    /// Variante full: stdio + setup pre-execve.
    pub fn incarnate_full(
        &self,
        card: &Card,
        stdio: ChildStdio,
        setup: ChildSetup,
    ) -> Result<IncarnateOutcome, IncarnateError> {
        if self.cfg.strict_caps {
            let v = self.dry_run(card);
            if !v.will_work {
                // Mapeamos el primer blocking a IncarnateError tipado.
                if let Some(first) = v.blocking.first() {
                    if first.contains("unprivileged_userns_clone") {
                        return Err(IncarnateError::UserNsDisabledBySysctl);
                    }
                    if first.contains("LSM") {
                        return Err(IncarnateError::UserNsRestrictedByLsm);
                    }
                    if let Some(ns) = which_ns_blocking(first) {
                        return Err(IncarnateError::NamespaceCapMissing { ns });
                    }
                    if first.contains("payload") {
                        return Err(IncarnateError::NonExecutablePayload);
                    }
                }
            }
        }

        let env_spec = EnvSpec {
            bus_sock: self.cfg.bus_sock.clone(),
            notify_socket: self.cfg.notify_socket.clone(),
            extra: self.cfg.extra_env.clone(),
        };

        let mut degradations = Vec::new();
        let pid = if namespaced::needs_namespacing(&card.soma.namespaces) {
            namespaced::incarnate_namespaced(card, &env_spec, &stdio, &setup, &mut degradations)?
        } else {
            plain::incarnate_plain(card, &env_spec, &stdio, &setup)?
        };
        Ok(IncarnateOutcome { pid, degradations })
    }
}

fn which_ns_blocking(msg: &str) -> Option<&'static str> {
    for n in ["mount", "pid", "net", "uts", "ipc", "user", "cgroup"] {
        if msg.starts_with(n) {
            return Some(match n {
                "mount" => "mount",
                "pid" => "pid",
                "net" => "net",
                "uts" => "uts",
                "ipc" => "ipc",
                "user" => "user",
                "cgroup" => "cgroup",
                _ => unreachable!(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::{Card, NamespaceSet, Payload};

    fn make_card(payload: Payload, ns: NamespaceSet) -> Card {
        let mut c = Card::new("test");
        c.payload = payload;
        c.soma.namespaces = ns;
        c
    }

    #[test]
    fn dry_run_native_no_ns_works() {
        let inc = Incarnator::new(IncarnatorConfig::default());
        let card = make_card(
            Payload::Native {
                exec: "/bin/true".into(),
                argv: vec![],
                envp: vec![],
            },
            NamespaceSet::default(),
        );
        let r = inc.dry_run(&card);
        assert!(r.will_work, "{:?}", r);
    }

    #[test]
    fn dry_run_wasm_payload_blocks() {
        let inc = Incarnator::new(IncarnatorConfig::default());
        let card = make_card(
            Payload::Wasm {
                module_sha256: [0u8; 32],
                entry: "main".into(),
            },
            NamespaceSet::default(),
        );
        let r = inc.dry_run(&card);
        assert!(!r.will_work);
        assert!(r.blocking.iter().any(|m| m.contains("payload")));
    }

    /// Smoke: redirección stdout via ChildStdio en path plain.
    /// Lanza /bin/echo con stdout conectado a un pipe que leemos.
    #[test]
    fn incarnate_with_stdout_redirection_captures_output() {
        use nix::fcntl::OFlag;
        use nix::unistd::{pipe2, read};
        use std::os::fd::{AsRawFd, IntoRawFd};

        let inc = Incarnator::new(IncarnatorConfig::default());
        let card = make_card(
            Payload::Native {
                exec: "/bin/echo".into(),
                argv: vec!["shuma-stdio".into()],
                envp: vec![],
            },
            NamespaceSet::default(),
        );

        let (r, w) = pipe2(OFlag::empty()).expect("pipe");
        let w_raw = w.into_raw_fd();
        let r_raw = r.as_raw_fd();

        let stdio = ChildStdio {
            stdin_fd: None,
            stdout_fd: Some(w_raw),
            stderr_fd: None,
        };
        let out = inc.incarnate_with(&card, stdio).expect("incarnate");

        // Cerramos nuestro extremo de write (el hijo tiene su dup2).
        // Plain path: Command toma ownership y cierra al spawn.
        // Namespaced path: el padre todavía tiene una copia... pero en plain
        // no aplica. Para este test usamos plain (NamespaceSet vacío).

        // Cosechamos para no zombi.
        let _ = nix::sys::wait::waitpid(out.pid, None);

        // Leemos la salida.
        let mut buf = [0u8; 64];
        let n = read(r_raw, &mut buf).expect("read");
        assert!(n > 0);
        let s = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(s.contains("shuma-stdio"), "got: {s:?}");
        // r se cierra al drop del OwnedFd.
    }

    /// child_pre_exec aplica chdir + NoNewPrivs en path plain.
    #[test]
    fn child_pre_exec_chdir_changes_pwd() {
        use crate::{ChildPreExec, ChildSetup};
        use nix::fcntl::OFlag;
        use nix::unistd::{pipe2, read};
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, IntoRawFd};

        let inc = Incarnator::new(IncarnatorConfig::default());
        // Comando: /bin/pwd. Si chdir funciona, output = /tmp.
        let card = make_card(
            Payload::Native {
                exec: "/bin/pwd".into(),
                argv: vec![],
                envp: vec![],
            },
            NamespaceSet::default(),
        );

        let (r, w) = pipe2(OFlag::empty()).expect("pipe");
        let w_raw = w.into_raw_fd();
        let r_raw = r.as_raw_fd();

        let stdio = ChildStdio {
            stdin_fd: None,
            stdout_fd: Some(w_raw),
            stderr_fd: None,
        };
        let setup = ChildSetup::new()
            .with(ChildPreExec::Chdir(CString::new("/tmp").unwrap()))
            .with(ChildPreExec::NoNewPrivs);
        let out = inc.incarnate_full(&card, stdio, setup).expect("incarnate");

        let _ = nix::sys::wait::waitpid(out.pid, None);

        let mut buf = [0u8; 64];
        let n = read(r_raw, &mut buf).expect("read");
        let s = std::str::from_utf8(&buf[..n]).unwrap();
        assert!(s.starts_with("/tmp"), "pwd output was: {s:?}");
    }

    /// Smoke: encarnar /bin/true sin ns. No requiere root.
    #[test]
    fn incarnate_plain_true_succeeds() {
        let inc = Incarnator::new(IncarnatorConfig::default());
        let card = make_card(
            Payload::Native {
                exec: "/bin/true".into(),
                argv: vec![],
                envp: vec![],
            },
            NamespaceSet::default(),
        );
        let out = inc.incarnate(&card).expect("plain incarnation");
        assert!(out.pid.as_raw() > 0);
        // Cosechamos para no dejar zombi.
        let _ = nix::sys::wait::waitpid(out.pid, None);
    }
}
