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
    out: PathBuf,
    bins: BTreeMap<String, PathBuf>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut seed: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut bins: BTreeMap<String, PathBuf> = BTreeMap::new();

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seed" => {
                seed = Some(it.next().context("--seed requiere path")?.into());
            }
            "--out" => {
                out = Some(it.next().context("--out requiere path")?.into());
            }
            "--bin" => {
                let kv = it.next().context("--bin requiere label=path")?;
                let (label, path) = kv
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--bin esperaba label=path, vino {kv:?}"))?;
                bins.insert(label.to_string(), PathBuf::from(path));
            }
            "-h" | "--help" => {
                eprintln!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("argumento desconocido: {other}"),
        }
    }

    Ok(Args {
        seed: seed.ok_or_else(|| anyhow!("falta --seed"))?,
        out: out.ok_or_else(|| anyhow!("falta --out"))?,
        bins,
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
    let card = EntityCard::from_path(&args.seed)
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

    let _: &mut Vec<u8> = w.finish()?;

    let gz = gzip(&buf).context("comprimiendo cpio")?;
    let out_size = gz.len();
    if let Some(parent) = args.out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&args.out, &gz)
        .with_context(|| format!("escribiendo {}", args.out.display()))?;

    eprintln!(
        "arje-packager :: {} -> {} ({} entradas, {} bytes gzipeados)",
        args.seed.display(),
        args.out.display(),
        required.len() + 12, // dirs + dev + init + seed + binarios
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
