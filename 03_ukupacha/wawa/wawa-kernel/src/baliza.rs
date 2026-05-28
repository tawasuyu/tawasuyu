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
    /// Carmesi profundo de fallo ULTIMO (Fase 26), ya codificado: corrupcion
    /// del superbloque o del manifiesto detectada en el arranque, antes de
    /// que el escritorio compositor haya nacido. Tiñe la pantalla ENTERA.
    pixel_carmesi: AtomicU32,
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
            pixel_carmesi: AtomicU32::new(0),
        }
    }

    /// Enciende la baliza con los datos de una pantalla viva.
    pub(crate) fn encender(
        &self,
        pantalla: &Pantalla,
        pixel_alerta: u32,
        pixel_oom: u32,
        pixel_carmesi: u32,
    ) {
        self.paso_bytes.store(pantalla.paso_bytes, Ordering::Relaxed);
        self.ancho.store(pantalla.ancho, Ordering::Relaxed);
        self.alto.store(pantalla.alto, Ordering::Relaxed);
        self.bytes_por_pixel
            .store(pantalla.bytes_por_pixel, Ordering::Relaxed);
        self.pixel_alerta.store(pixel_alerta, Ordering::Relaxed);
        self.pixel_oom.store(pixel_oom, Ordering::Relaxed);
        self.pixel_carmesi.store(pixel_carmesi, Ordering::Relaxed);
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

    /// Pinta el FRAMEBUFFER ENTERO con un pixel solido. Es la herramienta del
    /// `aborto_fatal_carmesi`: cuando el arranque colapsa antes de que el
    /// escritorio haya nacido, el unico testimonio visual posible es tiñir la
    /// pantalla de un color que el operador reconozca de un vistazo. No pasa
    /// por el lienzo, ni por la consola, ni por el compositor: escribe
    /// directamente sobre el framebuffer fisico, byte a byte, volatilmente.
    fn pintar_completo(&self, pixel: u32) {
        let alto = self.alto.load(Ordering::Relaxed);
        self.pintar_banda(0, alto, pixel);
    }

    /// Escribe un caracter ASCII en (x, y) sobre el framebuffer fisico, con
    /// un pixel `tinta` por bit encendido del glifo. Escala fija de 2x2
    /// pixeles por celda de la matriz 5x7 — un texto legible sobre cualquier
    /// resolucion sin depender del rasterizador vectorial (`fontdue`) ni del
    /// heap. Solo se intenta si (x + ancho_caracter*2) cabe en pantalla.
    fn dibujar_caracter_volatil(&self, x: usize, y: usize, c: u8, tinta: u32, escala: usize) {
        let base = self.base.load(Ordering::Acquire);
        if base.is_null() {
            return;
        }
        let ancho = self.ancho.load(Ordering::Relaxed);
        let alto = self.alto.load(Ordering::Relaxed);
        let paso = self.paso_bytes.load(Ordering::Relaxed);
        let bpp = self.bytes_por_pixel.load(Ordering::Relaxed);
        let glifo = glifo_ascii(c);
        for fila in 0..7 {
            let bits = glifo[fila];
            for col in 0..5 {
                if bits & (1 << (4 - col)) == 0 {
                    continue;
                }
                // Bloque de `escala`x`escala` pixeles para este bit.
                for dy in 0..escala {
                    let py = y + fila * escala + dy;
                    if py >= alto {
                        continue;
                    }
                    for dx in 0..escala {
                        let px = x + col * escala + dx;
                        if px >= ancho {
                            continue;
                        }
                        // SEGURIDAD: (px, py) acotado por las dimensiones de
                        // la pantalla viva ya publicada.
                        unsafe {
                            escribir_pixel_volatil(
                                base.add(py * paso + px * bpp),
                                tinta,
                                bpp,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Escribe una cadena ASCII en (x, y). Cada caracter avanza 6 bloques de
    /// `escala` (5 + 1 de separacion). Cortada al borde de la pantalla.
    fn dibujar_texto_volatil(&self, x: usize, y: usize, texto: &[u8], tinta: u32, escala: usize) {
        let avance = 6 * escala;
        for (i, &c) in texto.iter().enumerate() {
            self.dibujar_caracter_volatil(x + i * avance, y, c, tinta, escala);
        }
    }
}

// =============================================================================
//  FUENTE ASCII 5x7 — solo lo que el carmesi necesita
// -----------------------------------------------------------------------------
//  Una matriz minima de glifos para que la traza del aborto sea legible en
//  pantalla. Los 5 bits bajos de cada byte representan los pixeles de una
//  fila, el bit alto a la izquierda. Cubre A-Z, 0-9, ' ', ':', '.', '_', '-'
//  y '/'. Caracteres fuera de esta lista se rotulan como un bloque solido —
//  pista visible de que el aborto ocurrio antes de saber componer texto.
// =============================================================================
fn glifo_ascii(c: u8) -> [u8; 7] {
    match c {
        b' ' => [0; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b'/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        b'A' => [0x0E, 0x11, 0x11, 0x11, 0x1F, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1E, 0x09, 0x09, 0x09, 0x09, 0x09, 0x1E],
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        b'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        b'a'..=b'z' => glifo_ascii(c - b'a' + b'A'),
        _ => [0x1F; 7],
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
    // FASE 60 :: si el scanout lo gobierna virtio-gpu, la franja recien tiñida
    // vive en memoria del huesped; volcarla para que se VEA, no solo en serie.
    crate::drivers::gpu::presentar_baliza();
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
    // FASE 60 :: volcar la franja al anfitrion si el kernel gobierna la GPU.
    crate::drivers::gpu::presentar_baliza();
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

// =============================================================================
//  ABORTO FATAL CARMESI — Fase 26
// -----------------------------------------------------------------------------
//  Cuando el arranque detecta corrupcion en una estructura de disco que decia
//  ser de Wawa —el superbloque con magia correcta pero version, log o
//  manifiesto no descifrables— el kernel ABDICA. NO reformatea: hacerlo
//  arrasaria datos del operador que un comando de reparacion offline podria
//  rescatar. NO sigue arrancando: ponerse a servir un escritorio sobre datos
//  cuya integridad no podemos atestiguar es traicionar la promesa del SO.
//
//  En su lugar, pinta la pantalla ENTERA de CARMESI PROFUNDO, rotula la
//  causa en la franja superior con fuente ASCII estatica, vuelca la traza
//  larga por COM1 y entra en bucle HLT. El operador apaga la maquina y
//  acude al disco con una herramienta offline; el kernel cumplio su deber
//  delatando, no tapando, la herida.
// =============================================================================

/// Aborta el arranque con la BALIZA CARMESI. Disable interrupts, tiñe el
/// framebuffer, rotula la traza corta en pantalla, escribe la larga por
/// serie, y queda en `hlt` para siempre. Diseñada para invocarse desde
/// `almacen::init` y otros caminos del arranque donde no hay forma legitima
/// de continuar.
///
/// `traza_corta` se rotula en pantalla (mayusculas, espacios y digitos
/// solamente —el resto cae a bloques solidos, una pista visible de que la
/// cadena tenia caracteres ajenos a la fuente embebida—). `traza_serial`
/// va sin recorte al puerto COM1 para diagnostico offline.
pub fn aborto_fatal_carmesi(traza_corta: &[u8], traza_serial: &str) -> ! {
    // Las interrupciones se silencian PRIMERO: ningun IRQ ha de venir a
    // re-encender el reactor o a corromper aun mas el estado del disco.
    x86_64::instructions::interrupts::disable();

    // Tiñir la pantalla entera. Si la baliza nunca se encendio —arranque
    // muy temprano— pintar_completo es un no-op silencioso; aun asi
    // continuamos: la traza serial llega de todas formas.
    let pixel = BALIZA_PANICO.pixel_carmesi.load(Ordering::Relaxed);
    BALIZA_PANICO.pintar_completo(pixel);

    // Rotular la traza corta en blanco sobre el carmesi, en la franja
    // superior. Centro horizontal aproximado; si el texto es mas largo
    // que el ancho disponible, se recorta al borde.
    let ancho_pantalla = BALIZA_PANICO.ancho.load(Ordering::Relaxed);
    let bpp = BALIZA_PANICO.bytes_por_pixel.load(Ordering::Relaxed);
    // Blanco encodeado al format del framebuffer: en RGB y BGR, todos los
    // bits a 1 en los tres canales bajos. Para U8 (luminancia), saturar.
    let blanco: u32 = match bpp {
        1 => 0xFF,
        _ => 0x00FF_FFFF,
    };
    let escala = 3usize;
    let avance = 6 * escala;
    let largo = traza_corta.len().saturating_mul(avance);
    let x = (ancho_pantalla.saturating_sub(largo)) / 2;
    let y = 40usize;
    BALIZA_PANICO.dibujar_texto_volatil(x, y, traza_corta, blanco, escala);

    // FASE 60 :: el carmesi entero y su rotulo ya estan tiñidos en el
    // framebuffer del huesped; un unico `flush` los cruza al scanout para que
    // el operador los vea. `try_lock` adentro: jamas cuelga si la GPU estaba
    // ocupada al colapsar.
    crate::drivers::gpu::presentar_baliza();

    // Traza larga por COM1. Encabezado obligatorio para que un grep
    // ofline distinga este aborto de cualquier otro panic.
    let _ = writeln!(Serie);
    let _ = writeln!(Serie, "*** wawa :: ABORTO FATAL CARMESI ***");
    let _ = writeln!(Serie, "  traza :: {traza_serial}");
    let _ = writeln!(Serie, "  causa :: corrupcion detectada al arrancar");
    let _ = writeln!(
        Serie,
        "  accion :: usuario apaga la maquina; herramienta offline diagnostica"
    );

    crate::detener()
}
