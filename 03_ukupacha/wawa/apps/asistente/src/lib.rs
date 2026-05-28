// =============================================================================
//  wawa :: apps/asistente — Fase 60 v3+v4 :: scaffolding del asistente WASM
// -----------------------------------------------------------------------------
//  Asistente conversacional dentro de wawa. Vamos por capas:
//
//  - v1 :: UI puro: pinta el fondo, el titulo, el roadmap.
//  - v2 :: input de texto local (sys_get_scancode + traduccion +
//          buffer QUERY). Sin red todavia: Enter no manda nada.
//  - v3 :: sys_red_enviar / sys_red_recibir sobre `CANAL_ASISTENTE`.
//  - v4 :: presentar propuestas y disparar la firma humana via
//          `daemon-firma` cuando aplique.
//
//  Este archivo cubre v1+v2.
// =============================================================================

#![no_std]

// --- Capacidades del kernel `wawa` que esta app usa. v1+v2 necesitan
//     `sys_render_frame` y `sys_get_scancode`; v3+ sumara las de red. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    /// Devuelve el ultimo scancode pulsado en bruto, o 0 si la cola del
    /// teclado de la app esta vacia. Es la misma syscall que `mudanza`
    /// usa para anti-rebote del SPACE.
    fn sys_get_scancode() -> u32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria. DEBE encajar con la region que el manifiesto reserve
//     para esta app cuando se siembre en GENESIS. ---
const ANCHO: usize = 480;
const ALTO: usize = 240;

// --- Paleta. v1 usa colores hardcoded (alineados con la paleta del
//     compositor: indigo oscuro de fondo, slate de panel, indigo
//     brillante de acento, blanco suave de tinta). v2 leera la paleta
//     activa via `sys_config_paleta` cuando integre el sistema de temas. ---
const FONDO: u32 = 0x12_16_20;
const PANEL: u32 = 0x1B_21_30;
const ACENTO: u32 = 0x6E_8C_DC;
const TINTA: u32 = 0xE8_EC_F4;
const SUTIL: u32 = 0x8C_98_AA;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- Estado de v2: el operador escribe una query en vivo. ---

/// Cota dura de la query — caracteres ASCII. Por encima, los keystrokes
/// se descartan en silencio (el operador ve que el texto no crece).
const QUERY_MAX: usize = 64;
static mut QUERY: [u8; QUERY_MAX] = [0; QUERY_MAX];
static mut QUERY_LEN: usize = 0;

/// Anti-rebote: el ultimo scancode procesado. Solo el flanco
/// scancode_actual != scancode_previo cuenta como pulsacion (igual
/// patron que `mudanza::SPACE_PREV`).
static mut SCANCODE_PREV: u32 = 0;

/// FASE 60 v2 :: el ultimo carcater visible para que el operador sepa
/// que el input lo esta viendo. Es un byte ASCII o 0 si no hay nada
/// reciente. Util para validacion visual del scaffolding antes de que
/// haya red.
static mut ULTIMO_CHAR: u8 = 0;

/// El kernel invoca esta funcion UNA sola vez, al instanciar el modulo.
/// Pinta el primer fotograma de modo que la ventana no nazca vacia.
#[no_mangle]
pub extern "C" fn init() {
    pintar();
    volcar();
}

/// Un fotograma de trabajo. v2 :: drena el scancode pendiente, lo
/// traduce a ASCII si aplica y lo apila en `QUERY`. Sin red todavia,
/// Enter no manda nada (v3).
#[no_mangle]
pub extern "C" fn tick() {
    procesar_teclado();
    pintar();
    volcar();
}

/// Lee el scancode pendiente y, si es un flanco de subida nuevo,
/// actualiza el estado: append a `QUERY` si es printable, pop si es
/// backspace, Enter es no-op (v3 lo conectara). Make codes (bit 7
/// limpio) son los unicos que producen efecto; los break codes
/// (bit 7 puesto) se ignoran — la pulsacion ya quedo contada en su make.
fn procesar_teclado() {
    let actual = unsafe { sys_get_scancode() };
    let prev = unsafe { SCANCODE_PREV };
    // Solo el flanco sirve: si llega el mismo scancode dos ticks
    // seguidos sin cambiar, no lo re-procesamos.
    if actual == prev {
        return;
    }
    unsafe { SCANCODE_PREV = actual };
    if actual == 0 || actual >= 0x80 {
        // Cola vacia o break code; ignorar.
        return;
    }
    let sc = actual as u8;
    // Backspace (scancode 0x0E en set 1).
    if sc == 0x0E {
        unsafe {
            if QUERY_LEN > 0 {
                QUERY_LEN -= 1;
                QUERY[QUERY_LEN] = 0;
            }
        }
        return;
    }
    // Enter (scancode 0x1C en set 1) — v3 lo enviará por Akasha. Hoy
    // marca `ULTIMO_CHAR = '\n'` como pista visual y no hace mas.
    if sc == 0x1C {
        unsafe { ULTIMO_CHAR = b'\n' };
        return;
    }
    // Letra/cifra/espacio: append si cabe.
    if let Some(byte) = traducir_scancode_a_ascii(sc) {
        unsafe {
            if QUERY_LEN < QUERY_MAX {
                QUERY[QUERY_LEN] = byte;
                QUERY_LEN += 1;
                ULTIMO_CHAR = byte;
            }
        }
    }
}

/// Mapa minimo de scancodes set 1 a ASCII MAYUSCULA — la app usa la
/// fuente que solo tiene mayusculas, asi que no perdemos info al subir
/// a uppercase. Sin shift detection: el operador escribe mayusculas
/// siempre (consistente con el resto de las apps del kernel).
fn traducir_scancode_a_ascii(sc: u8) -> Option<u8> {
    // Cifras '1'-'9' en 0x02..0x0A, '0' en 0x0B.
    if (0x02..=0x0A).contains(&sc) {
        return Some(b'1' + (sc - 0x02));
    }
    if sc == 0x0B {
        return Some(b'0');
    }
    // Espacio en 0x39.
    if sc == 0x39 {
        return Some(b' ');
    }
    // Letras QWERTY en set 1. Tabla escrita a mano — chiquita y
    // determinista, sin alocacion.
    let letra = match sc {
        0x10 => b'Q', 0x11 => b'W', 0x12 => b'E', 0x13 => b'R',
        0x14 => b'T', 0x15 => b'Y', 0x16 => b'U', 0x17 => b'I',
        0x18 => b'O', 0x19 => b'P',
        0x1E => b'A', 0x1F => b'S', 0x20 => b'D', 0x21 => b'F',
        0x22 => b'G', 0x23 => b'H', 0x24 => b'J', 0x25 => b'K',
        0x26 => b'L',
        0x2C => b'Z', 0x2D => b'X', 0x2E => b'C', 0x2F => b'V',
        0x30 => b'B', 0x31 => b'N', 0x32 => b'M',
        _ => return None,
    };
    Some(letra)
}

// =============================================================================
//  Pintado del fotograma
// =============================================================================

fn pintar() {
    // SEGURIDAD: durante `init` y `tick` esta es la unica via de acceso al
    // LIENZO; el kernel jamas reentra el modulo mientras una de ellas corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    // Fondo plano + barra de titulo.
    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, FONDO);
    rellenar_rect(lienzo, 0, 0, ANCHO, 36, PANEL);
    dibujar_texto(lienzo, b"ASISTENTE", 18, 10, 2, ACENTO);
    rellenar_rect(lienzo, 0, 36, ANCHO, 2, ACENTO);

    // FASE 60 v2 :: la zona de input. Caja con el prompt y el contenido
    // de QUERY. Vacio cuando el operador no escribio nada todavia.
    let mut y = 56;
    dibujar_texto(lienzo, b"PROMPT:", 18, y, 1, SUTIL);
    y += 14;
    rellenar_rect(lienzo, 18, y, ANCHO - 36, 24, PANEL);
    rellenar_rect(lienzo, 18, y, 2, 24, ACENTO); // borde izq del input
    // El texto de la query, en mayusculas (la fuente solo tiene mayus).
    // SEGURIDAD: lectura de mutable static en contexto single-threaded
    // — solo `tick` muta `QUERY`/`QUERY_LEN`, y no reentra mientras
    // `pintar` corre.
    let (query, query_len): (&[u8], usize) = unsafe { (&QUERY[..QUERY_LEN], QUERY_LEN) };
    dibujar_texto(lienzo, query, 28, y + 8, 1, TINTA);
    // Cursor al final — un guion bajo grueso.
    let cursor_x = 28 + query_len * 6;
    if cursor_x < ANCHO - 12 {
        rellenar_rect(lienzo, cursor_x, y + 16, 4, 2, ACENTO);
    }
    y += 32;

    // Roadmap — recordatorio de lo que aun falta.
    dibujar_texto(lienzo, b"V1  UI  V2  INPUT  V3  RED  V4  FIRMA", 18, y, 1, SUTIL);
    y += 14;
    dibujar_texto(lienzo, b"CANAL ASISTENTE  AS  0X4153", 18, y, 1, SUTIL);
    y += 18;

    // FASE 60 v2 :: pista visual del ultimo char aceptado. Sirve para
    // verificar end-to-end que el flujo IRQ -> kernel -> WASM funciona
    // cuando ejecutas en QEMU. Vacio si nada paso aun.
    let ult = unsafe { ULTIMO_CHAR };
    if ult == b'\n' {
        dibujar_texto(lienzo, b"ULTIMO: ENTER", 18, y, 1, SUTIL);
    } else if ult != 0 {
        dibujar_texto(lienzo, b"ULTIMO:", 18, y, 1, SUTIL);
        let buf = [ult; 1];
        dibujar_texto(lienzo, &buf, 18 + 9 * 6, y, 1, TINTA);
    }

    // Pie: una franja sutil que marca el limite de la region.
    rellenar_rect(lienzo, 0, ALTO - 2, ANCHO, 2, ACENTO);
}

/// Entrega el lienzo completo al kernel. (ptr, len) apunta SIEMPRE dentro
/// de nuestra memoria lineal; el host lo verifica sin piedad.
fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    // SEGURIDAD: `sys_render_frame` es una capacidad del host; el (ptr,
    // len) describe nuestra propia memoria lineal.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

// =============================================================================
//  Primitivas de pintado — sin asignacion, sin dependencias
// =============================================================================

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
//  Mini-tipografia 5x7 — solo los caracteres que esta app usa
// =============================================================================

const FA: usize = 5; // ancho del glifo
const FH: usize = 7; // alto del glifo

fn glifo(c: u8) -> [u8; FH] {
    match c {
        b' ' => [0; 7],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
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
        _ => [0x1F; 7],
    }
}

fn dibujar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, escala: usize, color: u32) {
    let mut cursor_x = x;
    for &c in texto {
        let g = glifo(c);
        for (fila, bits) in g.iter().enumerate() {
            for col in 0..FA {
                if bits & (1 << (FA - 1 - col)) != 0 {
                    let px0 = cursor_x + col * escala;
                    let py0 = y + fila * escala;
                    rellenar_rect(lienzo, px0, py0, escala, escala, color);
                }
            }
        }
        cursor_x += (FA + 1) * escala;
    }
}
