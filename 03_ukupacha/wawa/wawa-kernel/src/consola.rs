// =============================================================================
//  renaser :: kernel/src/consola.rs — la superficie de texto e imagen
// -----------------------------------------------------------------------------
//  La consola une el lienzo intermedio, la pantalla fisica y una pluma de
//  escritura. Rasteriza cada glifo con `fontdue` al vuelo —el texto convertido,
//  por fin, en dibujo— y tambien sabe volcar fotogramas crudos del userspace
//  WASM. Es global y serializada tras un `Mutex`: las tareas escriben en ella.
// =============================================================================

use spin::{Mutex, Once};

use crate::grafico::{codificar, Color, Lienzo, Pantalla, RegionPantalla};
use crate::texto;

/// Margen del texto respecto al borde del lienzo, en pixeles.
const MARGEN: usize = 40;
/// Tamaño de la tipografia, en pixeles.
const TAM_FUENTE: f32 = 30.0;
/// Altura de avance de una linea de texto, en pixeles.
const ALTO_LINEA: usize = 40;
/// Posicion vertical de la primera linea base.
const BASE_INICIAL: usize = MARGEN + 30;

/// Interpola dos colores segun una cobertura: `0` => `fondo`, `255` => `tinta`.
fn mezclar(fondo: Color, tinta: Color, cobertura: u8) -> Color {
    let canal = |a: u8, b: u8| -> u8 {
        let c = cobertura as u16;
        ((a as u16 * (255 - c) + b as u16 * c) / 255) as u8
    };
    Color {
        r: canal(fondo.r, tinta.r),
        g: canal(fondo.g, tinta.g),
        b: canal(fondo.b, tinta.b),
    }
}

/// La consola grafica de renaser: doble bufer, pantalla fisica y pluma.
pub(crate) struct Consola {
    lienzo: Lienzo,
    pantalla: Pantalla,
    /// Posicion horizontal de la pluma de escritura.
    pluma_x: usize,
    /// Linea base vertical de la pluma de escritura.
    base_y: usize,
}

// SEGURIDAD: `Consola` encierra, via `Pantalla`, un puntero crudo al
// framebuffer. Ese puntero es valido durante toda la vida del kernel y todo
// acceso a la consola se serializa tras un `Mutex`. En un sistema de un solo
// nucleo, esto la hace segura de compartir entre el hilo principal y las tareas.
unsafe impl Send for Consola {}

impl Consola {
    /// Crea una consola con la pluma en la esquina superior izquierda.
    pub(crate) fn nueva(lienzo: Lienzo, pantalla: Pantalla) -> Consola {
        Consola {
            lienzo,
            pantalla,
            pluma_x: MARGEN,
            base_y: BASE_INICIAL,
        }
    }

    /// Lleva la pluma al inicio de la siguiente linea. Al llegar al fondo,
    /// limpia el lienzo: una pizarra nueva.
    fn nueva_linea(&mut self) {
        self.pluma_x = MARGEN;
        self.base_y += ALTO_LINEA;
        if self.base_y + ALTO_LINEA >= self.lienzo.alto {
            self.lienzo.limpiar(Color::LIENZO_EN_REPOSO);
            self.base_y = BASE_INICIAL;
        }
    }

    /// Escribe un caracter: rasteriza su glifo y avanza la pluma.
    fn escribir_char(&mut self, caracter: char) {
        if caracter == '\n' {
            self.nueva_linea();
            return;
        }
        let (metricas, cobertura) = texto::rasterizar(caracter, TAM_FUENTE);
        // Salto de linea automatico al alcanzar el margen derecho.
        if self.pluma_x + metricas.advance_width as usize + MARGEN > self.lienzo.ancho {
            self.nueva_linea();
        }
        self.dibujar_glifo(&metricas, &cobertura);
        self.pluma_x += metricas.advance_width as usize;
    }

    /// Escribe una cadena completa, caracter a caracter.
    pub(crate) fn escribir(&mut self, texto: &str) {
        for caracter in texto.chars() {
            self.escribir_char(caracter);
        }
    }

    /// Funde un mapa de cobertura de `fontdue` sobre el lienzo, en la pluma.
    fn dibujar_glifo(&mut self, metricas: &fontdue::Metrics, cobertura: &[u8]) {
        // Origen del glifo: la pluma desplazada por las metricas. El mapa de
        // `fontdue` se recorre de arriba a abajo desde la cima del glifo.
        let inicio_x = self.pluma_x as isize + metricas.xmin as isize;
        let inicio_y = self.base_y as isize - metricas.ymin as isize - metricas.height as isize;

        for fila in 0..metricas.height {
            for col in 0..metricas.width {
                let opacidad = cobertura[fila * metricas.width + col];
                if opacidad == 0 {
                    continue; // pixel transparente: no toca el fondo
                }
                let x = inicio_x + col as isize;
                let y = inicio_y + fila as isize;
                if x < 0 || y < 0 {
                    continue;
                }
                let color = mezclar(Color::LIENZO_EN_REPOSO, Color::TEXTO, opacidad);
                self.lienzo.pintar_pixel(x as usize, y as usize, color);
            }
        }
    }

    /// Compone un fotograma crudo del userspace WASM —pixeles `0x00RRGGBB`, con
    /// sus limites ya verificados por el host— sobre la SUB-REGION asignada a su
    /// aplicacion. Cada pixel se recodifica al formato nativo del framebuffer y
    /// se deposita desplazado por `(region.x, region.y)`: una app jamas escribe
    /// fuera de su ventana, y varias cohabitan el lienzo sin pisarse.
    fn volcar_marco(&mut self, region: RegionPantalla, datos: &[u8]) {
        for (indice, trozo) in datos.chunks_exact(4).enumerate() {
            let columna = indice % region.ancho;
            let fila = indice / region.ancho;
            if fila >= region.alto {
                break; // el fotograma excede el alto de la region: se ignora el resto
            }
            let x = region.x + columna;
            let y = region.y + fila;
            if x >= self.lienzo.ancho || y >= self.lienzo.alto {
                continue; // recorte firme: nada se pinta fuera del lienzo
            }
            let p = u32::from_le_bytes([trozo[0], trozo[1], trozo[2], trozo[3]]);
            let color = Color {
                r: (p >> 16) as u8,
                g: (p >> 8) as u8,
                b: p as u8,
            };
            self.lienzo.pixeles[y * self.lienzo.ancho + x] =
                codificar(self.lienzo.formato, color);
        }
        self.presentar();
    }

    /// Inunda una region entera con un color plano y la presenta. Es la baliza
    /// de desalojo: cuando una aplicacion falla, su ventana se tatua de purpura.
    fn pintar_region(&mut self, region: RegionPantalla, color: Color) {
        self.lienzo
            .rellenar_rect(region.x, region.y, region.ancho, region.alto, color);
        self.presentar();
    }

    /// Vuelca el lienzo sobre la pantalla fisica.
    pub(crate) fn presentar(&mut self) {
        self.pantalla.presentar(&self.lienzo);
    }
}

/// La consola global de renaser. Se funde en el arranque; despues, las tareas
/// asincronas y las capacidades del userspace escriben en ella tras su `Mutex`.
pub(crate) static CONSOLA: Once<Mutex<Consola>> = Once::new();

/// Puerta del kernel para la capacidad `sys_render_frame` del userspace WASM:
/// compone sobre la consola global un fotograma —cuyos limites el host ya
/// verifico matematicamente contra la memoria lineal del modulo— dentro de la
/// region de pantalla que el kernel asigno a esa aplicacion.
pub(crate) fn volcar_marco_wasm(region: RegionPantalla, datos: &[u8]) {
    if let Some(consola) = CONSOLA.get() {
        consola.lock().volcar_marco(region, datos);
    }
}

/// Tatua la baliza de desalojo sobre la region de una aplicacion que el kernel
/// ha dado por terminada. El color delata la causa —purpura para una falla de
/// ejecucion o de combustible, amarillo palido para un desbordo de memoria—. Es
/// una advertencia NO fatal: la app muere, el kernel y sus vecinas siguen vivos.
pub(crate) fn pintar_desalojo(region: RegionPantalla, color: Color) {
    if let Some(consola) = CONSOLA.get() {
        consola.lock().pintar_region(region, color);
    }
}
