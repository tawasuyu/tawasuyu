//! Lanzamiento de pipelines y reaping cooperativo de hijos muertos.

use super::*;

impl WorkspaceManager {
    /// Lanza todas las Cards de un Pipeline. Devuelve (label, pid) por nodo.
    /// La conexión via flows queda librada al broker (cuando haya integración
    /// completa con sidecar; v1 sólo lanza).
    pub async fn run_pipeline(
        &self,
        spec: &PipelineSpec,
    ) -> Result<Vec<(String, Pid)>, CoreError> {
        spec.validate()?;
        let workspace_label = {
            let g = self.inner.lock().await;
            let ws = g
                .workspaces
                .get(&spec.workspace)
                .ok_or(CoreError::WorkspaceNotFound(spec.workspace))?;
            ws.spec.label.clone()
        };
        let mut launched = Vec::new();
        for (i, node) in spec.nodes.iter().enumerate() {
            let card = node.to_card(i, &workspace_label)?;
            let out = self.incarnator.incarnate(&card)?;
            let mut g = self.inner.lock().await;
            if let Some(ws) = g.workspaces.get_mut(&spec.workspace) {
                ws.commands.insert(
                    card.id,
                    CommandState {
                        id: card.id,
                        label: node.label.clone(),
                        pid: out.pid,
                        alive: true,
                        exit_status: None,
                        stdout: None, // run_pipeline NO captura (conecta por pipes).
                        stderr: None,
                        pipeline_id: None,
                    },
                );
            }
            launched.push((node.label.clone(), out.pid));
        }
        Ok(launched)
    }

    /// Cosecha hijos terminados (no-bloqueante). Llamar periódicamente desde
    /// el daemon o ante SIGCHLD. Marca `alive=false` y guarda exit_status.
    pub async fn reap_dead(self: &Arc<Self>) {
        let mut to_restart: Vec<RestartSpec> = Vec::new();
        let mut to_enforce_kill: Vec<WorkspaceId> = Vec::new();
        {
            let mut g = self.inner.lock().await;
            for ws in g.workspaces.values_mut() {
                for cmd in ws.commands.values_mut() {
                    if !cmd.alive {
                        continue;
                    }
                    match waitpid(cmd.pid, Some(WaitPidFlag::WNOHANG)) {
                        Ok(WaitStatus::Exited(_, code)) => {
                            cmd.alive = false;
                            cmd.exit_status = Some(code);
                        }
                        Ok(WaitStatus::Signaled(_, sig, _)) => {
                            cmd.alive = false;
                            cmd.exit_status = Some(128 + (sig as i32));
                        }
                        _ => {}
                    }
                }
            }
            // Quota enforcement: chequear breach por workspace y aplicar policy.
            // Lo hacemos dentro del mismo lock para tener una lectura
            // consistente; el kill real va fuera del lock.
            for (ws_id, ws) in g.workspaces.iter() {
                let rl = &ws.spec.soma.rlimits;
                let qe = &ws.spec.quota_enforce;
                // Sólo aplicamos si hay al menos una action != None.
                if qe.mem == shuma_card::QuotaAction::None
                    && qe.nproc == shuma_card::QuotaAction::None
                {
                    continue;
                }
                // Medir RSS y nproc vivos sin pasar por workspace_stats
                // (que tomaría el lock recursivo). Hacemos un read directo.
                let alive: Vec<i32> = ws
                    .commands
                    .values()
                    .filter(|c| c.alive)
                    .map(|c| c.pid.as_raw())
                    .collect();
                let nproc_alive = alive.len() as u32;
                let mem_used: u64 = alive
                    .iter()
                    .filter_map(|pid| read_proc_rss(*pid))
                    .sum();

                let mem_breach = matches!(rl.mem_bytes, Some(limit) if mem_used > limit);
                let nproc_breach = matches!(rl.nproc, Some(limit) if nproc_alive > limit);

                let mut kill_needed = false;
                if mem_breach {
                    match qe.mem {
                        shuma_card::QuotaAction::Log => {
                            warn!(%ws_id, used = mem_used, limit = ?rl.mem_bytes, "quota breach: memory");
                        }
                        shuma_card::QuotaAction::Kill => {
                            warn!(%ws_id, used = mem_used, limit = ?rl.mem_bytes, "quota breach: KILLING");
                            kill_needed = true;
                        }
                        _ => {}
                    }
                }
                if nproc_breach {
                    match qe.nproc {
                        shuma_card::QuotaAction::Log => {
                            warn!(%ws_id, alive = nproc_alive, limit = ?rl.nproc, "quota breach: nproc");
                        }
                        shuma_card::QuotaAction::Kill => {
                            warn!(%ws_id, alive = nproc_alive, limit = ?rl.nproc, "quota breach: KILLING");
                            kill_needed = true;
                        }
                        _ => {}
                    }
                }
                if kill_needed {
                    to_enforce_kill.push(*ws_id);
                }
            }
            // Pipeline supervisor: detectar pipelines cuyos comandos tienen
            // failure. Marca para restart si tiene supervisor.
            // Esto se hace cuando TODOS los comandos del pipeline están
            // dead Y al menos uno tiene exit!=0 (sino podría disparar
            // restart mientras otros comandos aún corren — incorrecto).
            let supervisor_ids: Vec<Ulid> = g.pipeline_supervisors.keys().copied().collect();
            for pipe_id in supervisor_ids {
                // ¿Hay algún comando vivo de este pipeline?
                let mut all_dead = true;
                let mut any_failed = false;
                for ws in g.workspaces.values() {
                    for cmd in ws.commands.values() {
                        if cmd.pipeline_id != Some(pipe_id) {
                            continue;
                        }
                        if cmd.alive {
                            all_dead = false;
                        } else if cmd.exit_status.map_or(false, |s| s != 0) {
                            any_failed = true;
                        }
                    }
                }
                if all_dead && any_failed {
                    // Push a queue si no estaba ya.
                    if !g.pending_pipeline_restarts.contains(&pipe_id) {
                        g.pending_pipeline_restarts.push(pipe_id);
                    }
                }
            }
            // Detectar restart_specs cuyo command_id ya está dead con exit!=0.
            let mut to_remove: Vec<Ulid> = Vec::new();
            for (cmd_id, spec) in g.restart_specs.iter() {
                let mut should_restart = false;
                let mut should_drop = false;
                'outer: for ws in g.workspaces.values() {
                    if let Some(cmd) = ws.commands.get(cmd_id) {
                        if !cmd.alive {
                            match cmd.exit_status {
                                Some(0) => should_drop = true,
                                Some(_) => should_restart = true,
                                None => {}
                            }
                            break 'outer;
                        }
                    }
                }
                if should_drop {
                    to_remove.push(*cmd_id);
                } else if should_restart {
                    to_restart.push(spec.clone());
                    to_remove.push(*cmd_id);
                }
            }
            for id in to_remove {
                g.restart_specs.remove(&id);
            }
        }
        // Quota enforcement: kill workspaces fuera del lock.
        for ws_id in to_enforce_kill {
            let _ = self.stop_with_grace(ws_id, std::time::Duration::ZERO).await;
        }
        // Schedule restart fuera del lock.
        for mut spec in to_restart {
            let mgr = self.clone();
            let backoff = std::time::Duration::from_millis(spec.backoff_ms);
            // Subir el backoff para la PRÓXIMA falla, no esta.
            spec.backoff_ms = (spec.backoff_ms * 2).min(spec.max_backoff_ms);
            spec.restart_count += 1;
            let restart_n = spec.restart_count;
            tokio::spawn(async move {
                tokio::time::sleep(backoff).await;
                info!(
                    backoff_ms = backoff.as_millis() as u64,
                    restart = restart_n,
                    "restarting failed command"
                );
                let workspace = spec.workspace;
                if let Err(e) = mgr
                    .run_with_options(workspace, spec.exec.clone(), spec.argv.clone(), spec.envp.clone(), true)
                    .await
                {
                    warn!(?e, "restart failed to launch");
                    return;
                }
                // Preservar backoff acumulado: localizar el nuevo command_id
                // (el más reciente vivo en el workspace) y sobreescribir.
                let new_cmd_id = {
                    let g = mgr.inner.lock().await;
                    g.workspaces.get(&workspace).and_then(|ws| {
                        ws.commands
                            .values()
                            .filter(|c| c.alive)
                            .max_by_key(|c| c.id)
                            .map(|c| c.id)
                    })
                };
                if let Some(new_id) = new_cmd_id {
                    let mut g = mgr.inner.lock().await;
                    if let Some(existing) = g.restart_specs.get_mut(&new_id) {
                        existing.backoff_ms = spec.backoff_ms;
                        existing.restart_count = spec.restart_count;
                    }
                }
            });
        }
    }
}
