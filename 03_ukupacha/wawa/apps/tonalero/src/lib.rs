// =============================================================================
//  renaser :: apps/tonalero — Fase 22 :: la Configuracion como nodo del grafo
// -----------------------------------------------------------------------------
//  El testigo visual del bucle de Configuracion. Muestra los cinco colores de
//  la paleta activa como swatches etiquetados, rotula el idioma, y propone
//  una rotacion al pulsar SPACE. Toda la lectura es PASIVA: el kernel ya dejo
//  idioma y paleta en el ContextoCapacidades antes de cederle este `tick`;
//  la app las lee con dos capacidades de veintipocos bytes, sin sondeo, sin
//  bloqueo, frame-lock perfecto.
//
//  La barra espaciadora invoca `sys_config_proponer`: el kernel engendra un
//  nodo nuevo del grafo, reancla el manifiesto al hash recien creado, y el
//  proximo fotograma —de esta app y de TODAS las demas— pinta con la paleta
//  nueva. Sin estados mutables globales: el "ahora" es el hash al que apunta
//  el manifiesto vivo.
// =============================================================================

#![no_std]

// --- Las capacidades del host que el kernel inyecta a esta app. -----------
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_config_proponer(idioma: u32, paleta_ptr: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// =============================================================================
//  Geometria del lienzo. DEBE encajar con la region que `boot` reservo en su
//  GENESIS — el kernel rechaza cualquier fotograma de otro tamaño.
// =============================================================================

const ANCHO: usize = 480;
const ALTO: usize = 300;

const MARGEN: usize = 12;
const TITULO_ALTO: usize = 28;
const PIE_ALTO: usize = 20;
const SWATCHES_GAP: usize = 6;

const NUM_SWATCHES: usize = 5;
const ETIQUETAS: [&[u8]; NUM_SWATCHES] = [
    b"PRIMARIO",
    b"SECUNDARIO",
    b"FONDO",
    b"TEXTO",
    b"ACENTO",
];

const ESCALA_TITULO: usize = 3;
const ESCALA_ETIQUETA: usize = 2;
const ESCALA_PIE: usize = 2;

// =============================================================================
//  Estado de la app — un puñado de estaticos en SU propia memoria lineal.
// =============================================================================

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];
static mut IDIOMA: u16 = 0;
static mut SPACE_PRESS: bool = false;

#[no_mangle]
pub extern "C" fn init() {
    refrescar_contexto();
    pintar();
    volcar();
}

#[no_mangle]
pub extern "C" fn tick() {
    refrescar_contexto();

    let scancode = unsafe { sys_get_scancode() };
    let space_ahora = scancode == 0x39; // SPACE en scancode set 1
    if space_ahora && !unsafe { SPACE_PRESS } {
        proponer_rotacion();
    }
    unsafe { SPACE_PRESS = space_ahora };

    pintar();
    volcar();
}

// =============================================================================
//  Lectura del contexto y propuesta de cambio — el cinturon de Configuracion
// =============================================================================

fn refrescar_contexto() {
    let idioma = unsafe { sys_config_idioma() } as u16;
    unsafe { IDIOMA = idioma };
    // SEGURIDAD: PALETA mide 20 bytes en esta memoria lineal; el kernel
    // valida los limites del puntero antes de copiar.
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

fn proponer_rotacion() {
    let actual = unsafe { PALETA };
    let mut rotada = [0u8; 20];
    rotada[0..16].copy_from_slice(&actual[4..20]);
    rotada[16..20].copy_from_slice(&actual[0..4]);
    let idioma = unsafe { IDIOMA } as u32;
    let _ = unsafe { sys_config_proponer(idioma, rotada.as_ptr() as u32) };
}

// =============================================================================
//  Pintado del fotograma — un bloque encima del otro, sin sorpresas
// =============================================================================

fn pintar() {
    let paleta = unsafe { PALETA };
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    let fondo = color_u32(paleta, 2);
    let tinta = color_u32(paleta, 3);
    let acento = color_u32(paleta, 4);

    // 1. Fondo plano del lienzo entero.
    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, fondo);

    // 2. Banda del titulo: barra horizontal con TONALERO centrado y un
    //    subrayado fino en color acento — un acabado simple, no decorativo.
    rellenar_rect(lienzo, 0, 0, ANCHO, TITULO_ALTO, color_atenuar(paleta, 2, 0xE0));
    let titulo = b"TONALERO";
    let titulo_ancho = ancho_texto(titulo, ESCALA_TITULO);
    let titulo_x = (ANCHO - titulo_ancho) / 2;
    let titulo_y = (TITULO_ALTO - 7 * ESCALA_TITULO) / 2;
    dibujar_texto(lienzo, titulo, titulo_x, titulo_y, ESCALA_TITULO, tinta);
    rellenar_rect(lienzo, 0, TITULO_ALTO, ANCHO, 2, acento);

    // 3. Cinco filas de swatch + etiqueta. Cada swatch es una baldosa de
    //    color con un borde sutil; la etiqueta vive a su derecha, en tinta
    //    del color "texto" de la paleta.
    let area_swatches_y = TITULO_ALTO + 4 + MARGEN;
    let area_swatches_alto = ALTO - area_swatches_y - PIE_ALTO - MARGEN;
    let fila_alto =
        (area_swatches_alto - SWATCHES_GAP * (NUM_SWATCHES - 1)) / NUM_SWATCHES;
    let swatch_ancho = 70_usize;
    let swatch_x = MARGEN * 2;
    let etiqueta_x = swatch_x + swatch_ancho + MARGEN;

    for (i, etiqueta) in ETIQUETAS.iter().enumerate() {
        let y = area_swatches_y + i * (fila_alto + SWATCHES_GAP);
        let c = color_u32(paleta, i);
        // Borde fino del swatch — un marco un tono mas oscuro que el color.
        let borde = color_atenuar(paleta, i, 0x80);
        rellenar_rect(lienzo, swatch_x, y, swatch_ancho, fila_alto, borde);
        rellenar_rect(
            lienzo,
            swatch_x + 2,
            y + 2,
            swatch_ancho - 4,
            fila_alto - 4,
            c,
        );
        // Etiqueta vertical-centrada respecto al swatch.
        let texto_y = y + (fila_alto - 7 * ESCALA_ETIQUETA) / 2;
        dibujar_texto(lienzo, etiqueta, etiqueta_x, texto_y, ESCALA_ETIQUETA, tinta);
    }

    // 4. Pie: idioma activo + atajo. "ES   SPACE: ROTAR".
    let pie_y = ALTO - PIE_ALTO;
    rellenar_rect(lienzo, 0, pie_y - 2, ANCHO, 2, acento);
    let mut linea: [u8; 32] = [b' '; 32];
    let idioma = unsafe { IDIOMA };
    linea[0] = ((idioma & 0xFF) as u8).to_ascii_uppercase();
    linea[1] = (((idioma >> 8) & 0xFF) as u8).to_ascii_uppercase();
    let cola = b"   SPACE: ROTAR";
    linea[3..3 + cola.len()].copy_from_slice(cola);
    let pie_texto = &linea[..3 + cola.len()];
    let pie_texto_y = pie_y + (PIE_ALTO - 7 * ESCALA_PIE) / 2;
    dibujar_texto(lienzo, pie_texto, MARGEN * 2, pie_texto_y, ESCALA_PIE, tinta);
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

// =============================================================================
//  Color — la paleta es RGBA8, el kernel decodifica el lienzo como BGRA
// =============================================================================

/// Convierte el color `n` de la paleta (4 bytes RGBA) al u32 little-endian
/// con B en el byte bajo que el kernel decodifica en `componer_fotograma`.
fn color_u32(paleta: [u8; 20], n: usize) -> u32 {
    let base = n * 4;
    let r = paleta[base] as u32;
    let g = paleta[base + 1] as u32;
    let b = paleta[base + 2] as u32;
    b | (g << 8) | (r << 16)
}

/// Igual que `color_u32` pero atenua cada canal multiplicando por
/// `factor / 256`. Util para sombras de borde y bandas atenuadas, sin
/// requerir mas colores en la paleta.
fn color_atenuar(paleta: [u8; 20], n: usize, factor: u32) -> u32 {
    let base = n * 4;
    let r = (paleta[base] as u32 * factor) >> 8;
    let g = (paleta[base + 1] as u32 * factor) >> 8;
    let b = (paleta[base + 2] as u32 * factor) >> 8;
    b | (g << 8) | (r << 16)
}

fn rellenar_rect(lienzo: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    let x_fin = (x + w).min(ANCHO);
    let y_fin = (y + h).min(ALTO);
    for fila in y..y_fin {
        let base = fila * ANCHO;
        for col in x..x_fin {
            lienzo[base + col] = color;
        }
    }
}

// =============================================================================
//  Mini-tipografia 5x7 mayuscula — bastante para etiquetar la paleta
// =============================================================================

const FUENTE_ANCHO: usize = 5;
const FUENTE_ALTO: usize = 7;
const FUENTE_AVANCE: usize = FUENTE_ANCHO + 1;

/// Cada caracter ocupa 7 filas de 5 bits, los cinco bits bajos del byte.
/// Bit alto = pixel encendido. Cubre A-Z, 0-9, espacio, dos puntos y guion;
/// caracteres fuera de tabla se rotulan como un bloque solido para que
/// quien los vea sepa que falta un glifo.
fn glifo(c: u8) -> [u8; FUENTE_ALTO] {
    match c {
        b' ' => [0x00; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
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
        _ => [0x1F; 7],
    }
}

/// Ancho en pixeles que ocupara `texto` rendereado a la escala dada. Cuenta
/// `FUENTE_AVANCE` por caracter menos un espaciado final.
fn ancho_texto(texto: &[u8], escala: usize) -> usize {
    if texto.is_empty() {
        return 0;
    }
    (texto.len() * FUENTE_AVANCE - 1) * escala
}

/// Rendereiza `texto` empezando en (x, y) con bloques cuadrados de `escala`
/// pixeles. El recorte se hace contra los limites del lienzo entero.
fn dibujar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, escala: usize, color: u32) {
    let mut cursor_x = x;
    for &c in texto {
        let g = glifo(c);
        for (fila, bits) in g.iter().enumerate() {
            for col in 0..FUENTE_ANCHO {
                if bits & (1 << (FUENTE_ANCHO - 1 - col)) != 0 {
                    let px0 = cursor_x + col * escala;
                    let py0 = y + fila * escala;
                    rellenar_rect(lienzo, px0, py0, escala, escala, color);
                }
            }
        }
        cursor_x += FUENTE_AVANCE * escala;
    }
}
