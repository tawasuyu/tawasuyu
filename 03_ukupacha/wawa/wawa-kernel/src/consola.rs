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

// =============================================================================
//  CAPAS — la descripcion de una recomposicion del escritorio (Fase 9)
// -----------------------------------------------------------------------------
//  Cuando hay ventanas flotantes, el escritorio no se pinta ventana a ventana:
//  el compositor entrega la lista de CAPAS —ordenada de atras hacia adelante— y
//  la consola las funde en ese orden de una sola pasada. El solapamiento de las
//  ventanas se resuelve solo, por el orden del pintado.
// =============================================================================

/// El contenido visible de una capa al recomponer el escritorio.
pub(crate) enum Contenido<'a> {
    /// La ventana aun no ha pintado: solo se ve su panel de reposo.
    Panel,
    /// El ultimo fotograma de la ventana — su lienzo natural crudo.
    Fotograma(&'a [u8]),
    /// La ventana fue desalojada: su baliza, un color plano.
    Baliza(Color),
}

/// Una capa del escritorio: una ventana, su marco y lo que muestra. El
/// compositor arma con ellas una lista ordenada de atras hacia adelante.
pub(crate) struct Capa<'a> {
    /// El marco donde la ventana vive en pantalla.
    pub(crate) marco: RegionPantalla,
    /// El tamaño natural del lienzo de la app — su fotograma mide esto.
    pub(crate) nat_ancho: usize,
    pub(crate) nat_alto: usize,
    /// Lo que la capa muestra.
    pub(crate) contenido: Contenido<'a>,
    /// ¿Tiene esta ventana el foco del compositor?
    pub(crate) enfocada: bool,
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
    /// sus limites ya verificados por el host— dentro del MARCO que el
    /// compositor asigno a su aplicacion. El fotograma mide el tamaño NATURAL
    /// de la app (`nat_ancho × nat_alto`); se CENTRA en el marco —que pudo
    /// hacerse mayor o menor que ese natural— y se recorta con firmeza a sus
    /// bordes. Una app jamas pinta un pixel fuera de su marco. NO traza borde
    /// ni presenta: de eso se encargan `volcar_marco` y `recomponer`.
    fn componer_fotograma(
        &mut self,
        marco: RegionPantalla,
        nat_ancho: usize,
        nat_alto: usize,
        datos: &[u8],
    ) {
        if nat_ancho == 0 || nat_alto == 0 {
            return;
        }
        // Centrar el fotograma natural dentro del marco. Si el natural excede
        // al marco, el desplazamiento queda en cero y el sobrante se recorta.
        let off_x = marco.x + marco.ancho.saturating_sub(nat_ancho) / 2;
        let off_y = marco.y + marco.alto.saturating_sub(nat_alto) / 2;
        let marco_x_fin = marco.x + marco.ancho;
        let marco_y_fin = marco.y + marco.alto;

        for (indice, trozo) in datos.chunks_exact(4).enumerate() {
            let columna = indice % nat_ancho;
            let fila = indice / nat_ancho;
            if fila >= nat_alto {
                break; // el fotograma excede su alto natural: se ignora el resto
            }
            let x = off_x + columna;
            let y = off_y + fila;
            // Recorte firme: al marco —el confinamiento de la app— y al lienzo.
            if x >= marco_x_fin || y >= marco_y_fin {
                continue;
            }
            if x >= self.lienzo.ancho || y >= self.lienzo.alto {
                continue;
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
    }

    /// Compone el fotograma de una app en su marco, le traza el borde de foco y
    /// presenta. El camino RAPIDO del compositor: sin ventanas flotantes
    /// ninguna ventana solapa a otra y basta repintar la que cambia.
    fn volcar_marco(
        &mut self,
        marco: RegionPantalla,
        nat_ancho: usize,
        nat_alto: usize,
        datos: &[u8],
        enfocada: bool,
    ) {
        self.componer_fotograma(marco, nat_ancho, nat_alto, datos);
        // El borde del compositor: delata, de un vistazo, quien tiene el foco.
        self.dibujar_borde(marco, enfocada);
        self.presentar();
    }

    /// Recompone el escritorio entero de una sola pasada (Fase 9). Inunda el
    /// area de apps con el reposo del lienzo y funde sobre ella cada capa, EN
    /// ORDEN —de atras hacia adelante—: asi el solapamiento de las ventanas
    /// flotantes se resuelve por si solo, sin recortes ni mascaras. Cada capa
    /// pinta primero su panel —el cromo de la ventana— y, encima, su contenido;
    /// una sola presentacion cierra la pasada.
    fn recomponer(&mut self, area: RegionPantalla, capas: &[Capa]) {
        self.lienzo.rellenar_rect(
            area.x,
            area.y,
            area.ancho,
            area.alto,
            Color::LIENZO_EN_REPOSO,
        );
        for capa in capas {
            let m = capa.marco;
            match &capa.contenido {
                Contenido::Panel => {
                    self.lienzo
                        .rellenar_rect(m.x, m.y, m.ancho, m.alto, Color::PANEL);
                }
                Contenido::Fotograma(datos) => {
                    // El panel primero —el cromo que rodea el lienzo— y el
                    // fotograma natural centrado encima.
                    self.lienzo
                        .rellenar_rect(m.x, m.y, m.ancho, m.alto, Color::PANEL);
                    self.componer_fotograma(m, capa.nat_ancho, capa.nat_alto, datos);
                }
                Contenido::Baliza(color) => {
                    self.lienzo.rellenar_rect(m.x, m.y, m.ancho, m.alto, *color);
                }
            }
            self.dibujar_borde(m, capa.enfocada);
        }
        self.presentar();
    }

    /// Inunda una region entera con un color plano —la baliza de desalojo: una
    /// app que falla tatua su marco de purpura— y le traza su borde de foco.
    fn pintar_region(&mut self, region: RegionPantalla, color: Color, enfocada: bool) {
        self.lienzo
            .rellenar_rect(region.x, region.y, region.ancho, region.alto, color);
        self.dibujar_borde(region, enfocada);
        self.presentar();
    }

    /// Traza un borde de 3 px alrededor de un marco: indigo brillante si la
    /// ventana tiene el foco del compositor, gris mate si no (Fase 8c).
    fn dibujar_borde(&mut self, marco: RegionPantalla, enfocada: bool) {
        const GROSOR: usize = 3;
        let color = if enfocada { Color::FOCO } else { Color::SIN_FOCO };
        // Lados superior e inferior.
        self.lienzo
            .rellenar_rect(marco.x, marco.y, marco.ancho, GROSOR, color);
        self.lienzo.rellenar_rect(
            marco.x,
            marco.y + marco.alto.saturating_sub(GROSOR),
            marco.ancho,
            GROSOR,
            color,
        );
        // Lados izquierdo y derecho.
        self.lienzo
            .rellenar_rect(marco.x, marco.y, GROSOR, marco.alto, color);
        self.lienzo.rellenar_rect(
            marco.x + marco.ancho.saturating_sub(GROSOR),
            marco.y,
            GROSOR,
            marco.alto,
            color,
        );
    }

    /// Vuelca el lienzo sobre la pantalla fisica.
    pub(crate) fn presentar(&mut self) {
        self.pantalla.presentar(&self.lienzo);
    }
}

/// La consola global de renaser. Se funde en el arranque; despues, las tareas
/// asincronas y las capacidades del userspace escriben en ella tras su `Mutex`.
pub(crate) static CONSOLA: Once<Mutex<Consola>> = Once::new();

/// Compone un fotograma del userspace —ya cacheado por el compositor— centrado
/// en su marco teselado, con su borde de foco. La invoca `compositor` al
/// recibir un `sys_render_frame` y al recomponer el escritorio tras un mando.
pub(crate) fn volcar_marco(
    marco: RegionPantalla,
    nat_ancho: usize,
    nat_alto: usize,
    datos: &[u8],
    enfocada: bool,
) {
    if let Some(consola) = CONSOLA.get() {
        consola
            .lock()
            .volcar_marco(marco, nat_ancho, nat_alto, datos, enfocada);
    }
}

/// Recompone el escritorio entero respetando el orden-Z de sus capas (Fase 9).
/// La invoca `compositor` al arrancar y siempre que hay ventanas flotantes: el
/// solapamiento obliga a repintar el escritorio en bloque, no ventana a
/// ventana. Las capas llegan ya ordenadas de atras hacia adelante.
pub(crate) fn recomponer(area: RegionPantalla, capas: &[Capa]) {
    if let Some(consola) = CONSOLA.get() {
        consola.lock().recomponer(area, capas);
    }
}

/// Tatua la baliza de desalojo sobre el marco de una aplicacion que el kernel
/// ha dado por terminada, con su borde de foco. El color delata la causa
/// —purpura para una falla de ejecucion o de combustible, amarillo palido para
/// un desbordo de memoria—. Es una advertencia NO fatal: la app muere, el
/// kernel y sus vecinas siguen vivos.
pub(crate) fn pintar_desalojo(marco: RegionPantalla, color: Color, enfocada: bool) {
    if let Some(consola) = CONSOLA.get() {
        consola.lock().pintar_region(marco, color, enfocada);
    }
}
