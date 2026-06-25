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
    --harvest-cas    cosechar los binarios de los servicios (leídos de <root>) al
                     CAS local (BLAKE3), para que `arje-cas-aoe` los distribuya
                     por Akasha Over Ether
    --attest-from <f>  integra concesiones YA firmadas (JSON: objeto o array),
                     p. ej. emitidas por un `hammer commit`. Verifica la firma
                     de cada una y descarta inválidas; deduplica por bytecode.
                     Combinable con --rootkey. Si todas comparten autor, lo
                     ancla. (B.3: el binario que la IA mutó queda atestado.)
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
    let mut attest_from: Option<PathBuf> = None;
    let mut harvest_cas = false;

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
            "--attest-from" => {
                attest_from = Some(PathBuf::from(
                    args.next().ok_or_else(|| anyhow::anyhow!("--attest-from necesita un valor"))?,
                ))
            }
            "--harvest-cas" => harvest_cas = true,
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

    // Binarios de cada servicio leídos del sistema fuente (`root/<exec>`). Se
    // necesitan para firmar (--rootkey), cross-checkear concesiones
    // (--attest-from) y/o cosechar al CAS (--harvest-cas). Se leen UNA vez.
    if rootkey.is_some() || attest_from.is_some() || harvest_cas {
        let (bins, skipped) = collect_exec_bins(&seed, &root);
        for s in &skipped {
            eprintln!(
                "arje-absorb: aviso: no pude leer «{s}» bajo {} — queda SIN atestar/cosechar",
                root.display()
            );
        }

        // Atestación al arranque (A1): firma propia (--rootkey) y/o integración
        // de concesiones pre-firmadas (--attest-from, p. ej. de hammer).
        aplicar_atestacion(&mut seed, &bins, &rootkey, gen_rootkey, &attest_from)?;

        // Cosecha al CAS: los binarios de los servicios quedan direccionados por
        // su BLAKE3, así `arje-cas-aoe` los distribuye por Akasha.
        if harvest_cas {
            if bins.is_empty() {
                eprintln!("arje-absorb: aviso: 0 binarios legibles — nada que cosechar al CAS.");
            } else {
                let hashes = arje_cas::cosechar(bins.values().map(|b| b.as_slice()))?;
                eprintln!(
                    "arje-absorb: cosechados {} binario(s) al CAS en {}",
                    hashes.len(),
                    arje_cas::cas_root().display(),
                );
            }
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

/// Aplica atestación al seed sobre los `bins` ya leídos: firma propia con
/// `--rootkey` y/o integra concesiones pre-firmadas con `--attest-from`, y
/// reconcilia el ancla soberana del manifiesto resultante. No-op si no se pidió
/// ninguna.
fn aplicar_atestacion(
    seed: &mut card_core::Card,
    bins: &std::collections::BTreeMap<String, Vec<u8>>,
    rootkey: &Option<PathBuf>,
    gen_rootkey: bool,
    attest_from: &Option<PathBuf>,
) -> anyhow::Result<()> {
    // (1) Firma propia con la rootkey del operador (mismo firmador que
    //     packager/installer → manifiesto idéntico por cualquier ruta).
    if let Some(rootkey_path) = rootkey {
        let nueva = !rootkey_path.exists();
        let seed_key = arje_attest::load_or_gen_rootkey(rootkey_path, gen_rootkey)?;
        if nueva {
            eprintln!("arje-absorb: rootkey nueva generada en {}", rootkey_path.display());
        }
        if bins.is_empty() {
            eprintln!("arje-absorb: aviso: 0 binarios legibles — no se firma manifiesto propio.");
        } else {
            let (pubkey, concesiones) = arje_attest::firmar_arbol(seed_key, bins);
            seed.attest = concesiones;
            seed.attest_rootkey = Some(pubkey);
            eprintln!(
                "arje-absorb: atestación: {} binario(s) firmado(s) por rootkey propia.",
                seed.attest.len()
            );
        }
    }

    // (2) Integración de concesiones pre-firmadas (p. ej. de un hammer commit).
    if let Some(af) = attest_from {
        integrar_concesiones_hammer(seed, bins, af)?;
    }

    // (3) Reconciliar el ancla soberana del manifiesto resultante.
    reconciliar_anclaje(seed);
    Ok(())
}

/// Integra concesiones **pre-firmadas** (un objeto JSON o un array) en
/// `seed.attest`. Verifica la firma de cada una y descarta las inválidas; no
/// confía a ciegas. Deduplica por `bytecode`. Reporta cuántas cubren un binario
/// de servicio de este seed vs. cuántas son «huérfanas» (p. ej. una concesión
/// de `arje-zero`, que no es un servicio absorbido — es legítima: el gate la usa
/// igual por hash al boot).
fn integrar_concesiones_hammer(
    seed: &mut card_core::Card,
    bins: &std::collections::BTreeMap<String, Vec<u8>>,
    path: &Path,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let txt = std::fs::read_to_string(path)
        .with_context(|| format!("leyendo concesiones {}", path.display()))?;
    // Acepta un objeto único o un array de concesiones.
    let entrantes: Vec<arje_attest::ConcesionCapacidad> =
        match serde_json::from_str::<Vec<_>>(&txt) {
            Ok(v) => v,
            Err(_) => vec![serde_json::from_str(&txt)
                .with_context(|| format!("parseando concesiones {}", path.display()))?],
        };

    let hashes_bin: std::collections::BTreeSet<[u8; 32]> =
        bins.values().map(|b| arje_attest::hash_de(b)).collect();
    let mut vistos: std::collections::BTreeSet<[u8; 32]> =
        seed.attest.iter().map(|c| c.bytecode).collect();

    let (mut integradas, mut rechazadas, mut dup, mut huerfanas) = (0u32, 0u32, 0u32, 0u32);
    for c in entrantes {
        if !arje_attest::firma_valida(&c) {
            rechazadas += 1;
            eprintln!("arje-absorb: aviso: concesión con firma inválida — descartada.");
            continue;
        }
        if !vistos.insert(c.bytecode) {
            dup += 1;
            continue;
        }
        if !hashes_bin.contains(&c.bytecode) {
            huerfanas += 1;
        }
        seed.attest.push(c);
        integradas += 1;
    }
    eprintln!(
        "arje-absorb: concesiones pre-firmadas: {integradas} integrada(s), \
         {rechazadas} rechazada(s), {dup} duplicada(s), {huerfanas} sin binario en este seed."
    );
    Ok(())
}

/// Reconcilia `attest_rootkey` con los autores del manifiesto. Si no hay ancla
/// y todas las concesiones comparten un autor, lo ancla (seed auto-consistente);
/// si hay autores mixtos, avisa. Si ya hay ancla (de `--rootkey`), avisa de las
/// concesiones con autor distinto (fallarían el pin de autor bajo `Halt`).
fn reconciliar_anclaje(seed: &mut card_core::Card) {
    if seed.attest.is_empty() {
        return;
    }
    let autores: std::collections::BTreeSet<[u8; 32]> =
        seed.attest.iter().map(|c| c.autor).collect();
    match seed.attest_rootkey {
        None => {
            if autores.len() == 1 {
                let a = *autores.iter().next().unwrap();
                seed.attest_rootkey = Some(a);
                let pubhex = arje_attest::rootkey_a_hex(&a);
                eprintln!("arje-absorb: ancla = único autor del manifiesto ({pubhex}).");
                eprintln!("arje-absorb: {}", arje_attest::guia_anclado_soberano(&pubhex));
            } else {
                eprintln!(
                    "arje-absorb: aviso: manifiesto con {} autores distintos — sin ancla. \
                     Bajo política Halt sólo pasaría el autor anclado; usá Warn o re-firmá \
                     con una raíz única.",
                    autores.len()
                );
            }
        }
        Some(rk) => {
            let ajenos = autores.iter().filter(|a| **a != rk).count();
            if ajenos > 0 {
                eprintln!(
                    "arje-absorb: aviso: {ajenos} concesión(es) con autor ≠ rootkey anclada — \
                     bajo Halt fallarían el pin de autor (usá Warn o anclá la raíz que las firmó)."
                );
            }
            eprintln!("arje-absorb: {}", arje_attest::guia_anclado_soberano(&arje_attest::rootkey_a_hex(&rk)));
        }
    }
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

    #[test]
    fn integra_concesiones_prefirmadas_y_rechaza_invalidas() {
        use card_core::{Card, Payload};
        let tmp = tempfile::tempdir().unwrap();

        // Binario del servicio (lo que el manifiesto de hammer cubre).
        let mut bins = std::collections::BTreeMap::new();
        bins.insert("/usr/bin/svc".to_string(), b"service binary v2".to_vec());

        // "hammer" firma una concesión sobre ese binario y la emite como JSON.
        let (hammer_pub, conc) = arje_attest::firmar_arbol([42u8; 32], &bins);
        let json = tmp.path().join("hammer.attest.json");
        std::fs::write(&json, serde_json::to_vec(&conc).unwrap()).unwrap();

        // Seed con el servicio, sin attest.
        let mut seed = Card::new("seed");
        let mut svc = Card::new("svc");
        svc.payload = Payload::Native { exec: "/usr/bin/svc".into(), argv: vec![], envp: vec![] };
        seed.genesis.push(svc);

        // Integrar: la concesión válida entra y, sin rootkey y autor único, se ancla.
        integrar_concesiones_hammer(&mut seed, &bins, &json).unwrap();
        assert_eq!(seed.attest.len(), 1);
        assert_eq!(seed.attest[0].autor, hammer_pub);
        reconciliar_anclaje(&mut seed);
        assert_eq!(seed.attest_rootkey, Some(hammer_pub), "autor único → se ancla");

        // El binario vivo atesta Ok bajo la rootkey anclada (lo que hará el gate).
        let v = arje_attest::atestar_bytes(&seed.attest, b"service binary v2", Some(hammer_pub));
        assert!(v.es_ok(), "{}", v.motivo());

        // Una concesión (de OTRO binario) con firma corrupta se rechaza.
        let mut otro = std::collections::BTreeMap::new();
        otro.insert("/usr/bin/otro".to_string(), b"otro binary".to_vec());
        let (_p, mut mala) = arje_attest::firmar_arbol([7u8; 32], &otro);
        mala[0].firma[0] ^= 0xFF; // corromper la firma
        let json_mala = tmp.path().join("mala.json");
        std::fs::write(&json_mala, serde_json::to_vec(&mala).unwrap()).unwrap();
        let antes = seed.attest.len();
        integrar_concesiones_hammer(&mut seed, &bins, &json_mala).unwrap();
        assert_eq!(seed.attest.len(), antes, "una firma corrupta no debe integrarse");
    }

    #[test]
    fn harvest_cosecha_los_binarios_de_servicios_al_cas() {
        use card_core::{Card, Payload};
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::write(root.join("usr/bin/svc"), b"service binary harvest").unwrap();

        let mut seed = Card::new("seed");
        let mut svc = Card::new("svc");
        svc.payload = Payload::Native { exec: "/usr/bin/svc".into(), argv: vec![], envp: vec![] };
        seed.genesis.push(svc);

        // CAS aislado (único test de absorb que toca el CAS → sin carrera).
        let cas = tmp.path().join("cas");
        std::env::set_var("ENTE_CAS_ROOT", &cas);

        // La composición del harvest: leer los binarios de los servicios y
        // cosecharlos al CAS, direccionados por su BLAKE3.
        let (bins, _skipped) = collect_exec_bins(&seed, root);
        let hashes = arje_cas::cosechar(bins.values().map(|b| b.as_slice())).unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(
            arje_cas::resolve(&hashes[0]).unwrap().as_slice(),
            b"service binary harvest",
        );

        std::env::remove_var("ENTE_CAS_ROOT");
    }
}
