//! `arje-installer` — lib. Lógica pura del installer (sin tocar disco):
//! armar el initramfs vía [`arje_packager`], decidir el layout de la ESP,
//! producir el cmdline canónico y el contenido del `loader.conf` style
//! `arje.conf` que el bootloader (rEFInd, systemd-boot) entiende.
//!
//! Las operaciones I/O destructivas (sfdisk, mkfs.fat, mount, cp, efibootmgr)
//! viven en el binario — la lib se queda testeable sin root y sin `/dev/`.
//!
//! ## Doctrina de instalación UEFI
//!
//! El installer asume **firmware UEFI**. Para sistemas BIOS legacy hay que
//! sumar GRUB/syslinux — fuera del alcance de la primera versión.
//!
//! Dos modos:
//!
//! - **`to-partition`** (no destructivo): el usuario monta su ESP en alguna
//!   ruta y el installer pega kernel/initramfs/seed bajo `<esp>/EFI/arje/`.
//!   Opcionalmente registra una entrada NVRAM via `efibootmgr` apuntando
//!   directo al PE del kernel (EFISTUB) con el cmdline embebido. Sin
//!   bootloader en disco: la firmware ejecuta el kernel.
//!
//! - **`to-usb`** (destructivo): formatea un disco entero como GPT + una
//!   partición ESP FAT32, copia los archivos y, si encuentra un
//!   bootloader EFI disponible en el host (systemd-boot o rEFInd), lo
//!   instala. Si no, deja el kernel como `/EFI/BOOT/BOOTX64.EFI` y avisa
//!   que el USB necesitará efibootmgr en cada máquina destino.

use std::path::{Path, PathBuf};

use arje_card::EntityCard;
pub use arje_packager::{CpioWriter, EntryKind};

/// Layout fijo bajo la ESP. Cualquier cosa que escribimos cae acá.
///
/// Razón del subdir `EFI/arje/` y no `EFI/Linux/` (la convención de
/// distros UKI): mantener namespace propio para coexistir con un GRUB o
/// systemd-boot del host sin chocar. El usuario puede tener su distro
/// principal arriba y arje al lado sin pisarse.
pub struct EspLayout {
    pub root: PathBuf,
}

impl EspLayout {
    pub fn new(esp: impl Into<PathBuf>) -> Self {
        Self { root: esp.into() }
    }

    /// Directorio del namespace arje en la ESP — todo lo nuestro vive
    /// adentro. El padre (`EFI/`) queda compartido con otros loaders.
    pub fn arje_dir(&self) -> PathBuf {
        self.root.join("EFI/arje")
    }
    pub fn kernel(&self) -> PathBuf {
        self.arje_dir().join("vmlinuz")
    }
    pub fn initramfs(&self) -> PathBuf {
        self.arje_dir().join("initramfs.cpio.gz")
    }
    pub fn seed(&self) -> PathBuf {
        self.arje_dir().join("seed.card.json")
    }
    pub fn cmdline_txt(&self) -> PathBuf {
        self.arje_dir().join("cmdline.txt")
    }
    pub fn entries_dir(&self) -> PathBuf {
        self.root.join("loader/entries")
    }
    pub fn arje_entry(&self) -> PathBuf {
        self.entries_dir().join("arje.conf")
    }
    pub fn loader_conf(&self) -> PathBuf {
        self.root.join("loader/loader.conf")
    }
    pub fn bootx64_fallback(&self) -> PathBuf {
        self.root.join("EFI/BOOT/BOOTX64.EFI")
    }
}

/// Cmdline canónico para arje: incluye el `initrd=` con la ruta UEFI
/// (backslashes) hacia el initramfs en la ESP, más lo que el usuario quiera
/// agregar (p. ej. `console=ttyS0` para QEMU serial).
///
/// El kernel EFISTUB lee este cmdline como UEFI LoadOption args cuando lo
/// invoca la firmware directamente, o como `options` del .conf cuando lo
/// invoca un bootloader (systemd-boot/rEFInd). Ambos paths convergen.
pub fn canonical_cmdline(extra: &str) -> String {
    let base = r"initrd=\EFI\arje\initramfs.cpio.gz";
    // Flags de arranque **sin parpadeo** (ver SDD-ARRANQUE-SIN-PARPADEO.md):
    // - `quiet loglevel=3` — el kernel no escribe logs sobre el framebuffer
    //   (evita el flash de texto); deja errores graves.
    // - `vt.global_cursor_default=0` — sin cursor de consola parpadeando.
    // - `i915.fastboot=1` — takeover sin re-modeset en Intel (inocuo en otras
    //   GPUs: el driver que no existe ignora su parámetro). amdgpu ya hace
    //   seamless por defecto.
    // El splash nativo (`arje-splash`) cubre el resto del hueco hasta mirada.
    let flicker_free = "quiet loglevel=3 vt.global_cursor_default=0 i915.fastboot=1";
    let extra = extra.trim();
    if extra.is_empty() {
        format!("{base} {flicker_free}")
    } else {
        format!("{base} {flicker_free} {extra}")
    }
}

/// Genera el contenido del `loader/entries/arje.conf` — formato
/// systemd-boot/rEFInd-bootmgr (líneas `key value`).
///
/// `kernel_rel` e `initrd_rel` son rutas relativas a la ESP **con
/// backslashes** (convención UEFI). Las construye [`EspLayout`] por ti.
pub fn render_entry_conf(title: &str, cmdline_extra: &str) -> String {
    let cmdline = canonical_cmdline(cmdline_extra);
    format!(
        "title    {title}\n\
         linux    /EFI/arje/vmlinuz\n\
         initrd   /EFI/arje/initramfs.cpio.gz\n\
         options  {options}\n",
        options = strip_initrd_from_options(&cmdline),
    )
}

/// El `initrd` del cmdline ya está cubierto por la línea `initrd` del
/// .conf — algunos bootloaders se confunden si aparece dos veces. Strippeamos
/// el prefijo `initrd=...` (un solo token) cuando lo emitimos como
/// `options`.
fn strip_initrd_from_options(cmdline: &str) -> String {
    cmdline
        .split_whitespace()
        .filter(|tok| !tok.starts_with("initrd="))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Contenido del `loader/loader.conf` global — selecciona arje como entry
/// por defecto y le da 3 segundos al usuario para ver las opciones.
pub fn render_loader_conf() -> String {
    "default arje\ntimeout 3\nconsole-mode auto\n".to_string()
}

/// Construye el initramfs invocando a `arje-packager` con la lista de
/// binarios provista. Devuelve los bytes gzipeados listos para escribir
/// a la ESP, y la `EntityCard` parseada (el caller probablemente la
/// vuelve a serializar para guardar la `seed.card.json` en la ESP).
pub fn build_initramfs(
    seed_path: &Path,
    bins: &[(String, PathBuf)],
) -> anyhow::Result<(Vec<u8>, EntityCard)> {
    build_initramfs_with_assets(seed_path, bins, &[])
}

/// Como [`build_initramfs`], pero además hornea archivos extra `dest → src`
/// (config del splash, imagen, frames…). Espeja `arje-packager --asset`.
pub fn build_initramfs_with_assets(
    seed_path: &Path,
    bins: &[(String, PathBuf)],
    assets: &[(String, PathBuf)],
) -> anyhow::Result<(Vec<u8>, EntityCard)> {
    build_initramfs_with_assets_signed(seed_path, bins, assets, None)
}

/// Como [`build_initramfs_with_assets`], pero si `rootkey_seed` es `Some`,
/// **firma el manifiesto de atestación** (A1) sobre los binarios críticos antes
/// de serializar la seed — usa el MISMO `arje_packager::sign_seed_attest` que el
/// packager, así el manifiesto es idéntico por cualquier ruta de instalación. La
/// Card devuelta ya trae `attest`/`attest_rootkey`, de modo que el seed que el
/// caller escribe a la ESP queda firmado igual que el embebido en el initramfs.
pub fn build_initramfs_with_assets_signed(
    seed_path: &Path,
    bins: &[(String, PathBuf)],
    assets: &[(String, PathBuf)],
    rootkey_seed: Option<[u8; 32]>,
) -> anyhow::Result<(Vec<u8>, EntityCard)> {
    use anyhow::Context;
    let mut card = EntityCard::from_path(seed_path)
        .with_context(|| format!("cargando seed {}", seed_path.display()))?;

    // Recolectamos exec paths declarados — debe haber un binario por cada
    // label Native/Legacy del fractal. Repetimos la lógica del CLI del
    // packager acá para no acoplar al binario.
    let mut required: std::collections::BTreeMap<String, String> = Default::default();
    required.insert("arje-zero".into(), "sbin/arje-zero".into());
    collect_native(&card, &mut required);

    // Binarios referenciados por cards del card-store (`/etc/arje/cards.d/*.json`,
    // horneadas como assets): sus execs también deben instalarse (0o755), porque
    // el runtime las encarna por `SpawnCardFromDisk` y NO están en el genesis de
    // la seed. Sin esto, un bundle de sesión (p. ej. `session-gnome`) quedaría con
    // sus shims declarados pero sin binario en el arranque nativo.
    for (dest, src) in assets {
        let dest_norm = dest.trim_start_matches('/');
        if !(dest_norm.starts_with("etc/arje/cards.d/") && dest_norm.ends_with(".json")) {
            continue;
        }
        let store_card = EntityCard::from_path(src)
            .with_context(|| format!("parseando card del store {}", src.display()))?;
        collect_card_execs(&store_card, &mut required);
    }

    let bin_map: std::collections::BTreeMap<String, PathBuf> =
        bins.iter().cloned().collect();

    let mut tree: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    for (label, dest_rel) in &required {
        let src = bin_map.get(label).ok_or_else(|| {
            anyhow::anyhow!("falta --bin {label}=... (lo exige el fractal)")
        })?;
        let data = std::fs::read(src)
            .with_context(|| format!("leyendo binario {label} desde {}", src.display()))?;
        tree.insert(dest_rel.clone(), data);
    }

    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
    let mut w = CpioWriter::new(&mut buf);
    for dir in [
        "dev", "ente", "proc", "run", "sys", "sbin", "usr", "usr/lib", "usr/lib/arje",
    ] {
        w.append(dir, EntryKind::Directory)?;
    }
    w.append(
        "dev/console",
        EntryKind::CharDev { major: 5, minor: 1, perm: 0o600 },
    )?;
    w.append("init", EntryKind::Symlink { target: "sbin/arje-zero" })?;

    // Atestación (A1): firmamos el manifiesto sobre `tree` (los binarios ya
    // leídos, en orden de BTreeMap) ANTES de serializar la seed, para que tanto
    // el seed embebido en el initramfs como el que el caller escribe a la ESP
    // lleven el manifiesto. Reusa el firmador del packager → manifiesto idéntico.
    if let Some(seed) = rootkey_seed {
        arje_packager::sign_seed_attest(&mut card, &tree, seed);
    }

    let seed_bytes = serde_json::to_vec_pretty(&card)?;
    w.append(
        "ente/seed.card.json",
        EntryKind::Regular { data: &seed_bytes, perm: 0o644 },
    )?;
    for (rel, data) in &tree {
        w.append(rel, EntryKind::Regular { data, perm: 0o755 })?;
    }

    // Assets extra (config del splash, imagen, frames) con sus directorios padre.
    let mut dirs: std::collections::BTreeSet<String> = [
        "dev", "ente", "proc", "run", "sys", "sbin", "usr", "usr/lib", "usr/lib/arje",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for (dest, src) in assets {
        let dest = dest.trim_start_matches('/');
        let comps: Vec<&str> = dest.split('/').collect();
        let mut acc = String::new();
        for comp in &comps[..comps.len().saturating_sub(1)] {
            if comp.is_empty() {
                continue;
            }
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(comp);
            if dirs.insert(acc.clone()) {
                w.append(&acc, EntryKind::Directory)?;
            }
        }
        let data = std::fs::read(src)
            .with_context(|| format!("leyendo asset {dest} desde {}", src.display()))?;
        w.append(dest, EntryKind::Regular { data: &data, perm: 0o644 })?;
    }
    let _: &mut Vec<u8> = w.finish()?;

    let gz = arje_packager::gzip(&buf)?;
    Ok((gz, card))
}

fn collect_native(
    card: &EntityCard,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    use arje_card::Payload;
    for child in &card.genesis {
        match &child.payload {
            Payload::Native { exec, .. } | Payload::Legacy { exec, .. } => {
                let rel = exec.strip_prefix('/').unwrap_or(exec).to_string();
                out.insert(child.label.clone(), rel);
            }
            _ => {}
        }
        collect_native(child, out);
    }
}

/// Como [`collect_native`] pero incluye el exec de la **propia** card, no
/// sólo su genesis. Para cards del card-store, que pueden ser un único Ente
/// Native (su exec está en la raíz) o un bundle `Virtual` con los Entes en
/// `genesis` (p. ej. `session-gnome`).
fn collect_card_execs(
    card: &EntityCard,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    use arje_card::Payload;
    if let Payload::Native { exec, .. } | Payload::Legacy { exec, .. } = &card.payload {
        let rel = exec.strip_prefix('/').unwrap_or(exec).to_string();
        out.insert(card.label.clone(), rel);
    }
    collect_native(card, out);
}

/// Args del comando `efibootmgr` para crear una entrada NVRAM directa al
/// kernel (EFISTUB), sin bootloader intermedio. La firmware UEFI le pasa
/// `options` como UEFI LoadOption args, que el stub del kernel interpreta
/// como cmdline.
///
/// `disk` es el dispositivo block (p. ej. `/dev/sda`) y `partition` el
/// índice de la ESP (1-based). `efi_kernel_path` es la ruta DENTRO de la
/// partición ESP con backslashes (`\EFI\arje\vmlinuz`).
///
/// Devuelve los args como `Vec<String>` para que el caller los pase a
/// `Command::new("efibootmgr").args(...)`. La función no ejecuta nada —
/// es testeable.
pub fn efibootmgr_create_args(
    disk: &str,
    partition: u32,
    label: &str,
    efi_kernel_path: &str,
    cmdline: &str,
) -> Vec<String> {
    vec![
        "--create".into(),
        "--disk".into(),
        disk.into(),
        "--part".into(),
        partition.to_string(),
        "--loader".into(),
        efi_kernel_path.into(),
        "--label".into(),
        label.into(),
        "--unicode".into(),
        cmdline.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_firma_seed_y_los_binarios_atestan() {
        use arje_card::{EntityCard, Payload};

        let dir = tempfile::tempdir().unwrap();
        // Binarios fake del fractal (arje-zero + un Ente Native).
        let zero = dir.path().join("arje-zero");
        let app = dir.path().join("miapp");
        std::fs::write(&zero, b"fake arje-zero bytes").unwrap();
        std::fs::write(&app, b"fake miapp bytes").unwrap();

        // Seed con un genesis Native que apunta a /usr/bin/miapp.
        let mut seed = EntityCard::new("seed-installer-test");
        let mut hijo = EntityCard::new("miapp");
        hijo.payload = Payload::Native {
            exec: "/usr/bin/miapp".into(),
            argv: vec![],
            envp: vec![],
        };
        seed.genesis.push(hijo);
        let seed_path = dir.path().join("seed.card.json");
        std::fs::write(&seed_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();

        let bins = vec![
            ("arje-zero".to_string(), zero.clone()),
            ("miapp".to_string(), app.clone()),
        ];
        let rootkey = [9u8; 32];

        // Sin rootkey: el seed sale SIN attest (compat).
        let (_gz, plano) =
            build_initramfs_with_assets_signed(&seed_path, &bins, &[], None).unwrap();
        assert!(plano.attest.is_empty());
        assert!(plano.attest_rootkey.is_none());

        // Con rootkey: firma arje-zero + miapp y ancla el manifiesto.
        let (_gz, card) =
            build_initramfs_with_assets_signed(&seed_path, &bins, &[], Some(rootkey)).unwrap();
        assert_eq!(card.attest.len(), 2, "debe firmar arje-zero + miapp");
        let pubkey = card.attest_rootkey.expect("attest_rootkey poblada");

        // Los binarios vivos atestan Ok contra el manifiesto firmado (lo mismo
        // que hará `arje-zero` al boot / `--attest-check`).
        for bytes in [b"fake arje-zero bytes".as_slice(), b"fake miapp bytes".as_slice()] {
            let v = arje_attest::atestar_bytes(&card.attest, bytes, Some(pubkey));
            assert!(v.es_ok(), "debería atestar Ok, fue {}", v.motivo());
        }
        // Un binario no firmado cae en NoAtestada.
        let impostor = arje_attest::atestar_bytes(&card.attest, b"impostor", Some(pubkey));
        assert!(!impostor.es_ok());
    }

    #[test]
    fn card_store_bundle_exige_binarios_de_sus_shims() {
        use arje_card::{EntityCard, Payload};

        let dir = tempfile::tempdir().unwrap();
        let zero = dir.path().join("arje-zero");
        let shim = dir.path().join("arje-logind-compat");
        std::fs::write(&zero, b"fake arje-zero").unwrap();
        std::fs::write(&shim, b"fake logind shim").unwrap();

        // Seed mínima (sin el shim en su genesis).
        let seed = EntityCard::new("seed-min");
        let seed_path = dir.path().join("seed.card.json");
        std::fs::write(&seed_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();

        // Fragmento bundle: Virtual con un shim Native en genesis.
        let mut bundle = EntityCard::new("session-gnome");
        let mut logind = EntityCard::new("compat-logind");
        logind.payload = Payload::Native {
            exec: "/usr/lib/arje/arje-logind-compat".into(),
            argv: vec![],
            envp: vec![],
        };
        bundle.genesis.push(logind);
        let frag_path = dir.path().join("session-gnome.card.json");
        std::fs::write(&frag_path, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();

        let assets = vec![(
            "etc/arje/cards.d/session-gnome.json".to_string(),
            frag_path.clone(),
        )];

        // Sin el --bin del shim: el bundle del store lo exige → falla claro.
        let bins_sin = vec![("arje-zero".to_string(), zero.clone())];
        let err = build_initramfs_with_assets_signed(&seed_path, &bins_sin, &assets, None)
            .unwrap_err();
        assert!(
            err.to_string().contains("compat-logind"),
            "debe exigir el binario del shim del bundle: {err}"
        );

        // Con el --bin (por label del fragmento): hornea OK.
        let bins = vec![
            ("arje-zero".to_string(), zero),
            ("compat-logind".to_string(), shim),
        ];
        let (gz, _card) =
            build_initramfs_with_assets_signed(&seed_path, &bins, &assets, None).unwrap();
        assert!(!gz.is_empty(), "el initramfs debe armarse con el shim instalado");
    }

    #[test]
    fn cmdline_canonico_sin_extra() {
        let c = canonical_cmdline("");
        assert!(c.starts_with(r"initrd=\EFI\arje\initramfs.cpio.gz"), "{c}");
        // Flags flicker-free siempre presentes.
        assert!(c.contains("quiet"), "{c}");
        assert!(c.contains("vt.global_cursor_default=0"), "{c}");
        // Sin extra ⇒ no hay basura colgando al final.
        assert!(!c.trim_end().ends_with(' '), "{c}");
        assert_eq!(canonical_cmdline("   "), canonical_cmdline(""));
    }

    #[test]
    fn cmdline_canonico_con_extra() {
        let c = canonical_cmdline("console=ttyS0 panic=10");
        assert!(c.starts_with(r"initrd=\EFI\arje\initramfs.cpio.gz"), "{c}");
        assert!(c.contains("quiet"), "{c}");
        // El extra del usuario va al final, tras los flicker-free.
        assert!(c.ends_with("console=ttyS0 panic=10"), "{c}");
    }

    #[test]
    fn entry_conf_tiene_linux_initrd_y_options_sin_initrd() {
        let s = render_entry_conf("arje", "console=ttyS0 panic=10");
        assert!(s.contains("title    arje\n"), "{s}");
        assert!(s.contains("linux    /EFI/arje/vmlinuz\n"), "{s}");
        assert!(s.contains("initrd   /EFI/arje/initramfs.cpio.gz\n"), "{s}");
        // options no debe duplicar initrd=
        let opts_line = s.lines().find(|l| l.starts_with("options")).unwrap();
        assert!(!opts_line.contains("initrd="), "options duplicó initrd: {opts_line}");
        assert!(opts_line.contains("console=ttyS0"));
        assert!(opts_line.contains("panic=10"));
    }

    #[test]
    fn loader_conf_default_es_arje_con_timeout() {
        let s = render_loader_conf();
        assert!(s.contains("default arje"));
        assert!(s.contains("timeout 3"));
    }

    #[test]
    fn esp_layout_devuelve_rutas_canonicas() {
        let l = EspLayout::new("/mnt/esp");
        assert_eq!(l.kernel(), PathBuf::from("/mnt/esp/EFI/arje/vmlinuz"));
        assert_eq!(
            l.initramfs(),
            PathBuf::from("/mnt/esp/EFI/arje/initramfs.cpio.gz")
        );
        assert_eq!(l.arje_entry(), PathBuf::from("/mnt/esp/loader/entries/arje.conf"));
        assert_eq!(l.bootx64_fallback(), PathBuf::from("/mnt/esp/EFI/BOOT/BOOTX64.EFI"));
    }

    #[test]
    fn efibootmgr_args_son_los_que_esperaria_el_cli() {
        let args = efibootmgr_create_args(
            "/dev/sda",
            1,
            "arje",
            r"\EFI\arje\vmlinuz",
            r"initrd=\EFI\arje\initramfs.cpio.gz console=tty0",
        );
        // Forma esperada por `man efibootmgr`.
        assert_eq!(args[0], "--create");
        assert!(args.windows(2).any(|w| w[0] == "--disk" && w[1] == "/dev/sda"));
        assert!(args.windows(2).any(|w| w[0] == "--part" && w[1] == "1"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--loader" && w[1] == r"\EFI\arje\vmlinuz"));
        assert!(args.windows(2).any(|w| w[0] == "--label" && w[1] == "arje"));
        // El cmdline va como un solo arg luego de --unicode.
        let unicode_pos = args.iter().position(|a| a == "--unicode").unwrap();
        assert!(args[unicode_pos + 1].contains("initrd=\\EFI\\arje"));
        assert!(args[unicode_pos + 1].contains("console=tty0"));
    }
}
