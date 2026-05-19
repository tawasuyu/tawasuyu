//! `shuma-shell` — GUI dashboard del daemon shuma.
//!
//! Probe-style: conecta al socket del daemon cada 2s, pide
//! capabilities + workspace-list y los muestra en cards.
//! Si el daemon no está corriendo, marca DOWN.

use gpui::{div, prelude::*, px, Context, IntoElement, Render, SharedString, Window};
use shuma_protocol::{
    default_socket_path, read_frame, write_frame, CommandInfo, FlowInfo, FlowThroughputInfo,
    QuotaReportInfo, Request, Response, WorkspaceStatsInfo, WorkspaceSummary,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use nahual_widget_app_header::app_header;
use nahual_widget_banner::{banner_themed, Banner};
use nahual_widget_stat_card::stat_card;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
enum DaemonState {
    Pending,
    Down { reason: String },
    Up,
}

#[derive(Clone, Debug, Default)]
struct CapsSummary {
    kernel_version: (u32, u32, u32),
    user_ns: String,
    cgroup_v2: String,
    cgroup_delegated: bool,
    has_cap_sys_admin: bool,
}

struct Shell {
    socket_path: PathBuf,
    state: DaemonState,
    workspaces: Vec<WorkspaceSummary>,
    /// Comandos por workspace, indexados por workspace id.toString().
    commands: std::collections::BTreeMap<String, Vec<CommandInfo>>,
    saved_pipelines: Vec<String>,
    flows: Vec<FlowInfo>,
    /// Throughput por flow socket (bytes_total + bytes/s).
    flow_throughput: Vec<FlowThroughputInfo>,
    /// History de RSS por workspace (últimas N samples).
    stats_history: std::collections::BTreeMap<String, std::collections::VecDeque<WorkspaceStatsInfo>>,
    /// Quota report fresco por workspace.
    quotas: std::collections::BTreeMap<String, QuotaReportInfo>,
    caps: Option<CapsSummary>,
    last_probe_ms: u64,
    recent_log: Option<(String, String)>,
}

const STATS_HISTORY_LEN: usize = 24;

fn main() {
    launch_app("Shipote — Shell", (820., 560.), Shell::new);
}

impl Shell {
    fn new(cx: &mut Context<Self>) -> Self {
        let socket_path = default_socket_path();
        let socket_for_loop = socket_path.clone();
        cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            let bg = cx.background_executor().clone();
            loop {
                let path = socket_for_loop.clone();
                let started = std::time::Instant::now();
                let result = bg
                    .spawn(async move { probe_blocking(&path) })
                    .await;
                let elapsed = started.elapsed().as_millis() as u64;
                let _ = this.update(cx, |me, cx| {
                    match result {
                        Ok(snap) => {
                            me.state = DaemonState::Up;
                            me.workspaces = snap.workspaces;
                            me.commands = snap.commands;
                            me.saved_pipelines = snap.saved_pipelines;
                            me.flows = snap.flows;
                            me.flow_throughput = snap.flow_throughput;
                            me.quotas = snap.quotas;
                            // Hidratar history server-side para workspaces
                            // que no tenían history local (primer probe).
                            for ws in &me.workspaces {
                                let key = ws.id.to_string();
                                if !me.stats_history.contains_key(&key) {
                                    if let Some(hydrated) = snap.hydrate_history.get(&key) {
                                        me.stats_history.insert(
                                            key.clone(),
                                            hydrated.iter().cloned().collect(),
                                        );
                                    }
                                }
                            }
                            // Append fresh sample a la history por workspace.
                            for (ws_id, fresh) in &snap.fresh_stats {
                                let h = me
                                    .stats_history
                                    .entry(ws_id.clone())
                                    .or_default();
                                if h.len() >= STATS_HISTORY_LEN {
                                    h.pop_front();
                                }
                                h.push_back(fresh.clone());
                            }
                            // Limpiar history de workspaces que ya no existen.
                            let alive: std::collections::HashSet<String> = me
                                .workspaces
                                .iter()
                                .map(|w| w.id.to_string())
                                .collect();
                            me.stats_history.retain(|k, _| alive.contains(k));
                            me.caps = Some(snap.caps);
                            me.recent_log = snap.recent_log;
                        }
                        Err(reason) => {
                            me.state = DaemonState::Down { reason };
                            me.workspaces.clear();
                            me.commands.clear();
                            me.saved_pipelines.clear();
                            me.flows.clear();
                            me.flow_throughput.clear();
                            me.quotas.clear();
                            me.caps = None;
                            me.recent_log = None;
                        }
                    }
                    me.last_probe_ms = elapsed;
                    cx.notify();
                });
                timer.timer(POLL_INTERVAL).await;
            }
        })
        .detach();

        Self {
            socket_path,
            state: DaemonState::Pending,
            workspaces: Vec::new(),
            commands: std::collections::BTreeMap::new(),
            saved_pipelines: Vec::new(),
            flows: Vec::new(),
            flow_throughput: Vec::new(),
            stats_history: std::collections::BTreeMap::new(),
            quotas: std::collections::BTreeMap::new(),
            caps: None,
            last_probe_ms: 0,
            recent_log: None,
        }
    }
}

#[derive(Debug)]
struct Snapshot {
    workspaces: Vec<WorkspaceSummary>,
    commands: std::collections::BTreeMap<String, Vec<CommandInfo>>,
    saved_pipelines: Vec<String>,
    flows: Vec<FlowInfo>,
    flow_throughput: Vec<FlowThroughputInfo>,
    /// Stats fresco por workspace (id.toString → stats).
    fresh_stats: std::collections::BTreeMap<String, WorkspaceStatsInfo>,
    /// Quota report fresco por workspace.
    quotas: std::collections::BTreeMap<String, QuotaReportInfo>,
    /// Workspaces nuevos (no en history local): hidratamos history
    /// server-side al primer probe que los vea.
    hydrate_history: std::collections::BTreeMap<String, Vec<WorkspaceStatsInfo>>,
    caps: CapsSummary,
    /// tail del log del comando más reciente (label + bytes). None si no hay.
    recent_log: Option<(String, String)>,
}

fn probe_blocking(path: &std::path::Path) -> Result<Snapshot, String> {
    // Mini tokio runtime efímero por probe — no compartimos runtime con
    // GPUI. Costo aceptable cada 2s: setup ≈ <1 ms.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| format!("rt: {e}"))?;
    rt.block_on(async {
        let mut stream = UnixStream::connect(path)
            .await
            .map_err(|e| format!("connect: {e}"))?;
        write_frame(&mut stream, &Request::WorkspaceList)
            .await
            .map_err(|e| format!("write list: {e}"))?;
        let resp: Response = read_frame(&mut stream).await.map_err(|e| format!("read list: {e}"))?;
        let workspaces = match resp {
            Response::WorkspaceList { items } => items,
            other => return Err(format!("unexpected list resp: {other:?}")),
        };

        // Batched: stats+quota+commands+flow_sockets en 1 roundtrip por ws.
        // Para workspaces nuevos, también pedimos history server-side.
        let mut commands_map = std::collections::BTreeMap::new();
        let mut fresh_stats = std::collections::BTreeMap::new();
        let mut quotas = std::collections::BTreeMap::new();
        let mut hydrate_history = std::collections::BTreeMap::new();
        for w in &workspaces {
            write_frame(&mut stream, &Request::WorkspaceFullSummary { workspace: w.id })
                .await
                .map_err(|e| format!("write summary: {e}"))?;
            let resp: Response = read_frame(&mut stream)
                .await
                .map_err(|e| format!("read summary: {e}"))?;
            if let Response::WorkspaceFullSummary { stats, quota, commands, .. } = resp {
                let key = w.id.to_string();
                fresh_stats.insert(key.clone(), stats);
                quotas.insert(key.clone(), quota);
                if !commands.is_empty() {
                    commands_map.insert(key, commands);
                }
            }
            // History server-side (para hidratar si el shell es nuevo).
            write_frame(
                &mut stream,
                &Request::WorkspaceStatsHistory {
                    workspace: w.id,
                    tail: 24, // mismo cap que STATS_HISTORY_LEN
                },
            )
            .await
            .map_err(|e| format!("write history: {e}"))?;
            let resp: Response = read_frame(&mut stream)
                .await
                .map_err(|e| format!("read history: {e}"))?;
            if let Response::WorkspaceStatsHistory { samples } = resp {
                if !samples.is_empty() {
                    hydrate_history.insert(w.id.to_string(), samples);
                }
            }
        }

        // Saved pipelines.
        write_frame(&mut stream, &Request::PipelineSavedList)
            .await
            .map_err(|e| format!("write saved: {e}"))?;
        let resp: Response = read_frame(&mut stream)
            .await
            .map_err(|e| format!("read saved: {e}"))?;
        let saved_pipelines = match resp {
            Response::PipelineSavedList { names } => names,
            _ => Vec::new(),
        };

        // Flow channels activos (data plane).
        write_frame(&mut stream, &Request::FlowList)
            .await
            .map_err(|e| format!("write flows: {e}"))?;
        let resp: Response = read_frame(&mut stream)
            .await
            .map_err(|e| format!("read flows: {e}"))?;
        let flows = match resp {
            Response::FlowList { items } => items,
            _ => Vec::new(),
        };
        // Throughput per-socket.
        write_frame(&mut stream, &Request::FlowThroughput)
            .await
            .map_err(|e| format!("write throughput: {e}"))?;
        let resp: Response = read_frame(&mut stream)
            .await
            .map_err(|e| format!("read throughput: {e}"))?;
        let flow_throughput = match resp {
            Response::FlowThroughput { items } => items,
            _ => Vec::new(),
        };

        // Live tail: log del comando más reciente con bytes>0.
        let recent_log = {
            // Pick: comando con id más alto que tiene log_bytes>0, en cualquier workspace.
            let mut best: Option<(&str, &CommandInfo)> = None;
            for (ws, cmds) in &commands_map {
                for c in cmds {
                    if c.log_bytes == 0 {
                        continue;
                    }
                    let take = match &best {
                        Some((_, prev)) => c.id > prev.id,
                        None => true,
                    };
                    if take {
                        best = Some((ws.as_str(), c));
                    }
                }
            }
            match best {
                Some((ws_str, cmd)) => {
                    let ws_id: shuma_card::WorkspaceId = ws_str
                        .parse::<ulid::Ulid>()
                        .map(shuma_card::WorkspaceId)
                        .map_err(|e| format!("ulid parse: {e}"))?;
                    write_frame(
                        &mut stream,
                        &Request::CommandLogs {
                            workspace: ws_id,
                            command: cmd.id,
                            tail_bytes: 512,
                            stream: "both".into(),
                        },
                    )
                    .await
                    .map_err(|e| format!("write logs: {e}"))?;
                    let resp: Response = read_frame(&mut stream)
                        .await
                        .map_err(|e| format!("read logs: {e}"))?;
                    match resp {
                        Response::CommandLogs { bytes } => {
                            let text = String::from_utf8_lossy(&bytes).to_string();
                            Some((cmd.label.clone(), text))
                        }
                        _ => None,
                    }
                }
                None => None,
            }
        };

        write_frame(&mut stream, &Request::Capabilities)
            .await
            .map_err(|e| format!("write caps: {e}"))?;
        let resp: Response = read_frame(&mut stream).await.map_err(|e| format!("read caps: {e}"))?;
        let caps = match resp {
            Response::Capabilities {
                kernel_version,
                user_ns,
                cgroup_v2,
                cgroup_delegated,
                has_cap_sys_admin,
            } => CapsSummary {
                kernel_version,
                user_ns,
                cgroup_v2,
                cgroup_delegated,
                has_cap_sys_admin,
            },
            other => return Err(format!("unexpected caps resp: {other:?}")),
        };
        Ok(Snapshot {
            workspaces,
            commands: commands_map,
            saved_pipelines,
            flows,
            flow_throughput,
            fresh_stats,
            quotas,
            hydrate_history,
            caps,
            recent_log,
        })
    })
}

/// Render ASCII de sparkline para una serie de valores. `chars` define los
/// glifos (orden ascendente). Auto-scales al máximo de la serie.
fn sparkline(values: &[u64], width: usize) -> String {
    if values.is_empty() {
        return String::new();
    }
    const CHARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let take = values.len().min(width);
    let series = &values[values.len() - take..];
    let max = *series.iter().max().unwrap_or(&1);
    if max == 0 {
        return "▁".repeat(take);
    }
    series
        .iter()
        .map(|v| {
            let idx = ((*v as f64 / max as f64) * (CHARS.len() as f64 - 1.0)).round() as usize;
            CHARS[idx.min(CHARS.len() - 1)]
        })
        .collect()
}

impl Render for Shell {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app.clone();
        let text = theme.fg_text;
        let text_dim = theme.fg_muted;

        let accent_up = gpui::rgb(0xa3be8c);
        let accent_down = gpui::rgb(0xbf616a);
        let accent_pending = gpui::rgb(0x6a7280);
        let accent_info = gpui::rgb(0x88c0d0);

        let header_text = format!(
            "Daemon: {}  ·  reload {} ms",
            self.socket_path.display(),
            self.last_probe_ms
        );
        let header = app_header(cx, header_text);

        let status_banner = match &self.state {
            DaemonState::Pending => None,
            DaemonState::Down { reason } => Some(banner_themed(
                cx,
                Banner::Error,
                SharedString::from(format!("Daemon DOWN — {reason}")),
            )),
            DaemonState::Up => Some(banner_themed(
                cx,
                Banner::Success,
                SharedString::from("Daemon UP"),
            )),
        };

        let (status_value, status_descr, status_accent) = match &self.state {
            DaemonState::Pending => ("PENDING".to_string(), "primer probe…".to_string(), accent_pending),
            DaemonState::Down { reason } => ("DOWN".to_string(), reason.clone(), accent_down),
            DaemonState::Up => ("UP".to_string(), "shuma-daemon respondiendo".to_string(), accent_up),
        };

        let caps_items: Vec<String> = self
            .caps
            .as_ref()
            .map(|c| {
                vec![
                    format!(
                        "kernel:           {}.{}.{}",
                        c.kernel_version.0, c.kernel_version.1, c.kernel_version.2
                    ),
                    format!("user_ns:          {}", c.user_ns),
                    format!("cgroup_v2:        {}", c.cgroup_v2),
                    format!("cgroup_delegated: {}", c.cgroup_delegated),
                    format!("cap_sys_admin:    {}", c.has_cap_sys_admin),
                ]
            })
            .unwrap_or_default();
        let caps_value = if self.caps.is_some() { "OK".to_string() } else { "—".to_string() };

        let ws_items: Vec<String> = self
            .workspaces
            .iter()
            .map(|w| {
                let key = w.id.to_string();
                let history = self.stats_history.get(&key);
                let rss_series: Vec<u64> = history
                    .map(|h| h.iter().map(|s| s.rss_bytes.unwrap_or(0)).collect())
                    .unwrap_or_default();
                let spark = sparkline(&rss_series, STATS_HISTORY_LEN);
                let latest = history.and_then(|h| h.back());
                let (rss_now, peak, cpu_pct) = latest
                    .map(|s| (
                        s.rss_bytes.unwrap_or(0),
                        s.rss_peak_bytes.unwrap_or(0),
                        s.cpu_percent.unwrap_or(0.0),
                    ))
                    .unwrap_or((0, 0, 0.0));
                let rss_mb = rss_now as f64 / 1024.0 / 1024.0;
                let peak_mb = peak as f64 / 1024.0 / 1024.0;
                format!(
                    "{:<14} {:<14} {} {:>6.1}M peak {:>6.1}M  {:>5.1}%cpu",
                    &w.id.to_string()[20..],
                    w.label,
                    spark,
                    rss_mb,
                    peak_mb,
                    cpu_pct,
                )
            })
            .collect();
        let ws_count = self.workspaces.len().to_string();
        let ws_descr = if self.workspaces.is_empty() {
            "no hay workspaces vivos".to_string()
        } else {
            "id · label · cmds · uptime".to_string()
        };

        // Comandos: aplanamos por workspace, tomamos los más recientes (orden ULID ya temporal).
        let mut cmd_items: Vec<String> = Vec::new();
        let mut cmd_total = 0usize;
        for (ws_id, cmds) in &self.commands {
            cmd_total += cmds.len();
            for c in cmds.iter().rev().take(8) {
                let alive = if c.alive { "▶" } else { "✓" };
                let exit = c
                    .exit_status
                    .map(|s| format!(" exit={s}"))
                    .unwrap_or_default();
                cmd_items.push(format!(
                    "{} {} {:<20} pid={} logs={}B{}",
                    alive,
                    &ws_id[..6.min(ws_id.len())],
                    c.label,
                    c.pid,
                    c.log_bytes,
                    exit
                ));
            }
        }
        let cmd_count = cmd_total.to_string();
        let cmd_descr = if cmd_total == 0 {
            "no hay comandos lanzados".to_string()
        } else {
            "▶=alive ✓=exited · ws_prefix · label · pid · logs".to_string()
        };

        // Saved pipelines.
        let saved_count = self.saved_pipelines.len().to_string();
        let saved_items: Vec<String> = self.saved_pipelines.clone();
        let saved_descr = if saved_items.is_empty() {
            "shuma pipeline save <name> <file> para persistir".to_string()
        } else {
            "definiciones reusables vía run-saved".to_string()
        };

        // Quota breaches por workspace.
        let mut breach_items: Vec<String> = Vec::new();
        for (ws_id, q) in &self.quotas {
            for b in &q.breaches {
                let short = &ws_id[20..];
                breach_items.push(format!("{short}  {b}"));
            }
        }
        let breach_count = breach_items.len().to_string();
        let breach_descr = if breach_items.is_empty() {
            "todos los workspaces dentro de quota".to_string()
        } else {
            "ws_suffix · recurso · uso > limit".to_string()
        };

        // Flow channels (data plane) con throughput.
        let flow_count: usize = self.flows.iter().map(|f| f.sockets.len()).sum();
        // Lookup helper que NO captura por ref (evita issue de borrow
        // en el closure de flat_map).
        let find_tp = |s: &std::path::PathBuf| -> (f64, f64) {
            for t in &self.flow_throughput {
                if t.socket == *s {
                    return (t.bytes_total as f64 / 1024.0, t.bytes_per_sec / 1024.0);
                }
            }
            (0.0, 0.0)
        };
        let mut flow_items: Vec<String> = Vec::new();
        for f in &self.flows {
            let pipe = f.pipeline.to_string();
            let short_pipe = &pipe[pipe.len() - 6..];
            for s in &f.sockets {
                let name = s
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| s.display().to_string());
                let (total_kib, rate_kib) = find_tp(s);
                flow_items.push(format!(
                    "{short_pipe}  {:<48}  {:>7.1} KiB  {:>6.2} KiB/s",
                    name, total_kib, rate_kib
                ));
            }
        }
        let flow_descr = if flow_count == 0 {
            "pipelines con --tap exponen sockets aquí".to_string()
        } else {
            "pipe6 · socket · total · rate".to_string()
        };

        let body = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(16.))
            .py(px(16.))
            .child(stat_card(
                cx,
                "Estado",
                status_value,
                &status_descr,
                status_accent,
                text,
                text_dim,
                &[],
            ))
            .child(stat_card(
                cx,
                "Capabilities",
                caps_value,
                "kernel + namespaces + cgroup delegation",
                accent_info,
                text,
                text_dim,
                &caps_items,
            ))
            .child(stat_card(
                cx,
                "Workspaces",
                ws_count,
                &ws_descr,
                accent_info,
                text,
                text_dim,
                &ws_items,
            ))
            .child(stat_card(
                cx,
                "Comandos",
                cmd_count,
                &cmd_descr,
                accent_info,
                text,
                text_dim,
                &cmd_items,
            ))
            .child(stat_card(
                cx,
                "Saved pipelines",
                saved_count,
                &saved_descr,
                accent_info,
                text,
                text_dim,
                &saved_items,
            ))
            .child(stat_card(
                cx,
                "Flow channels",
                flow_count.to_string(),
                &flow_descr,
                accent_up,
                text,
                text_dim,
                &flow_items,
            ))
            .child(stat_card(
                cx,
                "Quota breaches",
                breach_count,
                &breach_descr,
                if breach_items.is_empty() { accent_up } else { accent_down },
                text,
                text_dim,
                &breach_items,
            ));

        // Live tail del comando más reciente con output.
        let live_card = self.recent_log.as_ref().map(|(label, content)| {
            // Cortamos a las últimas ~12 líneas para no inflar el panel.
            let lines: Vec<String> = content
                .lines()
                .rev()
                .take(12)
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            stat_card(
                cx,
                "Live tail",
                label.clone(),
                "últimas líneas del comando más reciente",
                accent_up,
                text,
                text_dim,
                &lines,
            )
        });

        let body = body.when_some(live_card, |d, c| d.child(c));

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            .child(header)
            .when_some(status_banner, |d, b| d.child(b))
            .child(body)
    }
}
