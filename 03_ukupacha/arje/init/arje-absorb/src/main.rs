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
    --rootkey <f>    firma el manifiesto de atestación (A1): una concesión por
                     binario de servicio (leído de <root>) bajo su BLAKE3,
                     anclada en el seed. Sin esto el seed va sin attest.
    --gen-rootkey    si --rootkey no existe, generarla (/dev/urandom, 0600)
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
    let mut rootkey: Option<PathBuf> = None;
    let mut gen_rootkey = false;

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
            "--rootkey" => {
                rootkey = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow::anyhow!("--rootkey necesita un valor"))?,
                ))
            }
            "--gen-rootkey" => gen_rootkey = true,
            other => anyhow::bail!("opción desconocida «{other}» (usá --help)"),
        }
    }
    if gen_rootkey && rootkey.is_none() {
        anyhow::bail!("--gen-rootkey requiere --rootkey <path> (dónde crear/leer la rootkey)");
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

    // Atestación al arranque (A1): si hay --rootkey, firmamos el manifiesto
    // sobre los binarios de cada servicio LEÍDOS del sistema fuente (`root`).
    // Captura el estado confiable de los binarios al absorber; el operador
    // ancla la pubkey fuera del seed para endurecer a Halt. Reusa el mismo
    // firmador (`arje_attest::firmar_arbol`) que packager/installer → manifiesto
    // idéntico por cualquier ruta. arje-zero NO se firma acá (no es un servicio
    // del init absorbido; lo agrega el packager/installer al armar la imagen).
    if let Some(rootkey_path) = &rootkey {
        let nueva = !rootkey_path.exists();
        let seed_key = arje_attest::load_or_gen_rootkey(rootkey_path, gen_rootkey)?;
        if nueva {
            eprintln!("arje-absorb: rootkey nueva generada en {}", rootkey_path.display());
        }
        let (bins, skipped) = collect_exec_bins(&seed, &root);
        for s in &skipped {
            eprintln!(
                "arje-absorb: aviso: no pude leer «{s}» bajo {} — queda SIN atestar",
                root.display()
            );
        }
        if bins.is_empty() {
            eprintln!("arje-absorb: aviso: 0 binarios legibles — no se firma manifiesto.");
        } else {
            let (pubkey, concesiones) = arje_attest::firmar_arbol(seed_key, &bins);
            let n = concesiones.len();
            seed.attest = concesiones;
            seed.attest_rootkey = Some(pubkey);
            let pubhex = arje_attest::rootkey_a_hex(&pubkey);
            eprintln!("arje-absorb: atestación: {n} binario(s) firmado(s) bajo rootkey {pubhex}");
            eprintln!("arje-absorb: {}", arje_attest::guia_anclado_soberano(&pubhex));
        }
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

/// Recolecta los binarios de cada servicio (genesis Native/Legacy, recursivo)
/// leyéndolos del sistema fuente bajo `root`: `exec` `/usr/bin/foo` →
/// `root/usr/bin/foo`. Deduplica por `exec` (un binario se firma una vez).
/// Devuelve `(bins legibles, execs que no se pudieron leer)` — absorb es un
/// survey read-only, así que un binario ausente se reporta y se saltea, no
/// aborta. Las claves del mapa son los `exec` (sólo ordenan; la verificación
/// es por hash).
fn collect_exec_bins(
    seed: &card_core::Card,
    root: &Path,
) -> (std::collections::BTreeMap<String, Vec<u8>>, Vec<String>) {
    use card_core::Payload;
    fn walk(
        card: &card_core::Card,
        root: &Path,
        bins: &mut std::collections::BTreeMap<String, Vec<u8>>,
        skipped: &mut Vec<String>,
    ) {
        if let Payload::Native { exec, .. } | Payload::Legacy { exec, .. } = &card.payload {
            if !bins.contains_key(exec) {
                let abs = root.join(exec.strip_prefix('/').unwrap_or(exec));
                match std::fs::read(&abs) {
                    Ok(bytes) => {
                        bins.insert(exec.clone(), bytes);
                    }
                    Err(_) => skipped.push(exec.clone()),
                }
            }
        }
        for hija in &card.genesis {
            walk(hija, root, bins, skipped);
        }
    }
    let mut bins = std::collections::BTreeMap::new();
    let mut skipped = Vec::new();
    for hija in &seed.genesis {
        walk(hija, root, &mut bins, &mut skipped);
    }
    skipped.sort();
    skipped.dedup();
    (bins, skipped)
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

    #[test]
    fn collect_exec_bins_lee_presentes_y_saltea_ausentes() {
        use card_core::{Card, Payload};
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::write(root.join("usr/bin/foo"), b"foo binary bytes").unwrap();

        // Seed con dos servicios: uno presente bajo root, otro ausente.
        let mut seed = Card::new("seed");
        let mut a = Card::new("foo");
        a.payload = Payload::Native { exec: "/usr/bin/foo".into(), argv: vec![], envp: vec![] };
        let mut b = Card::new("ghost");
        b.payload = Payload::Native { exec: "/usr/bin/ghost".into(), argv: vec![], envp: vec![] };
        seed.genesis.push(a);
        seed.genesis.push(b);

        let (bins, skipped) = collect_exec_bins(&seed, root);
        assert_eq!(bins.len(), 1, "sólo el binario presente se lee");
        assert_eq!(bins.get("/usr/bin/foo").unwrap().as_slice(), b"foo binary bytes");
        assert_eq!(skipped, vec!["/usr/bin/ghost".to_string()]);

        // El binario leído atesta Ok contra el manifiesto firmado (la misma
        // verificación que hará `arje-zero` al boot).
        let (pubkey, conc) = arje_attest::firmar_arbol([5u8; 32], &bins);
        assert_eq!(conc.len(), 1);
        let v = arje_attest::atestar_bytes(&conc, b"foo binary bytes", Some(pubkey));
        assert!(v.es_ok(), "el binario absorbido debería atestar Ok, fue {}", v.motivo());
        // Un impostor no atesta.
        assert!(!arje_attest::atestar_bytes(&conc, b"otra cosa", Some(pubkey)).es_ok());
    }
}
