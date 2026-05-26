//! `arje-loader` — bootloader EFI del fractal arje.
//!
//! Vive en la ESP como `/EFI/BOOT/BOOTX64.EFI` (path UEFI fallback). La
//! firmware lo ejecuta sin más; el loader:
//!
//! 1. Localiza el filesystem desde el cual fue cargado (LoadedImage →
//!    SimpleFileSystem).
//! 2. Lee `/loader/entries/arje.conf` y parsea `linux`/`initrd`/`options`
//!    (formato systemd-boot compatible).
//! 3. Carga el kernel (PE/COFF con EFISTUB) en un buffer y lo pasa a
//!    `BootServices::load_image` (FromBuffer).
//! 4. Setea las `LoadOptions` del kernel = cmdline con `initrd=` al frente.
//! 5. `start_image` → control al kernel.
//!
//! ## Por qué este crate existe
//!
//! `systemd-boot` y `rEFInd` son los bootloaders EFI estándar para Linux,
//! pero ambos vienen de proyectos externos con cadenas de build complejas.
//! Para que `arje-installer to-usb` funcione sin requerir que el host tenga
//! uno pre-empacado, el fractal lleva su propio loader — chico (~20 KB), en
//! Rust, entendible de punta a punta.
//!
//! ## Estado actual (2026-05-26)
//!
//! El loader compila, ejecuta y carga el kernel correctamente bajo OVMF.
//! La cadena `firmware UEFI → arje-loader → start_image(kernel)` funciona.
//!
//! **Sin embargo**, los kernels Linux ≥ 5.10 (Artix linux 7.0.8) **YA NO
//! soportan `initrd=` por cmdline** y exigen que el bootloader instale el
//! `LINUX_EFI_INITRD_MEDIA_GUID` LoadFile2 protocol. Sin ese protocol, el
//! kernel arranca pero falla con:
//!
//! ```text
//! EFI stub: ERROR: Failed to handle fs_proto
//! EFI stub: ERROR: Failed to load initrd: 0x8000000000000002
//! ```
//!
//! Implementar LoadFile2 protocol en uefi-rs 0.35 requiere:
//!
//! - Definir la struct con vtable manual de `EFI_LOAD_FILE2_PROTOCOL`.
//! - Construir un device path media-vendor con
//!   `5568e427-68fc-4f3d-ac74-ca555231cc68`.
//! - `boot::install_protocol_interface` con esa GUID + handle nuevo.
//! - El callback de la vtable copia los bytes del initrd al buffer del
//!   kernel.
//!
//! Es la próxima iteración. Mientras tanto: el `to-partition` flow con
//! `--register efibootmgr` arranca correctamente porque la NVRAM entry
//! incluye el cmdline y EFISTUB lee el initrd directo de la ESP por el
//! viejo método (compatible con kernels que aún tienen el fallback). Sólo
//! `to-usb` queda con esta limitación temporal.

#![no_main]
#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::{boot, CStr16};

/// Ruta canónica donde arje-installer deja la entry.
const ENTRY_PATH: &str = r"\loader\entries\arje.conf";

#[entry]
fn main() -> Status {
    uefi::helpers::init().expect("uefi::helpers::init falló");
    log::info!("arje-loader :: despertando");
    match run() {
        Ok(()) => Status::SUCCESS,
        Err(s) => {
            log::error!("arje-loader :: ERROR Status={s:?}");
            boot::stall(5_000_000);
            s
        }
    }
}

fn run() -> Result<(), Status> {
    let mut fs_root = open_boot_fs()?;
    let entry_text = read_file_utf8(&mut fs_root, ENTRY_PATH)?;
    let entry = parse_entry(&entry_text);
    log::info!(
        "arje-loader :: entry — linux={} initrd={} options={}",
        entry.linux.as_deref().unwrap_or("(falta)"),
        entry.initrd.as_deref().unwrap_or("(falta)"),
        entry.options.as_deref().unwrap_or(""),
    );

    let linux = entry.linux.ok_or(Status::INVALID_PARAMETER)?;
    let initrd = entry.initrd.ok_or(Status::INVALID_PARAMETER)?;
    let options = entry.options.unwrap_or_default();

    let kernel_bytes = read_file_bytes(&mut fs_root, &linux)?;
    log::info!("arje-loader :: kernel cargado, {} bytes", kernel_bytes.len());
    drop(fs_root);

    let kernel_handle = boot::load_image(
        boot::image_handle(),
        boot::LoadImageSource::FromBuffer {
            buffer: &kernel_bytes,
            file_path: None,
        },
    )
    .map_err(|e| {
        log::error!("load_image: {e:?}");
        e.status()
    })?;
    log::info!("arje-loader :: LoadImage OK");

    let cmdline = compose_cmdline(&initrd, &options);
    log::info!("arje-loader :: cmdline = {cmdline}");
    set_load_options(kernel_handle, &cmdline)?;

    log::info!("arje-loader :: start_image");
    boot::start_image(kernel_handle).map_err(|e| e.status())?;
    Ok(())
}

fn open_boot_fs() -> Result<uefi::proto::media::file::Directory, Status> {
    let loaded = boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle())
        .map_err(|e| e.status())?;
    let device = loaded.device().ok_or(Status::DEVICE_ERROR)?;
    drop(loaded);
    let mut fs = boot::open_protocol_exclusive::<SimpleFileSystem>(device)
        .map_err(|e| e.status())?;
    fs.open_volume().map_err(|e| e.status())
}

fn read_file_bytes(
    root: &mut uefi::proto::media::file::Directory,
    path: &str,
) -> Result<Vec<u8>, Status> {
    let mut handle = open_regular(root, path)?;
    let size = file_size(&mut handle)?;
    let mut buf = alloc::vec![0u8; size];
    handle.read(&mut buf).map_err(|e| e.status())?;
    Ok(buf)
}

fn read_file_utf8(
    root: &mut uefi::proto::media::file::Directory,
    path: &str,
) -> Result<String, Status> {
    let bytes = read_file_bytes(root, path)?;
    String::from_utf8(bytes).map_err(|_| Status::INVALID_PARAMETER)
}

fn open_regular(
    root: &mut uefi::proto::media::file::Directory,
    path: &str,
) -> Result<RegularFile, Status> {
    let mut buf = [0u16; 256];
    let cpath = CStr16::from_str_with_buf(path, &mut buf).map_err(|_| Status::INVALID_PARAMETER)?;
    let h = root
        .open(cpath, FileMode::Read, FileAttribute::empty())
        .map_err(|e| e.status())?;
    match h.into_type().map_err(|e| e.status())? {
        FileType::Regular(f) => Ok(f),
        FileType::Dir(_) => Err(Status::INVALID_PARAMETER),
    }
}

fn file_size(file: &mut RegularFile) -> Result<usize, Status> {
    let info = file.get_boxed_info::<FileInfo>().map_err(|e| e.status())?;
    Ok(info.file_size() as usize)
}

#[derive(Default)]
struct Entry {
    linux: Option<String>,
    initrd: Option<String>,
    options: Option<String>,
}

fn parse_entry(text: &str) -> Entry {
    let mut e = Entry::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let val = val.trim().to_string();
        match key.trim() {
            "linux" => e.linux = Some(val),
            "initrd" => e.initrd = Some(val),
            "options" => e.options = Some(val),
            _ => {}
        }
    }
    e
}

fn compose_cmdline(initrd_path: &str, options: &str) -> String {
    if options.contains("initrd=") {
        options.into()
    } else {
        let mut s = String::with_capacity(options.len() + initrd_path.len() + 12);
        s.push_str("initrd=");
        s.push_str(initrd_path);
        if !options.is_empty() {
            s.push(' ');
            s.push_str(options);
        }
        s
    }
}

fn set_load_options(kernel: uefi::Handle, cmdline: &str) -> Result<(), Status> {
    let mut loaded =
        boot::open_protocol_exclusive::<LoadedImage>(kernel).map_err(|e| e.status())?;
    let mut buf: Vec<u16> = cmdline.encode_utf16().collect();
    buf.push(0);
    let bytes = buf.len() * 2;
    unsafe {
        loaded.set_load_options(buf.as_ptr() as *const u8, bytes as u32);
    }
    // UEFI requiere que el buffer sobreviva al call; start_image dispara
    // el kernel y el loader nunca retorna al destructor — leak intencional.
    core::mem::forget(buf);
    drop(loaded);
    Ok(())
}

// Sin #[cfg(test)] inline porque el crate `uefi` define un `panic_impl`
// que colisiona con el de std al correr `cargo test` en el target del host.
// La lógica testeable (parse_entry, compose_cmdline) se duplicaría en un
// crate hermano testeable; por ahora la dejamos cubierta sólo por el
// smoke real bajo OVMF.
