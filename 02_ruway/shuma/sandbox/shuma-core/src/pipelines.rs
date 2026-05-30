//! Pipelines, supervisores, flows y pipelines guardados.

use super::*;

impl WorkspaceManager {
    pub fn new(cfg: IncarnatorConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                workspaces: HashMap::new(),
                saved_pipelines: HashMap::new(),
                pipeline_flows: HashMap::new(),
                restart_specs: HashMap::new(),
                pipeline_supervisors: HashMap::new(),
                pending_pipeline_restarts: Vec::new(),
            })),
            incarnator: Arc::new(Incarnator::new(cfg)),
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Marca el manager como dirty. Cualquier mutación que afecta al
    /// snapshot debería llamar esto.
    #[inline]
    pub(crate) fn mark_dirty(&self) {
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// True si hubo cambios desde el último `save_snapshot`. Útil para
    /// chequeos cooperativos (ej. monitoring que pollea cada N).
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Registra un supervisor para un pipeline con `restart_on_failure=true`.
    /// El daemon llama esto tras `run_pipeline` para que `reap_dead` agregue
    /// el pipeline a la cola de restart cuando algún command falle.
    pub async fn register_pipeline_supervisor(
        &self,
        pipeline_id: Ulid,
        workspace: WorkspaceId,
        spec: PipelineSpec,
        tap: bool,
    ) {
        if !spec.restart_on_failure {
            return;
        }
        tracing::debug!(%pipeline_id, label = %spec.label, "pipeline supervisor registered");
        let mut g = self.inner.lock().await;
        let initial_backoff = spec.restart_backoff_ms.max(50);
        g.pipeline_supervisors.insert(
            pipeline_id,
            PipelineSupervisor {
                workspace,
                spec,
                tap,
                restart_count: 0,
                current_backoff_ms: initial_backoff,
            },
        );
        drop(g);
        self.mark_dirty();
    }

    /// Variante que preserva backoff/count del supervisor anterior (para
    /// re-registrar tras un restart sin perder el throttle acumulado).
    pub async fn register_pipeline_supervisor_with_state(
        &self,
        pipeline_id: Ulid,
        workspace: WorkspaceId,
        spec: PipelineSpec,
        tap: bool,
        restart_count: u32,
        current_backoff_ms: u64,
    ) {
        if !spec.restart_on_failure {
            return;
        }
        let mut g = self.inner.lock().await;
        g.pipeline_supervisors.insert(
            pipeline_id,
            PipelineSupervisor {
                workspace,
                spec,
                tap,
                restart_count,
                current_backoff_ms,
            },
        );
    }

    /// Drena la cola de pipelines pendientes de restart y retorna las
    /// specs a relaunch. El daemon lo llama tras cada `reap_dead`.
    ///
    /// Aplica `restart_max`: si el supervisor ya pasó el límite, no se
    /// retorna y el supervisor se elimina (give-up). El backoff
    /// preserva el valor actual; el daemon decide cuándo aplicar el
    /// sleep antes del relaunch.
    pub async fn take_pending_restarts(&self) -> Vec<PipelineSupervisor> {
        let mut g = self.inner.lock().await;
        let pending = std::mem::take(&mut g.pending_pipeline_restarts);
        let mut out = Vec::with_capacity(pending.len());
        for old_id in pending {
            if let Some(mut sup) = g.pipeline_supervisors.remove(&old_id) {
                if sup.spec.restart_max > 0 && sup.restart_count >= sup.spec.restart_max {
                    tracing::warn!(
                        label = %sup.spec.label,
                        restart_count = sup.restart_count,
                        max = sup.spec.restart_max,
                        "pipeline restart_max reached — giving up"
                    );
                    continue; // no relaunch, supervisor discarded.
                }
                sup.restart_count += 1;
                out.push(sup);
            }
        }
        out
    }

    /// Registra los comandos lanzados por un pipeline en el workspace.
    /// Esto permite `pipeline_stop` (matar selectivamente sólo los pids
    /// de un pipeline). `pipeline_id` se setea en cada CommandState.
    pub async fn register_pipeline_commands(
        &self,
        workspace: WorkspaceId,
        pipeline_id: Ulid,
        commands: Vec<(String, i32)>,
    ) {
        let mut g = self.inner.lock().await;
        let Some(ws) = g.workspaces.get_mut(&workspace) else { return };
        for (label, pid) in commands {
            let cmd_id = Ulid::new();
            ws.commands.insert(
                cmd_id,
                CommandState {
                    id: cmd_id,
                    label,
                    pid: Pid::from_raw(pid),
                    alive: true,
                    exit_status: None,
                    stdout: None,
                    stderr: None,
                    pipeline_id: Some(pipeline_id),
                },
            );
        }
    }

    /// Detiene selectivamente los comandos de un pipeline. SIGTERM →
    /// `grace` → SIGKILL. Devuelve cantidad reapeada. Si no hay comandos
    /// del pipeline en ningún workspace, retorna 0.
    pub async fn stop_pipeline(
        &self,
        pipeline_id: Ulid,
        grace: std::time::Duration,
    ) -> u32 {
        // 1) Recolectamos pids de ese pipeline en todos los workspaces.
        let mut targets: Vec<Pid> = Vec::new();
        {
            let g = self.inner.lock().await;
            for ws in g.workspaces.values() {
                for cmd in ws.commands.values() {
                    if cmd.alive && cmd.pipeline_id == Some(pipeline_id) {
                        targets.push(cmd.pid);
                    }
                }
            }
        }
        if targets.is_empty() {
            return 0;
        }
        let initial = if grace.is_zero() { Signal::SIGKILL } else { Signal::SIGTERM };
        for pid in &targets {
            let _ = kill(*pid, initial);
        }
        let mut reaped = 0u32;
        let mut still = targets.clone();
        let deadline = std::time::Instant::now() + grace;
        let poll = std::time::Duration::from_millis(20);
        while !still.is_empty() && std::time::Instant::now() < deadline {
            still.retain(|pid| match waitpid(*pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => true,
                Ok(_) => {
                    reaped += 1;
                    false
                }
                Err(_) => false,
            });
            if !still.is_empty() {
                tokio::time::sleep(poll).await;
            }
        }
        for pid in &still {
            let _ = kill(*pid, Signal::SIGKILL);
            let _ = waitpid(*pid, None);
            reaped += 1;
        }
        // Marcar como dead en estado in-memory.
        let mut g = self.inner.lock().await;
        for ws in g.workspaces.values_mut() {
            for cmd in ws.commands.values_mut() {
                if cmd.pipeline_id == Some(pipeline_id) && cmd.alive {
                    cmd.alive = false;
                }
            }
        }
        // Drop flows del pipeline.
        g.pipeline_flows.remove(&pipeline_id);
        info!(%pipeline_id, reaped, "pipeline stopped");
        reaped
    }

    /// Retiene los FlowChannels de un pipeline para que sobrevivan al
    /// fin del request. Drop = cierre del data plane.
    pub async fn retain_pipeline_flows(
        &self,
        pipeline: Ulid,
        flows: Vec<crate::flow_channel::FlowChannel>,
    ) {
        self.inner.lock().await.pipeline_flows.insert(pipeline, flows);
    }

    /// Snapshot de counts agregados para health endpoint.
    pub async fn health_counts(&self) -> HealthCounts {
        let g = self.inner.lock().await;
        let alive_workspaces = g.workspaces.len() as u32;
        let alive_commands: u32 = g
            .workspaces
            .values()
            .map(|ws| ws.commands.values().filter(|c| c.alive).count() as u32)
            .sum();
        let alive_pipelines = g.pipeline_supervisors.len() as u32;
        let active_flows: u32 = g.pipeline_flows.values().map(|v| v.len() as u32).sum();
        HealthCounts {
            alive_workspaces,
            alive_commands,
            alive_pipelines,
            active_flows,
        }
    }

    /// Lista pipelines vivos con sus sockets activos.
    pub async fn list_flow_pipelines(&self) -> Vec<(Ulid, Vec<std::path::PathBuf>)> {
        let g = self.inner.lock().await;
        g.pipeline_flows
            .iter()
            .map(|(id, flows)| {
                (
                    *id,
                    flows.iter().map(|f| f.socket_path().to_path_buf()).collect(),
                )
            })
            .collect()
    }

    /// Throughput per-socket: bytes_total + bytes_per_sec por flow socket.
    pub async fn flow_throughput(&self) -> Vec<(std::path::PathBuf, u64, f64)> {
        let g = self.inner.lock().await;
        let mut out = Vec::new();
        for flows in g.pipeline_flows.values() {
            for fc in flows {
                out.push((
                    fc.socket_path().to_path_buf(),
                    fc.meter().total_bytes(),
                    fc.meter().bytes_per_sec(),
                ));
            }
        }
        out
    }

    /// Cierra el data plane de un pipeline (drop = remove_file de sockets).
    pub async fn drop_pipeline_flows(&self, pipeline: Ulid) -> bool {
        self.inner.lock().await.pipeline_flows.remove(&pipeline).is_some()
    }

    pub fn incarnator(&self) -> &Incarnator {
        &self.incarnator
    }

    /// Handle Arc-clonable del Incarnator, para que el pipeline lo pueda
    /// usar fuera del manager.
    pub fn incarnator_handle(&self) -> Arc<Incarnator> {
        self.incarnator.clone()
    }

    // -----------------------------------------------------------------
    // Saved pipelines (definiciones nombradas, no runs)
    // -----------------------------------------------------------------

    /// Guarda (o reemplaza) un PipelineSpec bajo `name`.
    pub async fn save_pipeline(&self, name: String, spec: PipelineSpec) {
        self.inner.lock().await.saved_pipelines.insert(name, spec);
        self.mark_dirty();
    }

    /// Devuelve los nombres de los pipelines guardados.
    pub async fn list_saved_pipelines(&self) -> Vec<String> {
        let g = self.inner.lock().await;
        let mut v: Vec<String> = g.saved_pipelines.keys().cloned().collect();
        v.sort();
        v
    }

    /// Recupera el PipelineSpec guardado bajo `name`.
    pub async fn get_saved_pipeline(&self, name: &str) -> Option<PipelineSpec> {
        self.inner.lock().await.saved_pipelines.get(name).cloned()
    }

    /// Elimina un saved pipeline.
    pub async fn drop_saved_pipeline(&self, name: &str) -> bool {
        let existed = self.inner.lock().await.saved_pipelines.remove(name).is_some();
        if existed {
            self.mark_dirty();
        }
        existed
    }
}
