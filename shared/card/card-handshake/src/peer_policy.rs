//! Política de admisión de peers libp2p: allowlist + denylist con hot
//! reload opcional.
//!
//! Capa de política sobre el trust criptográfico de Fase 3. Combina:
//!
//! - **Denylist**: peers explícitamente baneados. Si está, deny gana.
//! - **Allowlist**: si está set, sólo los peers listados pasan.
//!   Si no está set, modo abierto (todo peer Ed25519-válido pasa,
//!   sujeto sólo a denylist).
//!
//! Sin denylist y sin allowlist → modo totalmente abierto (compat
//! con todo lo anterior). Con allowlist y denylist a la vez, el
//! orden de evaluación es: deny first → allow check → admit.
//!
//! Aplica únicamente al path libp2p — el path Unix usa SO_PEERCRED
//! del kernel para autenticación local, no PeerId.
//!
//! ## Hot reload
//!
//! Si la política se construyó con [`PeerPolicy::watch_files`], un
//! thread dedicado vigila los archivos de allow/deny vía `notify`.
//! Cualquier cambio (write, create, modify, remove) dispara una
//! recarga atómica con debounce de 250ms (los editores típicos
//! producen varios eventos por save).
//!
//! Errores de reload (parse fallido, archivo eliminado) se loggean
//! pero NO bajan la política existente — aceptamos la versión
//! anterior hasta que el archivo vuelva a parsearse limpio. Esto
//! evita que un error de tipeo deje al Init en modo inseguro.
//!
//! ## Formato del archivo
//!
//! Idéntico para allow y deny: PeerId base58 por línea, `#` para
//! comentarios (línea entera o inline), líneas vacías ignoradas.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use brahman_net::{BrahmanNet, PeerId};
use tracing::{debug, info, warn};

/// Política de admisión combinada (allow + deny). Clone barato (todos
/// los campos son Arc o referencias inmutables).
#[derive(Clone)]
pub struct PeerPolicy {
    inner: Arc<RwLock<PolicyInner>>,
    paths: Arc<PolicyPaths>,
    /// `BrahmanNet` opcional asociado vía [`Self::attach_to_net`].
    /// Si está set, cada cambio en la denylist se sincroniza con el
    /// `block_list` behaviour del swarm — los peers baneados son
    /// rechazados ANTES del Noise handshake. `RwLock<Option<...>>`
    /// para que `attach_to_net` se pueda llamar después del
    /// constructor (típico en ente-zero: primero arma la policy,
    /// después el net, después attach).
    net: Arc<RwLock<Option<Arc<BrahmanNet>>>>,
}

#[derive(Default)]
struct PolicyInner {
    /// `Some(set)`: sólo peers en el set pasan. `None`: modo abierto.
    allow: Option<BTreeSet<PeerId>>,
    /// Peers baneados. Vacío = sin denylist.
    deny: BTreeSet<PeerId>,
}

#[derive(Default)]
struct PolicyPaths {
    allow_path: Option<PathBuf>,
    deny_path: Option<PathBuf>,
}

/// Decisión del gate de política para un peer dado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// El peer es admitido (no está en deny y, si hay allow, está en allow).
    Admit,
    /// El peer está explícitamente en la denylist.
    DeniedByDenylist,
    /// Hay allowlist configurada y el peer no está en ella.
    NotInAllowlist,
}

impl Decision {
    pub fn is_admitted(self) -> bool {
        matches!(self, Decision::Admit)
    }

    pub fn reason(self) -> &'static str {
        match self {
            Decision::Admit => "admit",
            Decision::DeniedByDenylist => "explicitly denied",
            Decision::NotInAllowlist => "not in allowlist",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("leer política en {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("línea {line_no} de {path}: PeerId inválido '{value}'")]
    InvalidPeerId {
        path: PathBuf,
        line_no: usize,
        value: String,
    },
}

impl PeerPolicy {
    /// Política totalmente abierta: todo peer pasa. Útil como default
    /// cuando no hay archivos configurados.
    pub fn open() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PolicyInner::default())),
            paths: Arc::new(PolicyPaths::default()),
            net: Arc::new(RwLock::new(None)),
        }
    }

    /// Construye una política inline con sets explícitos. Sin paths
    /// asociados, así que `reload` y `watch_files` son no-ops.
    pub fn from_sets(allow: Option<BTreeSet<PeerId>>, deny: BTreeSet<PeerId>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(PolicyInner { allow, deny })),
            paths: Arc::new(PolicyPaths::default()),
            net: Arc::new(RwLock::new(None)),
        }
    }

    /// Carga política desde archivos. Cada path es opcional: `None`
    /// significa "esa lista no aplica" (allow=None ⇒ modo abierto;
    /// deny=None ⇒ sin baneados). Asocia los paths internamente para
    /// que `reload` y `watch_files` los re-lean.
    pub fn from_files(
        allow_path: Option<&Path>,
        deny_path: Option<&Path>,
    ) -> Result<Self, PolicyError> {
        let allow = match allow_path {
            Some(p) => Some(parse_peer_set(p)?),
            None => None,
        };
        let deny = match deny_path {
            Some(p) => parse_peer_set(p)?,
            None => BTreeSet::new(),
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(PolicyInner { allow, deny })),
            paths: Arc::new(PolicyPaths {
                allow_path: allow_path.map(Path::to_path_buf),
                deny_path: deny_path.map(Path::to_path_buf),
            }),
            net: Arc::new(RwLock::new(None)),
        })
    }

    /// Evalúa si `peer` puede registrarse. Toma read lock — barato,
    /// concurrente, sin awaits.
    pub fn evaluate(&self, peer: &PeerId) -> Decision {
        let inner = match self.inner.read() {
            Ok(g) => g,
            Err(_) => {
                // Lock envenenado: degrada a "deny por seguridad".
                warn!("policy lock envenenado — deny por defecto");
                return Decision::DeniedByDenylist;
            }
        };
        if inner.deny.contains(peer) {
            return Decision::DeniedByDenylist;
        }
        if let Some(allow) = &inner.allow {
            if !allow.contains(peer) {
                return Decision::NotInAllowlist;
            }
        }
        Decision::Admit
    }

    /// Tamaño actual de cada lista, para logging. Tupla `(allow_count,
    /// deny_count)`. `allow_count = None` significa "modo abierto"
    /// (sin allowlist).
    pub fn sizes(&self) -> (Option<usize>, usize) {
        match self.inner.read() {
            Ok(g) => (g.allow.as_ref().map(|s| s.len()), g.deny.len()),
            Err(_) => (Some(0), 0),
        }
    }

    /// Recarga atómica desde los paths asociados. Si un archivo
    /// falla, la versión anterior persiste y el error se devuelve.
    /// Esto evita que un typo en el archivo deje al Init en modo
    /// inseguro.
    ///
    /// Si hay un `BrahmanNet` attached vía [`Self::attach_to_net`],
    /// el cambio de denylist se sincroniza con el `block_list` del
    /// swarm: se calcula el diff (added/removed) y se aplican
    /// `block_peer`/`unblock_peer` por cada cambio.
    pub fn reload(&self) -> Result<(), PolicyError> {
        let new_allow = match &self.paths.allow_path {
            Some(p) => Some(parse_peer_set(p)?),
            None => None,
        };
        let new_deny = match &self.paths.deny_path {
            Some(p) => parse_peer_set(p)?,
            None => BTreeSet::new(),
        };
        // Snapshot de la deny actual ANTES de mutar, para diff.
        let prev_deny = self
            .inner
            .read()
            .map(|g| g.deny.clone())
            .unwrap_or_default();
        if let Ok(mut inner) = self.inner.write() {
            inner.allow = new_allow;
            inner.deny = new_deny.clone();
        }
        self.sync_deny_to_swarm(&prev_deny, &new_deny);
        Ok(())
    }

    /// Asocia esta política a un `BrahmanNet`. Sincroniza el snapshot
    /// actual de la denylist con el `block_list` behaviour del swarm
    /// (cada peer baneado se rechaza ANTES del Noise handshake), y
    /// registra el net para re-sincronizarse en cada [`Self::reload`].
    ///
    /// Si ya había un net attached, se reemplaza (caso esperado:
    /// un Init no debería tener dos `BrahmanNet`s).
    pub fn attach_to_net(&self, net: Arc<BrahmanNet>) {
        // Sync inicial: bloquear todos los peers actualmente en deny.
        if let Ok(inner) = self.inner.read() {
            for peer in &inner.deny {
                net.block_peer(*peer);
            }
        }
        if let Ok(mut slot) = self.net.write() {
            *slot = Some(net);
        }
    }

    /// Calcula el diff entre `prev` y `new` y aplica
    /// `block_peer`/`unblock_peer` al net asociado (si hay).
    /// No-op si no hay net attached.
    fn sync_deny_to_swarm(&self, prev: &BTreeSet<PeerId>, new: &BTreeSet<PeerId>) {
        let net = match self.net.read() {
            Ok(g) => match g.as_ref() {
                Some(n) => n.clone(),
                None => return,
            },
            Err(_) => return,
        };
        for added in new.difference(prev) {
            net.block_peer(*added);
        }
        for removed in prev.difference(new) {
            net.unblock_peer(*removed);
        }
    }

    /// Arranca un thread que vigila los archivos asociados con
    /// `notify` y llama [`Self::reload`] cuando cambian. Debounce
    /// 250ms para coalescer múltiples eventos por save (los editores
    /// hacen Create+Modify+más).
    ///
    /// Devuelve un `JoinHandle` que el caller debe mantener vivo.
    /// Drop del handle no detiene el thread (notify watcher es
    /// sticky); para detener, terminar el proceso.
    ///
    /// No-op si no hay paths asociados (devuelve un handle dummy
    /// que termina inmediatamente).
    pub fn spawn_watcher(&self) -> std::io::Result<std::thread::JoinHandle<()>> {
        let allow_path = self.paths.allow_path.clone();
        let deny_path = self.paths.deny_path.clone();
        let policy = self.clone();

        if allow_path.is_none() && deny_path.is_none() {
            // Sin archivos a vigilar: spawn un thread que termina ya.
            return std::thread::Builder::new()
                .name("brahman-policy-watcher-noop".into())
                .spawn(|| {});
        }

        std::thread::Builder::new()
            .name("brahman-policy-watcher".into())
            .spawn(move || {
                run_watcher(policy, allow_path, deny_path);
            })
    }
}

impl std::fmt::Debug for PeerPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (allow, deny) = self.sizes();
        f.debug_struct("PeerPolicy")
            .field("allow", &allow)
            .field("deny", &deny)
            .field("allow_path", &self.paths.allow_path)
            .field("deny_path", &self.paths.deny_path)
            .finish()
    }
}

fn parse_peer_set(path: &Path) -> Result<BTreeSet<PeerId>, PolicyError> {
    let contents = std::fs::read_to_string(path).map_err(|e| PolicyError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut out = BTreeSet::new();
    for (idx, raw) in contents.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let peer = trimmed
            .parse::<PeerId>()
            .map_err(|_| PolicyError::InvalidPeerId {
                path: path.to_path_buf(),
                line_no,
                value: trimmed.to_string(),
            })?;
        out.insert(peer);
    }
    Ok(out)
}

const DEBOUNCE_MS: u64 = 250;

fn run_watcher(
    policy: PeerPolicy,
    allow_path: Option<PathBuf>,
    deny_path: Option<PathBuf>,
) {
    use notify::{RecursiveMode, Watcher};

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            warn!(?e, "notify watcher para policy no se pudo crear — hot reload deshabilitado");
            return;
        }
    };

    // Vigilamos los DIRECTORIOS de los archivos, no los archivos
    // directos. Los editores típicos hacen rename-and-replace (escriben
    // a tmp, rename al destino), lo que rompe el watch del archivo
    // pero NO el del directorio. Trade-off: recibimos más eventos
    // (cualquier archivo del dir), filtramos por path al procesar.
    for p in [&allow_path, &deny_path].iter().filter_map(|x| x.as_ref()) {
        if let Some(parent) = p.parent() {
            if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                warn!(path = %parent.display(), ?e, "watch failed");
            }
        }
    }

    let debounce = Duration::from_millis(DEBOUNCE_MS);
    let mut pending_at: Option<Instant> = None;

    loop {
        let timeout = match pending_at {
            Some(at) => debounce.saturating_sub(at.elapsed()).max(Duration::from_millis(10)),
            None => Duration::from_secs(60), // wakeup periódico, no esencial
        };

        match rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                // Sólo nos interesan eventos sobre los paths exactos.
                let touches_us = event.paths.iter().any(|p| {
                    Some(p) == allow_path.as_ref() || Some(p) == deny_path.as_ref()
                });
                if !touches_us {
                    continue;
                }
                debug!(?event.kind, "policy file event recibido — debounce");
                pending_at = Some(Instant::now());
            }
            Ok(Err(e)) => {
                warn!(?e, "notify error en policy watcher");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Some(at) = pending_at {
                    if at.elapsed() >= debounce {
                        match policy.reload() {
                            Ok(()) => {
                                let (a, d) = policy.sizes();
                                info!(
                                    allow = ?a,
                                    deny = d,
                                    "policy hot-reload completo"
                                );
                            }
                            Err(e) => {
                                warn!(?e, "policy hot-reload falló — manteniendo versión anterior");
                            }
                        }
                        pending_at = None;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                warn!("policy watcher channel cerrado — terminando thread");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brahman_net::Keypair;
    use tempfile::TempDir;

    fn fresh_peer() -> PeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    #[test]
    fn open_admits_anyone() {
        let p = PeerPolicy::open();
        assert_eq!(p.evaluate(&fresh_peer()), Decision::Admit);
    }

    #[test]
    fn allow_only_admits_listed() {
        let p1 = fresh_peer();
        let p2 = fresh_peer();
        let policy = PeerPolicy::from_sets(
            Some([p1].into_iter().collect()),
            BTreeSet::new(),
        );
        assert_eq!(policy.evaluate(&p1), Decision::Admit);
        assert_eq!(policy.evaluate(&p2), Decision::NotInAllowlist);
    }

    #[test]
    fn deny_overrides_open() {
        let p1 = fresh_peer();
        let p2 = fresh_peer();
        let policy = PeerPolicy::from_sets(None, [p1].into_iter().collect());
        assert_eq!(policy.evaluate(&p1), Decision::DeniedByDenylist);
        assert_eq!(policy.evaluate(&p2), Decision::Admit);
    }

    #[test]
    fn deny_overrides_allow() {
        // Conflicto explícito: p1 está en ambas. Deny gana.
        let p1 = fresh_peer();
        let policy = PeerPolicy::from_sets(
            Some([p1].into_iter().collect()),
            [p1].into_iter().collect(),
        );
        assert_eq!(policy.evaluate(&p1), Decision::DeniedByDenylist);
    }

    #[test]
    fn from_files_with_both_lists() {
        let p1 = fresh_peer();
        let p2 = fresh_peer();
        let p3 = fresh_peer();
        let tmp = TempDir::new().unwrap();
        let allow = tmp.path().join("allow.txt");
        let deny = tmp.path().join("deny.txt");
        std::fs::write(&allow, format!("{}\n{}\n", p1, p2)).unwrap();
        std::fs::write(&deny, format!("# baneado\n{}\n", p2)).unwrap();
        let policy = PeerPolicy::from_files(Some(&allow), Some(&deny)).unwrap();
        assert_eq!(policy.evaluate(&p1), Decision::Admit);
        assert_eq!(policy.evaluate(&p2), Decision::DeniedByDenylist); // deny gana
        assert_eq!(policy.evaluate(&p3), Decision::NotInAllowlist);
    }

    #[test]
    fn from_files_only_deny() {
        let p1 = fresh_peer();
        let p2 = fresh_peer();
        let tmp = TempDir::new().unwrap();
        let deny = tmp.path().join("deny.txt");
        std::fs::write(&deny, format!("{}\n", p1)).unwrap();
        let policy = PeerPolicy::from_files(None, Some(&deny)).unwrap();
        assert_eq!(policy.evaluate(&p1), Decision::DeniedByDenylist);
        assert_eq!(policy.evaluate(&p2), Decision::Admit);
    }

    #[test]
    fn reload_picks_up_changes() {
        let p1 = fresh_peer();
        let p2 = fresh_peer();
        let tmp = TempDir::new().unwrap();
        let allow = tmp.path().join("allow.txt");
        std::fs::write(&allow, format!("{}\n", p1)).unwrap();

        let policy = PeerPolicy::from_files(Some(&allow), None).unwrap();
        assert_eq!(policy.evaluate(&p1), Decision::Admit);
        assert_eq!(policy.evaluate(&p2), Decision::NotInAllowlist);

        // Mutar el archivo: ahora p2 está, p1 no.
        std::fs::write(&allow, format!("{}\n", p2)).unwrap();
        policy.reload().unwrap();
        assert_eq!(policy.evaluate(&p1), Decision::NotInAllowlist);
        assert_eq!(policy.evaluate(&p2), Decision::Admit);
    }

    #[test]
    fn reload_failure_preserves_previous_state() {
        let p1 = fresh_peer();
        let tmp = TempDir::new().unwrap();
        let allow = tmp.path().join("allow.txt");
        std::fs::write(&allow, format!("{}\n", p1)).unwrap();
        let policy = PeerPolicy::from_files(Some(&allow), None).unwrap();
        assert_eq!(policy.evaluate(&p1), Decision::Admit);

        // Romper el archivo con basura.
        std::fs::write(&allow, "this-is-not-a-peer-id\n").unwrap();
        let err = policy.reload();
        assert!(err.is_err(), "reload con typo debe fallar");

        // Estado anterior se mantiene.
        assert_eq!(
            policy.evaluate(&p1),
            Decision::Admit,
            "policy debe conservar la versión anterior tras fallo de reload"
        );
    }

    #[test]
    fn invalid_file_rejected_at_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.txt");
        std::fs::write(&path, "not-a-peer-id\n").unwrap();
        let err = PeerPolicy::from_files(Some(&path), None).unwrap_err();
        assert!(matches!(err, PolicyError::InvalidPeerId { .. }));
    }

    #[test]
    fn watcher_reloads_on_file_change() {
        // Test integración del watcher: arma policy con file, spawn
        // watcher, modifica el archivo, espera el debounce, verifica
        // que la policy refleja el cambio.
        let p1 = fresh_peer();
        let p2 = fresh_peer();
        let tmp = TempDir::new().unwrap();
        let allow = tmp.path().join("allow.txt");
        std::fs::write(&allow, format!("{}\n", p1)).unwrap();

        let policy = PeerPolicy::from_files(Some(&allow), None).unwrap();
        let _watcher = policy.spawn_watcher().unwrap();

        // Le damos un instante al watcher para subscribirse al dir.
        std::thread::sleep(Duration::from_millis(100));

        // Mutamos el archivo: p2 reemplaza a p1.
        std::fs::write(&allow, format!("{}\n", p2)).unwrap();

        // Esperamos > debounce + margen.
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if policy.evaluate(&p2) == Decision::Admit {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        assert_eq!(
            policy.evaluate(&p2),
            Decision::Admit,
            "watcher debería haber recargado la policy"
        );
        assert_eq!(
            policy.evaluate(&p1),
            Decision::NotInAllowlist,
            "p1 debería haber salido tras el reload"
        );
    }
}
