use super::*;

#[tokio::test]
async fn ttl_auto_stops_workspace() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "ttl-test".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: Some(std::time::Duration::from_millis(120)),
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _) = mgr.create(spec).await.unwrap();
    assert_eq!(mgr.list().await.len(), 1);
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert_eq!(
        mgr.list().await.len(),
        0,
        "TTL expirado: workspace debe haber sido removido"
    );
    let _ = id;
}

#[tokio::test]
async fn create_and_list_workspace() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "test".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _w) = mgr.create(spec).await.unwrap();
    let list = mgr.list().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
}

#[tokio::test]
async fn run_captures_stdout_to_log() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "logs".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _) = mgr.create(spec).await.unwrap();
    let summary = mgr
        .run(id, "/bin/echo".into(), vec!["captured-output".into()], vec![])
        .await
        .unwrap();
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        mgr.reap_dead().await;
        let logs = mgr
            .get_command_logs(id, summary.id, 0, LogStream::Stdout)
            .await
            .unwrap_or_default();
        if !logs.is_empty() {
            let s = String::from_utf8_lossy(&logs);
            assert!(s.contains("captured-output"), "got: {s:?}");
            return;
        }
    }
    panic!("logs never captured");
}

#[tokio::test]
async fn run_captures_stderr_separately() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "stderr".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _) = mgr.create(spec).await.unwrap();
    // sh -c "echo OUT; echo ERR >&2"
    let summary = mgr
        .run(
            id,
            "/bin/sh".into(),
            vec!["-c".into(), "echo OUT; echo ERR >&2".into()],
            vec![],
        )
        .await
        .unwrap();
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        mgr.reap_dead().await;
        let so = mgr
            .get_command_logs(id, summary.id, 0, LogStream::Stdout)
            .await
            .unwrap_or_default();
        let se = mgr
            .get_command_logs(id, summary.id, 0, LogStream::Stderr)
            .await
            .unwrap_or_default();
        if !so.is_empty() && !se.is_empty() {
            let so_s = String::from_utf8_lossy(&so);
            let se_s = String::from_utf8_lossy(&se);
            assert!(so_s.contains("OUT"), "stdout: {so_s:?}");
            assert!(se_s.contains("ERR"), "stderr: {se_s:?}");
            assert!(!so_s.contains("ERR"), "stdout no debería tener ERR");
            assert!(!se_s.contains("OUT"), "stderr no debería tener OUT");
            return;
        }
    }
    panic!("logs never captured on both streams");
}

#[tokio::test]
async fn restart_on_failure_relaunches_failing_command() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "restart".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _) = mgr.create(spec).await.unwrap();
    // /bin/false sale con exit=1. Con restart_on_failure=true debería
    // relanzarse al menos 1 vez (tras el backoff inicial de 200ms).
    let summary = mgr
        .run_with_options(id, "/bin/false".into(), vec![], vec![], true)
        .await
        .unwrap();
    let original_id = summary.id;
    // Esperamos ~500ms para que termine + reap + restart corra.
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        mgr.reap_dead().await;
        let g = mgr.inner.lock().await;
        if let Some(ws) = g.workspaces.get(&id) {
            let new_cmds: Vec<_> = ws.commands.keys().filter(|k| **k != original_id).collect();
            if !new_cmds.is_empty() {
                // Hay un nuevo command_id → restart funcionó.
                return;
            }
        }
    }
    panic!("restart never launched a new command");
}

#[tokio::test]
async fn pipeline_supervisor_queues_restart_on_failure() {
    use shuma_card::{CommandRef, DiscernPolicy, PipelineSpec};
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let (ws_id, _) = mgr.create(WorkspaceSpec {
        label: "psup".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    }).await.unwrap();
    let spec = PipelineSpec {
        label: "fail-pipeline".into(),
        workspace: ws_id,
        nodes: vec![CommandRef {
            label: "boom".into(),
            payload: card_core::Payload::Native {
                exec: "/bin/false".into(),
                argv: vec![],
                envp: vec![],
            },
            soma: Default::default(),
            flows: Default::default(),
            supervision: card_core::Supervision::OneShot,
        }],
        edges: vec![],
        discern: DiscernPolicy::default(),
        restart_on_failure: true,
        restart_backoff_ms: 200,
        restart_max_backoff_ms: 30_000,
        restart_max: 0,
    };
    let pipeline_id = ulid::Ulid::new();
    // Simulamos lo que haría el daemon: registramos un comando como
    // si fuera de pipeline. Usamos `register_pipeline_commands` con
    // un pid fake — pero como reaper hace waitpid, mejor lanzar de verdad.
    // Hack: usar /bin/false via run() y manualmente marcar pipeline_id.
    let summary = mgr.run(ws_id, "/bin/false".into(), vec![], vec![]).await.unwrap();
    // Marcar el comando con pipeline_id manualmente.
    {
        let mut g = mgr.inner.lock().await;
        if let Some(ws) = g.workspaces.get_mut(&ws_id) {
            if let Some(cmd) = ws.commands.get_mut(&summary.id) {
                cmd.pipeline_id = Some(pipeline_id);
            }
        }
    }
    mgr.register_pipeline_supervisor(pipeline_id, ws_id, spec, true).await;
    // Esperamos que reap detecte la falla y push a pending.
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        mgr.reap_dead().await;
        let pending = mgr.take_pending_restarts().await;
        if !pending.is_empty() {
            assert_eq!(pending[0].spec.label, "fail-pipeline");
            return;
        }
    }
    panic!("supervisor never queued a restart");
}

#[tokio::test]
async fn quota_enforce_nproc_kill_terminates_commands() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let mut spec = WorkspaceSpec {
        label: "qenforce".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: shuma_card::QuotaEnforcement {
            mem: shuma_card::QuotaAction::None,
            nproc: shuma_card::QuotaAction::Kill,
        },
    };
    spec.soma.rlimits.nproc = Some(1);
    let (id, _) = mgr.create(spec).await.unwrap();
    // Lanzo 2 procesos (cada uno sleep). nproc_limit=1 → breach inmediato.
    let _ = mgr.run(id, "/bin/sleep".into(), vec!["5".into()], vec![]).await.unwrap();
    let _ = mgr.run(id, "/bin/sleep".into(), vec!["5".into()], vec![]).await.unwrap();
    // Reaper detecta breach y mata workspace.
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        mgr.reap_dead().await;
        let alive = mgr.list().await;
        if alive.is_empty() {
            return; // workspace removido por stop()
        }
    }
    panic!("quota enforce kill never triggered");
}

#[tokio::test]
async fn workspace_stats_history_accumulates() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "history".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _) = mgr.create(spec).await.unwrap();
    // Necesitamos al menos un comando vivo para que `measure` no
    // retorne source=none (que igual se appendea, pero con stats vacíos).
    let _ = mgr
        .run(id, "/bin/sleep".into(), vec!["5".into()], vec![])
        .await
        .unwrap();
    // Llamar stats 5 veces.
    for _ in 0..5 {
        let _ = mgr.workspace_stats(id).await;
    }
    let history = mgr.workspace_stats_history(id, 0).await.unwrap();
    assert_eq!(history.len(), 5, "history debería tener 5 samples");
    // tail=3 retorna los últimos 3.
    let tail3 = mgr.workspace_stats_history(id, 3).await.unwrap();
    assert_eq!(tail3.len(), 3);
    // Cleanup.
    let _ = mgr.stop_with_grace(id, std::time::Duration::ZERO).await;
}

#[tokio::test]
async fn run_true_in_workspace() {
    let mgr = Arc::new(WorkspaceManager::new(IncarnatorConfig::default()));
    let spec = WorkspaceSpec {
        label: "exec".into(),
        soma: Default::default(),
        permissions: Default::default(),
        ttl: None,
        flow_dirs: vec![],
        on_exit: shuma_card::ExitPolicy::Reap,
        quota_enforce: Default::default(),
    };
    let (id, _) = mgr.create(spec).await.unwrap();
    let summary = mgr
        .run(id, "/bin/true".into(), vec![], vec![])
        .await
        .unwrap();
    assert!(summary.pid > 0);
    // Cosecha.
    std::thread::sleep(std::time::Duration::from_millis(100));
    mgr.reap_dead().await;
}
