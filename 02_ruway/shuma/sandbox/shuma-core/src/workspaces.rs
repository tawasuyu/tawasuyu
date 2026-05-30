//! Workspaces: alta/baja, stats/quota, listado, comandos.

use super::*;

impl WorkspaceManager {

    /// Label del workspace, si existe.
    pub async fn workspace_label(&self, id: WorkspaceId) -> Option<String> {
        self.inner
            .lock()
            .await
            .workspaces
            .get(&id)
            .map(|w| w.spec.label.clone())
    }

    /// Compara accounting real (RSS, commands_alive) contra los rlimits
    /// declarados en `SomaSpec`. Devuelve violaciones humanizadas. NO
    /// hace enforcement automático.
    pub async fn workspace_quota(&self, id: WorkspaceId) -> Option<stats::QuotaReport> {
        let stats_now = self.workspace_stats(id).await?;
        let g = self.inner.lock().await;
        let ws = g.workspaces.get(&id)?;
        let rl = &ws.spec.soma.rlimits;
        let mut report = stats::QuotaReport {
            mem_limit: rl.mem_bytes,
            nproc_limit: rl.nproc,
            breaches: Vec::new(),
        };
        if let (Some(limit), Some(used)) = (rl.mem_bytes, stats_now.rss_bytes) {
            if used > limit {
                report.breaches.push(format!(
                    "memory: {:.2} MiB > {:.2} MiB limit",
                    used as f64 / 1024.0 / 1024.0,
                    limit as f64 / 1024.0 / 1024.0,
                ));
            }
        }
        if let Some(limit) = rl.nproc {
            if stats_now.commands_alive > limit {
                report.breaches.push(format!(
                    "nproc: {} alive > {} limit",
                    stats_now.commands_alive, limit
                ));
            }
        }
        Some(report)
    }

    /// Estadísticas de recursos del workspace: RSS + CPU agregado de sus
    /// comandos vivos. Lee `/proc/<pid>/` directamente; si el spec declara
    /// `soma.cgroup.path`, también intenta el cgroup (más preciso, incluye
    /// descendants).
    ///
    /// `cpu_percent` se calcula entre samples consecutivos. Necesita ≥2
    /// llamadas para tener un valor (la primera siempre retorna `None`).
    pub async fn workspace_stats(&self, id: WorkspaceId) -> Option<stats::WorkspaceStats> {
        let mut g = self.inner.lock().await;
        let ws = g.workspaces.get_mut(&id)?;
        let alive: Vec<i32> = ws
            .commands
            .values()
            .filter(|c| c.alive)
            .map(|c| c.pid.as_raw())
            .collect();
        let total = ws.commands.len() as u32;
        let cgroup_path = if ws.spec.soma.cgroup.path.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(format!(
                "/sys/fs/cgroup{}",
                ws.spec.soma.cgroup.path
            )))
        };
        let mut s = stats::measure(&alive, cgroup_path.as_deref(), ws.started);
        s.commands_total = total;

        // CPU%: diff entre el sample actual y el previo, dividido por
        // wall time. 100% = 1 core saturado. >100% = varios cores.
        let now = Instant::now();
        if let Some(cpu_now) = s.cpu_usec {
            if let Some((prev_t, prev_cpu)) = ws.last_cpu_sample {
                let dt_us = now.duration_since(prev_t).as_micros() as u64;
                let d_cpu = cpu_now.saturating_sub(prev_cpu);
                if dt_us > 0 {
                    s.cpu_percent = Some(100.0 * d_cpu as f32 / dt_us as f32);
                }
            }
            ws.last_cpu_sample = Some((now, cpu_now));
        }
        // Append a history (ring buffer cap).
        if ws.stats_history.len() >= STATS_HISTORY_CAP {
            ws.stats_history.pop_front();
        }
        ws.stats_history.push_back(s.clone());
        Some(s)
    }

    /// Retorna las últimas N samples de stats (servidas desde el ring
    /// buffer interno). Sobrevive restart del shell.
    pub async fn workspace_stats_history(
        &self,
        id: WorkspaceId,
        tail: usize,
    ) -> Option<Vec<stats::WorkspaceStats>> {
        let g = self.inner.lock().await;
        let ws = g.workspaces.get(&id)?;
        let take = if tail == 0 { ws.stats_history.len() } else { tail };
        let skip = ws.stats_history.len().saturating_sub(take);
        Some(ws.stats_history.iter().skip(skip).cloned().collect())
    }

    pub async fn create(
        self: &Arc<Self>,
        spec: WorkspaceSpec,
    ) -> Result<(WorkspaceId, Vec<String>), CoreError> {
        self.create_with_id(WorkspaceId::new(), spec).await
    }

    /// Variante que acepta el ID. Útil para restore_snapshot: preserva
    /// ULIDs entre restarts, así clients que tracking workspace_id no se
    /// rompen.
    pub async fn create_with_id(
        self: &Arc<Self>,
        id: WorkspaceId,
        spec: WorkspaceSpec,
    ) -> Result<(WorkspaceId, Vec<String>), CoreError> {
        let card = spec.to_card(id)?;
        let mut warnings = self.incarnator.dry_run(&card).warnings;
        let ttl = spec.ttl;

        // Si el workspace declara cgroup path Y rlimits, intentamos
        // crear el cgroup y escribir memory.max/pids.max. El kernel
        // hace OOM kill al exceder memory.max — enforcement automático
        // sin policy adicional. Falla silenciosa si no hay delegation.
        if !spec.soma.cgroup.path.is_empty() {
            if let Ok(abs) = arje_incarnate::cgroup::ensure_cgroup(&spec.soma.cgroup) {
                let applied =
                    arje_incarnate::cgroup::apply_rlimits_to_cgroup(&abs, &spec.soma.rlimits);
                if !applied.is_empty() {
                    warnings.push(format!("cgroup limits applied: {}", applied.join(", ")));
                }
            }
        }
        let state = WorkspaceState {
            id,
            spec,
            root_card: card,
            commands: HashMap::new(),
            started: Instant::now(),
            last_cpu_sample: None,
            stats_history: std::collections::VecDeque::with_capacity(STATS_HISTORY_CAP),
        };
        self.inner.lock().await.workspaces.insert(id, state);
        self.mark_dirty();
        info!(%id, ?ttl, "workspace created");

        // Si tiene TTL, programar auto-stop. El task captura un weak ref
        // al manager para no impedir que se dropée si el daemon termina.
        if let Some(duration) = ttl {
            let mgr_weak = Arc::downgrade(self);
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                if let Some(mgr) = mgr_weak.upgrade() {
                    let exists = mgr.inner.lock().await.workspaces.contains_key(&id);
                    if exists {
                        info!(%id, "workspace TTL expired — auto-stop");
                        let _ = mgr.stop(id).await;
                    }
                }
            });
        }

        Ok((id, warnings))
    }

    pub async fn list(&self) -> Vec<WorkspaceSnapshot> {
        let g = self.inner.lock().await;
        g.workspaces
            .values()
            .map(|w| WorkspaceSnapshot {
                id: w.id,
                label: w.spec.label.clone(),
                commands: w.commands.len() as u32,
                uptime_ms: w.started.elapsed().as_millis() as u64,
            })
            .collect()
    }

    pub async fn stop(&self, id: WorkspaceId) -> Result<u32, CoreError> {
        self.stop_with_grace(id, std::time::Duration::from_millis(1000)).await
    }

    /// Variante con tiempo de gracia configurable. SIGTERM → espera `grace`
    /// → SIGKILL si quedan vivos. `grace=0` = SIGKILL inmediato.
    pub async fn stop_with_grace(
        &self,
        id: WorkspaceId,
        grace: std::time::Duration,
    ) -> Result<u32, CoreError> {
        let mut g = self.inner.lock().await;
        let ws = g.workspaces.remove(&id).ok_or(CoreError::WorkspaceNotFound(id))?;
        // También limpiamos flow_channels del workspace si los hubiera —
        // por workspace lo retenemos por pipeline, no por workspace.
        drop(g);
        self.mark_dirty();

        // 1) SIGTERM (o SIGKILL si grace=0) a todos vivos.
        let initial_signal = if grace.is_zero() { Signal::SIGKILL } else { Signal::SIGTERM };
        let alive_pids: Vec<Pid> = ws
            .commands
            .values()
            .filter(|c| c.alive)
            .map(|c| c.pid)
            .collect();
        for pid in &alive_pids {
            let _ = kill(*pid, initial_signal);
        }

        // 2) Esperar hasta `grace` haciendo polling WNOHANG.
        let mut reaped = 0u32;
        let mut still_alive: Vec<Pid> = alive_pids.clone();
        let deadline = std::time::Instant::now() + grace;
        let poll_interval = std::time::Duration::from_millis(20);
        while !still_alive.is_empty() && std::time::Instant::now() < deadline {
            still_alive.retain(|pid| match waitpid(*pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => true,
                Ok(_) => {
                    reaped += 1;
                    false
                }
                Err(_) => false,
            });
            if !still_alive.is_empty() {
                tokio::time::sleep(poll_interval).await;
            }
        }

        // 3) SIGKILL forzoso a los que quedan, y wait blocking.
        for pid in &still_alive {
            let _ = kill(*pid, Signal::SIGKILL);
            let _ = waitpid(*pid, None);
            reaped += 1;
        }
        info!(
            %id,
            reaped,
            grace_ms = grace.as_millis() as u64,
            sigkilled = still_alive.len(),
            "workspace stopped"
        );
        Ok(reaped)
    }

    /// Ejecuta un comando one-shot dentro de un workspace existente.
    /// Captura stdout+stderr en un ring buffer accesible vía
    /// [`get_command_logs`](Self::get_command_logs).
    pub async fn run(
        &self,
        id: WorkspaceId,
        exec: String,
        argv: Vec<String>,
        envp: Vec<(String, String)>,
    ) -> Result<CommandSummary, CoreError> {
        self.run_with_options(id, exec, argv, envp, false).await
    }

    /// Variante con `restart_on_failure`: si el comando muere con
    /// exit_status != 0, el reaper lo relauncha con backoff exponencial
    /// (200ms → 400 → 800 → … cap 30s).
    pub async fn run_with_options(
        &self,
        id: WorkspaceId,
        exec: String,
        argv: Vec<String>,
        envp: Vec<(String, String)>,
        restart_on_failure: bool,
    ) -> Result<CommandSummary, CoreError> {
        let workspace_label = {
            let g = self.inner.lock().await;
            let ws = g.workspaces.get(&id).ok_or(CoreError::WorkspaceNotFound(id))?;
            ws.spec.label.clone()
        };
        let cmd_ref = CommandRef {
            label: format!("run-{}", short_ulid(&Ulid::new())),
            payload: Payload::Native { exec, argv, envp },
            soma: Default::default(),
            flows: Default::default(),
            supervision: Supervision::OneShot,
        };
        let card = cmd_ref.to_card(0, &workspace_label)?;

        // Dos pipes O_CLOEXEC: uno para stdout, otro para stderr.
        use std::os::fd::IntoRawFd;
        let (sout_r, sout_w) =
            nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(|e| {
                CoreError::Incarnate(arje_incarnate::IncarnateError::Pipe(e))
            })?;
        let (serr_r, serr_w) =
            nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(|e| {
                CoreError::Incarnate(arje_incarnate::IncarnateError::Pipe(e))
            })?;
        let sout_r_fd = sout_r.into_raw_fd();
        let sout_w_fd = sout_w.into_raw_fd();
        let serr_r_fd = serr_r.into_raw_fd();
        let serr_w_fd = serr_w.into_raw_fd();

        let stdout_buf = logbuf::LogBuf::new();
        let stderr_buf = logbuf::LogBuf::new();

        let stdio = arje_incarnate::ChildStdio {
            stdin_fd: None,
            stdout_fd: Some(sout_w_fd),
            stderr_fd: Some(serr_w_fd),
        };
        let out = self.incarnator.incarnate_with(&card, stdio)?;
        let cmd_id = card.id;
        let cmd_label = cmd_ref.label.clone();
        let pid = out.pid;

        spawn_log_drainer(sout_r_fd, stdout_buf.clone());
        spawn_log_drainer(serr_r_fd, stderr_buf.clone());

        let mut g = self.inner.lock().await;
        if let Some(ws) = g.workspaces.get_mut(&id) {
            ws.commands.insert(
                cmd_id,
                CommandState {
                    id: cmd_id,
                    label: cmd_label.clone(),
                    pid,
                    alive: true,
                    exit_status: None,
                    stdout: Some(stdout_buf),
                    stderr: Some(stderr_buf),
                    pipeline_id: None,
                },
            );
        }
        if restart_on_failure {
            // Reextract exec/argv/envp del payload del CommandRef.
            if let Payload::Native { exec, argv, envp } = &cmd_ref.payload {
                g.restart_specs.insert(
                    cmd_id,
                    RestartSpec {
                        workspace: id,
                        exec: exec.clone(),
                        argv: argv.clone(),
                        envp: envp.clone(),
                        backoff_ms: 200,
                        max_backoff_ms: 30_000,
                        restart_count: 0,
                    },
                );
            }
        }
        for d in &out.degradations {
            warn!(?d, %id, "command incarnation degradation");
        }
        Ok(CommandSummary {
            id: cmd_id,
            label: cmd_label,
            pid: pid.as_raw(),
        })
    }

    /// Devuelve el tail del log capturado para `(workspace, command)`.
    /// `stream` selecciona stdout/stderr/both.
    pub async fn get_command_logs(
        &self,
        workspace: WorkspaceId,
        command: Ulid,
        tail_bytes: usize,
        stream: LogStream,
    ) -> Option<Vec<u8>> {
        let g = self.inner.lock().await;
        let ws = g.workspaces.get(&workspace)?;
        let cmd = ws.commands.get(&command)?;
        match stream {
            LogStream::Stdout => cmd.stdout.as_ref().map(|lb| lb.tail(tail_bytes)),
            LogStream::Stderr => cmd.stderr.as_ref().map(|lb| lb.tail(tail_bytes)),
            LogStream::Both => {
                let so = cmd.stdout.as_ref().map(|lb| lb.tail(tail_bytes)).unwrap_or_default();
                let se = cmd.stderr.as_ref().map(|lb| lb.tail(tail_bytes)).unwrap_or_default();
                let mut out = so;
                out.extend_from_slice(&se);
                Some(out)
            }
        }
    }

    /// Lista comandos de un workspace.
    pub async fn list_commands(&self, workspace: WorkspaceId) -> Vec<CommandInfo> {
        let g = self.inner.lock().await;
        let Some(ws) = g.workspaces.get(&workspace) else { return Vec::new() };
        let mut out: Vec<CommandInfo> = ws
            .commands
            .values()
            .map(|c| CommandInfo {
                id: c.id,
                label: c.label.clone(),
                pid: c.pid.as_raw(),
                alive: c.alive,
                exit_status: c.exit_status,
                log_bytes: c.stdout.as_ref().map(|l| l.written_total()).unwrap_or(0)
                    + c.stderr.as_ref().map(|l| l.written_total()).unwrap_or(0),
            })
            .collect();
        // Orden estable por ULID (temporal).
        out.sort_by_key(|c| c.id);
        out
    }

}
