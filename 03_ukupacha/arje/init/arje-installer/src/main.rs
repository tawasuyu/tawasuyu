//! `arje-installer` — CLI con dos subcomandos:
//!
//! - `to-partition --esp <mountpoint> --kernel <bzImage> --seed <card.json>
//!    [--bin LABEL=PATH]... [--cmdline "console=tty0 panic=10"]
//!    [--register --disk /dev/sda --part 1] [--label "arje"]`
//!
//!   No destructivo. Asume la ESP ya montada. Copia bajo `<esp>/EFI/arje/`
//!   y, si `--register` se pasa, llama a `efibootmgr` para registrar una
//!   entrada NVRAM directa al kernel (EFISTUB, sin bootloader). Si no se
//!   pasa, imprime el comando efibootmgr equivalente y termina — el
//!   usuario decide cuándo modificar la NVRAM.
//!
//! - `to-usb --device /dev/sdX --kernel ... --seed ... [--bin ...]...
//!    [--cmdline ...] --yes-destroy`
//!
//!   Destructivo: borra el contenido del device. Crea GPT + una partición
//!   ESP FAT32, monta en un tmpdir, copia los archivos y, si hay un
//!   bootloader UEFI disponible en el host (systemd-boot, rEFInd), lo
//!   instala. Si no lo hay, deja el kernel en `/EFI/BOOT/BOOTX64.EFI`
//!   (fallback UEFI) y reporta que el cmdline no se puede pasar sin
//!   bootloader — el usuario tendrá que `efibootmgr` en cada destino.
//!
//! Ambos modos requieren `arje-zero` compilado (estático) y los binarios
//! Native del genesis como `--bin LABEL=PATH`. Reusan la misma lib que
//! `arje-packager` para emitir el initramfs.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{anyhow, bail, Context};
use arje_installer::{
    build_initramfs, canonical_cmdline, efibootmgr_create_args, render_entry_conf,
    render_loader_conf, EspLayout,
};

const HELP: &str = "\
arje-installer — copia kernel + initramfs + seed a una ESP, o arma un USB GPT/ESP booteable.

USO:
    arje-installer to-partition --esp PATH --kernel PATH --seed PATH \\
        [--bin LABEL=PATH]... [--cmdline STR] [--label NAME] \\
        [--register --disk /dev/sdX --part N]

    arje-installer to-usb --device /dev/sdX --kernel PATH --seed PATH \\
        [--bin LABEL=PATH]... [--cmdline STR] [--label NAME] --yes-destroy

COMUNES:
    --kernel    bzImage del kernel Linux (p. ej. /boot/vmlinuz-linux).
    --seed      Seed canónica del fractal (.card.json).
    --bin       LABEL=PATH del binario por cada Ente Native/Legacy del genesis.
                'arje-zero' siempre se requiere.
    --cmdline   Args extra del kernel cmdline (sin el `initrd=` — lo agrega el
                installer). Ej.: \"console=ttyS0 panic=10\".
    --label     Nombre para la entrada de boot / .conf. Default: \"arje\".

to-partition:
    --esp       Ruta donde está montada la ESP (p. ej. /boot o /mnt/esp).
    --register  Llamar a efibootmgr para crear la NVRAM entry. Exige --disk y --part.
    --disk      Dispositivo de la ESP (p. ej. /dev/sda). Solo con --register.
    --part      Índice de la partición ESP (1-based). Solo con --register.

to-usb:
    --device       Dispositivo block completo (NO partición). Se borra entero.
    --yes-destroy  Confirmación explícita. Sin este flag aborta.
";

#[derive(Debug)]
struct CommonArgs {
    kernel: PathBuf,
    seed: PathBuf,
    bins: Vec<(String, PathBuf)>,
    cmdline_extra: String,
    label: String,
}

enum Mode {
    ToPartition {
        common: CommonArgs,
        esp: PathBuf,
        register: Option<(String, u32)>, // (disk, partition_index)
    },
    ToUsb {
        common: CommonArgs,
        device: PathBuf,
        confirmed: bool,
    },
}

fn parse_args() -> anyhow::Result<Mode> {
    let mut it = std::env::args().skip(1);
    let sub = it.next().context("falta subcomando (to-partition | to-usb)")?;
    match sub.as_str() {
        "-h" | "--help" => {
            eprintln!("{HELP}");
            std::process::exit(0);
        }
        "to-partition" => parse_to_partition(it),
        "to-usb" => parse_to_usb(it),
        other => bail!("subcomando desconocido: {other}"),
    }
}

/// Acumulador mutable usado durante el parse — los Options se promueven
/// a sus campos finales al cierre, errando si falta alguno requerido.
#[derive(Default)]
struct CommonAcc {
    kernel: Option<PathBuf>,
    seed: Option<PathBuf>,
    bins: Vec<(String, PathBuf)>,
    cmdline_extra: String,
    label: Option<String>,
}

impl CommonAcc {
    /// Procesa un flag si es común. Devuelve `true` si lo consumió.
    fn try_consume(
        &mut self,
        a: &str,
        args: &mut impl Iterator<Item = String>,
    ) -> anyhow::Result<bool> {
        match a {
            "--kernel" => self.kernel = Some(args.next().context("--kernel requiere path")?.into()),
            "--seed" => self.seed = Some(args.next().context("--seed requiere path")?.into()),
            "--bin" => {
                let kv = args.next().context("--bin requiere LABEL=PATH")?;
                let (l, p) = kv
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--bin esperaba LABEL=PATH, vino {kv:?}"))?;
                self.bins.push((l.to_string(), PathBuf::from(p)));
            }
            "--cmdline" => self.cmdline_extra = args.next().context("--cmdline requiere string")?,
            "--label" => self.label = Some(args.next().context("--label requiere nombre")?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn finalize(self) -> anyhow::Result<CommonArgs> {
        Ok(CommonArgs {
            kernel: self.kernel.ok_or_else(|| anyhow!("falta --kernel"))?,
            seed: self.seed.ok_or_else(|| anyhow!("falta --seed"))?,
            bins: self.bins,
            cmdline_extra: self.cmdline_extra,
            label: self.label.unwrap_or_else(|| "arje".to_string()),
        })
    }
}

fn parse_to_partition(mut args: impl Iterator<Item = String>) -> anyhow::Result<Mode> {
    let mut common = CommonAcc::default();
    let mut esp: Option<PathBuf> = None;
    let mut register = false;
    let mut disk: Option<String> = None;
    let mut part: Option<u32> = None;

    while let Some(a) = args.next() {
        if common.try_consume(&a, &mut args)? {
            continue;
        }
        match a.as_str() {
            "--esp" => esp = Some(args.next().context("--esp requiere path")?.into()),
            "--register" => register = true,
            "--disk" => disk = Some(args.next().context("--disk requiere path")?),
            "--part" => {
                part = Some(
                    args.next()
                        .context("--part requiere número")?
                        .parse()
                        .context("--part debe ser entero")?,
                );
            }
            other => bail!("flag desconocido en to-partition: {other}"),
        }
    }

    let register = if register {
        let d = disk.ok_or_else(|| anyhow!("--register exige --disk"))?;
        let p = part.ok_or_else(|| anyhow!("--register exige --part"))?;
        Some((d, p))
    } else {
        if disk.is_some() || part.is_some() {
            bail!("--disk/--part sólo tienen sentido con --register");
        }
        None
    };
    Ok(Mode::ToPartition {
        common: common.finalize()?,
        esp: esp.ok_or_else(|| anyhow!("falta --esp"))?,
        register,
    })
}

fn parse_to_usb(mut args: impl Iterator<Item = String>) -> anyhow::Result<Mode> {
    let mut common = CommonAcc::default();
    let mut device: Option<PathBuf> = None;
    let mut confirmed = false;

    while let Some(a) = args.next() {
        if common.try_consume(&a, &mut args)? {
            continue;
        }
        match a.as_str() {
            "--device" => device = Some(args.next().context("--device requiere path")?.into()),
            "--yes-destroy" => confirmed = true,
            other => bail!("flag desconocido en to-usb: {other}"),
        }
    }

    Ok(Mode::ToUsb {
        common: common.finalize()?,
        device: device.ok_or_else(|| anyhow!("falta --device"))?,
        confirmed,
    })
}

fn main() -> ExitCode {
    let mode = match parse_args() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("arje-installer :: ERROR {e:#}");
            eprintln!();
            eprintln!("{HELP}");
            return ExitCode::FAILURE;
        }
    };
    match run(mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("arje-installer :: ERROR {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(mode: Mode) -> anyhow::Result<()> {
    match mode {
        Mode::ToPartition { common, esp, register } => {
            ensure_dir_exists(&esp)?;
            let layout = EspLayout::new(&esp);
            stage_files(&common, &layout)?;
            write_optional_loader_files(&layout, &common)?;

            if let Some((disk, part)) = register {
                register_nvram_entry(&disk, part, &common)?;
                eprintln!(
                    "arje-installer :: NVRAM entry \"{}\" registrada — reboot para arrancar arje.",
                    common.label
                );
            } else {
                let cmdline = canonical_cmdline(&common.cmdline_extra);
                eprintln!(
                    "arje-installer :: archivos copiados a {esp}.\n\
                     Para registrar la entrada NVRAM y arrancar arje, corré:\n\n  \
                     efibootmgr {}\n\n  \
                     (sustituí --disk/--part por tu device y el índice de la ESP)",
                    pretty_efibootmgr(&common.label, &cmdline),
                    esp = esp.display(),
                );
            }
            Ok(())
        }
        Mode::ToUsb { common, device, confirmed } => {
            if !confirmed {
                bail!(
                    "to-usb borra el device entero — pasá --yes-destroy si estás segur@ \
                     ({} se va a re-particionar)",
                    device.display()
                );
            }
            install_to_usb(&device, &common)
        }
    }
}

fn ensure_dir_exists(p: &Path) -> anyhow::Result<()> {
    if !p.is_dir() {
        bail!("{} no es un directorio existente — montá la ESP primero", p.display());
    }
    Ok(())
}

fn stage_files(common: &CommonArgs, layout: &EspLayout) -> anyhow::Result<()> {
    let (initramfs, card) = build_initramfs(&common.seed, &common.bins)?;
    std::fs::create_dir_all(layout.arje_dir())
        .with_context(|| format!("mkdir {}", layout.arje_dir().display()))?;
    std::fs::copy(&common.kernel, layout.kernel())
        .with_context(|| format!("copy {} -> {}", common.kernel.display(), layout.kernel().display()))?;
    std::fs::write(layout.initramfs(), &initramfs)
        .with_context(|| format!("write {}", layout.initramfs().display()))?;
    let seed_bytes = serde_json::to_vec_pretty(&card)?;
    std::fs::write(layout.seed(), &seed_bytes)
        .with_context(|| format!("write {}", layout.seed().display()))?;
    std::fs::write(layout.cmdline_txt(), canonical_cmdline(&common.cmdline_extra))
        .with_context(|| format!("write {}", layout.cmdline_txt().display()))?;
    eprintln!(
        "arje-installer :: stage {} bytes kernel, {} bytes initramfs, {} bytes seed",
        std::fs::metadata(layout.kernel())?.len(),
        initramfs.len(),
        seed_bytes.len(),
    );
    Ok(())
}

/// Si existe la convención de loader (`<esp>/loader/`), escribir las
/// entradas y el `loader.conf`. Esto deja la ESP lista para systemd-boot
/// o rEFInd si el usuario los instala luego — son no-ops si nunca se
/// instala un bootloader.
fn write_optional_loader_files(layout: &EspLayout, common: &CommonArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(layout.entries_dir())
        .with_context(|| format!("mkdir {}", layout.entries_dir().display()))?;
    std::fs::write(
        layout.arje_entry(),
        render_entry_conf(&common.label, &common.cmdline_extra),
    )
    .with_context(|| format!("write {}", layout.arje_entry().display()))?;
    std::fs::write(layout.loader_conf(), render_loader_conf())
        .with_context(|| format!("write {}", layout.loader_conf().display()))?;
    Ok(())
}

fn register_nvram_entry(
    disk: &str,
    partition: u32,
    common: &CommonArgs,
) -> anyhow::Result<()> {
    let cmdline = canonical_cmdline(&common.cmdline_extra);
    let args = efibootmgr_create_args(
        disk,
        partition,
        &common.label,
        r"\EFI\arje\vmlinuz",
        &cmdline,
    );
    let status = Command::new("efibootmgr")
        .args(&args)
        .status()
        .context("ejecutando efibootmgr — instalalo si no lo tenés")?;
    if !status.success() {
        bail!("efibootmgr terminó con código {status}");
    }
    Ok(())
}

fn pretty_efibootmgr(label: &str, cmdline: &str) -> String {
    format!(
        "--create --disk /dev/sdX --part 1 --loader \\\\EFI\\\\arje\\\\vmlinuz \
         --label \"{label}\" --unicode \"{cmdline}\""
    )
}

fn install_to_usb(device: &Path, common: &CommonArgs) -> anyhow::Result<()> {
    // 1. Particionar con sfdisk (Linux nativo, sin parted).
    eprintln!("arje-installer :: particionando {} (GPT + ESP FAT32)…", device.display());
    let sfdisk_script = "label: gpt\n\
                         ,,U,*\n"; // U = EFI System, * = bootable flag
    let mut sfdisk = Command::new("sfdisk")
        .arg(device)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn sfdisk — instalalo si no lo tenés")?;
    {
        use std::io::Write;
        let stdin = sfdisk.stdin.as_mut().unwrap();
        stdin.write_all(sfdisk_script.as_bytes())?;
    }
    let st = sfdisk.wait()?;
    if !st.success() {
        bail!("sfdisk falló con código {st}");
    }

    // 2. Formatear partición 1 como FAT32.
    let part1 = first_partition_node(device);
    // Damos chance al kernel a registrar el partition node.
    udev_settle();
    eprintln!("arje-installer :: mkfs.fat -F32 -n ARJE {}", part1.display());
    let st = Command::new("mkfs.fat")
        .args(["-F32", "-n", "ARJE"])
        .arg(&part1)
        .status()
        .context("spawn mkfs.fat")?;
    if !st.success() {
        bail!("mkfs.fat falló con código {st}");
    }

    // 3. Mount en tmpdir.
    let mount = tempfile::tempdir().context("tempdir para mount")?;
    eprintln!(
        "arje-installer :: mount {} {}",
        part1.display(),
        mount.path().display()
    );
    let st = Command::new("mount")
        .arg(&part1)
        .arg(mount.path())
        .status()
        .context("spawn mount")?;
    if !st.success() {
        bail!("mount falló con código {st}");
    }

    // Garantía de unmount aun si la copia falla — el guardia mata sus
    // recursos en orden inverso al spawn.
    let _umount_guard = UmountGuard(mount.path().to_path_buf());

    // 4. Stage archivos (kernel/initramfs/seed/.conf).
    let layout = EspLayout::new(mount.path());
    stage_files(common, &layout)?;
    write_optional_loader_files(&layout, common)?;

    // 5. Bootloader. Buscamos systemd-bootx64.efi o refind_x64.efi.
    //    Si encontramos uno: lo copiamos a /EFI/BOOT/BOOTX64.EFI (fallback
    //    UEFI estándar) y al sistema le va a arrancar el bootloader, leer
    //    nuestro /loader/entries/arje.conf y bootear el kernel.
    //
    //    Si no encontramos: ponemos el kernel mismo en /EFI/BOOT/BOOTX64.EFI
    //    como último recurso. Funciona PERO sin cmdline embebido el
    //    kernel arrancará sin saber dónde está el initrd. El usuario tiene
    //    que registrar una NVRAM entry (efibootmgr) en cada máquina destino.
    std::fs::create_dir_all(layout.bootx64_fallback().parent().unwrap())?;
    match find_bootloader_efi() {
        Some(efi) => {
            std::fs::copy(&efi, layout.bootx64_fallback())
                .with_context(|| format!("copy {} -> {}", efi.display(), layout.bootx64_fallback().display()))?;
            eprintln!(
                "arje-installer :: bootloader {} instalado en {}",
                efi.display(),
                layout.bootx64_fallback().display()
            );
        }
        None => {
            std::fs::copy(&common.kernel, layout.bootx64_fallback())
                .with_context(|| format!("copy kernel -> BOOTX64.EFI"))?;
            eprintln!(
                "arje-installer :: sin bootloader en el host (ni systemd-boot ni rEFInd).\n\
                 El kernel quedó en /EFI/BOOT/BOOTX64.EFI como fallback UEFI. La\n\
                 firmware lo ejecutará pero SIN cmdline embebido — el initrd no\n\
                 se carga automáticamente. En cada máquina destino vas a tener que\n\
                 registrar una NVRAM entry con:\n\n  \
                 efibootmgr {}",
                pretty_efibootmgr(&common.label, &canonical_cmdline(&common.cmdline_extra)),
            );
        }
    }

    // 6. sync para que los buffers caigan al USB antes del umount.
    let _ = Command::new("sync").status();
    eprintln!("arje-installer :: USB listo. Removelo con seguridad antes de probarlo.");
    Ok(())
}

/// Heurística: `/dev/sdb` → `/dev/sdb1`; `/dev/nvme0n1` → `/dev/nvme0n1p1`.
fn first_partition_node(device: &Path) -> PathBuf {
    let s = device.to_string_lossy();
    if s.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        // Dispositivos que terminan en dígito (nvme, mmcblk) usan "p1".
        PathBuf::from(format!("{s}p1"))
    } else {
        PathBuf::from(format!("{s}1"))
    }
}

/// Espera best-effort a que udev registre los nodos de partición recién
/// creados — sin esto, `mkfs.fat` corre antes de que `/dev/sdb1` exista.
fn udev_settle() {
    let _ = Command::new("udevadm").arg("settle").status();
}

fn find_bootloader_efi() -> Option<PathBuf> {
    let candidates = [
        "/usr/lib/systemd/boot/efi/systemd-bootx64.efi",
        "/usr/share/refind/refind/refind_x64.efi",
        "/usr/share/refind/refind_x64.efi",
        "/boot/EFI/refind/refind_x64.efi",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

struct UmountGuard(PathBuf);
impl Drop for UmountGuard {
    fn drop(&mut self) {
        // No silenciamos el error — si umount falla, el usuario necesita
        // saberlo antes de tirar del USB.
        let st = Command::new("umount").arg(&self.0).status();
        match st {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("arje-installer :: WARN umount {} terminó con {s}", self.0.display()),
            Err(e) => eprintln!("arje-installer :: WARN umount {} falló: {e}", self.0.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_partition_node_sd() {
        assert_eq!(
            first_partition_node(Path::new("/dev/sdb")),
            PathBuf::from("/dev/sdb1")
        );
    }

    #[test]
    fn first_partition_node_nvme() {
        assert_eq!(
            first_partition_node(Path::new("/dev/nvme0n1")),
            PathBuf::from("/dev/nvme0n1p1")
        );
        assert_eq!(
            first_partition_node(Path::new("/dev/mmcblk0")),
            PathBuf::from("/dev/mmcblk0p1")
        );
    }

    #[test]
    fn pretty_efibootmgr_es_un_one_liner_copiable() {
        let s = pretty_efibootmgr("arje", r"initrd=\EFI\arje\initramfs.cpio.gz console=tty0");
        assert!(s.contains("--create"), "{s}");
        assert!(s.contains("--label \"arje\""), "{s}");
        // `--loader` necesita doble backslash en la shell (sino la shell come
        // los `\`). El cmdline pasa tal cual porque va dentro de `"..."`.
        assert!(s.contains(r"--loader \\EFI\\arje\\vmlinuz"), "{s}");
        assert!(s.contains(r#"--unicode "initrd=\EFI\arje"#), "{s}");
    }
}
