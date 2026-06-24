//! `churay-cli` — el frente headless del instalador (servidores, scripts, CI).
//! Misma lógica que la GUI; sin ventana.
//!
//! Uso:
//!   churay-cli [--system|--local] [--prefix DIR] <cmd> [args]
//!
//! Comandos:
//!   list                 lista las unidades (del repo remoto si CHURAY_REPO, si no del catálogo)
//!   check                compara lo instalado contra el manifiesto (remoto si hay)
//!   install <id…>        instala las unidades dadas
//!   update [<id…>]       reinstala las que tienen actualización (todas si no se dan ids)
//!   uninstall <id…>      desinstala
//!
//! Env: CHURAY_REPO (repo remoto firmado), CHURAY_BUNDLE, CHURAY_WORKSPACE,
//!      CHURAY_MODE=system|local.

use churay_core::install::Step;
use churay_core::{
    install_unit, pending_updates, suite_catalog, uninstall_unit, CurlFetcher, InstallConfig,
    InstallMode, InstalledState, Manifest, Unit, UpdateKind,
};

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut mode: Option<InstallMode> = None;
    let mut prefix_override: Option<std::path::PathBuf> = None;
    let mut rest: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--system" => mode = Some(InstallMode::System),
            "--local" => mode = Some(InstallMode::Local),
            "--prefix" => prefix_override = args.next().map(Into::into),
            _ => rest.push(a),
        }
    }
    let mode = mode.unwrap_or_else(|| match std::env::var("CHURAY_MODE").as_deref() {
        Ok("system") => InstallMode::System,
        _ => InstallMode::Local,
    });
    let mut cfg = InstallConfig::detect(mode);
    if let Some(p) = prefix_override {
        cfg.prefix = p;
    }

    let mut it = rest.into_iter();
    let cmd = it.next().unwrap_or_else(|| "list".into());
    let ids: Vec<String> = it.collect();

    // Catálogo de trabajo: el del repo remoto (con hashes → descargable) si está
    // configurado y responde; si no, el catálogo local.
    let (units, remote_manifest) = catalogo(&cfg);
    let code = match cmd.as_str() {
        "list" => cmd_list(&cfg, &units),
        "check" => cmd_check(&cfg, &units, remote_manifest.as_ref()),
        "install" => cmd_install(&cfg, &units, &ids),
        "update" => cmd_update(&cfg, &units, remote_manifest.as_ref(), &ids),
        "uninstall" => cmd_uninstall(&cfg, &units, &ids),
        other => {
            eprintln!("comando desconocido: {other}");
            2
        }
    };
    std::process::exit(code);
}

fn catalogo(cfg: &InstallConfig) -> (Vec<Unit>, Option<Manifest>) {
    if let Some(url) = &cfg.remote_base_url {
        match churay_core::fetch_signed_manifest(url, &CurlFetcher, None) {
            Ok(m) => return (m.units.clone(), Some(m)),
            Err(e) => eprintln!("aviso: no se pudo leer el repo remoto ({e}); uso catálogo local"),
        }
    }
    (suite_catalog(), None)
}

fn cmd_list(cfg: &InstallConfig, units: &[Unit]) -> i32 {
    let state = InstalledState::load(&cfg.prefix);
    println!("{:<22} {:<10} {:<8} {}", "ID", "VERSIÓN", "ALCANCE", "ESTADO");
    for u in units {
        let estado = if state.is_installed(&u.id) { "instalada" } else { "-" };
        let scope = if u.requires_root() { "sistema" } else { "app" };
        println!("{:<22} {:<10} {:<8} {}", u.id, u.version, scope, estado);
    }
    0
}

fn cmd_check(cfg: &InstallConfig, units: &[Unit], remote: Option<&Manifest>) -> i32 {
    let state = InstalledState::load(&cfg.prefix);
    let owned;
    let manifest = match remote {
        Some(m) => m,
        None => {
            owned = Manifest::new(churay_core::SUITE_VERSION, units.to_vec());
            &owned
        }
    };
    let pend: Vec<_> = pending_updates(&state, manifest)
        .into_iter()
        .filter(|u| u.kind == UpdateKind::Disponible)
        .collect();
    if pend.is_empty() {
        println!("todo al día ({} unidades instaladas)", state.units.len());
    } else {
        println!("{} actualización(es):", pend.len());
        for u in pend {
            println!(
                "  {} {} → {}",
                u.id,
                u.installed_version.unwrap_or_default(),
                u.available_version
            );
        }
    }
    0
}

fn cmd_install(cfg: &InstallConfig, units: &[Unit], ids: &[String]) -> i32 {
    if ids.is_empty() {
        eprintln!("uso: churay-cli install <id…>");
        return 2;
    }
    let mut state = InstalledState::load(&cfg.prefix);
    let mut fallos = 0;
    for id in ids {
        match units.iter().find(|u| &u.id == id) {
            Some(u) => fallos += instalar_una(cfg, u, &mut state),
            None => {
                eprintln!("× {id}: no está en el catálogo");
                fallos += 1;
            }
        }
    }
    if fallos == 0 {
        0
    } else {
        1
    }
}

fn cmd_update(
    cfg: &InstallConfig,
    units: &[Unit],
    remote: Option<&Manifest>,
    ids: &[String],
) -> i32 {
    let mut state = InstalledState::load(&cfg.prefix);
    let owned;
    let manifest = match remote {
        Some(m) => m,
        None => {
            owned = Manifest::new(churay_core::SUITE_VERSION, units.to_vec());
            &owned
        }
    };
    let pend: Vec<_> = pending_updates(&state, manifest)
        .into_iter()
        .filter(|u| u.kind == UpdateKind::Disponible)
        .filter(|u| ids.is_empty() || ids.contains(&u.id))
        .collect();
    if pend.is_empty() {
        println!("nada para actualizar");
        return 0;
    }
    let mut fallos = 0;
    for info in pend {
        if let Some(u) = units.iter().find(|u| u.id == info.id) {
            fallos += instalar_una(cfg, u, &mut state);
        }
    }
    if fallos == 0 {
        0
    } else {
        1
    }
}

fn cmd_uninstall(cfg: &InstallConfig, units: &[Unit], ids: &[String]) -> i32 {
    if ids.is_empty() {
        eprintln!("uso: churay-cli uninstall <id…>");
        return 2;
    }
    let mut state = InstalledState::load(&cfg.prefix);
    for id in ids {
        if let Some(u) = units.iter().find(|u| &u.id == id) {
            match uninstall_unit(cfg, u, &mut state) {
                Ok(()) => println!("✓ desinstalada {id}"),
                Err(e) => eprintln!("× {id}: {e}"),
            }
        } else {
            eprintln!("× {id}: no está en el catálogo");
        }
    }
    0
}

fn instalar_una(cfg: &InstallConfig, u: &Unit, state: &mut InstalledState) -> i32 {
    print!("· {} … ", u.id);
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let res = install_unit(cfg, u, state, &mut |step, _| {
        if matches!(step, Step::Descargando | Step::Compilando) {
            print!("{} ", paso(step));
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    });
    match res {
        Ok(()) => {
            println!("✓");
            0
        }
        Err(e) => {
            println!("✗ {e}");
            1
        }
    }
}

fn paso(s: Step) -> &'static str {
    match s {
        Step::Descargando => "bajando",
        Step::Compilando => "compilando",
        Step::Copiando => "copiando",
        _ => "",
    }
}
