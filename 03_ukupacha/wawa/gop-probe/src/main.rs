// =============================================================================
//  gop-probe :: src/main.rs — ¿que salidas graficas expone el firmware?
// -----------------------------------------------------------------------------
//  Un .efi minimo que corre en contexto UEFI (antes de ExitBootServices),
//  enumera TODOS los handles del Graphics Output Protocol y, por cada uno,
//  reporta el modo actual y la maxima resolucion soportada. Es el unico modo
//  de averiguar —con dos monitores fisicos conectados— si el firmware presenta
//  varios framebuffers GOP (caso en que forkear `bootloader_api` para pasarlos
//  todos es viable) o uno solo (caso en que multi-monitor exige un driver GPU
//  propio). QEMU no puede responder esto: hace falta el hierro real.
//
//  DOBLE SALIDA: cada linea va a la consola UEFI (ConOut, para LEER EN PANTALLA
//  en metal — la maquina del autor no tiene COM1) y tambien a COM1 (0x3F8, para
//  capturar en QEMU con `-serial stdio` durante la validacion). En metal sin
//  puerto serie, la escritura a 0x3F8 cae al vacio sin efecto.
//
//  Uso en metal: copiar gop-probe.efi a la ESP (p.ej. \EFI\BOOT\BOOTX64.EFI) y
//  arrancarlo con los dos monitores enchufados. Anotar el numero de outputs y
//  la resolucion de cada uno; ese dato decide el camino de multi-monitor.
// =============================================================================
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use core::arch::asm;

use uefi::boot::{OpenProtocolAttributes, OpenProtocolParams};
use uefi::prelude::*;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::{boot, Identify};

/// Emite un byte por el puerto de datos de COM1 (0x3F8). En metal sin UART
/// fisico no tiene efecto observable; en QEMU lo recoge `-serial stdio`.
fn com1(byte: u8) {
    unsafe {
        asm!("out dx, al", in("dx") 0x3F8u16, in("al") byte, options(nomem, nostack, preserves_flags));
    }
}

/// Escribe una linea EN AMBOS canales: la consola UEFI (pantalla, para metal) y
/// COM1 (captura en QEMU). Añade CRLF al serial.
fn reportar(linea: &str) {
    log::info!("{linea}");
    for &b in linea.as_bytes() {
        com1(b);
    }
    com1(b'\r');
    com1(b'\n');
}

#[entry]
fn main() -> Status {
    uefi::helpers::init().expect("uefi::helpers::init fallo");
    reportar("gop-probe :: vivo — enumerando salidas graficas (GOP)");

    let handles = match boot::locate_handle_buffer(boot::SearchType::ByProtocol(
        &GraphicsOutput::GUID,
    )) {
        Ok(h) => h,
        Err(e) => {
            reportar(&format!("gop-probe :: locate_handle_buffer fallo: {e:?}"));
            boot::stall(15_000_000);
            return e.status();
        }
    };

    reportar(&format!(
        "gop-probe :: {} handle(s) GraphicsOutput encontrados",
        handles.len()
    ));

    for (i, &handle) in handles.iter().enumerate() {
        // GetProtocol: lectura NO intrusiva. El driver de consola del firmware
        // suele tener el GOP abierto BY_DRIVER; un open EXCLUSIVE le fallaria
        // (UNSUPPORTED/ACCESS_DENIED) y nos haria perder ese output. GetProtocol
        // solo toma prestado el puntero de interfaz sin registrarse como
        // consumidor — ideal para inspeccionar sin perturbar.
        let gop = match unsafe {
            boot::open_protocol::<GraphicsOutput>(
                OpenProtocolParams {
                    handle,
                    agent: boot::image_handle(),
                    controller: None,
                },
                OpenProtocolAttributes::GetProtocol,
            )
        } {
            Ok(g) => g,
            Err(e) => {
                reportar(&format!("  output #{i} :: open_protocol fallo: {e:?}"));
                continue;
            }
        };

        let info = gop.current_mode_info();
        let (cw, ch) = info.resolution();
        reportar(&format!(
            "  output #{i} :: modo actual {cw}x{ch}, formato {:?}, stride {}",
            info.pixel_format(),
            info.stride()
        ));

        // Recorrer todos los modos para reportar la maxima resolucion.
        let mut total = 0usize;
        let mut mejor = (0usize, 0usize);
        for modo in gop.modes() {
            total += 1;
            let (w, h) = modo.info().resolution();
            if w.saturating_mul(h) > mejor.0.saturating_mul(mejor.1) {
                mejor = (w, h);
            }
        }
        reportar(&format!(
            "  output #{i} :: {total} modos soportados, maximo {}x{}",
            mejor.0, mejor.1
        ));
    }

    reportar(
        "gop-probe :: FIN. >1 output => forkear bootloader_api es viable; \
         1 output => multi-monitor exige driver GPU. (30s para leer)",
    );
    boot::stall(30_000_000);
    Status::SUCCESS
}
