//! Persistencia del estado del WorkspaceManager.
//!
//! v1: sólo `WorkspaceSpec`s vivos. Los comandos (PIDs) NO se persisten —
//! el kernel los mata al cerrar el daemon. Sólo la *intención declarada*
//! (Workspaces creados con su spec) sobrevive a un reboot del daemon.

use crate::WorkspaceManager;
use serde::{Deserialize, Serialize};
use shuma_card::{PipelineSpec, WorkspaceId, WorkspaceSpec};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// v2 agregó `saved_pipelines`. v3 agrega `live_pipelines`. v4 agrega
/// `stats_history` por workspace (sparkline survives daemon restart).
/// Versiones inferiores leen campos ausentes como vacío.
pub const SNAPSHOT_VERSION: u16 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipoteSnapshot {
    pub version: u16,
    pub timestamp_ms: u64,
    pub workspaces: Vec<WorkspaceEntry>,
    #[serde(default)]
    pub saved_pipelines: Vec<PipelineEntry>,
    /// Pipelines vivos con supervisor (`restart_on_failure=true`) al
    /// momento del snapshot. El daemon los relanza al restore.
    #[serde(default)]
    pub live_pipelines: Vec<LivePipelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: WorkspaceId,
    pub spec: WorkspaceSpec,
    /// Stats history persistida — cap reasonable para no inflar el JSON.
    /// Sólo se guardan campos serializables (no Instant).
    #[serde(default)]
    pub stats_history: Vec<PersistedStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedStats {
    pub commands_alive: u32,
    pub commands_total: u32,
    pub rss_bytes: Option<u64>,
    pub rss_peak_bytes: Option<u64>,
    pub cpu_usec: Option<u64>,
    pub cpu_percent: Option<f32>,
    pub cpu_cores: u32,
    pub uptime_ms: u64,
}

impl From<&crate::stats::WorkspaceStats> for PersistedStats {
    fn from(s: &crate::stats::WorkspaceStats) -> Self {
        Self {
            commands_alive: s.commands_alive,
            commands_total: s.commands_total,
            rss_bytes: s.rss_bytes,
            rss_peak_bytes: s.rss_peak_bytes,
            cpu_usec: s.cpu_usec,
            cpu_percent: s.cpu_percent,
            cpu_cores: s.cpu_cores,
            uptime_ms: s.uptime_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineEntry {
    pub name: String,
    pub spec: PipelineSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivePipelineEntry {
    pub workspace: WorkspaceId,
    pub spec: PipelineSpec,
    pub tap: bool,
}

impl ShipoteSnapshot {
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path)?;
        let snap: ShipoteSnapshot = serde_json::from_slice(&bytes)?;
        // v1 y v2 son compatibles forward (v1 sin saved_pipelines lee como vec vacío).
        if snap.version > SNAPSHOT_VERSION {
            anyhow::bail!(
                "snapshot version {} no soportada (esperada ≤ {})",
                snap.version,
                SNAPSHOT_VERSION
            );
        }
        Ok(snap)
    }
}

/// Path canónico del snapshot: `$XDG_STATE_HOME/shuma/state.json`,
/// fallback `$HOME/.local/state/shuma/state.json`,
/// fallback `/tmp/shuma-state-$UID.json`.
pub fn default_snapshot_path() -> PathBuf {
    if let Ok(state) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(state).join("shuma/state.json");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/state/shuma/state.json");
    }
    let uid = nix::unistd::getuid().as_raw();
    PathBuf::from(format!("/tmp/shuma-state-{uid}.json"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl WorkspaceManager {
    /// Toma snapshot del estado actual.
    pub async fn snapshot(&self) -> ShipoteSnapshot {
        const PERSIST_STATS_CAP: usize = 16;
        let g = self.inner.lock().await;
        let workspaces = g
            .workspaces
            .iter()
            .map(|(id, ws)| {
                // Persist sólo los últimos N samples — el resto crece
                // y el JSON se infla.
                let take = ws.stats_history.len().min(PERSIST_STATS_CAP);
                let skip = ws.stats_history.len() - take;
                let stats_history: Vec<PersistedStats> = ws
                    .stats_history
                    .iter()
                    .skip(skip)
                    .map(PersistedStats::from)
                    .collect();
                WorkspaceEntry {
                    id: *id,
                    spec: ws.spec.clone(),
                    stats_history,
                }
            })
            .collect();
        let saved_pipelines = g
            .saved_pipelines
            .iter()
            .map(|(name, spec)| PipelineEntry {
                name: name.clone(),
                spec: spec.clone(),
            })
            .collect();
        // Pipelines vivos con supervisor — preserva la intención. Los
        // pids/sockets/discernments son ephemeral y se regeneran al
        // restore (relaunch desde cero).
        let live_pipelines = g
            .pipeline_supervisors
            .values()
            .map(|sup| LivePipelineEntry {
                workspace: sup.workspace,
                spec: sup.spec.clone(),
                tap: sup.tap,
            })
            .collect();
        ShipoteSnapshot {
            version: SNAPSHOT_VERSION,
            timestamp_ms: now_ms(),
            workspaces,
            saved_pipelines,
            live_pipelines,
        }
    }

    /// Escribe snapshot a disco. Si `is_dirty()` es false **y** el path
    /// existe (snapshot previo válido), skip la escritura.
    pub async fn save_snapshot(&self, path: &Path) -> anyhow::Result<()> {
        if !self.is_dirty() && path.exists() {
            info!(path = %path.display(), "snapshot SKIPPED (clean)");
            return Ok(());
        }
        let snap = self.snapshot().await;
        snap.write(path)?;
        // Clear dirty: lo que está en disco es el current state.
        self.dirty
            .store(false, std::sync::atomic::Ordering::Relaxed);
        info!(path = %path.display(), workspaces = snap.workspaces.len(), "snapshot saved");
        Ok(())
    }

    /// Carga snapshot desde disco y restaura los Workspaces + saved
    /// pipelines. Devuelve los `live_pipelines` para que el caller
    /// (daemon) los relance — no podemos relanzarlos desde acá porque
    /// `run_pipeline` necesita `Incarnator` + `DiscernPipeline`.
    /// Errores no-fatales (workspaces inválidos) se loguean y se saltan.
    pub async fn restore_snapshot(
        self: &std::sync::Arc<Self>,
        path: &Path,
    ) -> anyhow::Result<RestoreOutcome> {
        let snap = match ShipoteSnapshot::read(path) {
            Ok(s) => s,
            Err(e) => {
                warn!(?e, path = %path.display(), "no snapshot — start fresh");
                return Ok(RestoreOutcome::default());
            }
        };
        let mut out = RestoreOutcome::default();
        for entry in snap.workspaces {
            // v2+: reusamos el id original así clients que tracking
            // workspace_id no se rompen al restart.
            let label = entry.spec.label.clone();
            let id = entry.id;
            let history = entry.stats_history;
            match self.create_with_id(id, entry.spec).await {
                Ok(_) => {
                    out.workspaces_restored += 1;
                    // Hidratar history persistida. Convertimos
                    // PersistedStats → WorkspaceStats (perdemos
                    // los campos no serializables como `source`).
                    if !history.is_empty() {
                        let mut g = self.inner.lock().await;
                        if let Some(ws) = g.workspaces.get_mut(&id) {
                            for ps in history {
                                ws.stats_history.push_back(crate::stats::WorkspaceStats {
                                    commands_alive: ps.commands_alive,
                                    commands_total: ps.commands_total,
                                    rss_bytes: ps.rss_bytes,
                                    rss_peak_bytes: ps.rss_peak_bytes,
                                    cpu_usec: ps.cpu_usec,
                                    cpu_percent: ps.cpu_percent,
                                    cpu_cores: ps.cpu_cores,
                                    source: "persisted".into(),
                                    uptime_ms: ps.uptime_ms,
                                });
                            }
                        }
                    }
                }
                Err(e) => warn!(?e, %label, "skipped workspace en restore"),
            }
        }
        for entry in snap.saved_pipelines {
            self.save_pipeline(entry.name, entry.spec).await;
            out.saved_pipelines_restored += 1;
        }
        out.live_pipelines = snap.live_pipelines;
        // Restore no cuenta como mutación — lo que está en disco es lo
        // que acabamos de cargar. Sin esto, el próximo SIGTERM siempre
        // re-escribiría aunque no hubiese cambios reales.
        self.dirty
            .store(false, std::sync::atomic::Ordering::Relaxed);
        info!(
            workspaces = out.workspaces_restored,
            saved_pipelines = out.saved_pipelines_restored,
            live_pipelines = out.live_pipelines.len(),
            "snapshot restored"
        );
        Ok(out)
    }
}

/// Lo que el caller del restore obtiene. Las `live_pipelines` requieren
/// `Incarnator + DiscernPipeline` para relanzarlas → el caller las
/// procesa (típicamente el daemon).
#[derive(Debug, Default)]
pub struct RestoreOutcome {
    pub workspaces_restored: usize,
    pub saved_pipelines_restored: usize,
    pub live_pipelines: Vec<LivePipelineEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkspaceManager;
    use arje_incarnate::IncarnatorConfig;
    use shuma_card::{ExitPolicy, WorkspaceSpec};
    use std::sync::Arc;

    fn sample_ws(label: &str) -> WorkspaceSpec {
        WorkspaceSpec {
            label: label.into(),
            soma: Default::default(),
            permissions: Default::default(),
            ttl: None,
            flow_dirs: vec![],
            on_exit: ExitPolicy::Reap,
            quota_enforce: Default::default(),
        }
    }

    #[tokio::test]
    async fn roundtrip_snapshot_preserves_ulids() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");

        let mgr1 = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let (id1, _) = mgr1.create(sample_ws("a")).await.unwrap();
        let (id2, _) = mgr1.create(sample_ws("b")).await.unwrap();
        mgr1.save_snapshot(&path).await.unwrap();

        let mgr2 = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let out = mgr2.restore_snapshot(&path).await.unwrap();
        assert_eq!(out.workspaces_restored, 2);
        let listed = mgr2.list().await;
        let restored_ids: std::collections::HashSet<_> = listed.iter().map(|s| s.id).collect();
        assert!(restored_ids.contains(&id1));
        assert!(restored_ids.contains(&id2));
    }

    #[tokio::test]
    async fn save_snapshot_skips_when_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");
        let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let _ = mgr.create(sample_ws("dirty-test")).await.unwrap();
        assert!(mgr.is_dirty(), "create debería marcar dirty");
        mgr.save_snapshot(&path).await.unwrap();
        assert!(!mgr.is_dirty(), "save_snapshot debería limpiar dirty");
        let mtime1 = std::fs::metadata(&path).unwrap().modified().unwrap();
        // Esperamos un pelín para que mtime cambie si fuera re-escrito.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        // Segundo save sin mutación → skip.
        mgr.save_snapshot(&path).await.unwrap();
        let mtime2 = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2, "skip cuando clean — mtime no cambia");
    }

    #[tokio::test]
    async fn snapshot_includes_saved_pipelines() {
        use shuma_card::{CommandRef, DiscernPolicy, PipelineSpec};
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");

        let mgr1 = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        let (ws_id, _) = mgr1.create(sample_ws("ws")).await.unwrap();
        let spec = PipelineSpec {
            label: "echo-cat".into(),
            workspace: ws_id,
            nodes: vec![CommandRef {
                label: "n1".into(),
                payload: brahman_card::Payload::Native {
                    exec: "/bin/echo".into(),
                    argv: vec!["hi".into()],
                    envp: vec![],
                },
                soma: Default::default(),
                flows: Default::default(),
                supervision: brahman_card::Supervision::OneShot,
            }],
            edges: vec![],
            discern: DiscernPolicy::default(),
            restart_on_failure: false,
            restart_backoff_ms: 200,
            restart_max_backoff_ms: 30_000,
            restart_max: 0,
        };
        mgr1.save_pipeline("daily".into(), spec).await;
        mgr1.save_snapshot(&path).await.unwrap();

        let mgr2 = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
        mgr2.restore_snapshot(&path).await.unwrap();
        let saved = mgr2.list_saved_pipelines().await;
        assert_eq!(saved, vec!["daily".to_string()]);
        let got = mgr2.get_saved_pipeline("daily").await.expect("saved");
        assert_eq!(got.label, "echo-cat");
    }

    #[test]
    fn default_path_ends_with_state_json() {
        let p = default_snapshot_path();
        assert!(p.to_string_lossy().ends_with("state.json"));
    }
}
