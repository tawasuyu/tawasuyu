//! `shuma` — CLI de administración del daemon.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use shuma_card::{load_pipeline_spec, load_workspace_spec, WorkspaceId};
use shuma_protocol::{default_socket_path, read_frame, write_frame, Request, Response};
use std::path::PathBuf;
use tokio::net::UnixStream;
use ulid::Ulid;

#[derive(Parser, Debug)]
#[command(name = "shuma", version, about = "Administración de shuma-daemon")]
struct Cli {
    /// Path al socket del daemon. Default: $XDG_RUNTIME_DIR/shuma.sock.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Health-check del daemon.
    Ping,

    /// Health endpoint estructurado.
    Health,

    /// Capacidades runtime detectadas por el daemon.
    Caps,

    /// Operaciones sobre Workspaces.
    #[command(subcommand)]
    Workspace(WsCmd),

    /// Ejecutar un comando one-shot dentro de un workspace.
    Run {
        /// ULID del workspace destino.
        #[arg(short = 'w', long)]
        workspace: String,
        /// Si exit != 0, relanzar con backoff exponencial.
        #[arg(long)]
        restart_on_failure: bool,
        /// Path del ejecutable.
        exec: String,
        /// Argumentos del comando.
        argv: Vec<String>,
    },

    /// Discernir el tipo de un archivo (ad-hoc, sin workspace).
    Discern {
        /// Path al archivo a discernir.
        path: PathBuf,
    },

    /// Listar comandos de un workspace.
    Commands {
        /// ULID del workspace.
        workspace: String,
    },

    /// Mostrar tail del log capturado de un comando.
    Logs {
        /// ULID del workspace.
        workspace: String,
        /// ULID del comando.
        command: String,
        /// Bytes desde el final (0 = todo).
        #[arg(long, default_value_t = 0)]
        tail: usize,
        /// Stream a leer: stdout | stderr | both.
        #[arg(long, default_value = "both")]
        stream: String,
        /// Seguir el log en vivo (poll cada 200ms hasta que el comando termine).
        #[arg(short = 'f', long)]
        follow: bool,
    },

    /// Pipeline DAG con flujo tipado.
    #[command(subcommand)]
    Pipeline(PipeCmd),

    /// Flow data plane (subscribirse a streams enriquecidos).
    #[command(subcommand)]
    Flow(FlowCmd),

    /// Sesiones PTY persistentes (tmux-like): viven en el daemon, sobreviven
    /// a la desconexión del cliente. Spawn / ls / attach / kill.
    #[command(subcommand)]
    Pty(PtyCmd),
}

#[derive(Subcommand, Debug)]
enum PtyCmd {
    /// Spawnear una sesión persistente y devolver su id (no se adjunta).
    Spawn {
        /// Etiqueta legible para listar (p. ej. "claude · repo X").
        #[arg(long, default_value = "")]
        label: String,
        /// Directorio de trabajo (default: el cwd actual).
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Programa a correr bajo el PTY.
        program: String,
        /// Argumentos del programa.
        args: Vec<String>,
    },
    /// Listar las sesiones (vivas y terminadas-no-reapeadas).
    Ls,
    /// Adjuntarse a una sesión: terminal full-duplex. Ctrl-] desadjunta
    /// (la sesión sigue viva); el proceso al terminar cierra la vista.
    Attach {
        /// ULID de la sesión.
        session: String,
    },
    /// Matar (o reapear, si ya murió) una sesión y quitarla del registro.
    Kill {
        /// ULID de la sesión.
        session: String,
    },
}

#[derive(Subcommand, Debug)]
enum FlowCmd {
    /// Listar pipelines activos con sus sockets de flow.
    List,
    /// Throughput por flow socket (bytes_total + bytes/s).
    Throughput,
    /// Cerrar el data plane de un pipeline (drop de todos sus sockets).
    Drop { pipeline: String },
    /// Suscribirse a un flow socket y volcar bytes a stdout.
    Tail {
        /// Path al Unix socket del flow.
        socket: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum PipeCmd {
    /// Lanzar un Pipeline desde un spec TOML/JSON.
    Run {
        /// Path al spec del pipeline.
        spec: PathBuf,
        /// Interponer un tap entre productor↔consumidor de cada edge para
        /// discernir el TypeRef del flujo.
        #[arg(long)]
        tap: bool,
        /// Variables `KEY=VALUE` para sustitución `${KEY}` en el spec.
        #[arg(long = "var", value_parser = parse_kv)]
        vars: Vec<(String, String)>,
        /// Tras lanzar, suscribir al primer flow socket y volcar bytes
        /// a stdout hasta EOF. Implica `--tap`.
        #[arg(long)]
        tail: bool,
    },
    /// Guardar un pipeline bajo un nombre (persiste con el snapshot).
    Save {
        /// Nombre simbólico.
        name: String,
        /// Path al spec.
        spec: PathBuf,
    },
    /// Listar nombres de pipelines guardados.
    SavedList,
    /// Eliminar un pipeline guardado (no afecta runs en curso).
    Drop { name: String },
    /// Detener un pipeline en curso por ID (SIGTERM → grace → SIGKILL
    /// sólo a sus comandos).
    Stop {
        /// ULID del pipeline (devuelto por `pipeline run`).
        pipeline: String,
        #[arg(long, default_value_t = 1000)]
        grace_ms: u64,
    },
    /// Ejecutar un pipeline guardado por nombre.
    RunSaved {
        name: String,
        #[arg(long)]
        tap: bool,
        #[arg(long = "var", value_parser = parse_kv)]
        vars: Vec<(String, String)>,
        #[arg(long)]
        tail: bool,
    },
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    let (k, v) = s.split_once('=').ok_or_else(|| format!("expected KEY=VALUE, got `{s}`"))?;
    Ok((k.to_string(), v.to_string()))
}

#[derive(Subcommand, Debug)]
enum WsCmd {
    /// Crear un workspace desde un spec TOML/JSON.
    Create {
        /// Path al spec del workspace.
        spec: PathBuf,
    },
    /// Listar workspaces vivos.
    List,
    /// Detener un workspace por ID.
    Stop {
        id: String,
        /// Milisegundos de gracia tras SIGTERM antes de SIGKILL.
        #[arg(long, default_value_t = 1000)]
        grace_ms: u64,
    },
    /// Resource accounting (RSS, CPU, comandos vivos).
    Stats {
        id: String,
    },
    /// Quota report: rlimits declarados vs uso actual.
    Quota {
        id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    bitacora::abrir("shuma");
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(default_socket_path);
    let mut stream = UnixStream::connect(&socket)
        .await
        .with_context(|| format!("connect {}", socket.display()))?;

    match cli.cmd {
        Cmd::Ping => {
            let resp = round_trip(&mut stream, Request::Ping).await?;
            match resp {
                Response::Pong => println!("pong"),
                other => print_unexpected(&other),
            }
        }

        Cmd::Health => {
            let resp = round_trip(&mut stream, Request::Health).await?;
            match resp {
                Response::Health {
                    version,
                    uptime_ms,
                    alive_workspaces,
                    alive_commands,
                    alive_pipelines,
                    active_flows,
                    dirty,
                } => {
                    println!("version:           {version}");
                    println!("uptime:            {} ms", uptime_ms);
                    println!("alive_workspaces:  {alive_workspaces}");
                    println!("alive_commands:    {alive_commands}");
                    println!("alive_pipelines:   {alive_pipelines}");
                    println!("active_flows:      {active_flows}");
                    println!("dirty:             {dirty}");
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Caps => {
            let resp = round_trip(&mut stream, Request::Capabilities).await?;
            match resp {
                Response::Capabilities {
                    kernel_version,
                    user_ns,
                    cgroup_v2,
                    cgroup_delegated,
                    has_cap_sys_admin,
                } => {
                    println!("kernel:           {}.{}.{}", kernel_version.0, kernel_version.1, kernel_version.2);
                    println!("user_ns:          {user_ns}");
                    println!("cgroup_v2:        {cgroup_v2}");
                    println!("cgroup_delegated: {cgroup_delegated}");
                    println!("cap_sys_admin:    {has_cap_sys_admin}");
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Workspace(WsCmd::Create { spec }) => {
            let ws = load_workspace_spec(&spec).with_context(|| format!("load {}", spec.display()))?;
            let resp = round_trip(&mut stream, Request::WorkspaceCreate { spec: ws }).await?;
            match resp {
                Response::WorkspaceCreated { id, warnings } => {
                    println!("{id}");
                    for w in warnings {
                        eprintln!("warning: {w}");
                    }
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Workspace(WsCmd::List) => {
            let resp = round_trip(&mut stream, Request::WorkspaceList).await?;
            match resp {
                Response::WorkspaceList { items } => {
                    if items.is_empty() {
                        println!("(no workspaces)");
                    }
                    for it in items {
                        println!(
                            "{}  {:<20}  cmds={}  uptime={}ms",
                            it.id, it.label, it.commands, it.uptime_ms
                        );
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Workspace(WsCmd::Stats { id }) => {
            let id = parse_ws_id(&id)?;
            let resp = round_trip(&mut stream, Request::WorkspaceStats { workspace: id }).await?;
            match resp {
                Response::WorkspaceStats { info } => {
                    println!("commands:   {} alive / {} total", info.commands_alive, info.commands_total);
                    let fmt_mib = |b: u64| format!("{:.2} MiB", b as f64 / 1024.0 / 1024.0);
                    let rss = info.rss_bytes.map(fmt_mib).unwrap_or_else(|| "—".into());
                    let peak = info.rss_peak_bytes.map(fmt_mib).unwrap_or_else(|| "—".into());
                    let cpu = info
                        .cpu_usec
                        .map(|u| format!("{:.3} s", u as f64 / 1_000_000.0))
                        .unwrap_or_else(|| "—".into());
                    let cpu_pct = info
                        .cpu_percent
                        .map(|p| format!("{p:.1} % ({:.1}% total / {} cores)",
                            if info.cpu_cores > 0 { p / info.cpu_cores as f32 } else { p },
                            info.cpu_cores))
                        .unwrap_or_else(|| "— (esperando 2do sample)".into());
                    println!("rss:        {rss}");
                    println!("rss_peak:   {peak}");
                    println!("cpu:        {cpu}");
                    println!("cpu_pct:    {cpu_pct}");
                    println!("source:     {}", info.source);
                    println!("uptime:     {} ms", info.uptime_ms);
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Workspace(WsCmd::Quota { id }) => {
            let id = parse_ws_id(&id)?;
            let resp = round_trip(&mut stream, Request::WorkspaceQuota { workspace: id }).await?;
            match resp {
                Response::WorkspaceQuota { info } => {
                    let mem = info
                        .mem_limit
                        .map(|b| format!("{:.2} MiB", b as f64 / 1024.0 / 1024.0))
                        .unwrap_or_else(|| "—".into());
                    let nproc = info
                        .nproc_limit
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "—".into());
                    println!("mem_limit:   {mem}");
                    println!("nproc_limit: {nproc}");
                    if info.breaches.is_empty() {
                        println!("breaches:    (none — dentro de quota)");
                    } else {
                        println!("breaches:");
                        for b in info.breaches {
                            println!("  - {b}");
                        }
                    }
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Workspace(WsCmd::Stop { id, grace_ms }) => {
            let id = parse_ws_id(&id)?;
            let resp = round_trip(&mut stream, Request::WorkspaceStop { id, grace_ms }).await?;
            match resp {
                Response::WorkspaceStopped { id, reaped } => {
                    println!("stopped {id} (reaped {reaped})");
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Run { workspace, exec, argv, restart_on_failure } => {
            let id = parse_ws_id(&workspace)?;
            let resp = round_trip(
                &mut stream,
                Request::Run {
                    workspace: id,
                    exec,
                    argv,
                    envp: vec![],
                    restart_on_failure,
                },
            )
            .await?;
            match resp {
                Response::RunStarted { command_id, pid, .. } => {
                    println!("{command_id}  pid={pid}");
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Pipeline(PipeCmd::Run { spec, tap, vars, tail }) => {
            let p = load_pipeline_spec(&spec).with_context(|| format!("load {}", spec.display()))?;
            // --tail implica --tap (no hay flow socket sin tap).
            let effective_tap = tap || tail;
            let resp = round_trip(
                &mut stream,
                Request::PipelineRun {
                    spec: p,
                    tap: effective_tap,
                    vars: vars.into_iter().collect(),
                },
            )
            .await?;
            let socket = print_pipeline_started_returning_socket(resp)?;
            if tail {
                if let Some(sock) = socket {
                    eprintln!("--- tailing {} ---", sock.display());
                    tail_socket(&sock).await?;
                } else {
                    eprintln!("--tail: no hay flow socket disponible");
                }
            }
        }

        Cmd::Pipeline(PipeCmd::Save { name, spec }) => {
            let p = load_pipeline_spec(&spec).with_context(|| format!("load {}", spec.display()))?;
            let resp = round_trip(&mut stream, Request::PipelineSave { name: name.clone(), spec: p }).await?;
            match resp {
                Response::PipelineSaved { name } => println!("saved {name}"),
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Pipeline(PipeCmd::SavedList) => {
            let resp = round_trip(&mut stream, Request::PipelineSavedList).await?;
            match resp {
                Response::PipelineSavedList { names } => {
                    if names.is_empty() {
                        println!("(no saved pipelines)");
                    }
                    for n in names {
                        println!("{n}");
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Pipeline(PipeCmd::Stop { pipeline, grace_ms }) => {
            let pid = Ulid::from_string(&pipeline).map_err(|e| anyhow!("invalid pipeline id: {e}"))?;
            let resp = round_trip(&mut stream, Request::PipelineStop { pipeline: pid, grace_ms }).await?;
            match resp {
                Response::PipelineStopped { pipeline, reaped } => {
                    println!("stopped pipeline {pipeline} (reaped {reaped})");
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Pipeline(PipeCmd::Drop { name }) => {
            let resp = round_trip(&mut stream, Request::PipelineDrop { name }).await?;
            match resp {
                Response::PipelineDropped { name, existed } => {
                    if existed {
                        println!("dropped {name}");
                    } else {
                        eprintln!("no existía: {name}");
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Pipeline(PipeCmd::RunSaved { name, tap, vars, tail }) => {
            let effective_tap = tap || tail;
            let resp = round_trip(
                &mut stream,
                Request::PipelineRunSaved {
                    name,
                    tap: effective_tap,
                    vars: vars.into_iter().collect(),
                },
            )
            .await?;
            let socket = print_pipeline_started_returning_socket(resp)?;
            if tail {
                if let Some(sock) = socket {
                    eprintln!("--- tailing {} ---", sock.display());
                    tail_socket(&sock).await?;
                } else {
                    eprintln!("--tail: no hay flow socket disponible");
                }
            }
        }

        Cmd::Commands { workspace } => {
            let ws = parse_ws_id(&workspace)?;
            let resp = round_trip(&mut stream, Request::CommandList { workspace: ws }).await?;
            match resp {
                Response::CommandList { items } => {
                    if items.is_empty() {
                        println!("(no commands)");
                    }
                    for c in items {
                        let alive = if c.alive { "alive" } else { "exited" };
                        let exit = c
                            .exit_status
                            .map(|s| format!("exit={s}"))
                            .unwrap_or_default();
                        println!(
                            "{}  {:<24} pid={:<7} {:<8} logs={} {}",
                            c.id, c.label, c.pid, alive, c.log_bytes, exit
                        );
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Logs { workspace, command, tail, stream: which_stream, follow } => {
            let ws = parse_ws_id(&workspace)?;
            let cmd_id = Ulid::from_string(&command).map_err(|e| anyhow!("invalid command id: {e}"))?;
            if !follow {
                let resp = round_trip(
                    &mut stream,
                    Request::CommandLogs {
                        workspace: ws,
                        command: cmd_id,
                        tail_bytes: tail,
                        stream: which_stream,
                    },
                )
                .await?;
                match resp {
                    Response::CommandLogs { bytes } => {
                        use std::io::Write;
                        let _ = std::io::stdout().write_all(&bytes);
                        let _ = std::io::stdout().flush();
                    }
                    Response::Error { message } => return Err(anyhow!(message)),
                    other => print_unexpected(&other),
                }
            } else {
                // Follow mode: poll cada 200ms. Mantenemos el último buffer
                // visto; cada round imprimimos el delta (suffix nuevo).
                // Limitación: si el ring rota más rápido que el poll, perdemos
                // bytes — pero el comportamiento es "best effort".
                use std::io::Write;
                let mut prev: Vec<u8> = Vec::new();
                loop {
                    let resp = round_trip(
                        &mut stream,
                        Request::CommandLogs {
                            workspace: ws,
                            command: cmd_id,
                            tail_bytes: 0,
                            stream: which_stream.clone(),
                        },
                    )
                    .await?;
                    let bytes = match resp {
                        Response::CommandLogs { bytes } => bytes,
                        Response::Error { message } => return Err(anyhow!(message)),
                        other => {
                            print_unexpected(&other);
                            break;
                        }
                    };
                    // Imprimir suffix nuevo si bytes es extension de prev.
                    if bytes.len() >= prev.len() && bytes[..prev.len()] == prev[..] {
                        let _ = std::io::stdout().write_all(&bytes[prev.len()..]);
                    } else {
                        // Ring rotó — reset y print todo.
                        let _ = std::io::stdout().write_all(&bytes);
                    }
                    let _ = std::io::stdout().flush();
                    prev = bytes;

                    // Si el comando terminó, salir tras un último read.
                    let list_resp = round_trip(
                        &mut stream,
                        Request::CommandList { workspace: ws },
                    )
                    .await?;
                    let mut still_alive = false;
                    if let Response::CommandList { items } = list_resp {
                        if let Some(c) = items.iter().find(|c| c.id == cmd_id) {
                            still_alive = c.alive;
                        }
                    }
                    if !still_alive {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }

        Cmd::Flow(FlowCmd::List) => {
            let resp = round_trip(&mut stream, Request::FlowList).await?;
            match resp {
                Response::FlowList { items } => {
                    if items.is_empty() {
                        println!("(no active flows)");
                    }
                    for it in items {
                        println!("{}", it.pipeline);
                        for s in it.sockets {
                            println!("  {}", s.display());
                        }
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Flow(FlowCmd::Throughput) => {
            let resp = round_trip(&mut stream, Request::FlowThroughput).await?;
            match resp {
                Response::FlowThroughput { items } => {
                    if items.is_empty() {
                        println!("(no active flows)");
                    }
                    for it in items {
                        let name = it.socket.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| it.socket.display().to_string());
                        let kib = it.bytes_total as f64 / 1024.0;
                        let kbs = it.bytes_per_sec / 1024.0;
                        println!("{:<60} {:>8.1} KiB total  {:>8.2} KiB/s", name, kib, kbs);
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Flow(FlowCmd::Drop { pipeline }) => {
            let pid = Ulid::from_string(&pipeline).map_err(|e| anyhow!("invalid pipeline id: {e}"))?;
            let resp = round_trip(&mut stream, Request::FlowDrop { pipeline: pid }).await?;
            match resp {
                Response::FlowDropped { pipeline, existed } => {
                    if existed {
                        println!("dropped {pipeline}");
                    } else {
                        eprintln!("no existía: {pipeline}");
                    }
                }
                other => print_unexpected(&other),
            }
        }

        Cmd::Flow(FlowCmd::Tail { socket }) => {
            // Subscribirse directo al socket — no pasamos por el daemon.
            use tokio::io::AsyncReadExt;
            let mut s = UnixStream::connect(&socket)
                .await
                .with_context(|| format!("connect {}", socket.display()))?;
            let mut buf = [0u8; 4096];
            loop {
                let n = s.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                use std::io::Write;
                let _ = std::io::stdout().write_all(&buf[..n]);
                let _ = std::io::stdout().flush();
            }
        }

        Cmd::Pty(PtyCmd::Spawn { label, cwd, program, args }) => {
            let cwd = cwd
                .or_else(|| std::env::current_dir().ok())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "/".into());
            let (rows, cols) = term_size();
            let resp = round_trip(
                &mut stream,
                Request::PtySpawn { cwd, program, args, rows, cols, label },
            )
            .await?;
            match resp {
                Response::PtySpawned { session } => {
                    println!("{session}");
                    eprintln!("adjuntate con: shuma pty attach {session}");
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Pty(PtyCmd::Ls) => {
            let resp = round_trip(&mut stream, Request::PtyList).await?;
            match resp {
                Response::PtyList { sessions } => {
                    if sessions.is_empty() {
                        println!("(sin sesiones)");
                    }
                    for s in sessions {
                        let estado = if s.alive {
                            format!("viva ({} adj)", s.attached)
                        } else {
                            format!("muerta (exit {})", s.exit_code.unwrap_or(-1))
                        };
                        let cmd = if s.args.is_empty() {
                            s.program.clone()
                        } else {
                            format!("{} {}", s.program, s.args.join(" "))
                        };
                        println!("{}  {:<22}  {:<18}  {}", s.session, s.label, estado, cmd);
                    }
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Pty(PtyCmd::Kill { session }) => {
            let id = Ulid::from_string(&session).map_err(|e| anyhow!("id inválido: {e}"))?;
            let resp = round_trip(&mut stream, Request::PtyKill { session: id }).await?;
            match resp {
                Response::PtyKilled { session, existed } => {
                    if existed {
                        println!("matada {session}");
                    } else {
                        eprintln!("no existía: {session}");
                    }
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }

        Cmd::Pty(PtyCmd::Attach { session }) => {
            let id = Ulid::from_string(&session).map_err(|e| anyhow!("id inválido: {e}"))?;
            attach_pty(stream, id).await?;
            return Ok(());
        }

        Cmd::Discern { path } => {
            let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            // Sample: hasta 4 KiB.
            let sample = bytes.into_iter().take(4096).collect();
            let resp = round_trip(
                &mut stream,
                Request::Discern {
                    sample,
                    hint_path: Some(path),
                },
            )
            .await?;
            match resp {
                Response::Discernment { ty, confidence, mime, lens } => {
                    println!("type:       {ty}");
                    println!("confidence: {confidence:.2}");
                    if let Some(m) = mime {
                        println!("mime:       {m}");
                    }
                    if let Some(l) = lens {
                        println!("lens:       {l}");
                    }
                }
                Response::Error { message } => return Err(anyhow!(message)),
                other => print_unexpected(&other),
            }
        }
    }

    Ok(())
}

async fn round_trip(stream: &mut UnixStream, req: Request) -> Result<Response> {
    write_frame(stream, &req).await?;
    let resp: Response = read_frame(stream).await?;
    Ok(resp)
}

fn parse_ws_id(s: &str) -> Result<WorkspaceId> {
    let u = Ulid::from_string(s).map_err(|e| anyhow!("invalid workspace id: {e}"))?;
    Ok(WorkspaceId(u))
}

fn print_unexpected(r: &Response) {
    eprintln!("unexpected response: {r:?}");
}

/// Imprime el resultado del launch del pipeline y retorna el path del
/// primer flow socket (si hay), útil para `--tail`.
fn print_pipeline_started_returning_socket(resp: Response) -> Result<Option<PathBuf>> {
    match resp {
        Response::PipelineStarted { pipeline, command_pids, edges } => {
            println!("pipeline {pipeline}");
            for (label, pid) in command_pids {
                println!("  {:<20} pid={pid}", label);
            }
            let mut first_socket: Option<PathBuf> = None;
            if !edges.is_empty() {
                println!("edges:");
                for e in &edges {
                    println!(
                        "  {}.{} → {}.{}  ty={:?}  mime={:?}  conf={:.2}",
                        e.from_label, e.from_output, e.to_label, e.to_input,
                        e.ty, e.mime, e.confidence,
                    );
                    if first_socket.is_none() {
                        first_socket = e.flow_socket.clone();
                    }
                }
            }
            Ok(first_socket)
        }
        Response::Error { message } => Err(anyhow!(message)),
        other => {
            print_unexpected(&other);
            Ok(None)
        }
    }
}

async fn tail_socket(socket: &std::path::Path) -> Result<()> {
    use tokio::io::AsyncReadExt;
    // Pequeña ventana de retry — el daemon retiene el flow channel
    // antes de retornar, así que en la práctica ya está bindeado.
    let mut s = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connect {}", socket.display()))?;
    let mut buf = [0u8; 4096];
    loop {
        let n = s.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        use std::io::Write;
        let _ = std::io::stdout().write_all(&buf[..n]);
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

// ===================================================================
// `shuma pty attach` — cliente full-duplex de una sesión persistente.
// ===================================================================

/// Tamaño actual del terminal `(filas, columnas)`; `(24, 80)` si no es un tty.
fn term_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
            (ws.ws_row, ws.ws_col)
        } else {
            (24, 80)
        }
    }
}

/// Pone el terminal en modo raw mientras está vivo y lo restaura al dropear
/// (incluso si `attach_pty` retorna por error o panic).
struct RawGuard {
    fd: i32,
    orig: libc::termios,
}

impl RawGuard {
    fn enter() -> Option<Self> {
        let fd = libc::STDIN_FILENO;
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut orig) != 0 {
                return None;
            }
            let mut raw = orig;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(RawGuard { fd, orig })
        }
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.orig);
        }
    }
}

/// Cómo terminó el attach.
enum AttachEnd {
    /// La sesión (su proceso) terminó con este código (`None` si fallo/EOF).
    Exited(Option<i32>),
    /// El usuario se desadjuntó (Ctrl-] o stdin EOF) — la sesión sigue viva.
    Detached,
}

/// Cliente full-duplex de `PtyAttach`: terminal en raw, teclas → `PtyInput`,
/// resizes (SIGWINCH) → `PtyResize`, y los `ExecBytes` del daemon → stdout.
/// **Ctrl-]** (0x1d) desadjunta sin matar la sesión.
async fn attach_pty(mut stream: UnixStream, session: Ulid) -> Result<()> {
    use std::io::Write as _;
    use tokio::io::AsyncReadExt as _;

    let (rows, cols) = term_size();
    write_frame(&mut stream, &Request::PtyAttach { session, rows, cols }).await?;

    let raw = RawGuard::enter();
    if raw.is_none() {
        eprintln!("aviso: no se pudo poner el terminal en raw (¿no es un tty?)");
    }

    let (mut rd, mut wr) = tokio::io::split(stream);

    // Lectora: ExecBytes → stdout; terminal → devuelve el exit code.
    let read_task = tokio::spawn(async move {
        loop {
            match read_frame::<Response, _>(&mut rd).await {
                Ok(Response::ExecBytes(b)) => {
                    let mut out = std::io::stdout();
                    let _ = out.write_all(&b);
                    let _ = out.flush();
                }
                Ok(Response::ExecExited(c)) => return Some(c),
                Ok(Response::ExecFailed(m)) => {
                    let _ = writeln!(std::io::stderr(), "\r\n✘ {m}");
                    return None;
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    });

    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 4096];

    let mut read_task = read_task; // mut para `&mut read_task` en select!
    let end: AttachEnd;
    loop {
        tokio::select! {
            res = &mut read_task => {
                end = AttachEnd::Exited(res.unwrap_or(None));
                break;
            }
            n = stdin.read(&mut buf) => {
                match n {
                    Ok(0) => { end = AttachEnd::Detached; break; }   // stdin EOF
                    Ok(n) => {
                        // Ctrl-] (0x1d) en cualquier parte del buffer = detach.
                        if buf[..n].contains(&0x1d) {
                            end = AttachEnd::Detached;
                            break;
                        }
                        if write_frame(&mut wr, &Request::PtyInput { bytes: buf[..n].to_vec() })
                            .await
                            .is_err()
                        {
                            end = AttachEnd::Detached;
                            break;
                        }
                    }
                    Err(_) => { end = AttachEnd::Detached; break; }
                }
            }
            _ = sigwinch.recv() => {
                let (rows, cols) = term_size();
                let _ = write_frame(&mut wr, &Request::PtyResize { rows, cols }).await;
            }
        }
    }

    // Restaurar el terminal antes del mensaje final.
    drop(raw);
    match end {
        AttachEnd::Exited(Some(c)) => eprintln!("\r\n— sesión terminó (exit {c}) —"),
        AttachEnd::Exited(None) => eprintln!("\r\n— sesión cerrada —"),
        AttachEnd::Detached => {
            // Cerrar la conexión = detach del lado del daemon (no la mata).
            read_task.abort();
            eprintln!("\r\n— desadjuntado (la sesión sigue viva: `shuma pty ls`) —");
        }
    }
    Ok(())
}
