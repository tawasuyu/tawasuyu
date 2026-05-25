//! Detección runtime de capacidades del kernel/proceso para aislamiento.
//!
//! Esto NO se cachea entre instancias — sysctls pueden cambiar entre boot, y
//! cgroup delegation depende del proceso concreto. Cada `Incarnator::new`
//! hace su detección al construirse.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CapabilitySet {
    pub kernel_version: (u32, u32, u32),
    pub has_cap_sys_admin: bool,
    pub user_ns: UserNsStatus,
    pub cgroup_v2: CgroupStatus,
    pub cgroup_delegated: bool,
    pub max_user_namespaces: Option<u64>,
    pub our_cgroup: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserNsStatus {
    Allowed,
    DisabledBySysctl,
    RestrictedByLsm,
    Unknown,
}

impl UserNsStatus {
    pub fn is_allowed(&self) -> bool {
        matches!(self, UserNsStatus::Allowed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CgroupStatus {
    Unified,
    Hybrid,
    Legacy,
    NotMounted,
}

impl CapabilitySet {
    pub fn detect() -> Self {
        Self {
            kernel_version: detect_kernel_version().unwrap_or((0, 0, 0)),
            has_cap_sys_admin: detect_cap_sys_admin(),
            user_ns: detect_user_ns(),
            cgroup_v2: detect_cgroup_status(),
            cgroup_delegated: detect_cgroup_delegated(),
            max_user_namespaces: read_u64("/proc/sys/user/max_user_namespaces"),
            our_cgroup: detect_our_cgroup(),
        }
    }

    /// ¿Podemos crear el namespace `ns`?
    /// Reglas:
    /// - user → necesita user_ns Allowed (o ya tener CAP_SYS_ADMIN, en cuyo caso no se crea uno nuevo).
    /// - resto → CAP_SYS_ADMIN, o crearlos junto con user ns nuevo.
    pub fn can_create_ns(&self, kind: NsKind) -> bool {
        match kind {
            NsKind::User => self.user_ns.is_allowed() || self.has_cap_sys_admin,
            _ => {
                self.has_cap_sys_admin
                    || (self.user_ns.is_allowed() && self.max_user_namespaces.unwrap_or(0) > 0)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NsKind {
    Mount,
    Pid,
    Net,
    Uts,
    Ipc,
    User,
    Cgroup,
}

impl NsKind {
    pub fn name(self) -> &'static str {
        match self {
            NsKind::Mount => "mount",
            NsKind::Pid => "pid",
            NsKind::Net => "net",
            NsKind::Uts => "uts",
            NsKind::Ipc => "ipc",
            NsKind::User => "user",
            NsKind::Cgroup => "cgroup",
        }
    }
}

fn detect_kernel_version() -> Option<(u32, u32, u32)> {
    let s = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let head = s.split(|c: char| !c.is_ascii_digit() && c != '.').next()?;
    let mut it = head.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    Some((a, b, c))
}

fn detect_cap_sys_admin() -> bool {
    // euid 0 implica caps por default. Modo simple: si euid==0, asumimos CAP_SYS_ADMIN.
    // Podríamos parsear /proc/self/status > CapEff, pero para nuestros usos el
    // discriminador útil es root vs no-root.
    nix::unistd::geteuid().is_root()
}

fn detect_user_ns() -> UserNsStatus {
    // Sysctl tradicional Debian/Ubuntu pre-24.
    if let Some(v) = read_u64("/proc/sys/kernel/unprivileged_userns_clone") {
        if v == 0 {
            return UserNsStatus::DisabledBySysctl;
        }
    }
    // AppArmor restriction (Ubuntu 24+). 1 = restringido, 2 = restricción aplicada.
    if let Some(v) = read_u64("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") {
        if v >= 1 {
            return UserNsStatus::RestrictedByLsm;
        }
    }
    if let Some(0) = read_u64("/proc/sys/user/max_user_namespaces") {
        return UserNsStatus::DisabledBySysctl;
    }
    UserNsStatus::Allowed
}

fn detect_cgroup_status() -> CgroupStatus {
    // /sys/fs/cgroup montado como cgroup2 → unified.
    let mounts = match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(s) => s,
        Err(_) => return CgroupStatus::NotMounted,
    };
    let mut has_v2 = false;
    let mut has_v1 = false;
    for line in mounts.lines() {
        // formato: ... - <fstype> <source> <opts>
        let parts: Vec<&str> = line.split(" - ").collect();
        if parts.len() < 2 {
            continue;
        }
        let tail = parts[1];
        let fields: Vec<&str> = tail.split_whitespace().collect();
        if fields.is_empty() {
            continue;
        }
        match fields[0] {
            "cgroup2" => has_v2 = true,
            "cgroup" => has_v1 = true,
            _ => {}
        }
    }
    match (has_v2, has_v1) {
        (true, false) => CgroupStatus::Unified,
        (true, true) => CgroupStatus::Hybrid,
        (false, true) => CgroupStatus::Legacy,
        (false, false) => CgroupStatus::NotMounted,
    }
}

fn detect_our_cgroup() -> Option<PathBuf> {
    let s = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let rel = s.lines().find_map(|l| l.strip_prefix("0::"))?.trim();
    let abs = if rel == "/" {
        PathBuf::from("/sys/fs/cgroup")
    } else {
        PathBuf::from(format!("/sys/fs/cgroup{rel}"))
    };
    Some(abs)
}

fn detect_cgroup_delegated() -> bool {
    // Heurística: ¿podemos escribir cgroup.subtree_control en nuestro cgroup
    // o crear subdirectorios? En cgroup v2 con Delegate=yes, el dueño es el uid
    // del usuario y `access(W_OK)` sobre el directorio devuelve OK.
    let Some(p) = detect_our_cgroup() else { return false };
    use nix::unistd::{access, AccessFlags};
    access(&p, AccessFlags::W_OK).is_ok()
}

fn read_u64(path: &str) -> Option<u64> {
    let s = std::fs::read_to_string(Path::new(path)).ok()?;
    s.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_does_not_panic() {
        let _ = CapabilitySet::detect();
    }

    #[test]
    fn ns_kind_names_unique() {
        let names = [
            NsKind::Mount.name(),
            NsKind::Pid.name(),
            NsKind::Net.name(),
            NsKind::Uts.name(),
            NsKind::Ipc.name(),
            NsKind::User.name(),
            NsKind::Cgroup.name(),
        ];
        let mut sorted = names.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }
}
