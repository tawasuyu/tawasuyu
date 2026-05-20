use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum IncarnateError {
    #[error("namespace `{ns}` requires CAP_SYS_ADMIN or CLONE_NEWUSER (neither available)")]
    NamespaceCapMissing { ns: &'static str },

    #[error("user namespaces blocked by sysctl kernel.unprivileged_userns_clone=0")]
    UserNsDisabledBySysctl,

    #[error("user namespaces restricted by LSM (apparmor/selinux)")]
    UserNsRestrictedByLsm,

    #[error("cgroup path `{path}` is not writable (delegation missing?)")]
    CgroupNotWritable { path: PathBuf },

    #[error("payload is not executable in this incarnation path (Wasm/Virtual not supported here)")]
    NonExecutablePayload,

    #[error("clone(2) failed: {0}")]
    Clone(#[source] nix::errno::Errno),

    #[error("pipe2(2) failed: {0}")]
    Pipe(#[source] nix::errno::Errno),

    #[error("post-clone setup: {0}")]
    PostClone(#[source] anyhow::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("invalid argv: contains NUL byte")]
    InvalidArgv,
}

/// Cuando `strict_caps = false`, errores no-fatales se reportan como
/// `Degradation` y la encarnación continúa con menos aislamiento del pedido.
#[derive(Debug, Clone)]
pub enum Degradation {
    NamespaceSkipped { ns: &'static str },
    CgroupSkipped { path: PathBuf, reason: String },
    CpuAffinitySkipped { reason: String },
    UidMapFailed { reason: String },
}
