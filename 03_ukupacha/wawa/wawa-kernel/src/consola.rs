// =============================================================================
//  renaser :: kernel/src/consola.rs — la superficie de texto e imagen
// -----------------------------------------------------------------------------
//  La consola une el lienzo intermedio, la pantalla fisica y una pluma de
//  escritura. Rasteriza cada glifo con `fontdue` al vuelo —el texto convertido,
//  por fin, en dibujo— y tambien sabe volcar fotogramas crudos del userspace
//  WASM. Es global y serializada tras un `Mutex`: las tareas escriben en ella.
// =============================================================================

use bootloader_api::info::PixelFormat;
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

/// El contenido visible de una capa. SIN LIFETIMES: una capa describe el
/// contenido por INDICE de ventana, no por referencia a sus bytes. Los bytes
/// los resuelve un `Resolver` que el compositor entrega a la consola. Esto
/// permite que el compositor mantenga buffers de capas pre-alocados y los
/// reuse sin alocar en cada recomposicion (Fase 25).
pub(crate) enum ContenidoSlot {
    /// La ventana aun no ha pintado: solo se ve su panel de reposo.
    Panel,
    /// El ultimo fotograma de la ventana — su lienzo natural crudo. El
    /// `usize` es el indice de la ventana; el `Resolver` entrega los bytes.
    Fotograma(usize),
    /// La ventana fue desalojada: su baliza, un color plano.
    Baliza(Color),
}

/// Una capa del escritorio: una ventana, su marco y lo que muestra. SIN
/// LIFETIMES — un slot puro: cabe en un Vec pre-alocado que se reusa con
/// `clear() + push()` sin tocar al asignador, fotograma tras fotograma.
pub(crate) struct CapaSlot {
    /// El marco donde la ventana vive en pantalla.
    pub(crate) marco: RegionPantalla,
    /// El tamaño natural del lienzo de la app — su fotograma mide esto.
    pub(crate) nat_ancho: usize,
    pub(crate) nat_alto: usize,
    /// Lo que la capa muestra.
    pub(crate) contenido: ContenidoSlot,
    /// ¿Tiene esta ventana el foco del compositor?
    pub(crate) enfocada: bool,
}

/// Una pestaña de la barra de tareas (Fase 14). SIN LIFETIMES — el nombre
/// se resuelve por indice de ventana via `Resolver`.
pub(crate) struct CeldaTaskbarSlot {
    pub(crate) region: RegionPantalla,
    /// Indice de la ventana cuyo nombre rotular en la pestaña.
    pub(crate) ventana: usize,
    /// Color de fondo: indigo del foco, slate del panel, o color de baliza.
    pub(crate) fondo: Color,
    /// Color de la tinta del texto.
    pub(crate) tinta: Color,
}

/// La barra de tareas del escritorio (Fase 14). Solo viaja con dos referencias:
/// el slice de celdas (apuntando al buffer pre-alocado del escritorio) y el
/// texto del reloj (apuntando a un buffer de PILA del propio recomponedor).
pub(crate) struct TaskbarSlot<'a> {
    pub(crate) area: RegionPantalla,
    pub(crate) launcher: RegionPantalla,
    pub(crate) celdas: &'a [CeldaTaskbarSlot],
    pub(crate) reloj: &'a str,
    pub(crate) reloj_region: RegionPantalla,
}

/// FASE 58 :: el overlay del launcher grafico. Pinta una caja centrada con
/// la lista de apps lanzables y resalta la seleccion vigente. La consola lo
/// recibe como ultima capa de la recomposicion —sobre la taskbar—, de modo
/// que aparezca por encima de todo lo demas. Vive como un slot sin lifetime
/// "propio" mas alla del fotograma: las cadenas vienen del `Vec<String>` del
/// escritorio, que el compositor sostiene mientras el lock este tomado.
pub(crate) struct LauncherOverlay<'a> {
    /// Region centrada que ocupa el overlay en pantalla.
    pub(crate) region: RegionPantalla,
    /// Nombres de las apps en el orden de la plantilla.
    pub(crate) items: &'a [alloc::string::String],
    /// Indice de la fila seleccionada — el operador la lanza con Enter.
    pub(crate) seleccion: usize,
}

/// Resolver de datos por indice de ventana. La consola lo invoca para
/// obtener los bytes del fotograma cacheado y el nombre de la pestaña; el
/// compositor implementa el rasgo con una vista sobre `escritorio.ventanas`.
/// El acoplamiento entre consola y compositor pasa a ser este rasgo, no las
/// estructuras concretas — la consola sigue ignorando que es una Ventana.
pub(crate) trait Resolver {
    /// Bytes del ultimo fotograma cacheado de la ventana `indice`.
    fn cache(&self, indice: usize) -> &[u8];
    /// Nombre legible de la ventana `indice` — el del manifiesto.
    fn nombre(&self, indice: usize) -> &str;
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
    ///
    /// El recorte se PRE-CALCULA por fila (no por pixel): conociendo `off_x`,
    /// `off_y`, `marco`, y el ancho/alto del lienzo, se deriva una sola vez
    /// cuantas filas y columnas validas hay. Despues:
    ///   * camino RAPIDO (BGR): el byte order del fotograma WASM coincide con
    ///     el encoding del lienzo — una `copy_nonoverlapping` por fila, sin
    ///     decodificar pixel a pixel.
    ///   * camino general (RGB/U8/Unknown): se decodifica cada pixel, pero
    ///     sin los `if x >= …` por iteracion —ya estamos dentro del recorte—.
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

        // Recorte: cuantas filas y columnas reales caben. El minimo entre el
        // natural, el marco y el propio lienzo —tres techos a respetar—.
        let lienzo_ancho = self.lienzo.ancho;
        let lienzo_alto = self.lienzo.alto;
        let filas = nat_alto
            .min(marco_y_fin.saturating_sub(off_y))
            .min(lienzo_alto.saturating_sub(off_y));
        let cols = nat_ancho
            .min(marco_x_fin.saturating_sub(off_x))
            .min(lienzo_ancho.saturating_sub(off_x));
        // Tambien recortado al volumen real de bytes que la app entrego: una
        // app puede haber cacheado un fotograma corto en su primer `init`.
        let filas_datos = datos.len() / (nat_ancho * 4);
        let filas = filas.min(filas_datos);
        if filas == 0 || cols == 0 {
            return;
        }

        // Camino RAPIDO: el fotograma WASM `0x00RRGGBB` en LE tiene bytes
        // `[B, G, R, 0]`; un FB BGR codifica `b | (g << 8) | (r << 16)`, cuyos
        // bytes en LE son tambien `[B, G, R, 0]`. Copia byte-a-byte de fila.
        if matches!(self.lienzo.format, PixelFormat::Bgr) {
            let bytes_por_fila = cols * 4;
            for fila in 0..filas {
                let src_inicio = fila * nat_ancho * 4;
                let dst_base = (off_y + fila) * lienzo_ancho + off_x;
                // SEGURIDAD: src_inicio + bytes_por_fila <= datos.len()
                // (cols <= nat_ancho y filas <= filas_datos); dst_base + cols
                // <= lienzo.pixeles.len() (off_y + filas <= lienzo_alto y
                // off_x + cols <= lienzo_ancho).
                unsafe {
                    let src = datos.as_ptr().add(src_inicio);
                    let dst = self.lienzo.pixeles.as_mut_ptr().add(dst_base) as *mut u8;
                    core::ptr::copy_nonoverlapping(src, dst, bytes_por_fila);
                }
            }
            return;
        }

        // Camino general: decodificar (R,G,B) y recodificar al format del FB.
        // Sin chequeos de limites por pixel — el recorte ya los garantizo.
        let format = self.lienzo.format;
        for fila in 0..filas {
            let src_inicio = fila * nat_ancho * 4;
            let dst_base = (off_y + fila) * lienzo_ancho + off_x;
            for col in 0..cols {
                let idx = src_inicio + col * 4;
                let b = datos[idx];
                let g = datos[idx + 1];
                let r = datos[idx + 2];
                self.lienzo.pixeles[dst_base + col] =
                    codificar(format, Color { r, g, b });
            }
        }
    }

    /// Compone el fotograma de una app en su marco, le traza el borde de foco y
    /// presenta SOLO esa region — el resto del lienzo no cambio. El camino
    /// RAPIDO del compositor: sin ventanas flotantes ninguna ventana solapa a
    /// otra y basta blittear el marco. Con apps pintando a 100 Hz, esto
    /// elimina ~99% del trafico FB para una app de 480×280 en pantalla 1280×720.
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
        self.presentar_region(marco);
    }

    /// Recompone el escritorio entero de una sola pasada (Fase 9). Inunda el
    /// area de apps con el reposo del lienzo y funde sobre ella cada capa, EN
    /// ORDEN —de atras hacia adelante—: asi el solapamiento de las ventanas
    /// flotantes se resuelve por si solo, sin recortes ni mascaras. Cada capa
    /// pinta primero su panel —el cromo de la ventana— y, encima, su contenido;
    /// una sola presentacion cierra la pasada.
    fn recomponer(
        &mut self,
        area: RegionPantalla,
        capas: &[CapaSlot],
        taskbar: &TaskbarSlot,
        resolver: &dyn Resolver,
        overlay: Option<&LauncherOverlay>,
    ) {
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
                ContenidoSlot::Panel => {
                    self.lienzo
                        .rellenar_rect(m.x, m.y, m.ancho, m.alto, Color::PANEL);
                }
                ContenidoSlot::Fotograma(indice) => {
                    // El panel primero —el cromo que rodea el lienzo— y el
                    // fotograma natural centrado encima. Los bytes los aporta
                    // el resolver: la consola no sabe que es una `Ventana`.
                    self.lienzo
                        .rellenar_rect(m.x, m.y, m.ancho, m.alto, Color::PANEL);
                    let datos = resolver.cache(*indice);
                    self.componer_fotograma(m, capa.nat_ancho, capa.nat_alto, datos);
                }
                ContenidoSlot::Baliza(color) => {
                    self.lienzo.rellenar_rect(m.x, m.y, m.ancho, m.alto, *color);
                }
            }
            self.dibujar_borde(m, capa.enfocada);
        }
        self.pintar_taskbar(taskbar, resolver);
        // FASE 58 :: si el launcher esta abierto, pintar su overlay como
        // ULTIMA capa, encima de la taskbar — el operador lo ve por encima
        // de todo y no se confunde con una ventana mas.
        if let Some(overlay) = overlay {
            self.pintar_launcher(overlay);
        }
        self.presentar();
    }

    /// FASE 58 :: pinta el overlay del launcher centrado en su region. Caja
    /// con fondo `PANEL`, borde `FOCO`, una linea de titulo y un renglon por
    /// item. La fila seleccionada se pinta con fondo `FOCO`. La consola
    /// asume que `overlay.region` cabe dentro del lienzo —el llamante calcula
    /// la geometria—. Las constantes vienen del compositor para evitar drift
    /// entre `region_launcher` y este pintado.
    fn pintar_launcher(&mut self, overlay: &LauncherOverlay) {
        use crate::compositor::{PICKER_ALTURA_FILA, PICKER_ALTURA_TITULO};
        const GROSOR_BORDE: usize = 3;
        const MARGEN_TEXTO: usize = 16;
        let altura_fila = PICKER_ALTURA_FILA;
        let altura_titulo = PICKER_ALTURA_TITULO;

        let r = overlay.region;
        // Fondo del panel.
        self.lienzo
            .rellenar_rect(r.x, r.y, r.ancho, r.alto, Color::PANEL);
        // Borde indigo grueso — delata que es modal.
        self.lienzo
            .rellenar_rect(r.x, r.y, r.ancho, GROSOR_BORDE, Color::FOCO);
        self.lienzo.rellenar_rect(
            r.x,
            r.y + r.alto.saturating_sub(GROSOR_BORDE),
            r.ancho,
            GROSOR_BORDE,
            Color::FOCO,
        );
        self.lienzo
            .rellenar_rect(r.x, r.y, GROSOR_BORDE, r.alto, Color::FOCO);
        self.lienzo.rellenar_rect(
            r.x + r.ancho.saturating_sub(GROSOR_BORDE),
            r.y,
            GROSOR_BORDE,
            r.alto,
            Color::FOCO,
        );

        // Titulo en la barra superior — un renglon con el atajo recordatorio.
        let titulo_base_y = r.y + altura_titulo - 8;
        self.pintar_etiqueta(
            r.x + MARGEN_TEXTO,
            titulo_base_y,
            "lanzar app  ::  Alt+J/K mueven  ::  Alt+Enter lanza  ::  Alt+Q cierra",
            14.0,
            Color::PANEL,
            Color::TEXTO,
        );

        // Filas — una por item del catalogo, dentro del area util por debajo
        // del titulo y por encima del borde inferior. Si no cabe alguna, se
        // omite en silencio: el operador puede mover la seleccion con J/K
        // hasta una visible (MVP — el scrolling viene despues).
        let filas_y0 = r.y + altura_titulo;
        let filas_y_max = r.y + r.alto.saturating_sub(GROSOR_BORDE + 4);
        for (i, item) in overlay.items.iter().enumerate() {
            let fila_y = filas_y0 + i * altura_fila;
            if fila_y + altura_fila > filas_y_max {
                break;
            }
            let seleccionada = i == overlay.seleccion;
            let fondo = if seleccionada {
                Color::FOCO
            } else {
                Color::PANEL
            };
            if seleccionada {
                // Pinta la franja completa de la fila — desde el borde
                // izquierdo del panel hasta el derecho, salvo el borde.
                self.lienzo.rellenar_rect(
                    r.x + GROSOR_BORDE,
                    fila_y,
                    r.ancho.saturating_sub(GROSOR_BORDE * 2),
                    altura_fila,
                    Color::FOCO,
                );
            }
            let base_y = fila_y + (altura_fila + 14) / 2;
            self.pintar_etiqueta(
                r.x + MARGEN_TEXTO,
                base_y,
                item.as_str(),
                16.0,
                fondo,
                Color::TEXTO,
            );
        }
    }

    /// Pinta la barra de tareas como ultima capa del escritorio (Fase 14/16):
    /// el fondo de la franja, una linea fina arriba que la separa de las apps,
    /// el lanzador a la izquierda, las pestañas en el medio y el reloj a la
    /// derecha.
    fn pintar_taskbar(&mut self, taskbar: &TaskbarSlot, resolver: &dyn Resolver) {
        // Fondo de la barra y linea de separacion.
        self.lienzo.rellenar_rect(
            taskbar.area.x,
            taskbar.area.y,
            taskbar.area.ancho,
            taskbar.area.alto,
            Color::PANEL,
        );
        self.lienzo.rellenar_rect(
            taskbar.area.x,
            taskbar.area.y,
            taskbar.area.ancho,
            1,
            Color::SIN_FOCO,
        );
        // El boton lanzador: un cuadrado indigo con un «+» centrado. Invita a
        // pulsar — al hacerlo, el compositor solicita un parto (igual que Alt+N).
        let l = taskbar.launcher;
        self.lienzo
            .rellenar_rect(l.x, l.y, l.ancho, l.alto, Color::FOCO);
        // El «+»: dos barras estrechas cruzadas en el centro. Mas legible que
        // una sola hace una cruz limpia, sin depender de la tipografia.
        let cx = l.x + l.ancho / 2;
        let cy = l.y + l.alto / 2;
        let radio: usize = 8;
        let grosor: usize = 2;
        // Barra horizontal.
        self.lienzo.rellenar_rect(
            cx.saturating_sub(radio),
            cy.saturating_sub(grosor / 2),
            radio * 2,
            grosor,
            Color::TEXTO,
        );
        // Barra vertical.
        self.lienzo.rellenar_rect(
            cx.saturating_sub(grosor / 2),
            cy.saturating_sub(radio),
            grosor,
            radio * 2,
            Color::TEXTO,
        );
        // Las pestañas. El nombre se resuelve por indice via el resolver.
        for celda in taskbar.celdas {
            let r = celda.region;
            self.lienzo
                .rellenar_rect(r.x, r.y, r.ancho, r.alto, celda.fondo);
            let base_y = r.y + (r.alto + 14) / 2;
            let nombre = resolver.nombre(celda.ventana);
            self.pintar_etiqueta(r.x + 10, base_y, nombre, 16.0, celda.fondo, celda.tinta);
        }
        // El reloj a la derecha: alineado a la izquierda de su region, sobre
        // el fondo del panel (sin caja propia — la barra es su lienzo).
        let r = taskbar.reloj_region;
        let base_y = r.y + (r.alto + 14) / 2;
        self.pintar_etiqueta(r.x, base_y, taskbar.reloj, 16.0, Color::PANEL, Color::TEXTO);
    }

    /// Rasteriza una cadena de texto a un tamaño dado, en (x, base_y), sobre
    /// un fondo conocido —del que toma la mezcla por cobertura del glifo—. Es
    /// la version sin estado de la pluma: el llamante decide donde escribir.
    fn pintar_etiqueta(
        &mut self,
        x: usize,
        base_y: usize,
        texto: &str,
        tamaño: f32,
        fondo: Color,
        tinta: Color,
    ) {
        let mut cursor = x;
        for caracter in texto.chars() {
            let (metricas, cobertura) = texto::rasterizar(caracter, tamaño);
            let inicio_x = cursor as isize + metricas.xmin as isize;
            let inicio_y = base_y as isize - metricas.ymin as isize - metricas.height as isize;
            for fila in 0..metricas.height {
                for col in 0..metricas.width {
                    let opacidad = cobertura[fila * metricas.width + col];
                    if opacidad == 0 {
                        continue;
                    }
                    let px = inicio_x + col as isize;
                    let py = inicio_y + fila as isize;
                    if px < 0 || py < 0 {
                        continue;
                    }
                    let color = mezclar(fondo, tinta, opacidad);
                    self.lienzo.pintar_pixel(px as usize, py as usize, color);
                }
            }
            cursor += metricas.advance_width as usize;
        }
    }

    /// Inunda una region entera con un color plano —la baliza de desalojo: una
    /// app que falla tatua su marco de purpura— y le traza su borde de foco.
    /// Solo presenta esa region: el resto del lienzo no se toca.
    fn pintar_region(&mut self, region: RegionPantalla, color: Color, enfocada: bool) {
        self.lienzo
            .rellenar_rect(region.x, region.y, region.ancho, region.alto, color);
        self.dibujar_borde(region, enfocada);
        self.presentar_region(region);
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

    /// Vuelca el lienzo sobre la pantalla fisica y estampa el puntero del raton
    /// como ultima capa, directamente sobre el framebuffer (Fase 13). El lienzo
    /// permanece libre de puntero — es el «save-under» natural—; el framebuffer
    /// lo recibe en cada volcado, asi que el puntero esta siempre encima.
    pub(crate) fn presentar(&mut self) {
        self.pantalla.presentar(&self.lienzo);
        if let Some((x, y)) = crate::drivers::raton::posicion() {
            self.pantalla.estampar_puntero(x, y);
        }
    }

    /// Vuelca SOLO una sub-region del lienzo a pantalla y re-estampa el
    /// puntero si su sprite intersecta esa region (el blit lo habria borrado).
    /// Si el puntero queda fuera, no se toca: el sprite que ya estaba sobre el
    /// framebuffer sigue intacto. Esta es la primitiva del camino rapido.
    pub(crate) fn presentar_region(&mut self, region: RegionPantalla) {
        self.pantalla.presentar_region(&self.lienzo, region);
        if let Some((x, y)) = crate::drivers::raton::posicion() {
            if region_solapa(region, sprite_puntero_rect(x, y)) {
                self.pantalla.estampar_puntero(x, y);
            }
        }
    }
}

/// Devuelve el rectangulo que el sprite del puntero ocupa en pantalla con su
/// vertice en `(x, y)`. Coincide con el sprite hardcodeado en `grafico::PUNTERO`
/// (12 columnas × 18 filas).
fn sprite_puntero_rect(x: usize, y: usize) -> RegionPantalla {
    RegionPantalla {
        x,
        y,
        ancho: 12,
        alto: 18,
    }
}

/// `true` si dos regiones se solapan en al menos un pixel.
fn region_solapa(a: RegionPantalla, b: RegionPantalla) -> bool {
    let a_x_fin = a.x.saturating_add(a.ancho);
    let a_y_fin = a.y.saturating_add(a.alto);
    let b_x_fin = b.x.saturating_add(b.ancho);
    let b_y_fin = b.y.saturating_add(b.alto);
    a.x < b_x_fin && b.x < a_x_fin && a.y < b_y_fin && b.y < a_y_fin
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
pub(crate) fn recomponer(
    area: RegionPantalla,
    capas: &[CapaSlot],
    taskbar: &TaskbarSlot,
    resolver: &dyn Resolver,
    overlay: Option<&LauncherOverlay>,
) {
    if let Some(consola) = CONSOLA.get() {
        consola
            .lock()
            .recomponer(area, capas, taskbar, resolver, overlay);
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

/// Vuelve a volcar el lienzo a pantalla y estampar el puntero (Fase 13). Sirve
/// para refrescar el puntero cuando el raton se mueve pero ninguna app pinta:
/// el lienzo es el mismo, pero el puntero esta en otro sitio.
pub(crate) fn refrescar() {
    if let Some(consola) = CONSOLA.get() {
        consola.lock().presentar();
    }
}
