//! `arje-packager` — CLI. Toma una Tarjeta Semilla canónica y un mapa de
//! binarios del host (label → path), produce un `initramfs.cpio.gz` listo
//! para pasar a `qemu -initrd` o para `mkinitcpio --image` equivalente.
//!
//! ## Uso
//!
//! ```text
//! arje-packager \
//!   --seed   03_ukupacha/arje/seeds/arje-qemu.card.json \
//!   --bin    arje-zero=/path/to/target/release/arje-zero \
//!   --bin    agetty-ttyS0=/sbin/agetty \
//!   --out    /tmp/arje-qemu.cpio.gz
//! ```
//!
//! El packager:
//!
//! 1. Carga + valida la seed con [`EntityCard::from_path`].
//! 2. Recorre `genesis` recursivamente; para cada Ente con payload `Native`,
//!    requiere que su `label` esté en el mapa `--bin` y copia el binario
//!    del host al árbol del archive con la ruta declarada en `exec`.
//! 3. Crea `/init` como symlink a `/sbin/arje-zero` (convención Linux —
//!    el kernel ejecuta `/init` y nada más).
//! 4. Crea los directorios mínimos (`/dev`, `/proc`, `/sys`, `/ente`,
//!    `/run`) y `/dev/console` como device node (necesario para la shell
//!    de rescate de arje-zero).
//! 5. Embebe la seed serializada en `/ente/seed.card.json`.
//! 6. Emite el cpio en memoria, lo gzipea y lo escribe a `--out`.
//!
//! ## Lo que NO hace
//!
//! - No resuelve dependencias dinámicas (`ldd`). Si los binarios del host
//!   son dinámicos, el initramfs necesita además `/lib*/ld-linux-*.so` y
//!   las `.so` correspondientes. Para arje-zero compilamos con
//!   `RUSTFLAGS=-C target-feature=+crt-static` y para los Entes nativos
//!   declaramos `musl-static` en sus Cargo.toml — así el packager queda
//!   pequeño y soberano.
//! - No genera el bzImage del kernel. Eso es responsabilidad del builder
//!   externo (e.g. `arje-host` apunta a un kernel ya compilado).
//! - No firma el archive. La integridad cruzada (seed ↔ binarios) sale
//!   del Capability fractal en tiempo de boot, no del packager.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context};
use arje_card::{EntityCard, Payload};
use arje_packager::{gzip, CpioWriter, EntryKind};

struct Args {
    seed: PathBuf,
    /// Initramfs cpio.gz de salida. Opcional si se pide sólo `--seed-out`.
    out: Option<PathBuf>,
    /// Emite el SEED firmado standalone (JSON) a este path, sin (o además del)
    /// cpio. Lo usa hammer: necesita el `Card::attest` firmado para inyectarlo
    /// en su product-rootfs (que arma como disco, no como initramfs).
    seed_out: Option<PathBuf>,
    bins: BTreeMap<String, PathBuf>,
    /// Archivos extra a hornear en el initramfs: `ruta-en-imagen` → `ruta-host`.
    /// P. ej. la config del splash y su imagen (`etc/arje/splash.conf`, etc).
    assets: BTreeMap<String, PathBuf>,
    /// Rootkey del seed para firmar el manifiesto de atestación (A1). 32 bytes
    /// raw. Si no se pasa, el seed se empaqueta sin `attest` (boot sin gate).
    rootkey: Option<PathBuf>,
    /// Si la rootkey no existe, generarla (32 bytes de `/dev/urandom`, 0600).
    gen_rootkey: bool,
    /// Cosechar los binarios empaquetados al CAS local (BLAKE3), para que
    /// `arje-cas-aoe` pueda distribuirlos por la red.
    harvest_cas: bool,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut seed: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut seed_out: Option<PathBuf> = None;
    let mut bins: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut assets: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut rootkey: Option<PathBuf> = None;
    let mut gen_rootkey = false;
    let mut harvest_cas = false;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seed" => {
                seed = Some(it.next().context("--seed requiere path")?.into());
            }
            "--out" => {
                out = Some(it.next().context("--out requiere path")?.into());
            }
            "--seed-out" => {
                seed_out = Some(it.next().context("--seed-out requiere path")?.into());
            }
            "--rootkey" => {
                rootkey = Some(it.next().context("--rootkey requiere path")?.into());
            }
            "--gen-rootkey" => {
                gen_rootkey = true;
            }
            "--harvest-cas" => {
                harvest_cas = true;
            }
            "--bin" => {
                let kv = it.next().context("--bin requiere label=path")?;
                let (label, path) = kv
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--bin esperaba label=path, vino {kv:?}"))?;
                bins.insert(label.to_string(), PathBuf::from(path));
            }
            "--asset" => {
                let kv = it.next().context("--asset requiere dest=src")?;
                let (dest, src) = kv
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--asset esperaba dest=src, vino {kv:?}"))?;
                let dest = dest.trim_start_matches('/').to_string();
                assets.insert(dest, PathBuf::from(src));
            }
            "-h" | "--help" => {
                eprintln!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("argumento desconocido: {other}"),
        }
    }

    if out.is_none() && seed_out.is_none() {
        bail!("falta --out (initramfs) y/o --seed-out (seed firmado); pasá al menos uno");
    }
    Ok(Args {
        seed: seed.ok_or_else(|| anyhow!("falta --seed"))?,
        out,
        seed_out,
        bins,
        assets,
        rootkey,
        gen_rootkey,
        harvest_cas,
    })
}

const HELP: &str = "\
arje-packager — initramfs cpio.gz desde una Tarjeta Semilla

USO:
    arje-packager --seed <CARD.json> --out <INITRAMFS.cpio.gz> [--bin LABEL=PATH]...

OPCIONES:
    --seed   Ruta a la seed canónica (.card.json) que describe el target.
    --out    Ruta del initramfs resultante (cpio newc + gzip).
    --bin    Mapea un label del genesis a un binario del host. Repetible.
             El packager exige una entrada por cada Payload::Native del fractal.
             Para el Ente raíz se asume label=\"arje-zero\".
    --asset  Hornea un archivo extra: dest-en-imagen=ruta-host. Repetible.
             P. ej. --asset etc/arje/splash.conf=/tmp/splash.conf
                    --asset etc/arje/splash.png=/ruta/logo.png
    --rootkey <FILE>  Rootkey (32 bytes raw) para FIRMAR el manifiesto de
             atestación al arranque (A1): una ConcesionCapacidad por binario
             crítico sobre su BLAKE3. Sin esta opción el seed va sin attest.
    --gen-rootkey     Si --rootkey no existe, generarla (/dev/urandom, 0600).
    --harvest-cas     Cosechar los binarios empaquetados al CAS local (BLAKE3),
             para que `arje-cas-aoe` los distribuya por Akasha Over Ether.
    -h, --help   Esta ayuda.
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("arje-packager :: ERROR {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let args = parse_args()?;
    let mut card = EntityCard::from_path(&args.seed)
        .with_context(|| format!("cargando seed {}", args.seed.display()))?;

    // Recolectamos todos los exec paths declarados por payloads Native del
    // fractal — el mapa --bin debe cubrirlos. Usamos un BTreeSet para
    // diagnósticos deterministas si falta alguno.
    let mut required: BTreeMap<String, String> = BTreeMap::new();
    required.insert("arje-zero".to_string(), "sbin/arje-zero".to_string());
    collect_native(&card, &mut required);

    let mut tree: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (label, dest_rel) in &required {
        let src = args
            .bins
            .get(label)
            .ok_or_else(|| anyhow!("falta --bin {label}=... (lo exige el fractal)"))?;
        let data = std::fs::read(src)
            .with_context(|| format!("leyendo binario {label} desde {}", src.display()))?;
        tree.insert(dest_rel.clone(), data);
    }

    // Atestación al arranque (A1): si hay rootkey, firmamos una
    // ConcesionCapacidad por cada binario crítico sobre su BLAKE3 y la
    // anclamos en la seed (`attest` + `attest_rootkey`). `arje-zero` las
    // verifica al boot (A2). Iteramos `tree` (ya ordenado por BTreeMap) para
    // que el manifiesto sea reproducible. `permisos = 0`: esto es atestación
    // de integridad, no concesión de capacidades (mapear card.permissions →
    // format::Permisos queda como follow-up).
    if let Some(rootkey_path) = &args.rootkey {
        let nueva = !rootkey_path.exists();
        let seed = arje_packager::load_or_gen_rootkey(rootkey_path, args.gen_rootkey)?;
        if nueva {
            eprintln!("arje-packager :: rootkey nueva generada en {}", rootkey_path.display());
        }
        let pubkey = arje_packager::sign_seed_attest(&mut card, &tree, seed);
        let pubhex = arje_packager::hex32(&pubkey);
        eprintln!(
            "arje-packager :: atestación: {n} binarios firmados bajo rootkey {pubhex}",
            n = card.attest.len(),
        );
        eprintln!("arje-packager :: {}", arje_packager::guia_anclado_soberano(&pubhex));
    }

    // Cosecha al CAS: los binarios empaquetados quedan direccionados por su
    // BLAKE3 (el mismo hash que firma la atestación), así `arje-cas-aoe` los
    // sirve por Akasha y un peer reproduce la imagen bajándolos por hash.
    if args.harvest_cas {
        let hashes = arje_cas::cosechar(tree.values().map(|b| b.as_slice()))?;
        eprintln!(
            "arje-packager :: cosechados {} binario(s) al CAS en {}",
            hashes.len(),
            arje_cas::cas_root().display(),
        );
    }

    // Seed firmado standalone (--seed-out): el JSON canónico de la Card ya con
    // `attest`/`attest_rootkey`. Es lo que hammer inyecta en su product-rootfs.
    if let Some(seed_out) = &args.seed_out {
        let seed_bytes = serde_json::to_vec_pretty(&card)?;
        if let Some(parent) = seed_out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(seed_out, &seed_bytes)
            .with_context(|| format!("escribiendo seed firmado {}", seed_out.display()))?;
        eprintln!("arje-packager :: seed firmado -> {}", seed_out.display());
    }

    // Sin --out: sólo se pidió el seed firmado, no el initramfs.
    let Some(out_path) = args.out.clone() else {
        return Ok(());
    };

    // Emitimos el archive.
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
    let mut w = CpioWriter::new(&mut buf);

    // Directorios mínimos. El kernel los necesita poblados ANTES de que
    // arje-zero los monte (proc, sys) o los use como base de overlay (run,
    // ente). dev se popula con console abajo.
    for dir in ["dev", "ente", "proc", "run", "sys", "sbin", "usr", "usr/lib", "usr/lib/arje"] {
        w.append(dir, EntryKind::Directory)?;
    }
    // /dev/console (char 5,1) — fija para que arje-zero pueda abrir la
    // shell de rescate aunque devtmpfs no se monte a tiempo.
    w.append(
        "dev/console",
        EntryKind::CharDev { major: 5, minor: 1, perm: 0o600 },
    )?;

    // /init → /sbin/arje-zero. El kernel ejecuta /init; encadenamos vía
    // symlink en lugar de duplicar el binario para mantener el archive
    // chico.
    w.append("init", EntryKind::Symlink { target: "sbin/arje-zero" })?;

    // Seed serializada (re-emitida en JSON canónico — no copiamos bytes
    // del archivo original porque la seed puede tener whitespace variable
    // que `EntityCard::from_path` ya normalizó).
    let seed_bytes = serde_json::to_vec_pretty(&card)?;
    w.append(
        "ente/seed.card.json",
        EntryKind::Regular { data: &seed_bytes, perm: 0o644 },
    )?;

    // Binarios — los emitimos en orden alfabético para que el archive sea
    // reproducible byte a byte ante mismas entradas.
    for (rel, data) in &tree {
        w.append(rel, EntryKind::Regular { data, perm: 0o755 })?;
    }

    // Assets extra (config del splash, imagen, frames…). Creamos los
    // directorios padre que falten, en orden, y escribimos los archivos 0644.
    let mut dirs_hechos: std::collections::BTreeSet<String> = [
        "dev", "ente", "proc", "run", "sys", "sbin", "usr", "usr/lib", "usr/lib/arje",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for (dest, src) in &args.assets {
        let mut acc = String::new();
        let comps: Vec<&str> = dest.split('/').collect();
        for comp in &comps[..comps.len().saturating_sub(1)] {
            if comp.is_empty() {
                continue;
            }
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(comp);
            if dirs_hechos.insert(acc.clone()) {
                w.append(&acc, EntryKind::Directory)?;
            }
        }
        let data = std::fs::read(src)
            .with_context(|| format!("leyendo asset {} desde {}", dest, src.display()))?;
        w.append(dest, EntryKind::Regular { data: &data, perm: 0o644 })?;
    }

    let _: &mut Vec<u8> = w.finish()?;

    let gz = gzip(&buf).context("comprimiendo cpio")?;
    let out_size = gz.len();
    if let Some(parent) = out_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&out_path, &gz)
        .with_context(|| format!("escribiendo {}", out_path.display()))?;

    eprintln!(
        "arje-packager :: {} -> {} ({} entradas, {} bytes gzipeados)",
        args.seed.display(),
        out_path.display(),
        required.len() + args.assets.len() + 12, // dirs + dev + init + seed + binarios + assets
        out_size,
    );
    Ok(())
}

/// Recorre `genesis` en DFS y registra cada exec declarado por un
/// `Payload::Native` o `Payload::Legacy` bajo su `label`. El valor en el
/// mapa es la ruta dentro del archive (sin `/` inicial).
fn collect_native(card: &EntityCard, out: &mut BTreeMap<String, String>) {
    for child in &card.genesis {
        match &child.payload {
            Payload::Native { exec, .. } | Payload::Legacy { exec, .. } => {
                let rel = exec
                    .strip_prefix('/')
                    .unwrap_or(exec)
                    .to_string();
                out.insert(child.label.clone(), rel);
            }
            Payload::Wasm { .. } => {
                // WASM no necesita binario en /usr/bin — el módulo va al CAS
                // por sha256. Cuando se implemente, este branch resolverá la
                // blob por hash, no por label.
            }
            Payload::Virtual => {}
        }
        collect_native(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_native_extrae_execs_recursivos() {
        let json = serde_json::json!({
            "schema_version": 1,
            "id": "01HQAR53D4M2NBV8KZTYXFHS01",
            "lineage": null,
            "label": "raiz",
            "provides": [], "requires": [],
            "permissions": {
                "networking": "none", "filesystem": "read-only",
                "ipc": { "allow": [] }, "processes": true
            },
            "soma": {
                "namespaces": {
                    "mount": false, "pid": false, "net": false,
                    "uts": false, "ipc": false, "user": false, "cgroup": false
                },
                "rlimits": { "mem_bytes": null, "nproc": null, "nofile": null },
                "cgroup": { "path": "x", "cpu_weight": null, "io_weight": null },
                "cpu_affinity": null
            },
            "payload": "Virtual",
            "supervision": "OneShot",
            "lifecycle": "daemon",
            "priority": "normal",
            "flow": { "input": [], "output": [] },
            "genesis": [
                {
                    "schema_version": 1,
                    "id": "01HQAR53D4M2NBV8KZTYXFHS02",
                    "lineage": null,
                    "label": "agetty",
                    "provides": [], "requires": [],
                    "permissions": {
                        "networking": "none", "filesystem": "read-only",
                        "ipc": { "allow": [] }, "processes": true
                    },
                    "soma": {
                        "namespaces": {
                            "mount": false, "pid": false, "net": false,
                            "uts": false, "ipc": false, "user": false, "cgroup": false
                        },
                        "rlimits": { "mem_bytes": null, "nproc": null, "nofile": null },
                        "cgroup": { "path": "x", "cpu_weight": null, "io_weight": null },
                        "cpu_affinity": null
                    },
                    "payload": { "Native": {
                        "exec": "/sbin/agetty", "argv": [], "envp": []
                    }},
                    "supervision": "OneShot",
                    "lifecycle": "daemon",
                    "priority": "normal",
                    "flow": { "input": [], "output": [] },
                    "genesis": []
                }
            ]
        });
        let card: EntityCard = serde_json::from_value(json).unwrap();
        let mut out = BTreeMap::new();
        collect_native(&card, &mut out);
        assert_eq!(out.get("agetty").map(String::as_str), Some("sbin/agetty"));
    }
}
