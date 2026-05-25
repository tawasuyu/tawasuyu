// =============================================================================
//  renaser :: kernel/src/baliza.rs — la red de seguridad visual del sistema
// -----------------------------------------------------------------------------
//  Sin consola de texto que valga cuando el sistema cae: si renaser colapsa, lo
//  DIBUJA. La baliza publica —de forma atomica y sin cerrojos— los datos
//  minimos del framebuffer, de modo que los manejadores de fallo puedan pintar
//  una franja de advertencia incluso cuando el resto del kernel ya no es fiable.
// =============================================================================

use core::fmt::Write;
use core::panic::PanicInfo;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};

use crate::grafico::{escribir_pixel_volatil, Pantalla};

// =============================================================================
//  TESTIMONIO POR EL PUERTO SERIE — para diagnosticar colapsos sin pantalla
// -----------------------------------------------------------------------------
//  En pantalla solo cabe la franja roja: un grito breve, sin matiz. Pero los
//  manejadores de fallo escriben TAMBIEN al puerto serie COM1 —que QEMU enruta
//  a la terminal de `cargo run` con `-serial stdio`—. Asi, cuando el kernel
//  colapsa fuera del Proxmox del autor, deja una pista legible de la causa.
// =============================================================================

/// Puerto de datos de COM1.
const SERIE_DATOS: u16 = 0x3F8;
/// Registro de estado de linea de COM1 — bit 5: el transmisor esta libre.
const SERIE_LSR: u16 = 0x3FD;

/// Envia un byte por COM1, con una espera acotada por si el firmware no nos lo
/// dejo configurado. Si se agota la paciencia, calla — antes mudo que cuelgue.
fn serie_escribir(byte: u8) {
    for _ in 0..1_000_000 {
        // SEGURIDAD: 0x3FD es el registro de estado de linea de COM1, fijo en
        // la arquitectura PC; leerlo es inocuo.
        let lsr = unsafe { x86_64::instructions::port::Port::<u8>::new(SERIE_LSR).read() };
        if lsr & 0x20 != 0 {
            // SEGURIDAD: 0x3F8 es el puerto de datos de COM1.
            unsafe { x86_64::instructions::port::Port::<u8>::new(SERIE_DATOS).write(byte) };
            return;
        }
    }
}

/// Sumidero de impresion al puerto serie — formatea sin tocar el heap. Publico
/// para que cualquier modulo del kernel pueda dejar trazas en COM1 con un
/// simple `writeln!(crate::baliza::Serie, "...", ...)`.
pub(crate) struct Serie;

impl Write for Serie {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            serie_escribir(b);
        }
        Ok(())
    }
}

/// Datos del framebuffer expuestos a los manejadores de fallo. Todo son
/// atomicos: la baliza es intrinsecamente `Sync`, sin cerrojos.
pub(crate) struct BalizaPanico {
    base: AtomicPtr<u8>,
    paso_bytes: AtomicUsize,
    ancho: AtomicUsize,
    alto: AtomicUsize,
    bytes_por_pixel: AtomicUsize,
    /// Rojo de alerta de colapso, ya codificado al format de la pantalla.
    pixel_alerta: AtomicU32,
    /// Naranja de agotamiento de memoria, ya codificado.
    pixel_oom: AtomicU32,
}

impl BalizaPanico {
    /// Baliza apagada: sin pantalla publicada todavia.
    const fn apagada() -> BalizaPanico {
        BalizaPanico {
            base: AtomicPtr::new(ptr::null_mut()),
            paso_bytes: AtomicUsize::new(0),
            ancho: AtomicUsize::new(0),
            alto: AtomicUsize::new(0),
            bytes_por_pixel: AtomicUsize::new(0),
            pixel_alerta: AtomicU32::new(0),
            pixel_oom: AtomicU32::new(0),
        }
    }

    /// Enciende la baliza con los datos de una pantalla viva.
    pub(crate) fn encender(&self, pantalla: &Pantalla, pixel_alerta: u32, pixel_oom: u32) {
        self.paso_bytes.store(pantalla.paso_bytes, Ordering::Relaxed);
        self.ancho.store(pantalla.ancho, Ordering::Relaxed);
        self.alto.store(pantalla.alto, Ordering::Relaxed);
        self.bytes_por_pixel
            .store(pantalla.bytes_por_pixel, Ordering::Relaxed);
        self.pixel_alerta.store(pixel_alerta, Ordering::Relaxed);
        self.pixel_oom.store(pixel_oom, Ordering::Relaxed);
        // La base se publica de ultima, con semantica `Release`.
        self.base.store(pantalla.base, Ordering::Release);
    }

    /// Pinta, directa y volatilmente, una banda horizontal sobre el framebuffer
    /// fisico. Es la herramienta de los manejadores de fallo: no confia ni en
    /// el lienzo, ni en el heap, ni en estructura dinamica alguna.
    fn pintar_banda(&self, y0: usize, altura: usize, pixel: u32) {
        let base = self.base.load(Ordering::Acquire);
        if base.is_null() {
            return;
        }
        let ancho = self.ancho.load(Ordering::Relaxed);
        let alto = self.alto.load(Ordering::Relaxed);
        let paso = self.paso_bytes.load(Ordering::Relaxed);
        let bpp = self.bytes_por_pixel.load(Ordering::Relaxed);

        let y_fin = (y0 + altura).min(alto);
        let mut y = y0.min(alto);
        while y < y_fin {
            let fila = y * paso;
            let mut x = 0;
            while x < ancho {
                // SEGURIDAD: (x, y) esta acotado por las dimensiones que la
                // baliza publico desde una pantalla real.
                unsafe {
                    escribir_pixel_volatil(base.add(fila + x * bpp), pixel, bpp);
                }
                x += 1;
            }
            y += 1;
        }
    }

    /// Altura de la franja de advertencia: ~8 % de la pantalla.
    fn franja(&self) -> usize {
        let alto = self.alto.load(Ordering::Relaxed);
        if alto == 0 {
            0
        } else {
            (alto / 12).max(1)
        }
    }
}

/// Instancia global de la baliza. Comienza apagada y se enciende en el arranque.
pub(crate) static BALIZA_PANICO: BalizaPanico = BalizaPanico::apagada();

// =============================================================================
//  MANEJADORES DE FALLO — cuando el sistema colapsa, lo DIBUJA
// =============================================================================

/// Si renaser colapsa, tatuamos una franja ROJA en lo alto del framebuffer Y
/// dejamos por COM1 un testimonio del panico: su mensaje, su lugar — la pista
/// que en pantalla no cabe.
#[panic_handler]
fn al_colapsar(info: &PanicInfo) -> ! {
    x86_64::instructions::interrupts::disable();
    BALIZA_PANICO.pintar_banda(
        0,
        BALIZA_PANICO.franja(),
        BALIZA_PANICO.pixel_alerta.load(Ordering::Relaxed),
    );
    let _ = writeln!(Serie);
    let _ = writeln!(Serie, "*** renaser :: panico ***");
    if let Some(lugar) = info.location() {
        let _ = writeln!(
            Serie,
            "  en {}:{}:{}",
            lugar.file(),
            lugar.line(),
            lugar.column()
        );
    }
    let _ = writeln!(Serie, "  {}", info.message());
    crate::detener()
}

/// Si el heap se agota, tatuamos una franja NARANJA y dejamos en el serie la
/// disposicion que reviento el techo: tamaño y alineamiento.
#[alloc_error_handler]
fn al_agotar_memoria(disposicion: core::alloc::Layout) -> ! {
    x86_64::instructions::interrupts::disable();
    BALIZA_PANICO.pintar_banda(
        0,
        BALIZA_PANICO.franja(),
        BALIZA_PANICO.pixel_oom.load(Ordering::Relaxed),
    );
    let _ = writeln!(Serie);
    let _ = writeln!(Serie, "*** renaser :: agotamiento de memoria ***");
    let _ = writeln!(
        Serie,
        "  layout: tamaño={} alineamiento={}",
        disposicion.size(),
        disposicion.align()
    );
    crate::detener()
}
