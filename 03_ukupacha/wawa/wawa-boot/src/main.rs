// =============================================================================
//  renaser :: boot/src/main.rs — Fase 1.5 :: el puente hacia el silicio
// -----------------------------------------------------------------------------
//  Un kernel bare-metal no nace solo: alguien debe fusionarlo con un cargador,
//  sellarlo en una imagen de disco arrancable y entregarlo al hardware. Esa es
//  la unica mision de este orquestador de ANFITRION.
//
//  El flujo es deliberadamente lineal y sin ambiguedad:
//
//    1. Localizar el ELF nativo del kernel (lo inyecta la dep. de artefacto).
//    2. Fusionarlo con el cargador UEFI en una imagen de disco GPT.
//    3. Lanzar QEMU con esa imagen y el firmware OVMF.
//
//  Cada paso que pueda fallar lo hace en voz alta, con un mensaje accionable:
//  preferimos un error claro a un arranque silencioso hacia la nada.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;

/// Ruta del ELF del kernel, ya compilado para `x86_64-unknown-none`.
///
/// La dependencia de artefacto define esta variable de entorno en tiempo de
/// compilacion: cuando este binario de anfitrion existe, el kernel ya existe.
const KERNEL_ELF: &str = env!("CARGO_BIN_FILE_KERNEL_kernel");

/// Firmware UEFI OVMF tal como lo empaqueta Artix Linux (paquete `edk2-ovmf`).
/// Es la imagen combinada codigo+variables, apta para `-bios`.
const OVMF_POR_DEFECTO: &str = "/usr/share/edk2/x64/OVMF.4m.fd";

/// Nombre de la imagen de disco UEFI que renaser genera.
const NOMBRE_IMAGEN: &str = "renaser-uefi.img";

/// Ruta del disco de objetos del grafo persistente (Fase 6.1c). Relativa al
/// directorio de trabajo —la raiz del repo—, comun a `boot` y a QEMU.
const NOMBRE_DISCO: &str = "target/disk.img";

/// Tamaño del disco de objetos: 32 MiB. Se crea como fichero disperso.
const TAM_DISCO: u64 = 32 * 1024 * 1024;

fn main() {
    if let Err(fallo) = orquestar() {
        // Un error de orquestacion se anuncia en rojo y aborta con codigo 1:
        // ninguna falla del anfitrion debe disfrazarse de exito.
        eprintln!("\x1b[1;31m[renaser/boot] fallo:\x1b[0m {fallo}");
        std::process::exit(1);
    }
}

/// Ejecuta, en orden, las tres operaciones de la Fase 1.5.
fn orquestar() -> Result<(), String> {
    // --- 1. Localizar el artefacto del kernel. ---
    let kernel = Path::new(KERNEL_ELF);
    if !kernel.is_file() {
        return Err(format!(
            "no se encontro el ELF del kernel en {}\n  \
             (¿se interrumpio la compilacion de la dependencia de artefacto?)",
            kernel.display()
        ));
    }
    println!("[renaser/boot] kernel localizado :: {}", kernel.display());

    // --- 2. Fusionar kernel + cargador UEFI en una imagen de disco. ---
    let imagen = ruta_imagen(kernel);
    println!("[renaser/boot] forjando imagen UEFI :: {}", imagen.display());
    bootloader::UefiBoot::new(kernel)
        .create_disk_image(&imagen)
        .map_err(|e| format!("la crate `bootloader` no pudo crear la imagen UEFI: {e:?}"))?;

    // --- 3. Garantizar el disco de objetos del grafo persistente. ---
    preparar_disco_objetos()?;

    // --- 4. Lanzar QEMU sobre esa imagen. ---
    let ovmf = localizar_ovmf()?;
    lanzar_qemu(&imagen, &ovmf)
}

/// Garantiza la existencia del disco de objetos del grafo persistente. Si no
/// existe, lo forja como un fichero disperso de 32 MiB, ENTERAMENTE A CERO: el
/// kernel, al no hallar la firma de su superbloque, lo formateara como un grafo
/// virgen. Si ya existe, lo respeta — el grafo perdura entre arranques.
fn preparar_disco_objetos() -> Result<(), String> {
    let disco = Path::new(NOMBRE_DISCO);
    if disco.is_file() {
        println!("[renaser/boot] disco de objetos presente :: {}", disco.display());
        return Ok(());
    }
    if let Some(directorio) = disco.parent() {
        std::fs::create_dir_all(directorio)
            .map_err(|e| format!("no se pudo crear el directorio del disco de objetos: {e}"))?;
    }
    // Forjar el disco: un fichero disperso, a cero, de 32 MiB. El kernel
    // escribira su superbloque la primera vez que lo monte.
    let fichero = std::fs::File::create(disco)
        .map_err(|e| format!("no se pudo crear el disco de objetos «{}»: {e}", disco.display()))?;
    fichero
        .set_len(TAM_DISCO)
        .map_err(|e| format!("no se pudo dimensionar el disco de objetos: {e}"))?;
    println!(
        "[renaser/boot] disco de objetos forjado :: {} ({} MiB, virgen)",
        disco.display(),
        TAM_DISCO / (1024 * 1024)
    );
    Ok(())
}

/// Calcula la ruta de la imagen: junto al propio ELF del kernel, es decir,
/// dentro de `target/`. Una ubicacion predecible y siempre escribible.
fn ruta_imagen(kernel: &Path) -> PathBuf {
    kernel
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(NOMBRE_IMAGEN)
}

/// Resuelve la ruta del firmware OVMF. Permite sobreescribirla con la variable
/// de entorno `RENASER_OVMF` para entornos cuyo `edk2-ovmf` viva en otra ruta.
fn localizar_ovmf() -> Result<String, String> {
    let ruta = std::env::var("RENASER_OVMF").unwrap_or_else(|_| OVMF_POR_DEFECTO.to_string());
    if Path::new(&ruta).is_file() {
        Ok(ruta)
    } else {
        Err(format!(
            "firmware UEFI OVMF no encontrado en «{ruta}»\n  \
             instala el paquete `edk2-ovmf`, o exporta RENASER_OVMF=<ruta a OVMF.fd>"
        ))
    }
}

/// Invoca QEMU como subproceso. Los argumentos se ciñen a las primitivas
/// minimas necesarias para que el Framebuffer GOP cobre vida:
///
///   * `-bios`            firmware UEFI OVMF.
///   * `-drive raw`       la imagen de disco UEFI, sin capa de traduccion.
///   * `-vga std`         VGA estandar => framebuffer lineal que el GOP expone.
///   * `-serial stdio`    telemetria serial del procesador hacia esta consola.
///   * `--no-reboot`      un fallo triple detiene la maquina en vez de reiniciar
///                        en bucle: asi la baliza de panico permanece visible.
///   * `virtio-blk-pci`   el disco de objetos, sobre el bus PCI (q35 es x86_64;
///                        `virtio-blk-device`, su gemelo MMIO, es cosa de ARM).
fn lanzar_qemu(imagen: &Path, ovmf: &str) -> Result<(), String> {
    println!("[renaser/boot] arrancando QEMU :: la superficie indigo nace ahora\n");

    let mut qemu = Command::new("qemu-system-x86_64");
    // `accel=kvm:tcg` intenta KVM y, si no esta disponible, recae en TCG puro.
    qemu.arg("-machine").arg("q35,accel=kvm:tcg")
        .arg("-m").arg("256M")
        .arg("-bios").arg(ovmf)
        .arg("-drive").arg(format!("format=raw,file={}", imagen.display()))
        .arg("-vga").arg("std")
        .arg("-serial").arg("stdio")
        .arg("--no-reboot")
        // El disco de objetos, como dispositivo virtio-blk sobre el bus PCI.
        .arg("-drive").arg(format!("format=raw,file={NOMBRE_DISCO},if=none,id=drv0"))
        .arg("-device").arg("virtio-blk-pci,drive=drv0");

    // Cualquier argumento extra tras `--` se reenvia a QEMU intacto.
    // Ejemplo: `cargo run -p boot -- -display none -d int`.
    qemu.args(std::env::args().skip(1));

    match qemu.status() {
        Ok(estado) if estado.success() => {
            println!("\n[renaser/boot] QEMU finalizo limpiamente.");
            Ok(())
        }
        Ok(estado) => Err(format!("QEMU termino con estado anomalo: {estado}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "`qemu-system-x86_64` no esta en el PATH; instala el paquete `qemu-full`".to_string(),
        ),
        Err(e) => Err(format!("no se pudo ejecutar QEMU: {e}")),
    }
}
