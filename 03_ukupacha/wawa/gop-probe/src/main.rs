// =============================================================================
//  gop-probe :: src/main.rs — ¿que salidas graficas expone el firmware?
// -----------------------------------------------------------------------------
//  Un .efi minimo que corre en contexto UEFI (antes de ExitBootServices),
//  enumera TODOS los handles del Graphics Output Protocol y, por cada uno,
//  reporta el modo actual, la maxima resolucion y la DIRECCION BASE de su
//  framebuffer. Es el unico modo de averiguar —con dos monitores fisicos
//  conectados— si el firmware presenta varios framebuffers GOP (caso en que
//  forkear `bootloader_api` para pasarlos todos es viable) o uno solo (caso en
//  que multi-monitor exige un driver GPU propio). QEMU no puede responder esto:
//  hace falta el hierro real.
//
//  PRUEBA VISUAL (lo decisivo): tras reportar el texto, PINTA cada salida con un
//  COLOR distinto y (i+1) barras blancas centradas. En metal con dos monitores:
//    * si el SEGUNDO monitor se enciende con su color y sus barras, el firmware
//      lo expone como un framebuffer direccionable aparte —forkear el cargador
//      es viable y acabamos de probar que podemos dibujarlo—;
//    * si queda negro, esa salida no es direccionable por separado —multi-monitor
//      exige driver GPU—.
//  El color/barra evita depender de leer texto diminuto en el segundo monitor: se
//  ve de un vistazo y se CUENTA cual salida es cual.
//
//  DOBLE SALIDA de texto: cada linea va a la consola UEFI (ConOut, para LEER EN
//  PANTALLA en metal — la maquina del autor no tiene COM1) y tambien a COM1
//  (0x3F8, para capturar en QEMU con `-serial stdio`). En metal sin puerto serie,
//  la escritura a 0x3F8 cae al vacio sin efecto.
//
//  Uso en metal: copiar gop-probe.efi a la ESP (p.ej. \EFI\BOOT\BOOTX64.EFI) y
//  arrancarlo con los dos monitores enchufados. Leer el conteo y la resolucion en
//  el monitor primario; luego mirar si el segundo se pinta.
// =============================================================================
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use core::arch::asm;

use uefi::boot::{OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol};
use uefi::prelude::*;
use uefi::proto::console::gop::{BltOp, BltPixel, GraphicsOutput};
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

/// Abre un handle GraphicsOutput con `GetProtocol` —lectura/uso NO intrusivo—.
/// El driver de consola del firmware suele tener el GOP abierto BY_DRIVER; un
/// open EXCLUSIVE le fallaria y nos haria perder ese output. `GetProtocol` solo
/// toma prestado el puntero de interfaz sin registrarse como consumidor; basta
/// para inspeccionar Y para dibujar via `blt` (que entra al firmware, no muta
/// la struct). Devuelve `None` si el open falla.
fn abrir_gop(handle: Handle) -> Option<ScopedProtocol<GraphicsOutput>> {
    unsafe {
        boot::open_protocol::<GraphicsOutput>(
            OpenProtocolParams {
                handle,
                agent: boot::image_handle(),
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )
        .ok()
    }
}

/// El color con que se pinta la salida `i`. Cuatro colores saturados y bien
/// distintos entre si; a partir del quinto output se reciclan (improbable).
fn color_salida(i: usize) -> BltPixel {
    match i % 4 {
        0 => BltPixel::new(0, 40, 200),   // azul    = salida 0
        1 => BltPixel::new(0, 170, 40),   // verde   = salida 1
        2 => BltPixel::new(210, 30, 0),   // rojo    = salida 2
        _ => BltPixel::new(180, 0, 180),  // magenta = salida 3
    }
}

/// Pinta una salida: rellena toda la pantalla con su color y dibuja (i+1) barras
/// blancas verticales centradas, para CONTAR de un vistazo cual salida es. Usa
/// `blt`/`VideoFill`, agnostico al formato de pixel (sirve hasta en modo BltOnly,
/// sin framebuffer lineal). Errores de blt se ignoran: si una salida no admite
/// blt, simplemente no se pinta —dato en si mismo—.
fn pintar(gop: &mut ScopedProtocol<GraphicsOutput>, i: usize) {
    let (cw, ch) = gop.current_mode_info().resolution();
    if cw == 0 || ch == 0 {
        return;
    }
    let _ = gop.blt(BltOp::VideoFill {
        color: color_salida(i),
        dest: (0, 0),
        dims: (cw, ch),
    });

    // (i+1) barras blancas centradas, ocupando el tercio central en alto.
    let barras = i + 1;
    let ancho_barra = (cw / 40).max(8);
    let hueco = ancho_barra;
    let total = barras * ancho_barra + barras.saturating_sub(1) * hueco;
    let x0 = cw.saturating_sub(total) / 2;
    let alto = ch / 3;
    let y0 = (ch - alto) / 2;
    for b in 0..barras {
        let x = x0 + b * (ancho_barra + hueco);
        if x + ancho_barra > cw {
            break;
        }
        let _ = gop.blt(BltOp::VideoFill {
            color: BltPixel::new(255, 255, 255),
            dest: (x, y0),
            dims: (ancho_barra, alto),
        });
    }
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

    // --- FASE 1 :: TEXTO. Reportar modo, resolucion maxima y BASE del
    //               framebuffer de cada salida. Se lee en el monitor primario. ---
    for (i, &handle) in handles.iter().enumerate() {
        let mut gop = match abrir_gop(handle) {
            Some(g) => g,
            None => {
                reportar(&format!("  output #{i} :: open_protocol fallo"));
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

        // BASE y tamaño del framebuffer lineal — el dato que el fork del
        // cargador necesita para saber si las salidas viven en memoria aparte.
        let mut fb = gop.frame_buffer();
        reportar(&format!(
            "  output #{i} :: framebuffer base {:#x}, {} bytes",
            fb.as_mut_ptr() as usize,
            fb.size()
        ));
    }

    reportar("gop-probe :: (texto leido) 12s y empieza la PRUEBA VISUAL...");
    boot::stall(12_000_000);

    // --- FASE 2 :: VISUAL. Pintar cada salida con su color + barras. El
    //               monitor primario pierde el texto (ya se leyo); lo decisivo
    //               es si el SEGUNDO monitor se enciende con su color. ---
    for (i, &handle) in handles.iter().enumerate() {
        if let Some(mut gop) = abrir_gop(handle) {
            pintar(&mut gop, i);
        }
    }

    reportar(
        "gop-probe :: PINTADO. Mira el SEGUNDO monitor: color+barras => \
         direccionable (forkear cargador viable). Negro => exige driver GPU. 40s.",
    );
    boot::stall(40_000_000);
    Status::SUCCESS
}
