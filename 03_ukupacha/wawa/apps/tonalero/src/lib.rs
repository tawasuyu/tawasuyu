// =============================================================================
//  renaser :: apps/tonalero — Fase 22/27 :: testigo de Configuracion + temas
// -----------------------------------------------------------------------------
//  El testigo visual del bucle de Configuracion. Lee idioma y paleta del
//  ContextoCapacidades (inyeccion pasiva por el kernel cada `tick`), pinta
//  cinco swatches etiquetados sobre la paleta activa, y muestra una linea
//  de estado en el idioma activo. SPACE no rota la paleta in-place: cicla
//  entre TRES TEMAS PRESET (Fase 27) — cada uno con su (idioma, paleta) — y
//  los propone al kernel via `sys_config_proponer`. El frame-lock del
//  kernel garantiza que TODAS las apps reciben el cambio en el mismo
//  fotograma.
// =============================================================================

#![no_std]

#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_puntero(salida: u32) -> i32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_config_proponer(idioma: u32, paleta_ptr: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// =============================================================================
//  Geometria del lienzo
// =============================================================================

const ANCHO: usize = 480;
const ALTO: usize = 300;

const MARGEN: usize = 12;
const TITULO_ALTO: usize = 28;
const PIE_ALTO: usize = 32;
const SWATCHES_GAP: usize = 6;

const NUM_SWATCHES: usize = 5;

const ESCALA_TITULO: usize = 3;
const ESCALA_ETIQUETA: usize = 2;
const ESCALA_PIE: usize = 2;

// =============================================================================
//  Catalogo de TEMAS PRESET (Fase 27)
// -----------------------------------------------------------------------------
//  Cada tema fija una (idioma, paleta) coordinada. SPACE cicla entre los tres;
//  el kernel ve cada propuesta entera y la inyecta a todas las apps en el
//  proximo `tick`. La paleta se rotula en el idioma activo —ese mapeo vive
//  en `etiquetas_por_idioma`—.
// =============================================================================

struct Tema {
    idioma: u16,
    paleta: [u8; 20],
}

const TEMAS: [Tema; 3] = [
    // 0 — Wawa por defecto (azul renaser + ambar) :: es
    Tema {
        idioma: idioma_le(*b"es"),
        paleta: [
            0x20, 0x80, 0xC0, 0xFF, // primario   :: azul renaser
            0x60, 0x60, 0x60, 0xFF, // secundario :: gris medio
            0x00, 0x00, 0x00, 0xFF, // fondo      :: negro
            0xFF, 0xFF, 0xFF, 0xFF, // texto      :: blanco
            0xF0, 0x90, 0x20, 0xFF, // acento     :: ambar
        ],
    },
    // 1 — Indigo Profundo Nocturno :: en
    Tema {
        idioma: idioma_le(*b"en"),
        paleta: [
            0x6A, 0x5A, 0xCD, 0xFF, // primario   :: slate blue
            0x48, 0x3D, 0x8B, 0xFF, // secundario :: dark slate blue
            0x0A, 0x0A, 0x28, 0xFF, // fondo      :: indigo nocturno
            0xE6, 0xE6, 0xFA, 0xFF, // texto      :: lavanda blanca
            0x8A, 0x2B, 0xE2, 0xFF, // acento     :: violeta electrico
        ],
    },
    // 2 — Ambar Terminal Monocromo :: qu (quechua)
    Tema {
        idioma: idioma_le(*b"qu"),
        paleta: [
            0xFF, 0xB8, 0x00, 0xFF, // primario   :: ambar saturado
            0xA0, 0x72, 0x00, 0xFF, // secundario :: ambar profundo
            0x10, 0x06, 0x00, 0xFF, // fondo      :: casi negro calido
            0xFF, 0xD8, 0x70, 0xFF, // texto      :: ambar claro
            0xFF, 0x40, 0x00, 0xFF, // acento     :: naranja brasa
        ],
    },
];

/// Pack ISO 639-1 letras en u16 little-endian, igual que `format::idioma_iso639`.
const fn idioma_le(letras: [u8; 2]) -> u16 {
    (letras[0] as u16) | ((letras[1] as u16) << 8)
}

// =============================================================================
//  Estado de la app — un puñado de estaticos
// =============================================================================

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];
static mut IDIOMA: u16 = 0;
static mut SPACE_PRESS: bool = false;
static mut RATON_IZQ_PREV: bool = false;
static mut RATON_X: u16 = 0;
static mut RATON_Y: u16 = 0;
static mut RATON_DENTRO: bool = false;

/// Cursor del catalogo de temas: cuando SPACE sube, cicla 0->1->2->0 y
/// propone al kernel el nuevo tema. Si el kernel rechaza (sin foco, paleta
/// desbordada), el cursor avanza igual — el siguiente intento del usuario
/// trabaja sobre el siguiente tema, sin atascarse en uno bloqueado.
static mut CURSOR_TEMA: u8 = 0;

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
    let space_ahora = scancode == 0x39;
    if space_ahora && !unsafe { SPACE_PRESS } {
        proponer_siguiente_tema();
    }
    unsafe { SPACE_PRESS = space_ahora };

    // Drenar eventos del puntero; un clic izquierdo dentro del lienzo
    // tambien cicla tema — el mismo gesto que SPACE pero gobernado por la
    // geometria local que el kernel entrega ya traducida.
    let mut buffer = [0u8; 5];
    let mut izq_flanco_subida = false;
    loop {
        let n = unsafe { sys_puntero(buffer.as_mut_ptr() as u32) };
        if n != 5 {
            break;
        }
        let lx = u16::from_le_bytes([buffer[0], buffer[1]]);
        let ly = u16::from_le_bytes([buffer[2], buffer[3]]);
        let botones = buffer[4];
        let izq = (botones & 0x01) != 0;
        if izq && !unsafe { RATON_IZQ_PREV } {
            izq_flanco_subida = true;
        }
        unsafe {
            RATON_X = lx;
            RATON_Y = ly;
            RATON_DENTRO = true;
            RATON_IZQ_PREV = izq;
        }
    }
    if izq_flanco_subida {
        proponer_siguiente_tema();
    }

    pintar();
    volcar();
}

// =============================================================================
//  Contexto y propuesta
// =============================================================================

fn refrescar_contexto() {
    let idioma = unsafe { sys_config_idioma() } as u16;
    unsafe { IDIOMA = idioma };
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

fn proponer_siguiente_tema() {
    let cur = unsafe { CURSOR_TEMA } as usize;
    let proximo = (cur + 1) % TEMAS.len();
    unsafe { CURSOR_TEMA = proximo as u8 };
    let tema = &TEMAS[proximo];
    let _ = unsafe {
        sys_config_proponer(tema.idioma as u32, tema.paleta.as_ptr() as u32)
    };
}

// =============================================================================
//  Textos por idioma (Fase 27)
// -----------------------------------------------------------------------------
//  Cada idioma cubierto tiene su tabla de rotulos de longitud fija. La
//  comparacion del idioma activo es un `match` numerico — sin lookup en
//  tabla, sin allocacion—. Idiomas no cubiertos caen al ingles, que es el
//  conjunto de fonemas mas portatil del sistema actual.
// =============================================================================

struct Locale {
    /// Las cinco etiquetas de los swatches, en el orden de la paleta:
    /// primario / secundario / fondo / texto / acento.
    swatches: [&'static [u8]; NUM_SWATCHES],
    /// Linea de estado en el cuerpo del panel.
    estado: &'static [u8],
    /// Atajo rotulado en el pie.
    atajo: &'static [u8],
}

fn locale_para(idioma: u16) -> &'static Locale {
    match idioma {
        x if x == idioma_le(*b"es") => &ES,
        x if x == idioma_le(*b"en") => &EN,
        x if x == idioma_le(*b"qu") => &QU,
        _ => &EN, // fallback portable
    }
}

const ES: Locale = Locale {
    swatches: [
        b"PRIMARIO",
        b"SECUNDARIO",
        b"FONDO",
        b"TEXTO",
        b"ACENTO",
    ],
    estado: b"SISTEMA COMPOSITOR ACTIVO",
    atajo: b"SPACE  CAMBIA TEMA",
};

const EN: Locale = Locale {
    swatches: [
        b"PRIMARY",
        b"SECONDARY",
        b"BACKGROUND",
        b"TEXT",
        b"ACCENT",
    ],
    estado: b"ACTIVE COMPOSITOR SYSTEM",
    atajo: b"SPACE  CHANGE THEME",
};

const QU: Locale = Locale {
    swatches: [
        b"NAWPAQ",       // primario
        b"ISKAYNIQ",     // secundario
        b"UJUKUNA",      // fondo
        b"QILLQA",       // texto
        b"REQSICHIQ",    // acento
    ],
    estado: b"PURIY KAMACHI KAUSAYNINPI",
    atajo: b"SPACE  TEMA TIKRAY",
};

// =============================================================================
//  Pintado
// =============================================================================

fn pintar() {
    let paleta = unsafe { PALETA };
    let idioma = unsafe { IDIOMA };
    let locale = locale_para(idioma);
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    let fondo = color_u32(paleta, 2);
    let tinta = color_u32(paleta, 3);
    let acento = color_u32(paleta, 4);

    // Fondo plano.
    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, fondo);

    // Cabecera con titulo TONALERO + subrayado acento.
    rellenar_rect(lienzo, 0, 0, ANCHO, TITULO_ALTO, color_atenuar(paleta, 2, 0xE0));
    let titulo = b"TONALERO";
    let titulo_ancho = ancho_texto(titulo, ESCALA_TITULO);
    let titulo_x = (ANCHO - titulo_ancho) / 2;
    let titulo_y = (TITULO_ALTO - 7 * ESCALA_TITULO) / 2;
    dibujar_texto(lienzo, titulo, titulo_x, titulo_y, ESCALA_TITULO, tinta);
    rellenar_rect(lienzo, 0, TITULO_ALTO, ANCHO, 2, acento);

    // Cinco swatches etiquetados (idioma-activo).
    let area_swatches_y = TITULO_ALTO + 4 + MARGEN;
    let area_swatches_alto = ALTO - area_swatches_y - PIE_ALTO - MARGEN;
    let fila_alto =
        (area_swatches_alto - SWATCHES_GAP * (NUM_SWATCHES - 1)) / NUM_SWATCHES;
    let swatch_ancho = 70_usize;
    let swatch_x = MARGEN * 2;
    let etiqueta_x = swatch_x + swatch_ancho + MARGEN;

    for (i, etiqueta) in locale.swatches.iter().enumerate() {
        let y = area_swatches_y + i * (fila_alto + SWATCHES_GAP);
        let c = color_u32(paleta, i);
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
        let texto_y = y + (fila_alto - 7 * ESCALA_ETIQUETA) / 2;
        dibujar_texto(lienzo, etiqueta, etiqueta_x, texto_y, ESCALA_ETIQUETA, tinta);
    }

    // Pie en DOS lineas: codigo de idioma + estado del compositor; atajo.
    let pie_y = ALTO - PIE_ALTO;
    rellenar_rect(lienzo, 0, pie_y - 2, ANCHO, 2, acento);

    let mut linea_estado = [b' '; 64];
    linea_estado[0] = (idioma & 0xFF) as u8;
    linea_estado[1] = ((idioma >> 8) & 0xFF) as u8;
    if !linea_estado[0].is_ascii_alphabetic() {
        linea_estado[0] = b'?';
    }
    if !linea_estado[1].is_ascii_alphabetic() {
        linea_estado[1] = b'?';
    }
    // Mayusculas para la traza visual.
    linea_estado[0] = linea_estado[0].to_ascii_uppercase();
    linea_estado[1] = linea_estado[1].to_ascii_uppercase();
    linea_estado[2] = b' ';
    linea_estado[3] = b' ';
    let max_estado = (locale.estado.len()).min(60);
    linea_estado[4..4 + max_estado].copy_from_slice(&locale.estado[..max_estado]);
    dibujar_texto(
        lienzo,
        &linea_estado[..4 + max_estado],
        MARGEN * 2,
        pie_y + 2,
        ESCALA_PIE,
        tinta,
    );
    dibujar_texto(
        lienzo,
        locale.atajo,
        MARGEN * 2,
        pie_y + 16,
        ESCALA_PIE,
        acento,
    );

    // Cruz del puntero, si esta dentro del lienzo.
    if unsafe { RATON_DENTRO } {
        let cx = unsafe { RATON_X } as usize;
        let cy = unsafe { RATON_Y } as usize;
        if cx < ANCHO && cy < ALTO {
            let largo = 6_usize;
            let x0 = cx.saturating_sub(largo);
            let x1 = (cx + largo).min(ANCHO);
            let y0 = cy.saturating_sub(largo);
            let y1 = (cy + largo).min(ALTO);
            rellenar_rect(lienzo, x0, cy.saturating_sub(1), x1 - x0, 2, acento);
            rellenar_rect(lienzo, cx.saturating_sub(1), y0, 2, y1 - y0, acento);
        }
    }
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

// =============================================================================
//  Helpers
// =============================================================================

fn color_u32(paleta: [u8; 20], n: usize) -> u32 {
    let base = n * 4;
    let r = paleta[base] as u32;
    let g = paleta[base + 1] as u32;
    let b = paleta[base + 2] as u32;
    b | (g << 8) | (r << 16)
}

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
//  Mini-tipografia 5x7
// =============================================================================

const FUENTE_ANCHO: usize = 5;
const FUENTE_ALTO: usize = 7;
const FUENTE_AVANCE: usize = FUENTE_ANCHO + 1;

fn glifo(c: u8) -> [u8; FUENTE_ALTO] {
    match c {
        b' ' => [0x00; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b'?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
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

fn ancho_texto(texto: &[u8], escala: usize) -> usize {
    if texto.is_empty() {
        return 0;
    }
    (texto.len() * FUENTE_AVANCE - 1) * escala
}

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
