//! `arje-loader` — bootloader EFI del fractal arje.
//!
//! Vive en la ESP como `/EFI/BOOT/BOOTX64.EFI` (path UEFI fallback). La
//! firmware lo ejecuta sin más; el loader:
//!
//! 1. Localiza el filesystem desde el cual fue cargado (LoadedImage →
//!    SimpleFileSystem).
//! 2. Lee `/loader/entries/arje.conf` y parsea `linux`/`initrd`/`options`
//!    (formato systemd-boot compatible).
//! 3. Carga el initramfs a memoria y **registra un handle nuevo con un
//!    LoadFile2 protocol + device path media-vendor cuya GUID es
//!    `LINUX_EFI_INITRD_MEDIA_GUID`**. Es la API que el EFISTUB del
//!    kernel ≥ 5.10 usa para encontrar su initrd — sin ella el kernel
//!    arranca y muere con "Failed to load initrd".
//! 4. Carga el kernel (PE/COFF con EFISTUB) en un buffer y lo pasa a
//!    `BootServices::load_image` (FromBuffer).
//! 5. Setea las `LoadOptions` del kernel = cmdline (sin `initrd=` porque
//!    ahora va por el protocol).
//! 6. `start_image` → control al kernel; el kernel busca el handle del
//!    LoadFile2 + media-vendor, llama load_file dos veces (una para el
//!    tamaño, otra para los bytes), descomprime el initramfs y arranca.
//!
//! ## Por qué este crate existe
//!
//! `systemd-boot` y `rEFInd` son los bootloaders EFI estándar para Linux,
//! pero ambos vienen de proyectos externos con cadenas de build complejas.
//! Para que `arje-installer to-usb` funcione sin requerir que el host tenga
//! uno pre-empacado, el fractal lleva su propio loader — chico, en Rust,
//! entendible de punta a punta.

#![no_main]
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

use uefi::prelude::*;
use uefi::proto::device_path::DevicePath;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::{boot, guid, CStr16, Guid, Identify};

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

    // 1) Initramfs a memoria y registramos un handle con LoadFile2.
    let initrd_bytes = read_file_bytes(&mut fs_root, &initrd)?;
    log::info!("arje-loader :: initramfs leído, {} bytes", initrd_bytes.len());
    install_initrd_loadfile2(initrd_bytes)?;
    log::info!("arje-loader :: LoadFile2 LINUX_EFI_INITRD instalado");

    // 2) Kernel a memoria.
    let kernel_bytes = read_file_bytes(&mut fs_root, &linux)?;
    log::info!("arje-loader :: kernel leído, {} bytes", kernel_bytes.len());
    drop(fs_root);

    // 3) LoadImage del kernel desde el buffer.
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

    // 4) Cmdline. Cuando el initrd va por LoadFile2, no necesitamos
    //    `initrd=` en el cmdline. Lo dejamos en el cmdline sólo si las
    //    options del .conf no usan LoadFile2 (compat con bootloaders
    //    viejos) — el kernel preferirá LoadFile2 si lo encuentra.
    let cmdline = compose_cmdline(&initrd, &options);
    log::info!("arje-loader :: cmdline = {cmdline}");
    set_load_options(kernel_handle, &cmdline)?;

    log::info!("arje-loader :: start_image");
    boot::start_image(kernel_handle).map_err(|e| e.status())?;
    Ok(())
}

// =====================================================================
// LoadFile2 protocol + media-vendor device path con LINUX_EFI_INITRD_MEDIA_GUID
// =====================================================================
//
// El kernel Linux ≥ 5.10 busca handles que cumplan:
//   1. Soportan EFI_LOAD_FILE2_PROTOCOL (GUID 4006c0c1-...)
//   2. Tienen un device path con UN ÚNICO nodo de tipo MEDIA/VENDOR
//      cuya GUID interna sea LINUX_EFI_INITRD_MEDIA_GUID
//      (5568e427-68fc-4f3d-ac74-ca555231cc68).
//
// Cuando los encuentra, llama LoadFile dos veces:
//   - Primera: BufferSize=0, Buffer=NULL → debemos devolver el tamaño
//     real en BufferSize y EFI_BUFFER_TOO_SMALL.
//   - Segunda: con buffer alocado → copiamos los bytes y devolvemos SUCCESS.

const LINUX_EFI_INITRD_MEDIA_GUID: Guid = guid!("5568e427-68fc-4f3d-ac74-ca555231cc68");
const LOAD_FILE2_GUID: Guid = guid!("4006c0c1-fcb3-403e-996d-4a6c8724e06d");

/// vtable de EFI_LOAD_FILE2_PROTOCOL — un único método.
#[repr(C)]
struct LoadFile2Protocol {
    load_file: unsafe extern "efiapi" fn(
        this: *mut LoadFile2Protocol,
        file_path: *const c_void,
        boot_policy: u8,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> Status,
}

// Estado del initrd accesible desde el callback. El callback es `extern
// "efiapi"` (C ABI), no puede capturar — así que la única forma de
// pasarle los bytes es una static. El bytes se leakea (Box::leak)
// porque vive más allá del scope normal: el kernel lo lee después de
// que `start_image` transfiere control.
static mut INITRD_PTR: *const u8 = ptr::null();
static mut INITRD_LEN: usize = 0;

unsafe extern "efiapi" fn initrd_load_file(
    _this: *mut LoadFile2Protocol,
    _file_path: *const c_void,
    _boot_policy: u8,
    buffer_size: *mut usize,
    buffer: *mut c_void,
) -> Status {
    // SAFETY: el kernel UEFI EFISTUB pasa punteros válidos para ambos
    // parámetros (excepto buffer puede ser NULL en la primera llamada).
    let len = INITRD_LEN;
    if buffer.is_null() {
        *buffer_size = len;
        return Status::BUFFER_TOO_SMALL;
    }
    if *buffer_size < len {
        *buffer_size = len;
        return Status::BUFFER_TOO_SMALL;
    }
    ptr::copy_nonoverlapping(INITRD_PTR, buffer as *mut u8, len);
    *buffer_size = len;
    Status::SUCCESS
}

/// La vtable estática que pasamos a UEFI. Vive todo el programa.
static LOAD_FILE2_VTABLE: LoadFile2Protocol = LoadFile2Protocol {
    load_file: initrd_load_file,
};

/// Device path con un solo nodo MEDIA/VENDOR cuya GUID es
/// LINUX_EFI_INITRD_MEDIA_GUID, seguido del END node. 24 bytes
/// totales. Lo construimos en runtime (no puede ser const porque
/// `Guid::to_bytes` no es const en uefi-rs 0.35) y lo leakeamos.
fn build_initrd_device_path() -> &'static [u8] {
    // Layout:
    //   offset 0:  type=0x04 (MEDIA), subtype=0x03 (VENDOR), length=0x0014 (20)
    //   offset 4:  vendor_guid (16 bytes)
    //   offset 20: type=0x7f (END), subtype=0xff (END_ENTIRE), length=0x0004 (4)
    let mut v: Vec<u8> = alloc::vec![0u8; 24];
    v[0] = 0x04; // MEDIA
    v[1] = 0x03; // VENDOR_MEDIA
    v[2] = 20;
    v[3] = 0;
    v[4..20].copy_from_slice(LINUX_EFI_INITRD_MEDIA_GUID.to_bytes().as_ref());
    v[20] = 0x7f; // END
    v[21] = 0xff; // END_ENTIRE
    v[22] = 4;
    v[23] = 0;
    Box::leak(v.into_boxed_slice())
}

/// Crea un handle nuevo, le instala el DevicePath con el media-vendor
/// (GUID del initrd) y el LoadFile2 protocol. Los bytes del initrd se
/// leakean y quedan accesibles desde el callback static.
fn install_initrd_loadfile2(initrd_bytes: Vec<u8>) -> Result<(), Status> {
    // Leakeamos los bytes — el kernel los va a leer DESPUÉS de
    // start_image, así que el destructor no puede correr.
    let leaked: &'static [u8] = Box::leak(initrd_bytes.into_boxed_slice());
    // SAFETY: la static sólo se escribe acá, antes de que el callback
    // pueda invocarse (el kernel no corre hasta start_image).
    unsafe {
        INITRD_PTR = leaked.as_ptr();
        INITRD_LEN = leaked.len();
    }

    let dp_bytes = build_initrd_device_path();

    // SAFETY: el GUID y la interface satisfacen sus contratos UEFI:
    //   - DevicePath GUID + puntero a bytes válidos de un DevicePath bien-
    //     formado (vendor node + end node).
    //   - LoadFile2 GUID + puntero a una vtable estática válida.
    unsafe {
        let h = boot::install_protocol_interface(
            None,
            &DevicePath::GUID,
            dp_bytes.as_ptr() as *const c_void,
        )
        .map_err(|e| {
            log::error!("install DevicePath: {e:?}");
            e.status()
        })?;
        boot::install_protocol_interface(
            Some(h),
            &LOAD_FILE2_GUID,
            &LOAD_FILE2_VTABLE as *const _ as *const c_void,
        )
        .map_err(|e| {
            log::error!("install LoadFile2: {e:?}");
            e.status()
        })?;
    }
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
    // UEFI File::open exige separadores de path estilo Windows ('\'). Las
    // entries de systemd-boot usan '/' — convertimos in-place al pasar.
    let normalized: String = path.chars().map(|c| if c == '/' { '\\' } else { c }).collect();
    let mut buf = [0u16; 256];
    let cpath = CStr16::from_str_with_buf(&normalized, &mut buf)
        .map_err(|_| Status::INVALID_PARAMETER)?;
    let h = root
        .open(cpath, FileMode::Read, FileAttribute::empty())
        .map_err(|e| {
            log::error!("File::open({normalized}): {e:?}");
            e.status()
        })?;
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
