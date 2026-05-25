//! `arje-absorb` — traduce la configuración de otro init a una Semilla.
//!
//! Lee la configuración de un init clásico (sysvinit, runit, dinit,
//! OpenRC) y emite una Tarjeta Semilla brahman (`Card` JSON) con cada
//! servicio como hija `genesis` de `arje-zero`. Es el paso «absorber»
//! de la migración: `scripts/migrate-to-arje.sh` lo usa para no perder
//! los servicios del sistema al cambiar de init.
//!
//! La Semilla resultante NO endurece el sandbox — conserva el
//! comportamiento del init viejo. Revisala antes de instalarla.

use std::path::{Path, PathBuf};
use std::process::exit;

mod card;
mod dinit;
mod model;
mod openrc;
mod runit;
mod sysvinit;

const HELP: &str = "\
arje-absorb — absorbe la configuración de otro init en una Semilla brahman.

USO:
    arje-absorb [--from <init>] [--root <dir>] [--output <archivo>] [--label <s>]

OPCIONES:
    --from <init>    sysvinit | runit | dinit | openrc | auto   (def: auto)
    --root <dir>     raíz del sistema a leer                    (def: /)
    --output <f>     archivo de salida, o '-' para stdout       (def: -)
    --label <s>      label de la Semilla raíz       (def: arje.seed.absorbed)
    --with-carmen    agrega carmen-dm (gestor de login gráfico) a la Semilla
    -h, --help       esta ayuda

Emite una Tarjeta Semilla con cada servicio del init ajeno como hija
genesis de arje-zero. Revisala antes de instalarla como
/ente/seed.card.json.";

fn main() {
    if let Err(e) = run() {
        eprintln!("arje-absorb: error: {e:#}");
        exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let mut from = "auto".to_string();
    let mut root = PathBuf::from("/");
    let mut output = "-".to_string();
    let mut label = "arje.seed.absorbed".to_string();
    let mut with_carmen = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "--from" => {
                from = args.next().ok_or_else(|| anyhow::anyhow!("--from necesita un valor"))?
            }
            "--root" => {
                root = PathBuf::from(
                    args.next().ok_or_else(|| anyhow::anyhow!("--root necesita un valor"))?,
                )
            }
            "--output" => {
                output =
                    args.next().ok_or_else(|| anyhow::anyhow!("--output necesita un valor"))?
            }
            "--label" => {
                label =
                    args.next().ok_or_else(|| anyhow::anyhow!("--label necesita un valor"))?
            }
            "--with-carmen" => with_carmen = true,
            other => anyhow::bail!("opción desconocida «{other}» (usá --help)"),
        }
    }

    let init = if from == "auto" {
        detect(&root).ok_or_else(|| {
            anyhow::anyhow!(
                "no pude autodetectar el init en {} — pasá --from <init>",
                root.display()
            )
        })?
    } else {
        from.clone()
    };

    let services = match init.as_str() {
        "sysvinit" => sysvinit::absorb(&root)?,
        "runit" => runit::absorb(&root)?,
        "dinit" => dinit::absorb(&root)?,
        "openrc" => openrc::absorb(&root)?,
        other => anyhow::bail!(
            "init «{other}» no soportado (sysvinit | runit | dinit | openrc | auto)"
        ),
    };

    eprintln!(
        "arje-absorb: init «{init}» → {} servicio(s) absorbido(s).",
        services.len()
    );
    if services.is_empty() {
        eprintln!("arje-absorb: aviso: 0 servicios — la Semilla quedará sin hijas.");
    }

    let mut seed = card::build_seed(&label, &services);
    if with_carmen {
        seed.genesis.push(card::carmen_dm_card());
        eprintln!("arje-absorb: agregado carmen-dm (gestor de login gráfico).");
    }
    seed.validate()
        .map_err(|e| anyhow::anyhow!("la Semilla generada no valida: {e}"))?;
    let json = seed.to_json_pretty()?;

    if output == "-" {
        println!("{json}");
    } else {
        std::fs::write(&output, json)
            .map_err(|e| anyhow::anyhow!("escribiendo {output}: {e}"))?;
        eprintln!("arje-absorb: Semilla escrita en {output}");
    }
    Ok(())
}

/// Autodetecta el init presente en `root`. El orden importa: lo más
/// específico primero — un sistema OpenRC suele llevar también un
/// `/etc/inittab` (para las consolas), así que sysvinit va último.
fn detect(root: &Path) -> Option<String> {
    if root.join("etc/dinit.d").is_dir() {
        return Some("dinit".to_string());
    }
    if root.join("etc/runlevels").is_dir() || root.join("sbin/openrc").exists() {
        return Some("openrc".to_string());
    }
    if root.join("etc/runit").is_dir() || root.join("etc/sv").is_dir() {
        return Some("runit".to_string());
    }
    if root.join("etc/inittab").exists() {
        return Some("sysvinit".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_dinit() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("etc/dinit.d")).unwrap();
        std::fs::create_dir_all(tmp.path().join("etc/runlevels")).unwrap();
        assert_eq!(detect(tmp.path()).as_deref(), Some("dinit"));
    }

    #[test]
    fn detect_openrc_over_sysvinit() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("etc/runlevels/default")).unwrap();
        std::fs::write(tmp.path().join("etc/inittab"), "").unwrap();
        assert_eq!(detect(tmp.path()).as_deref(), Some("openrc"));
    }

    #[test]
    fn detect_none_on_empty_root() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(detect(tmp.path()).is_none());
    }
}
