// =============================================================================
//  wawa :: apps/asistente — Fase 60 v3 :: scaffolding del asistente WASM
// -----------------------------------------------------------------------------
//  Asistente conversacional dentro de wawa. v1 (este archivo) es UI puro:
//  pinta el fondo, el titulo y un mensaje de estado — todavia sin red,
//  sin input, sin consulta al `asistente-puente`. La prueba de v1 es que
//  el modulo se instancie y pinte dentro de la region asignada por el
//  kernel, sin pedir capacidades nuevas.
//
//  Hoja de ruta (ver `docs/ASISTENTE_WAWA.md`):
//  - v1 (este commit) :: scaffolding init+tick+pintura.
//  - v2 :: input de texto (sys_get_scancode + buffer local).
//  - v3 :: sys_red_enviar / sys_red_recibir sobre `CANAL_ASISTENTE`.
//  - v4 :: presentar propuestas y disparar la firma humana via
//          `daemon-firma` cuando aplique.
// =============================================================================

#![no_std]

// --- Capacidades del kernel `wawa` que esta app usa. v1 solo necesita
//     `sys_render_frame`; v2+ sumara las otras. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
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

/// El kernel invoca esta funcion UNA sola vez, al instanciar el modulo.
/// Pinta el primer fotograma de modo que la ventana no nazca vacia.
#[no_mangle]
pub extern "C" fn init() {
    pintar();
    volcar();
}

/// Un fotograma de trabajo. v1 es idempotente — la escena es estatica.
/// v2 traera estado (query en curso, ultima propuesta, etc.) y este
/// `tick` lo refrescara segun los scancodes y los mensajes Akasha
/// pendientes.
#[no_mangle]
pub extern "C" fn tick() {
    pintar();
    volcar();
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
    // Linea fina debajo del titulo para separar visualmente.
    rellenar_rect(lienzo, 0, 36, ANCHO, 2, ACENTO);

    // Cuerpo: estado del scaffolding.
    let mut y = 56;
    dibujar_texto(lienzo, b"SCAFFOLDING V1", 18, y, 1, TINTA);
    y += 14;
    dibujar_texto(lienzo, b"SIN RED  SIN INPUT", 18, y, 1, SUTIL);
    y += 22;

    // Tres lineas que describen la hoja de ruta — al operador que vea
    // este fotograma le queda claro que la pieza es real pero aun no
    // habla con el puente.
    dibujar_texto(lienzo, b"V2  INPUT DE TEXTO", 18, y, 1, SUTIL);
    y += 14;
    dibujar_texto(lienzo, b"V3  CANAL ASISTENTE  AS  0X4153", 18, y, 1, SUTIL);
    y += 14;
    dibujar_texto(lienzo, b"V4  PROPUESTA  FIRMA HUMANA", 18, y, 1, SUTIL);

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
