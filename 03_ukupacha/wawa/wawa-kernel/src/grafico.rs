// =============================================================================
//  renaser :: kernel/src/grafico.rs — el sustrato grafico del sistema
// -----------------------------------------------------------------------------
//  En renaser el texto es un caso particular del dibujo, y el dibujo descansa
//  sobre este modulo: el color, el framebuffer fisico (`Pantalla`) y el lienzo
//  intermedio en RAM (`Lienzo`) que sostiene la tecnica de doble bufer.
// =============================================================================

use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::{OriginDimensions, Size};
use embedded_graphics::pixelcolor::{Rgb888, RgbColor};
use embedded_graphics::Pixel;

/// Ancho maximo de lienzo soportado (Full HD).
pub(crate) const ANCHO_MAX: usize = 1920;
/// Alto maximo de lienzo soportado (Full HD).
pub(crate) const ALTO_MAX: usize = 1080;
/// Capacidad del lienzo intermedio, en pixeles de 32 bits.
const PIXELES_MAX: usize = ANCHO_MAX * ALTO_MAX;

// =============================================================================
//  COLOR — la unidad indivisible del lenguaje visual de renaser
// =============================================================================

/// Color de 24 bits, independiente del format fisico del framebuffer.
#[derive(Clone, Copy)]
pub(crate) struct Color {
    pub(crate) r: u8,
    pub(crate) g: u8,
    pub(crate) b: u8,
}

impl Color {
    /// Reposo del lienzo: un indigo casi negro, sereno y persistente.
    pub(crate) const LIENZO_EN_REPOSO: Color = Color {
        r: 0x12,
        g: 0x16,
        b: 0x20,
    };

    /// Panel del compositor (Fase 8): un slate apenas mas claro que el reposo.
    /// Tiñe cada marco teselado, de modo que el teselado se vea —como una
    /// rejilla de paneles— aunque sus apps aun no hayan pintado nada.
    pub(crate) const PANEL: Color = Color {
        r: 0x1B,
        g: 0x21,
        b: 0x30,
    };

    /// Borde de la ventana ENFOCADA (Fase 8c): un indigo brillante. Delata, de
    /// un vistazo, a quien recibe el teclado.
    pub(crate) const FOCO: Color = Color {
        r: 0x4B,
        g: 0x00,
        b: 0x82,
    };

    /// Borde de una ventana sin foco (Fase 8c): un gris mate, discreto — marca
    /// el marco sin reclamar la atencion.
    pub(crate) const SIN_FOCO: Color = Color {
        r: 0x3A,
        g: 0x40,
        b: 0x4E,
    };

    /// Alerta de colapso: un rojo saturado, imposible de ignorar.
    pub(crate) const ALERTA: Color = Color {
        r: 0xD4,
        g: 0x1E,
        b: 0x2C,
    };

    /// Agotamiento de memoria (OOM): un naranja de advertencia.
    pub(crate) const OOM: Color = Color {
        r: 0xFF,
        g: 0xA5,
        b: 0x00,
    };

    /// Tinta del texto: un blanco suave, legible sobre el indigo.
    pub(crate) const TEXTO: Color = Color {
        r: 0xE8,
        g: 0xEC,
        b: 0xF4,
    };

    /// Desalojo de una aplicacion: un purpura inequivoco. Distinto del rojo de
    /// colapso del kernel y del naranja de OOM — porque esto NO es un colapso:
    /// es el kernel conteniendo a un inquilino discolo y siguiendo vivo.
    pub(crate) const DESALOJO: Color = Color {
        r: 0x8B,
        g: 0x5C,
        b: 0xF6,
    };

    /// Desalojo por desbordo de memoria: un amarillo palido. Distingue al
    /// inquilino que revento su techo ESPACIAL del que agoto su tiempo (purpura).
    pub(crate) const DESALOJO_MEMORIA: Color = Color {
        r: 0xFF,
        g: 0xFF,
        b: 0xE0,
    };

    /// FASE 58 v6 :: resalte de los caracteres que matchean la query del
    /// launcher (Spotlight-classic). Un ambar dorado que contrasta tanto sobre
    /// `PANEL` (filas no seleccionadas) como sobre `FOCO` (fila seleccionada),
    /// y no choca con la semantica de ninguno de los otros colores de baliza.
    pub(crate) const RESALTE_BUSQUEDA: Color = Color {
        r: 0xF4,
        g: 0xC4,
        b: 0x30,
    };

    /// Carmesi profundo: la baliza ULTIMA del kernel — fallo de inicializacion
    /// no recuperable (Fase 25). Se invoca cuando un superbloque que dice
    /// estar firmado no supera la verificacion criptografica, o cuando un
    /// invariante de la cadena de arranque cae. NO es un desalojo de app —
    /// el kernel entero abdica de seguir presentando un escritorio que ya
    /// no puede atestiguar como legitimo—. La pantalla queda en este color
    /// y la traza completa va al puerto serial para diagnostico offline.
    pub(crate) const FATAL_CARMESI: Color = Color {
        r: 0x8B,
        g: 0x00,
        b: 0x00,
    };
}

// =============================================================================
//  REGION — la sub-superficie que el kernel asigna a cada aplicacion
// =============================================================================

/// Una sub-region rectangular de la pantalla, en pixeles. El kernel asigna una
/// a cada aplicacion del userspace: es, a la vez, su ventana al mundo y su
/// confinamiento — una app jamas pinta un pixel fuera de la suya.
#[derive(Clone, Copy)]
pub(crate) struct RegionPantalla {
    /// Desplazamiento horizontal de la esquina superior izquierda.
    pub(crate) x: usize,
    /// Desplazamiento vertical de la esquina superior izquierda.
    pub(crate) y: usize,
    /// Ancho de la region, en pixeles.
    pub(crate) ancho: usize,
    /// Alto de la region, en pixeles.
    pub(crate) alto: usize,
}

/// Traduce un [`Color`] logico al valor nativo de 32 bits que el framebuffer
/// espera, respetando el orden de canales que reporta el firmware UEFI.
pub(crate) fn codificar(format: PixelFormat, color: Color) -> u32 {
    let (r, g, b) = (color.r as u32, color.g as u32, color.b as u32);
    match format {
        PixelFormat::Rgb => r | (g << 8) | (b << 16),
        PixelFormat::Bgr => b | (g << 8) | (r << 16),
        PixelFormat::U8 => (r * 54 + g * 183 + b * 19) >> 8,
        PixelFormat::Unknown {
            red_position,
            green_position,
            blue_position,
        } => (r << red_position) | (g << green_position) | (b << blue_position),
        _ => r | (g << 8) | (b << 16),
    }
}

// =============================================================================
//  ESCRITURA VOLATIL — la unica celula que toca memoria de video real
// =============================================================================

/// Deposita un pixel ya codificado en una direccion del framebuffer fisico.
/// Las escrituras son **volatiles**: el optimizador no puede elidirlas.
///
/// # Seguridad
///
/// `destino` debe apuntar a memoria de video valida y escribible para
/// `bytes_por_pixel` bytes, correctamente alineada para el ancho de escritura.
#[inline]
pub(crate) unsafe fn escribir_pixel_volatil(destino: *mut u8, valor: u32, bytes_por_pixel: usize) {
    match bytes_por_pixel {
        4 => unsafe { ptr::write_volatile(destino.cast::<u32>(), valor) },
        3 => unsafe {
            ptr::write_volatile(destino, valor as u8);
            ptr::write_volatile(destino.add(1), (valor >> 8) as u8);
            ptr::write_volatile(destino.add(2), (valor >> 16) as u8);
        },
        2 => unsafe { ptr::write_volatile(destino.cast::<u16>(), valor as u16) },
        _ => unsafe { ptr::write_volatile(destino, valor as u8) },
    }
}

// =============================================================================
//  LIENZO INTERMEDIO — el corazon del doble bufer
// =============================================================================

/// Respaldo estatico del lienzo, alineado a pagina. Vive en `.bss`.
#[repr(align(4096))]
struct LienzoEstatico(UnsafeCell<[u32; PIXELES_MAX]>);

// SEGURIDAD: el acceso se serializa mediante `LIENZO_ENTREGADO`, que garantiza
// un unico prestamo mutable durante toda la vida del sistema.
unsafe impl Sync for LienzoEstatico {}

/// Memoria de respaldo del lienzo intermedio.
static MEMORIA_LIENZO: LienzoEstatico = LienzoEstatico(UnsafeCell::new([0u32; PIXELES_MAX]));

/// Centinela de entrega unica del lienzo.
static LIENZO_ENTREGADO: AtomicBool = AtomicBool::new(false);

/// Entrega — exactamente una vez — el prestamo mutable de la memoria del lienzo.
pub(crate) fn reclamar_memoria_lienzo() -> Option<&'static mut [u32]> {
    if LIENZO_ENTREGADO.swap(true, Ordering::AcqRel) {
        return None;
    }
    // SEGURIDAD: el `swap` anterior garantiza que este es el unico prestamo
    // mutable de `MEMORIA_LIENZO` durante toda la ejecucion.
    let arreglo: &'static mut [u32; PIXELES_MAX] = unsafe { &mut *MEMORIA_LIENZO.0.get() };
    Some(arreglo.as_mut_slice())
}

/// Superficie de dibujo en RAM. Cada pixel se almacena ya codificado.
pub(crate) struct Lienzo {
    pub(crate) pixeles: &'static mut [u32],
    pub(crate) ancho: usize,
    pub(crate) alto: usize,
    pub(crate) format: PixelFormat,
}

impl Lienzo {
    /// Construye un lienzo sobre la memoria de respaldo reclamada.
    pub(crate) fn nuevo(
        memoria: &'static mut [u32],
        ancho: usize,
        alto: usize,
        format: PixelFormat,
    ) -> Lienzo {
        Lienzo {
            pixeles: memoria,
            ancho,
            alto,
            format,
        }
    }

    /// Pinta un unico pixel. Las coordenadas fuera del lienzo se ignoran.
    pub(crate) fn pintar_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.ancho && y < self.alto {
            self.pixeles[y * self.ancho + x] = codificar(self.format, color);
        }
    }

    /// Rellena un rectangulo, recortado con firmeza a los limites del lienzo.
    /// Una fila completa se rellena con `slice::fill`, que LLVM materializa
    /// como un `memset` SIMD —cientos de pixeles por instruccion donde antes
    /// iba un bucle de asignaciones escalares—.
    pub(crate) fn rellenar_rect(
        &mut self,
        x0: usize,
        y0: usize,
        ancho: usize,
        alto: usize,
        color: Color,
    ) {
        let valor = codificar(self.format, color);
        let x_ini = x0.min(self.ancho);
        let y_ini = y0.min(self.alto);
        let x_fin = (x0 + ancho).min(self.ancho);
        let y_fin = (y0 + alto).min(self.alto);
        if x_fin <= x_ini || y_fin <= y_ini {
            return;
        }
        let mut y = y_ini;
        while y < y_fin {
            let base = y * self.ancho;
            self.pixeles[base + x_ini..base + x_fin].fill(valor);
            y += 1;
        }
    }

    /// Inunda el lienzo entero con un color plano. Una sola pasada `fill`
    /// sobre el slice util del lienzo —no toca el respaldo no usado—.
    pub(crate) fn limpiar(&mut self, color: Color) {
        let valor = codificar(self.format, color);
        let usados = self.ancho.saturating_mul(self.alto).min(self.pixeles.len());
        self.pixeles[..usados].fill(valor);
    }
}

// `embedded-graphics` ve el lienzo como un `DrawTarget`: sus primitivas
// vectoriales pueden dibujar directamente sobre el.
impl DrawTarget for Lienzo {
    type Color = Rgb888;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixeles: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(punto, color) in pixeles {
            if let (Ok(x), Ok(y)) = (usize::try_from(punto.x), usize::try_from(punto.y)) {
                self.pintar_pixel(
                    x,
                    y,
                    Color {
                        r: color.r(),
                        g: color.g(),
                        b: color.b(),
                    },
                );
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Lienzo {
    fn size(&self) -> Size {
        Size::new(self.ancho as u32, self.alto as u32)
    }
}

// =============================================================================
//  PANTALLA — el framebuffer fisico GOP, envuelto en seguridad
// =============================================================================

/// Vista segura del framebuffer lineal entregado por el firmware UEFI.
pub(crate) struct Pantalla {
    pub(crate) base: *mut u8,
    pub(crate) ancho: usize,
    pub(crate) alto: usize,
    pub(crate) paso_bytes: usize,
    pub(crate) bytes_por_pixel: usize,
    /// Formato de pixel — necesario para estampar capas que se pintan
    /// DIRECTAMENTE sobre el framebuffer, no sobre el lienzo (Fase 13).
    pub(crate) format: PixelFormat,
}

impl Pantalla {
    /// FASE 60 :: adopta un framebuffer ARBITRARIO ya mapeado — el caso del
    /// scanout que el kernel posee via virtio-gpu—. El format es B8G8R8A8
    /// (BGRA, 4 bpp): los bytes salen B,G,R,A, que es justo lo que codifica
    /// `PixelFormat::Bgr` (la A queda en 0, ignorada por el scanout). A
    /// diferencia del GOP, escribir aqui NO presenta: hace falta un
    /// `gpu::presentar()` que cruce la frontera hacia el anfitrion.
    pub(crate) fn sobre_framebuffer(
        base: *mut u8,
        ancho: usize,
        alto: usize,
        paso_bytes: usize,
    ) -> Pantalla {
        Pantalla {
            base,
            ancho,
            alto,
            paso_bytes,
            bytes_por_pixel: 4,
            format: PixelFormat::Bgr,
        }
    }

    /// Adopta el framebuffer descrito por `info`. La memoria de video es
    /// permanente, asi que conservar su puntero crudo es legitimo.
    pub(crate) fn adoptar(framebuffer: &mut FrameBuffer, info: FrameBufferInfo) -> Pantalla {
        let base = framebuffer.buffer_mut().as_mut_ptr();
        Pantalla {
            base,
            ancho: info.width,
            alto: info.height,
            paso_bytes: info.stride * info.bytes_per_pixel,
            bytes_por_pixel: info.bytes_per_pixel,
            format: info.pixel_format,
        }
    }

    /// Vuelca el lienzo intermedio sobre la pantalla fisica de un solo gesto.
    /// El camino RAPIDO (32 bpp, el universal en GOP UEFI moderno) hace una
    /// `copy_nonoverlapping` POR FILA: el lienzo ya esta codificado al format
    /// del framebuffer, asi que blittear es memcpy puro. En 1920×1080, esto
    /// pasa de ~2M write_volatile escalares a ~1080 memcpys de 7680 bytes —el
    /// optimizador lo materializa con instrucciones `rep movsq` o equivalentes
    /// SIMD—. Para FB de 1/2/3 bpp se recae al bucle volatil pixel a pixel.
    pub(crate) fn presentar(&mut self, lienzo: &Lienzo) {
        let region = RegionPantalla {
            x: 0,
            y: 0,
            ancho: self.ancho.min(lienzo.ancho),
            alto: self.alto.min(lienzo.alto),
        };
        self.presentar_region(lienzo, region);
    }

    /// Vuelca una SUB-REGION del lienzo al framebuffer. Es el corazon del
    /// camino rapido del compositor: cuando solo un marco cambio, basta
    /// blittear ese marco —no la pantalla entera—. La region se recorta tanto
    /// al lienzo como a la pantalla fisica antes de tocar memoria.
    pub(crate) fn presentar_region(&mut self, lienzo: &Lienzo, region: RegionPantalla) {
        // Recorte: x_fin / y_fin a las dos superficies.
        let x_ini = region.x.min(self.ancho).min(lienzo.ancho);
        let y_ini = region.y.min(self.alto).min(lienzo.alto);
        let x_fin = region
            .x
            .saturating_add(region.ancho)
            .min(self.ancho)
            .min(lienzo.ancho);
        let y_fin = region
            .y
            .saturating_add(region.alto)
            .min(self.alto)
            .min(lienzo.alto);
        if x_fin <= x_ini || y_fin <= y_ini {
            return;
        }
        let ancho = x_fin - x_ini;

        if self.bytes_por_pixel == 4 {
            let bytes_por_fila = ancho * 4;
            for y in y_ini..y_fin {
                let fila_lienzo = y * lienzo.ancho + x_ini;
                let fila_fisica = y * self.paso_bytes + x_ini * 4;
                // SEGURIDAD: `fila_lienzo + ancho` cae dentro de `lienzo.pixeles`
                // (recorte garantiza x_fin <= lienzo.ancho), y `fila_fisica +
                // bytes_por_fila` dentro del framebuffer. Memoria de video WC
                // del firmware UEFI; memcpy es la operacion canonica de blit y
                // LLVM no la elide (el *mut u8 cruza frontera FFI del cargador).
                unsafe {
                    let src = lienzo.pixeles.as_ptr().add(fila_lienzo) as *const u8;
                    let dst = self.base.add(fila_fisica);
                    ptr::copy_nonoverlapping(src, dst, bytes_por_fila);
                }
            }
        } else {
            for y in y_ini..y_fin {
                let fila_fisica = y * self.paso_bytes;
                let fila_lienzo = y * lienzo.ancho;
                for x in x_ini..x_fin {
                    let pixel = lienzo.pixeles[fila_lienzo + x];
                    unsafe {
                        let destino = self.base.add(fila_fisica + x * self.bytes_por_pixel);
                        escribir_pixel_volatil(destino, pixel, self.bytes_por_pixel);
                    }
                }
            }
        }
    }
}

// =============================================================================
//  EL PUNTERO DEL RATON — un sprite estampado sobre el framebuffer (Fase 13)
// -----------------------------------------------------------------------------
//  El puntero NO vive en el lienzo: el lienzo es el escritorio limpio, y se
//  recompone con frecuencia. El puntero es una capa de PRESENTACION que cada
//  volcado vuelve a sellar sobre el framebuffer, despues de copiar el lienzo.
//  Asi no hay save-under que mantener: el lienzo HACE de save-under, y el
//  framebuffer recibe el puntero como ultimo gesto.
// =============================================================================

/// Ancho del sprite del puntero, en pixeles.
const PUNTERO_ANCHO: usize = 12;
/// El sprite del puntero — una flecha noroeste. `#` es el borde oscuro, `*` el
/// relleno claro, `.` transparente.
const PUNTERO: [&[u8; PUNTERO_ANCHO]; 18] = [
    b"#...........",
    b"##..........",
    b"#*#.........",
    b"#**#........",
    b"#***#.......",
    b"#****#......",
    b"#*****#.....",
    b"#******#....",
    b"#*******#...",
    b"#********#..",
    b"#*********#.",
    b"#*****#####.",
    b"#**#**#.....",
    b"#*#.#**#....",
    b"##..#**#....",
    b"#....#**#...",
    b".....#**#...",
    b"......###...",
];

/// Lado del recurso de cursor por hardware de virtio-gpu: un cuadrado fijo de
/// 64×64 px (lo exige `CURSOR_RECT` de la crate). El sprite del puntero —12×18—
/// se ancla en su esquina noroeste; el resto queda transparente.
pub(crate) const CURSOR_HW_LADO: usize = 64;

/// Renderiza el sprite del puntero (`PUNTERO`) en un buffer 64×64 en formato
/// B8G8R8A8 (el unico que el recurso de cursor de virtio-gpu admite), listo para
/// `gpu::instalar_cursor`. Asi el cursor por HARDWARE comparte la misma flecha
/// que el estampado por software, sin duplicar el dibujo. Pixeles `#` van al
/// borde oscuro, `*` al relleno claro, el resto transparente (alpha 0). El
/// vertice de la flecha (0,0) es el punto caliente. Bytes en orden B,G,R,A —
/// la A=0 marca transparencia, A=0xFF opacidad.
pub(crate) fn cursor_bgra_64() -> alloc::vec::Vec<u8> {
    const BORDE: [u8; 4] = [0x18, 0x12, 0x10, 0xFF];
    const RELLENO: [u8; 4] = [0xF8, 0xF2, 0xF0, 0xFF];
    let mut imagen = alloc::vec![0u8; CURSOR_HW_LADO * CURSOR_HW_LADO * 4];
    for (fila, linea) in PUNTERO.iter().enumerate() {
        for (col, &celda) in linea.iter().enumerate() {
            let bgra = match celda {
                b'#' => BORDE,
                b'*' => RELLENO,
                _ => continue,
            };
            let base = (fila * CURSOR_HW_LADO + col) * 4;
            imagen[base..base + 4].copy_from_slice(&bgra);
        }
    }
    imagen
}

impl Pantalla {
    /// Estampa el sprite del puntero del raton sobre el framebuffer, con su
    /// vertice en (x, y). El sprite se recorta con firmeza a los limites de la
    /// pantalla. NO altera el lienzo: la proxima recomposicion lo deja intacto;
    /// el siguiente volcado lo vuelve a estampar (Fase 13).
    pub(crate) fn estampar_puntero(&mut self, x: usize, y: usize) {
        let borde = codificar(
            self.format,
            Color {
                r: 0x10,
                g: 0x12,
                b: 0x18,
            },
        );
        let relleno = codificar(
            self.format,
            Color {
                r: 0xF0,
                g: 0xF2,
                b: 0xF8,
            },
        );
        for (fila, linea) in PUNTERO.iter().enumerate() {
            let py = y + fila;
            if py >= self.alto {
                break;
            }
            for (col, &celda) in linea.iter().enumerate() {
                let px = x + col;
                if px >= self.ancho {
                    continue;
                }
                let valor = match celda {
                    b'#' => borde,
                    b'*' => relleno,
                    _ => continue,
                };
                // SEGURIDAD: (px, py) acotado a las dimensiones reales del
                // framebuffer; el desplazamiento cae dentro de la memoria de
                // video que el firmware nos entrego.
                unsafe {
                    let destino = self
                        .base
                        .add(py * self.paso_bytes + px * self.bytes_por_pixel);
                    escribir_pixel_volatil(destino, valor, self.bytes_por_pixel);
                }
            }
        }
    }
}
