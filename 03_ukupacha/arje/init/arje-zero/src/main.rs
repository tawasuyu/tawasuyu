//! Ente #0 — el primer Ente. PID 1 del fractal.
//!
//! Reglas no negociables:
//!   1. NUNCA lógica de servicio aquí. Sólo: leer Semilla, cosechar zombis,
//!      mediar capacidades, propagar eventos.
//!   2. Single-threaded. Cualquier paralelismo se delega a Entes worker.
//!      Un panic en un thread de PID 1 = kernel panic.
//!   3. Errores de hijos son *eventos* en `graph_tx`, no `Result` propagado.
//!
//! Este archivo es sólo wireup. La lógica vive en:
//!   - `seed`        : construcción/restauración de la Tarjeta Semilla
//!   - `bus`         : listener Unix + auth via SO_PEERCRED
//!   - `graph::*`    : estado del fractal (lifecycle, topology, shutdown,
//!                     bus_mediator, devices, capabilities)
//!   - `events`      : tipos de eventos del bucle primordial
//!   - crates externos del workspace para CAS, soma, wasm, snapshot, kernel.

mod attest_gate;
mod brain_glue;
mod bus;
mod events;
mod graph;
mod keypair_store;
mod profile;
mod seed;

use anyhow::Context;
use arje_brain::{audit::AuditAction, BrainState, IntrospectServer};
use arje_bus::{BusRequest, PeerCreds};
use arje_kernel::{become_child_subreaper, bootstrap_kernel_surface, spawn_sigchld_stream, spawn_uevent_stream, Watchdog};
use events::{ExitStatus, GraphEvent, ShutdownReason};
use graph::EnteGraph;
use nix::errno::Errno;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{getpid, Pid};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

struct CliArgs {
    checkpoint: Option<PathBuf>,
    restore: Option<PathBuf>,
    rules: Option<PathBuf>,
    rules_out: Option<PathBuf>,
    audit_head: Option<PathBuf>,
    metrics_addr: Option<String>,
    brain_half_life: Option<f64>,
    autopromote_secs: Option<u64>,
    /// Modo dry-run de atestación (`--attest-check [seed]`): corre el gate
    /// real off-boot y reporta, sin volverse PID 1.
    attest_check: bool,
    attest_seed: Option<PathBuf>,
}

fn parse_args() -> CliArgs {
    let mut args = std::env::args().skip(1).peekable();
    let mut checkpoint = None;
    let mut restore = None;
    let mut rules = None;
    let mut rules_out = None;
    let mut audit_head = None;
    let mut metrics_addr = None;
    let mut brain_half_life = None;
    let mut autopromote_secs = None;
    let mut attest_check = false;
    let mut attest_seed = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--checkpoint" => checkpoint = args.next().map(PathBuf::from),
            "--restore" => restore = args.next().map(PathBuf::from),
            "--rules" => rules = args.next().map(PathBuf::from),
            "--rules-out" => rules_out = args.next().map(PathBuf::from),
            "--audit-head" => audit_head = args.next().map(PathBuf::from),
            "--metrics-addr" => metrics_addr = args.next(),
            "--brain-half-life" => brain_half_life = args.next().and_then(|s| s.parse().ok()),
            "--autopromote-secs" => autopromote_secs = args.next().and_then(|s| s.parse().ok()),
            "--attest-check" => {
                attest_check = true;
                // Ruta del seed opcional como siguiente token (si no es otra flag).
                if let Some(next) = args.peek() {
                    if !next.starts_with("--") {
                        attest_seed = args.next().map(PathBuf::from);
                    }
                }
            }
            other => warn!(arg = %other, "argumento desconocido, ignorado"),
        }
    }
    CliArgs {
        checkpoint, restore, rules, rules_out, audit_head,
        metrics_addr, brain_half_life, autopromote_secs,
        attest_check, attest_seed,
    }
}

fn main() -> anyhow::Result<()> {
    bitacora::abrir("arje");
    init_tracing();
    let cli = parse_args();

    // Modo diagnóstico: dry-run de atestación off-boot. NO se vuelve PID 1 ni
    // monta nada — corre el mismo gate que el arranque y reporta. Pensado para
    // que el operador valide ANTES de endurecer la política a Halt y reiniciar.
    if cli.attest_check {
        return attest_check_main(cli.attest_seed);
    }

    let pid = getpid();
    let dev_mode = pid != Pid::from_raw(1);

    if dev_mode {
        warn!(?pid, "ente-zero corriendo en DEV MODE (no PID 1) — kernel surface no se monta");
        return run(cli, true);
    }

    info!("ente-zero despierta como PID 1");
    // Eco temprano al ring buffer del kernel: garantiza que el «estoy
    // vivo» aparezca en TODAS las consolas (VGA + serial), no sólo en
    // el `/dev/console` apuntado por el último `console=` del cmdline.
    write_to_kmsg("despierta como PID 1");
    // Doctrina dura: PID 1 NUNCA puede salir — el kernel haría panic
    // ("Attempted to kill init") y, con `panic=N` en el cmdline, la
    // máquina cae en un reboot-loop. Por eso cualquier fallo de arranque
    // se desvía a una shell de rescate: deja diagnosticar y reparar en
    // vez de reiniciar a ciegas cada diez segundos.
    match run(cli, false) {
        Ok(()) => emergency_shell(
            "el bucle primordial terminó — el fractal pidió shutdown",
        ),
        Err(e) => emergency_shell(&format!("{e:#}")),
    }
}

/// Arranque + bucle primordial. En PID 1, cualquier `Err` que devuelva
/// lo intercepta `main` y lo convierte en shell de rescate.
fn run(cli: CliArgs, dev_mode: bool) -> anyhow::Result<()> {
    if !dev_mode {
        bootstrap_kernel_surface().context("bootstrap kernel surface")?;
        become_child_subreaper().context("PR_SET_CHILD_SUBREAPER")?;
    }

    let card = seed::load(dev_mode, cli.restore.as_deref())?;

    // current_thread runtime: ver doctrina al inicio del módulo.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;

    rt.block_on(primordial_loop(
        card, dev_mode,
        cli.checkpoint, cli.restore, cli.rules, cli.rules_out,
        cli.audit_head, cli.metrics_addr, cli.brain_half_life,
        cli.autopromote_secs,
    ))
}

/// Dry-run de atestación (`--attest-check [seed]`). Carga el seed (del path
/// dado o de los candidatos canónicos), corre el **mismo** gate que el boot
/// (`attest_gate::check`, sin abortar) y reporta por unidad. Sale con código 1
/// si algún binario crítico no atesta — así es gateable en scripts/CI. NO se
/// vuelve PID 1.
fn attest_check_main(seed_path: Option<PathBuf>) -> anyhow::Result<()> {
    use anyhow::anyhow;

    let seed = match seed_path {
        Some(p) => arje_brain::load_card_file(&p)
            .with_context(|| format!("cargar seed {}", p.display()))?,
        None => {
            let cands = [
                "/ente/seed.card.json", "/ente/seed.card",
                "seed.card.json", "seed.card",
            ];
            let found = cands.iter().map(PathBuf::from).find(|p| p.exists());
            match found {
                Some(p) => arje_brain::load_card_file(&p)
                    .with_context(|| format!("cargar seed {}", p.display()))?,
                None => return Err(anyhow!(
                    "no encontré un seed; pasá la ruta: arje-zero --attest-check <seed.card.json>"
                )),
            }
        }
    };

    println!("== arje-zero · dry-run de atestación (--attest-check) ==");
    println!(
        "seed: {} · política {:?} · {} concesión(es) en el manifiesto",
        seed.label, seed.attest_policy, seed.attest.len(),
    );

    if seed.attest.is_empty() {
        println!(
            "seed SIN manifiesto de atestación (attest vacío) — el gate es no-op; \
             nada que verificar. Firmá el seed con `arje-packager --rootkey` para activarlo.",
        );
        return Ok(());
    }

    match attest_gate::ancla_fuente() {
        Some(src) => println!("ancla soberana externa: {src}"),
        None if seed.attest_rootkey.is_some() => println!(
            "ancla soberana externa: NINGUNA — se usa la rootkey auto-declarada del \
             seed (modelo débil: un seed reescrito podría reemplazarla)",
        ),
        None => println!(
            "ancla soberana externa: NINGUNA y el seed no declara rootkey — sólo se \
             valida firma+hash, no la procedencia",
        ),
    }
    println!();

    let verdicts = attest_gate::check(&seed);
    let mut fail = 0usize;
    for v in &verdicts {
        let mark = if v.verdict.es_ok() {
            "✓"
        } else {
            fail += 1;
            "✗"
        };
        println!("  {mark} {:<44} {}", v.binary, v.verdict.motivo());
    }
    let ok = verdicts.len().saturating_sub(fail);
    println!();
    println!(
        "Resumen: {ok} ✓ / {fail} ✗ sobre {} binario(s) crítico(s).",
        verdicts.len(),
    );

    if fail > 0 {
        match seed.attest_policy {
            arje_card::AttestPolicy::Halt => println!(
                "Con política Halt, este arranque ABORTARÍA a la shell de rescate. \
                 NO endurezcas a Halt hasta que esto dé 0 ✗.",
            ),
            arje_card::AttestPolicy::Degraded => println!(
                "Con política Degraded, arrancaría DEGRADADO (binarios marcados comprometidos).",
            ),
            arje_card::AttestPolicy::Warn => println!(
                "Con política Warn, arrancaría igual (sólo se registra el aviso).",
            ),
        }
        std::process::exit(1);
    }
    println!("Todos los binarios críticos atestan. Es seguro endurecer la política a Halt.");
    Ok(())
}

/// Último recurso de PID 1: imprime el diagnóstico en la consola y abre
/// una shell de rescate. **Nunca retorna** — si lo hiciera, el proceso
/// saldría y el kernel haría panic.
fn emergency_shell(reason: &str) -> ! {
    let banner = format!(
        "\n\n\
         ===============  arje-zero — ARRANQUE FALLIDO  ================\n\
         {reason}\n\
         ---------------------------------------------------------------\n\
         Se abre una shell de rescate sobre la consola. Revisá el sistema\n\
         (p. ej. /ente/seed.card.json y los binarios en /usr/sbin) y\n\
         reiniciá con `reboot -f`. Salir de la shell la vuelve a abrir.\n\n",
    );
    write_to_console(&banner);
    error!(reason = %reason.replace('\n', " "), "arranque de PID 1 fallido");
    loop {
        match spawn_console_shell() {
            Ok(status) => write_to_console(&format!(
                "\n[arje-zero] la shell de rescate terminó ({status}) — reabriendo.\n",
            )),
            Err(e) => {
                write_to_console(&format!(
                    "\n[arje-zero] no hay shell de rescate disponible: {e}\n\
                     PID 1 queda en espera pasiva — usá la consola del proveedor.\n",
                ));
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
        }
    }
}

/// Abre una shell interactiva con stdin/stdout/stderr sobre
/// `/dev/console` y espera a que termine.
fn spawn_console_shell() -> std::io::Result<std::process::ExitStatus> {
    let shell = ["/bin/sh", "/bin/bash", "/usr/bin/sh", "/usr/bin/bash"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "ninguna shell en /bin ni /usr/bin",
            )
        })?;
    let console = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/console")?;
    std::process::Command::new(shell)
        .stdin(console.try_clone()?)
        .stdout(console.try_clone()?)
        .stderr(console)
        .env("HOME", "/root")
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("PS1", "arje-rescate# ")
        .env("TERM", "linux")
        .status()
}

/// Escribe un mensaje directo a `/dev/console`, **a `/dev/kmsg`**
/// (ring buffer del kernel, que se hace eco a todas las consolas
/// registradas), y a stderr de respaldo. Así el banner se ve en VGA y
/// serial sin importar cuál sea el último `console=` del cmdline.
fn write_to_console(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/console") {
        let _ = f.write_all(msg.as_bytes());
    }
    write_to_kmsg(msg);
    eprint!("{msg}");
}

/// Escribe un mensaje al `/dev/kmsg` del kernel — éste lo replica a
/// todas las consolas registradas (`console=` del cmdline). Es el
/// canal que usa systemd para que sus avisos se vean tanto en la VGA
/// como en el serial.
fn write_to_kmsg(msg: &str) {
    use std::io::Write;
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") else {
        return;
    };
    // `<1>` = ALERT, prioridad alta — aparece incluso con loglevel bajo.
    for line in msg.lines() {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            let _ = writeln!(f, "<1>arje-zero: {trimmed}");
        }
    }
}

async fn primordial_loop(
    seed_card: arje_card::EntityCard,
    dev_mode: bool,
    checkpoint_path: Option<PathBuf>,
    restore_path: Option<PathBuf>,
    rules_path: Option<PathBuf>,
    rules_out: Option<PathBuf>,
    audit_head: Option<PathBuf>,
    metrics_addr: Option<String>,
    brain_half_life: Option<f64>,
    autopromote_secs: Option<u64>,
) -> anyhow::Result<()> {
    info!(seed_id = %seed_card.id, label = %seed_card.label, "Ente #0 entra al bucle primordial");

    let (graph_tx, mut graph_rx) = mpsc::channel::<GraphEvent>(64);
    let mut sigchld = spawn_sigchld_stream()?;
    // Uevents puede fallar en dev (sin CAP_NET_ADMIN). Degradamos a un
    // canal nunca-listo en lugar de abortar el bucle primordial.
    let mut uevents = match spawn_uevent_stream() {
        Ok(rx) => rx,
        Err(e) => {
            warn!(?e, "uevents deshabilitados (probablemente falta CAP_NET_ADMIN)");
            let (_keep_tx, rx) = mpsc::channel::<arje_kernel::UEvent>(1);
            std::mem::forget(_keep_tx);
            rx
        }
    };

    // Bus interno: listener antes de spawn de hijos para que su Announce
    // tenga adónde llegar. Su path se inyecta en ENTE_BUS_SOCK por soma.
    let bus_sock = bus::default_socket_path();
    let bus_path = bus::spawn_bus(bus_sock, graph_tx.clone())?;
    arje_soma::set_bus_sock(bus_path.to_string_lossy().into_owned());

    // Brahman protocol: handshake socket + broker compartido.
    //
    // Es un canal paralelo al ente-bus, dedicado a módulos "brahman
    // conscientes" que se presentan con una Card y declaran flujos
    // tipados. Si el bind falla (socket en uso, FS no escribible),
    // degradamos a "modo bus-only" — la doctrina de PID 1 no rompe
    // por subsistemas opcionales.
    // Contexto operativo del broker: configurable por env var. Útil para
    // distinguir test/prod/foreground sin recompilar. Sin la var, los
    // biases per-contexto declarados en las Cards quedan inactivos.
    let broker_context = std::env::var("BRAHMAN_BROKER_CONTEXT").ok();
    if let Some(ctx) = &broker_context {
        info!(context = %ctx, "brahman broker bajo contexto operativo");
    }
    let chasqui_broker = std::sync::Arc::new(tokio::sync::Mutex::new(
        chasqui_broker::Broker::new(chasqui_broker::BrokerConfig {
            strategy: chasqui_broker::MatchStrategy::default(),
            current_context: broker_context.clone(),
        }),
    ));

    // Brahman-net opcional: si BRAHMAN_LISTEN_MULTIADDR está set,
    // levantamos la malla P2P y la pasamos como ServerConfig.net (Fase
    // 2 wire) para que cada Card con outputs se anuncie al DHT y
    // pueda ser descubierta por nodos remotos. Identidad libp2p
    // persistida en disco vía keypair_store (peer_id estable across
    // reboots).
    let card_net = setup_brahman_net(dev_mode).await;

    // Política opcional de peers libp2p: allowlist + denylist + hot
    // reload. Activada si BRAHMAN_PEER_ALLOWLIST o BRAHMAN_PEER_DENYLIST
    // están set. Sin ninguna, modo totalmente abierto (Fase 3 sin
    // restricción adicional). El watcher se queda vivo en background
    // observando los archivos para hot reload.
    let (brahman_policy, _policy_watcher) = setup_brahman_policy();

    // Si tenemos AMBOS net y policy, attachamos: el deny de la
    // policy se proyecta al block_list del swarm para rechazar
    // conexiones ANTES del Noise handshake (más eficiente que
    // rechazar en el handshake brahman). Cada hot-reload de la
    // policy también re-sincroniza vía diff.
    if let (Some(net), Some(policy)) = (&card_net, &brahman_policy) {
        policy.attach_to_net(net.clone());
        let (allow, deny) = policy.sizes();
        info!(
            allow = ?allow,
            deny = deny,
            "policy attached al swarm — denies enforcedeados a nivel libp2p"
        );
    }

    let brahman_sock = card_handshake::transport::default_socket_path();
    match card_handshake::server::Server::bind(
        &brahman_sock,
        card_handshake::server::ServerConfig {
            init_attached: true,
            broker: Some(chasqui_broker.clone()),
            net: card_net.clone(),
            policy: brahman_policy.clone(),
        },
    ) {
        Ok(server) => {
            info!(socket = %brahman_sock.display(), "brahman handshake escuchando (Unix)");
            // Si hay malla P2P, además del Unix accept loop levantamos
            // el accept loop libp2p sobre el mismo Server compartido.
            // Las sesiones locales y remotas conviven en las mismas
            // tablas (sessions, push_table, broker).
            let server = std::sync::Arc::new(server);
            if let Some(net) = card_net.clone() {
                let s_libp2p = server.clone();
                let n_libp2p = net.clone();
                tokio::spawn(async move {
                    if let Err(e) = card_handshake::network::run_libp2p_accept_loop(
                        s_libp2p, n_libp2p,
                    )
                    .await
                    {
                        warn!(?e, "brahman handshake libp2p accept loop cayó");
                    }
                });
                info!(
                    "brahman handshake escuchando también vía libp2p (peer_id {})",
                    net.peer_id
                );
            }
            // Unix accept loop: usa Arc<Server> en lugar del consume
            // de run() para coexistir con el libp2p accept loop.
            let s_unix = server.clone();
            tokio::spawn(async move {
                loop {
                    match s_unix.accept_one().await {
                        Ok(session) => {
                            tokio::spawn(async move {
                                if let Err(e) = session.handle().await {
                                    warn!(?e, "session Unix terminó con error");
                                }
                            });
                        }
                        Err(e) => {
                            warn!(?e, "brahman handshake accept_one Unix falló");
                            break;
                        }
                    }
                }
            });
        }
        Err(e) => {
            warn!(?e, socket = %brahman_sock.display(), "brahman handshake deshabilitado");
        }
    }

    // Brahman admin: socket separado para snapshots de estado (sesiones +
    // matches del broker). Misma política de degradación grácil.
    let admin_sock = card_admin::transport::default_socket_path();
    match card_admin::server::AdminServer::bind(
        &admin_sock,
        chasqui_broker.clone(),
        card_admin::server::AdminConfig {
            init_attached: true,
            current_context: broker_context.clone(),
        },
    ) {
        Ok(admin) => {
            info!(socket = %admin_sock.display(), "brahman admin escuchando");
            tokio::spawn(async move {
                if let Err(e) = admin.run().await {
                    warn!(?e, "brahman admin server cayó");
                }
            });
        }
        Err(e) => {
            warn!(?e, socket = %admin_sock.display(), "brahman admin deshabilitado");
        }
    }

    // Atestación al arranque (A2): verificar los binarios críticos contra el
    // manifiesto firmado del seed ANTES de incarnar el target. Con política
    // `Halt` un fallo aborta acá (antes del genesis); los veredictos se vuelcan
    // al audit log una vez que el brain existe (más abajo). `seed_card` se mueve
    // al grafo en la línea siguiente, así que capturamos la política antes.
    let attest_policy = seed_card.attest_policy;
    let attest_verdicts = attest_gate::run(&seed_card)?;

    let mut graph = EnteGraph::new(seed_card);
    graph.instantiate_seed_dependencies(&graph_tx).await?;

    // Cerebro: BrainState compartido + servidor de introspección.
    // Window de 1024 eventos — suficiente para correlaciones interesantes
    // sin gastar memoria de PID 1. En dev bajamos el umbral de cristalización
    // para que el demo (pocos eventos) produzca cristales observables.
    let mut brain = if dev_mode {
        // Umbrales relajados para que el demo (pocos eventos) produzca
        // cristales observables. Con P(b|a) normalizada a [0,1], los
        // valores típicos en muestras pequeñas son 0.2-0.5.
        BrainState::with_params(1024, arje_brain::CrystallizationParams {
            min_support: 2,
            min_conditional_prob: 0.3,
            min_pmi: 1.0,
        })
    } else {
        BrainState::new(1024)
    };
    if let Some(out_path) = rules_out {
        brain = brain.with_rules_out(out_path);
    }
    if let Some(hl) = brain_half_life {
        let mut obs = brain.observer.write().await;
        // Reemplazar con un observer nuevo que tenga half-life. Estado
        // anterior (vacío en este punto) descartado.
        *obs = arje_brain::Observer::new(1024).with_half_life(hl);
        info!(hl_secs = hl, "observer con time-decay activo");
    }
    if let Some(secs) = autopromote_secs {
        arje_brain::spawn_autopromote_loop(
            brain.clone(),
            arje_brain::AutopromoteParams {
                interval_secs: secs,
                threshold: brain.params, // mismo threshold que crystals manuales
            },
        );
    }

    // Volcado de la atestación al arranque (A2) a la cadena de audit, ahora
    // que el brain existe. Quedan ancladas al CAS y son auditables con
    // `verify_chain_from_cas` / `brainctl audit --kind attestation-check`.
    if !attest_verdicts.is_empty() {
        let mut audit = brain.audit.write().await;
        for v in &attest_verdicts {
            audit.append(AuditAction::AttestationCheck {
                binary: v.binary.clone(),
                got_hash: v.got_hash,
                verdict: v.verdict.motivo().to_string(),
                policy: format!("{:?}", attest_policy),
            });
        }
    }

    // Brain restore: si hay --restore <path>, cargamos el snapshot adjunto
    // <path>.brain.json. Counters preservados across reboots.
    if let Some(rpath) = &restore_path {
        let brain_path = rpath.with_extension("brain.json");
        if brain_path.exists() {
            match read_brain_snapshot(&brain_path) {
                Ok(snap) => {
                    let total = snap.total;
                    let kinds = snap.marginal.len();
                    let restored = arje_brain::Observer::from_snapshot(snap);
                    *brain.observer.write().await = restored;
                    info!(
                        path = %brain_path.display(),
                        total, kinds,
                        "brain snapshot restaurado"
                    );
                }
                Err(e) => warn!(?e, path = %brain_path.display(), "brain snapshot read falló"),
            }
        }
    }
    // Si --audit-head, configuramos el head pointer y arrancamos auto-flush.
    if let Some(head_path) = audit_head {
        // Re-creamos el AuditLog con head pointer.
        let new_audit = arje_brain::audit::AuditLog::new()
            .with_head_pointer(head_path);
        *brain.audit.write().await = new_audit;
        spawn_audit_auto_flush(brain.clone());
    }

    // Carga inicial de reglas desde JSON/JSONL si --rules path proporcionado.
    if let Some(path) = &rules_path {
        match arje_brain::load_rules_file(path) {
            Ok(rules) => {
                let mut engine = brain.engine.write().await;
                for r in rules {
                    engine.insert(r);
                }
                info!(count = engine.len(), path = %path.display(), "reglas cargadas");
            }
            Err(e) => warn!(?e, path = %path.display(), "carga de reglas falló"),
        }
    }

    // Endpoint Prometheus opcional. En dev por defecto en 127.0.0.1:9911 si
    // el flag no se pasó.
    let metrics_addr = metrics_addr.or_else(|| {
        if dev_mode { Some("127.0.0.1:9911".to_string()) } else { None }
    });
    if let Some(addr_s) = metrics_addr {
        match addr_s.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                let s = brain.clone();
                tokio::spawn(async move {
                    if let Err(e) = arje_brain::serve_metrics(s, addr).await {
                        warn!(?e, "metrics server cayó");
                    }
                });
            }
            Err(e) => warn!(?e, addr = %addr_s, "metrics-addr inválido"),
        }
    }
    spawn_brain_introspect(brain.clone());
    // Sembrar las raíces vivas del CAS para el GC ANTES de servir introspect:
    // así un `gc-cas` temprano ya respeta los Wasm y binarios cosechados del
    // seed. Luego se refresca en el tick periódico (cubre spawns dinámicos).
    *brain.cas_roots.write().await = graph.cas_roots();
    let brain_sink = brain_glue::GraphSink {
        graph_tx: graph_tx.clone(),
    };

    // Demo automático del forwarding (sólo dev, sólo si el binario existe).
    if dev_mode && std::path::Path::new("target/debug/ente-echo").exists() {
        spawn_echo_smoke_test(bus_path.clone());
    }

    // En dev mode no tenemos hijos por defecto y el bucle se quedaría inerte.
    let dev_exit = if dev_mode {
        Some(tokio::time::sleep(Duration::from_secs(4)))
    } else {
        None
    };
    tokio::pin!(dev_exit);

    // Watchdog de hardware: si este bucle se cuelga y deja de acariciar,
    // el kernel reinicia la máquina en vez de dejarla muerta para siempre.
    // Sólo en PID 1 real (no dev). `ARJE_WATCHDOG_SECS=0` lo desactiva;
    // sin la var, default 30 s. Si no hay `/dev/watchdog`, sigue sin él.
    let mut watchdog = if dev_mode {
        None
    } else {
        let secs = std::env::var("ARJE_WATCHDOG_SECS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(30);
        if secs == 0 {
            info!("watchdog deshabilitado (ARJE_WATCHDOG_SECS=0)");
            None
        } else {
            Watchdog::arm(secs)
        }
    };
    let pet_interval = watchdog
        .as_ref()
        .map(|w| w.pet_interval())
        .unwrap_or_else(|| Duration::from_secs(3600));
    // Tick que despierta el bucle a tiempo para acariciar cuando está ocioso.
    let mut wd_tick = tokio::time::interval(pet_interval);
    wd_tick.tick().await; // descartar primer tick inmediato
    let mut last_pet = std::time::Instant::now();

    // GC de capability grants expirados — corre cada 10 segundos.
    let mut grant_purge = tokio::time::interval(Duration::from_secs(10));
    grant_purge.tick().await; // descartar primer tick inmediato

    loop {
        tokio::select! {
            biased;

            Some(_) = sigchld.recv() => {
                reap_until_empty(&mut graph, &graph_tx).await;
            }

            Some(uevt) = uevents.recv() => {
                graph.on_uevent(uevt, &graph_tx).await;
            }

            Some(evt) = graph_rx.recv() => {
                // Cerebro observa antes que el grafo mute. Snapshot del
                // SubjectInfo se hace contra el estado pre-mutación.
                feed_brain(&brain, &brain_sink, &graph, &evt).await;
                if dispatch_graph_event(&mut graph, evt, &graph_tx, &checkpoint_path, &brain).await {
                    // Shutdown limpio: desarmar el watchdog para que soltar el
                    // device no reinicie la máquina.
                    if let Some(wd) = watchdog.take() {
                        wd.disarm();
                    }
                    return Ok(());
                }
            }

            _ = grant_purge.tick() => {
                let n = graph.purge_expired_grants();
                if n > 0 {
                    info!(purged = n, active = graph.active_grants_count(), "GC capability grants");
                }
                // Refrescar las raíces vivas del CAS (Wasm + binarios cosechados)
                // para que un `gc-cas` no barra lo que entró/salió desde el último
                // tick. Barato: recorre las Cards vivas.
                *brain.cas_roots.write().await = graph.cas_roots();
            }

            _ = wd_tick.tick() => {
                // Sólo despierta el bucle; el acariciado real va abajo.
            }

            _ = async { dev_exit.as_mut().as_pin_mut().unwrap().await }, if dev_mode => {
                info!("dev mode: timer expirado, cerrando bucle primordial");
                let _ = graph_tx.send(GraphEvent::Shutdown {
                    reason: ShutdownReason::SeedRequested,
                }).await;
            }
        }

        // Latido del watchdog tras CADA iteración del bucle. Si un handler de
        // arriba se cuelga, no se llega acá → no se acaricia → el kernel
        // reinicia. El chequeo "due" evita falsos reboots por starvation bajo
        // ráfaga de eventos: cada vuelta que procesa algo también late si toca.
        let mut wd_dead = false;
        if let Some(wd) = watchdog.as_mut() {
            if last_pet.elapsed() >= pet_interval {
                match wd.pet() {
                    Ok(()) => last_pet = std::time::Instant::now(),
                    Err(e) => {
                        warn!(?e, "watchdog murió al acariciar — desarmando supervisión");
                        wd_dead = true;
                    }
                }
            }
        }
        if wd_dead {
            watchdog = None;
        }
    }
}

/// Re-floor: drena los Entes aparcados que ya tienen piso (su capability volvió
/// a tener proveedor) y los re-encola como `SpawnRequest` en orden topológico,
/// por el canal (no reentrante; cada re-spawn vuelve a disparar el drenaje en
/// cascada). Se llama tras los eventos que pueden AGREGAR proveedores: spawns y
/// requests del bus (incluido `UpdateCapabilities`, la señal de readiness de un
/// daemon). Barato cuando no hay nadie aparcado.
async fn refloor(graph: &mut EnteGraph, tx: &mpsc::Sender<GraphEvent>) {
    let seed = graph.seed_id();
    for card in graph.drain_refloorable() {
        let label = card.label.clone();
        info!(%label, "re-floor: piso disponible — re-spawneando Ente aparcado");
        if tx
            .send(GraphEvent::SpawnRequest { card, requester: seed })
            .await
            .is_err()
        {
            warn!(%label, "re-floor: graph_tx cerrado");
        }
    }
}

/// Devuelve `true` si el bucle primordial debe terminar.
async fn dispatch_graph_event(
    graph: &mut EnteGraph,
    evt: GraphEvent,
    tx: &mpsc::Sender<GraphEvent>,
    checkpoint: &Option<PathBuf>,
    brain: &BrainState,
) -> bool {
    match evt {
        GraphEvent::EnteDied { id, status } => {
            graph.on_death(id, status, tx).await;
        }
        GraphEvent::CapabilityRequested { from, cap, reply } => {
            graph.mediate_capability(from, cap, reply).await;
        }
        GraphEvent::SpawnRequest { card, requester } => {
            if let Err(e) = graph.authorize_and_spawn(card, requester).await {
                warn!(?e, "spawn request error");
            }
            // Si este spawn devolvió el piso (registró un proveedor que esperaban
            // Entes aparcados — p. ej. el compositor que reaparece), re-erígelos.
            refloor(graph, tx).await;
        }
        GraphEvent::BusRequest { peer, from, request, outbound, reply } => {
            if let Some(action) = bus_request_to_audit(&peer, &from, &request) {
                brain.audit.write().await.append(action);
            }
            graph.on_bus_request(peer, from, request, outbound, reply).await;
            // Un request del bus puede AGREGAR proveedores: `UpdateCapabilities`
            // (un daemon anuncia su readiness — "ya escucho mi socket"), `RunCard`,
            // `SpawnCardFromDisk`. Cada uno puede ser el piso que esperaba alguien.
            refloor(graph, tx).await;
        }
        GraphEvent::BusResponse { seq, response } => {
            graph.on_bus_response(seq, response).await;
        }
        GraphEvent::BusConnClosed { ente_id } => {
            graph.on_bus_conn_closed(ente_id).await;
        }
        GraphEvent::BrainInvoke { cap, blob } => {
            graph.forward_brain_invoke(cap, blob).await;
        }
        GraphEvent::BrainNotify { target_id, message } => {
            graph.forward_brain_notify(target_id, message).await;
        }
        GraphEvent::BrainSpawn { card } => {
            graph.forward_brain_spawn(card).await;
        }
        GraphEvent::BrainInhibit { reason } => {
            brain.audit.write().await.append(AuditAction::BrainInhibit {
                reason: reason.clone(),
            });
            graph.apply_brain_inhibit(reason);
        }
        GraphEvent::Shutdown { reason } => {
            warn!(?reason, "shutdown del fractal");
            if let Some(path) = checkpoint.as_ref() {
                // Snapshot del grafo
                let snap = graph.snapshot();
                match snap.write(path) {
                    Ok(()) => info!(path = %path.display(), entes = snap.entes.len(), "snapshot fractal persistido"),
                    Err(e) => warn!(?e, "snapshot write falló"),
                }
                // Snapshot del cerebro (observer state) en archivo adjunto
                let brain_path = path.with_extension("brain.json");
                let obs_snap = brain.observer.write().await.snapshot();
                match write_brain_snapshot(&brain_path, &obs_snap) {
                    Ok(()) => info!(
                        path = %brain_path.display(),
                        total = obs_snap.total,
                        kinds = obs_snap.marginal.len(),
                        "snapshot brain persistido"
                    ),
                    Err(e) => warn!(?e, "brain snapshot write falló"),
                }
            }
            graph.cascade_shutdown().await;
            return true;
        }
    }
    false
}

/// Mapea una `BusRequest` a un `AuditAction` cuando vale la pena registrarla.
/// Cubre acciones privilegiadas (KillEnte, SpawnCardFromDisk) y todas las de
/// power-mgmt (incluso anónimas — anonimato es información). Otras (Announce,
/// Invoke, ListEntes, UpdateCapabilities) son routine y no se auditan aquí.
fn bus_request_to_audit(
    peer: &PeerCreds,
    from: &Option<ulid::Ulid>,
    req: &BusRequest,
) -> Option<AuditAction> {
    match req {
        BusRequest::KillEnte { target, signal } => from.map(|caller| AuditAction::KillEnte {
            caller, target: *target, signal: *signal,
        }),
        BusRequest::SpawnCardFromDisk { name } => from.map(|caller| AuditAction::SpawnCardFromDisk {
            caller, name: name.clone(),
        }),
        BusRequest::RunCard { card } => from.map(|caller| AuditAction::RunCard {
            caller, label: card.label.clone(),
        }),
        BusRequest::SetCpuWeight { cgroup_path, weight } => from.map(|caller| AuditAction::Cgroup {
            caller, cgroup: cgroup_path.clone(), change: format!("cpu.weight={weight}"),
        }),
        BusRequest::Freeze { cgroup_path, frozen } => from.map(|caller| AuditAction::Cgroup {
            caller, cgroup: cgroup_path.clone(),
            change: format!("freeze={}", if *frozen { "on" } else { "off" }),
        }),
        BusRequest::SetMemoryMax { cgroup_path, bytes } => from.map(|caller| AuditAction::Cgroup {
            caller, cgroup: cgroup_path.clone(), change: format!("memory.max={bytes}"),
        }),
        BusRequest::SetMemoryHigh { cgroup_path, bytes } => from.map(|caller| AuditAction::Cgroup {
            caller, cgroup: cgroup_path.clone(), change: format!("memory.high={bytes}"),
        }),
        BusRequest::SetIoWeight { cgroup_path, weight } => from.map(|caller| AuditAction::Cgroup {
            caller, cgroup: cgroup_path.clone(), change: format!("io.weight={weight}"),
        }),
        BusRequest::PowerOff { interactive } => Some(AuditAction::PowerMgmt {
            caller: *from, peer_pid: peer.pid, kind: "PowerOff".into(), interactive: *interactive,
        }),
        BusRequest::Reboot { interactive } => Some(AuditAction::PowerMgmt {
            caller: *from, peer_pid: peer.pid, kind: "Reboot".into(), interactive: *interactive,
        }),
        BusRequest::Suspend { interactive } => Some(AuditAction::PowerMgmt {
            caller: *from, peer_pid: peer.pid, kind: "Suspend".into(), interactive: *interactive,
        }),
        BusRequest::Hibernate { interactive } => Some(AuditAction::PowerMgmt {
            caller: *from, peer_pid: peer.pid, kind: "Hibernate".into(), interactive: *interactive,
        }),
        _ => None,
    }
}

async fn reap_until_empty(graph: &mut EnteGraph, tx: &mpsc::Sender<GraphEvent>) {
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => return,
            Ok(WaitStatus::Exited(pid, code)) => {
                emit_death(graph, tx, pid, ExitStatus::Exit(code)).await;
            }
            Ok(WaitStatus::Signaled(pid, sig, _core)) => {
                emit_death(graph, tx, pid, ExitStatus::Killed(sig)).await;
            }
            Ok(_) => continue, // Stopped/Continued — irrelevantes
            Err(Errno::ECHILD) => return,
            Err(e) => {
                error!(?e, "waitpid fallo no recuperable en bucle de reaping");
                return;
            }
        }
    }
}

async fn emit_death(
    graph: &EnteGraph,
    tx: &mpsc::Sender<GraphEvent>,
    pid: Pid,
    status: ExitStatus,
) {
    let id = match graph.lookup_pid(pid) {
        Some(id) => id,
        None => {
            // Proceso adoptado (subreaper): no está en nuestro grafo.
            info!(?pid, ?status, "huérfano cosechado (no en grafo)");
            return;
        }
    };
    let _ = tx.send(GraphEvent::EnteDied { id, status }).await;
}

fn spawn_echo_smoke_test(bus_path: PathBuf) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        match arje_bus::BusClient::connect(&bus_path).await {
            Ok(mut client) => {
                let req = arje_bus::BusRequest::Invoke {
                    cap: arje_echo::echo_capability(),
                    blob: b"hola fractal forwardeado".to_vec(),
                };
                match client.call(req).await {
                    Ok(arje_bus::BusResponse::Invoked { result }) => {
                        info!(echo = %String::from_utf8_lossy(&result), "Invoke ECHO round-trip OK");
                    }
                    Ok(other) => warn!(?other, "Invoke ECHO respuesta inesperada"),
                    Err(e) => warn!(?e, "Invoke ECHO falló"),
                }
            }
            Err(e) => warn!(?e, "no se pudo conectar al bus para test"),
        }
    });
}

fn write_brain_snapshot(path: &std::path::Path, snap: &arje_brain::observer::ObserverSnapshot) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(snap)?;
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn read_brain_snapshot(path: &std::path::Path) -> anyhow::Result<arje_brain::observer::ObserverSnapshot> {
    let bytes = std::fs::read(path)?;
    let snap: arje_brain::observer::ObserverSnapshot = serde_json::from_slice(&bytes)?;
    Ok(snap)
}

/// Telemetría persistente del arranque: el log de arje-zero (PID 1) sobrevive el
/// reboot y se lee desde otra sesión. Sin esto, un arranque que muere no deja
/// rastro (kmsg/consola se pierden al reiniciar). Acá quedan los nacimientos y
/// **muertes con exit-status** de cada Ente, y los panics de PID 1.
const BOOT_LOG: &str = "/var/log/arje/boot.log";

/// Writer del boot.log (append). Re-crea el dir best-effort por si `/` recién
/// pasó a rw después de init_tracing. Si no se puede abrir, cae a un sink mudo
/// (nunca paniquea desde el camino de logging).
fn boot_log_writer() -> Box<dyn std::io::Write> {
    let _ = std::fs::create_dir_all("/var/log/arje");
    match std::fs::OpenOptions::new().create(true).append(true).open(BOOT_LOG) {
        Ok(f) => Box::new(f),
        Err(_) => Box::new(std::io::sink()),
    }
}

/// Un panic de PID 1 = la muerte del fractal. Lo dejamos en disco (boot.log) y
/// en kmsg/consola para que el «por qué» no se pierda al reiniciar.
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("PANIC en arje-zero (PID 1): {info}\n");
        write_to_console(&msg);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(BOOT_LOG) {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
        prev(info);
    }));
}

fn init_tracing() {
    use tracing_subscriber::fmt;
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    use tracing_subscriber::EnvFilter;
    let _ = std::fs::create_dir_all("/var/log/arje");
    install_panic_hook();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_zero=debug,info"));
    // Tee: cada evento al boot.log en disco (telemetría que sobrevive el reboot)
    // Y a stderr/consola (vista en vivo). `with_ansi(false)`: el archivo queda
    // legible sin códigos de color.
    let writer = (boot_log_writer as fn() -> Box<dyn std::io::Write>).and(std::io::stderr);
    // try_init: bitacora::abrir ya puede haber instalado el subscriber global.
    let _ = fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_ansi(false)
        .with_writer(writer)
        .try_init();
}

fn brain_introspect_path() -> PathBuf {
    if let Ok(p) = std::env::var("ENTE_BRAIN_SOCK") {
        return p.into();
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()));
    format!("{runtime}/ente-brain.sock").into()
}

/// Auto-flush del audit log a CAS cada 10 segundos. Ejecuta best-effort:
/// si el flush falla lo logeamos pero no abortamos. La integridad del log
/// queda garantizada por su hash chain — re-flushar es idempotente.
fn spawn_audit_auto_flush(state: BrainState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        tick.tick().await; // descartar primer tick inmediato
        loop {
            tick.tick().await;
            let mut audit = state.audit.write().await;
            match audit.flush_to_cas() {
                Ok(0) => {} // nada nuevo
                Ok(n) => info!(written = n, total = audit.flushed_count(), "audit auto-flush"),
                Err(e) => warn!(?e, "audit auto-flush falló"),
            }
        }
    });
}

fn spawn_brain_introspect(state: BrainState) {
    let path = brain_introspect_path();
    tokio::spawn(async move {
        let server = IntrospectServer::new(state);
        if let Err(e) = server.serve(&path).await {
            warn!(?e, "introspect server cayó");
        }
    });
}

/// Registra el evento en el observer y dispatcha cualquier regla matched.
/// Para reglas Sequence: pasamos los últimos N eventos del observer como
/// history al engine.
async fn feed_brain(
    brain: &BrainState,
    sink: &brain_glue::GraphSink,
    graph: &EnteGraph,
    evt: &GraphEvent,
) {
    let Some((kind, subj)) = brain_glue::graph_event_to_brain(evt, graph) else { return };
    let history: Vec<arje_brain::TimedEvent> = {
        let mut obs = brain.observer.write().await;
        obs.record(kind.clone());
        // Snapshot de los últimos 16 eventos — suficiente para cualquier
        // Sequence pattern razonable. Clone hace una sola alocación.
        obs.recent(16).cloned().collect()
    };
    let rules = {
        let engine = brain.engine.read().await;
        engine.dispatch(&kind, &subj, &history)
    };
    if !rules.is_empty() {
        arje_brain::dispatch_actions(&rules, sink).await;
    }
}

/// Inicializa la malla `brahman-net` opcional. Activa sólo si
/// `BRAHMAN_LISTEN_MULTIADDR` está set. Identidad libp2p persistente
/// vía `keypair_store`. Bootstrap del DHT vía `BRAHMAN_BOOTSTRAP_PEERS`
/// (lista coma-separada de multiaddrs, opcional).
///
/// Toda fase de setup degrada grácilmente: si la keypair no carga,
/// si el listen falla, si bootstrap dial falla — loggea y devuelve
/// `None`. El Init sigue funcionando en modo Unix-only.
async fn setup_brahman_net(
    dev_mode: bool,
) -> Option<std::sync::Arc<card_net::BrahmanNet>> {
    let listen_addr = match std::env::var("BRAHMAN_LISTEN_MULTIADDR") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            tracing::debug!(
                "brahman-net deshabilitado (sin BRAHMAN_LISTEN_MULTIADDR)"
            );
            return None;
        }
    };

    let multiaddr: card_net::Multiaddr = match listen_addr.parse() {
        Ok(m) => m,
        Err(e) => {
            warn!(addr = %listen_addr, ?e, "BRAHMAN_LISTEN_MULTIADDR inválido — net deshabilitado");
            return None;
        }
    };

    let keypair_path = keypair_store::default_path(dev_mode);
    let (keypair, loaded) = match keypair_store::load_or_generate(&keypair_path) {
        Ok(kp) => kp,
        Err(e) => {
            warn!(path = %keypair_path.display(), ?e, "no pude cargar/generar keypair libp2p — net deshabilitado");
            return None;
        }
    };
    info!(
        path = %keypair_path.display(),
        peer_id = %keypair.public().to_peer_id(),
        loaded = loaded,
        "identidad libp2p {}",
        if loaded { "cargada" } else { "generada y persistida" }
    );

    let net = match card_net::BrahmanNet::with_keypair(keypair) {
        Ok(n) => std::sync::Arc::new(n),
        Err(e) => {
            warn!(?e, "BrahmanNet::with_keypair falló — net deshabilitado");
            return None;
        }
    };

    let actual = net.listen(multiaddr).await;
    info!(addr = %actual, peer_id = %net.peer_id, "brahman-net escuchando");

    // Bootstrap opcional: dial-ar a peers conocidos para entrar al
    // DHT. Sin bootstrap, el nodo arranca aislado hasta que alguien
    // se conecte a él.
    if let Ok(bootstrap) = std::env::var("BRAHMAN_BOOTSTRAP_PEERS") {
        let mut dialed = 0usize;
        for entry in bootstrap.split(',').filter(|s| !s.is_empty()) {
            match entry.parse::<card_net::Multiaddr>() {
                Ok(addr) => {
                    net.dial(addr.clone());
                    dialed += 1;
                    tracing::debug!(peer = %addr, "dial bootstrap");
                }
                Err(e) => {
                    warn!(entry = %entry, ?e, "bootstrap multiaddr inválido — saltado");
                }
            }
        }
        if dialed > 0 {
            info!(count = dialed, "bootstrap peers dial-eados");
        }
    }

    Some(net)
}

/// Carga la política de peers libp2p desde los archivos apuntados por
/// `BRAHMAN_PEER_ALLOWLIST` y/o `BRAHMAN_PEER_DENYLIST`, y arranca un
/// watcher para hot reload sobre cualquier cambio.
///
/// - Sin ninguna env: `(None, None)` → modo totalmente abierto.
/// - Con cualquiera (o ambas) set: política activa + watcher vivo.
/// - Si los archivos fallan al cargar: degrada a `(None, None)`,
///   loggea, NO rompe el bucle primordial (doctrina PID 1).
///
/// Devuelve la política y el `JoinHandle` del watcher (que el caller
/// debe mantener para que el thread no se aborte). Si no hay paths,
/// el watcher es un no-op que termina inmediato.
fn setup_brahman_policy() -> (
    Option<card_handshake::peer_policy::PeerPolicy>,
    Option<std::thread::JoinHandle<()>>,
) {
    let allow_path = std::env::var("BRAHMAN_PEER_ALLOWLIST")
        .ok()
        .filter(|s| !s.is_empty());
    let deny_path = std::env::var("BRAHMAN_PEER_DENYLIST")
        .ok()
        .filter(|s| !s.is_empty());

    if allow_path.is_none() && deny_path.is_none() {
        tracing::debug!(
            "BRAHMAN_PEER_ALLOWLIST y BRAHMAN_PEER_DENYLIST no set — modo abierto (todo peer pasa)"
        );
        return (None, None);
    }

    let allow_pb = allow_path.as_deref().map(std::path::Path::new);
    let deny_pb = deny_path.as_deref().map(std::path::Path::new);

    let policy = match card_handshake::peer_policy::PeerPolicy::from_files(allow_pb, deny_pb) {
        Ok(p) => p,
        Err(e) => {
            warn!(
                ?e,
                allow = ?allow_path,
                deny = ?deny_path,
                "policy de peers inválida — degradando a modo abierto (sin restricción)"
            );
            return (None, None);
        }
    };

    let (allow_count, deny_count) = policy.sizes();
    info!(
        allow = ?allow_count,
        deny = deny_count,
        allow_path = ?allow_path,
        deny_path = ?deny_path,
        "policy de peers libp2p cargada"
    );

    // Spawn watcher para hot reload. Errores aquí no son fatales —
    // tendrías política sin reload, que es razonable.
    let watcher = match policy.spawn_watcher() {
        Ok(h) => Some(h),
        Err(e) => {
            warn!(?e, "policy watcher no se pudo crear — hot reload deshabilitado");
            None
        }
    };

    (Some(policy), watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;

    fn peer() -> PeerCreds {
        PeerCreds { pid: 1234, uid: 0, gid: 0 }
    }

    #[test]
    fn audit_killente_solo_con_caller_autenticado() {
        let target = Ulid::new();
        let caller = Ulid::new();
        let req = BusRequest::KillEnte { target, signal: 15 };
        // Sin caller — KillEnte requiere auth en bus_mediator, no se audita
        // intención si no hay identidad que registrar.
        assert!(bus_request_to_audit(&peer(), &None, &req).is_none());
        // Con caller — emite KillEnte capturando los tres campos.
        let entry = bus_request_to_audit(&peer(), &Some(caller), &req).expect("audita");
        match entry {
            AuditAction::KillEnte { caller: c, target: t, signal } => {
                assert_eq!(c, caller);
                assert_eq!(t, target);
                assert_eq!(signal, 15);
            }
            other => panic!("esperaba KillEnte, fue {other:?}"),
        }
    }

    #[test]
    fn audit_spawncardfromdisk_solo_con_caller() {
        let caller = Ulid::new();
        let req = BusRequest::SpawnCardFromDisk { name: "foo".into() };
        assert!(bus_request_to_audit(&peer(), &None, &req).is_none());
        let entry = bus_request_to_audit(&peer(), &Some(caller), &req).expect("audita");
        match entry {
            AuditAction::SpawnCardFromDisk { caller: c, name } => {
                assert_eq!(c, caller);
                assert_eq!(name, "foo");
            }
            other => panic!("esperaba SpawnCardFromDisk, fue {other:?}"),
        }
    }

    #[test]
    fn audit_runcard_solo_con_caller_lleva_label() {
        let caller = Ulid::new();
        let card = arje_card::WireCard::from(arje_card::EntityCard::new("mi-app"));
        let req = BusRequest::RunCard { card };
        // Anónimo no audita (RunCard requiere identidad autenticada).
        assert!(bus_request_to_audit(&peer(), &None, &req).is_none());
        let entry = bus_request_to_audit(&peer(), &Some(caller), &req).expect("audita");
        match entry {
            AuditAction::RunCard { caller: c, label } => {
                assert_eq!(c, caller);
                assert_eq!(label, "mi-app");
            }
            other => panic!("esperaba RunCard, fue {other:?}"),
        }
    }

    #[test]
    fn audit_powermgmt_acepta_anonimo() {
        // Power-mgmt no requiere auth en el grafo, por lo que el anonimato
        // también es información — debe auditarse igual.
        let req = BusRequest::PowerOff { interactive: true };
        let entry = bus_request_to_audit(&peer(), &None, &req).expect("audita");
        match entry {
            AuditAction::PowerMgmt { caller, peer_pid, kind, interactive } => {
                assert_eq!(caller, None);
                assert_eq!(peer_pid, 1234);
                assert_eq!(kind, "PowerOff");
                assert!(interactive);
            }
            other => panic!("esperaba PowerMgmt, fue {other:?}"),
        }
    }

    #[test]
    fn audit_power_mgmt_distingue_los_cuatro_kinds() {
        for (req, expected) in [
            (BusRequest::PowerOff { interactive: false }, "PowerOff"),
            (BusRequest::Reboot { interactive: false }, "Reboot"),
            (BusRequest::Suspend { interactive: false }, "Suspend"),
            (BusRequest::Hibernate { interactive: false }, "Hibernate"),
        ] {
            let entry = bus_request_to_audit(&peer(), &None, &req).expect("audita");
            match entry {
                AuditAction::PowerMgmt { kind, .. } => assert_eq!(kind, expected),
                other => panic!("esperaba PowerMgmt({expected}), fue {other:?}"),
            }
        }
    }

    #[test]
    fn audit_acciones_routine_no_se_auditan() {
        // Announce, ListEntes, Invoke, UpdateCapabilities son routine —
        // emitirlas como audit metería ruido sin valor histórico.
        for req in [
            BusRequest::Announce { capabilities: vec![] },
            BusRequest::ListEntes,
            BusRequest::Invoke {
                cap: arje_card::Capability::Journal,
                blob: vec![],
            },
            BusRequest::UpdateCapabilities { adds: vec![], removes: vec![] },
        ] {
            assert!(
                bus_request_to_audit(&peer(), &Some(Ulid::new()), &req).is_none(),
                "routine no debería auditarse: {req:?}"
            );
        }
    }
}
