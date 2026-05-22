// =============================================================================
//  renaser :: kernel/src/baliza.rs — la red de seguridad visual del sistema
// -----------------------------------------------------------------------------
//  Sin consola de texto que valga cuando el sistema cae: si renaser colapsa, lo
//  DIBUJA. La baliza publica —de forma atomica y sin cerrojos— los datos
//  minimos del framebuffer, de modo que los manejadores de fallo puedan pintar
//  una franja de advertencia incluso cuando el resto del kernel ya no es fiable.
// =============================================================================

use core::panic::PanicInfo;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};

use crate::grafico::{escribir_pixel_volatil, Pantalla};

/// Datos del framebuffer expuestos a los manejadores de fallo. Todo son
/// atomicos: la baliza es intrinsecamente `Sync`, sin cerrojos.
pub(crate) struct BalizaPanico {
    base: AtomicPtr<u8>,
    paso_bytes: AtomicUsize,
    ancho: AtomicUsize,
    alto: AtomicUsize,
    bytes_por_pixel: AtomicUsize,
    /// Rojo de alerta de colapso, ya codificado al formato de la pantalla.
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

/// Si renaser colapsa, tatuamos una franja ROJA en lo alto del framebuffer.
#[panic_handler]
fn al_colapsar(_info: &PanicInfo) -> ! {
    x86_64::instructions::interrupts::disable();
    BALIZA_PANICO.pintar_banda(
        0,
        BALIZA_PANICO.franja(),
        BALIZA_PANICO.pixel_alerta.load(Ordering::Relaxed),
    );
    crate::detener()
}

/// Si el heap se agota, tatuamos una franja NARANJA: un fallo distinto al
/// colapso, y distinguible de un vistazo.
#[alloc_error_handler]
fn al_agotar_memoria(_disposicion: core::alloc::Layout) -> ! {
    x86_64::instructions::interrupts::disable();
    BALIZA_PANICO.pintar_banda(
        0,
        BALIZA_PANICO.franja(),
        BALIZA_PANICO.pixel_oom.load(Ordering::Relaxed),
    );
    crate::detener()
}
