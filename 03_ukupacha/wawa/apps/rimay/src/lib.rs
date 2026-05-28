// =============================================================================
//  renaser :: apps/rimay — espejo bare-metal del verbo de embeddings
// -----------------------------------------------------------------------------
//  El subdominio host `rimay` sirve embeddings de texto: trait `Provider`
//  + cosine entre vectores. Esta app es su reflejo bare-metal:
//
//    - El backend es el MOCK determinista (FNV-1a + LCG, copiado de
//      `rimay-verbo-mock` y aterrizado a `#![no_std]`). No hay daemon,
//      no hay socket, no hay descarga de modelo — todo cabe en la
//      memoria lineal del módulo.
//    - El producto observable es honesto: cosine(A, B) sobre vectores
//      hash-determinísticos. NO es similitud semántica. Sirve para
//      verificar el contrato (mismo texto → coseno 1.0, textos
//      distintos → coseno ≈ 0) sin que la app pretenda entender lengua.
//
//  Interacción:
//    - SPACE  : avanza al siguiente par de textos pre-baked
//    - ENTER  : vuelve al primero
//
//  La fuente es la misma mini-tipografía 5x7 de `asistente`, con un par
//  de glifos extra (=, /, comilla simple) para encajar las cadenas.
// =============================================================================

#![no_std]

// --- Las dos capacidades del kernel `renaser` que esta app necesita. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    /// Compone un búfer de píxeles (de ESTA memoria lineal) en la región
    /// que el kernel asignó a esta aplicación.
    fn sys_render_frame(ptr: u32, len: u32);
    /// Último scancode crudo del teclado, o 0 si la cola está vacía.
    fn sys_get_scancode() -> u32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometría del lienzo (debe coincidir con la región que asigna el
//     host: el kernel rechaza fotogramas de tamaño distinto). ---
const ANCHO: usize = 480;
const ALTO: usize = 560;
const BYTES_FOTOGRAMA: u32 = (ANCHO * ALTO * 4) as u32;

// --- Paleta. Los colores son u32 0x00RRGGBB porque el host espera
//     formato XRGB8888 (ver hello_wasm). ---
const FONDO: u32 = 0x0A_18_30;
const TITULO: u32 = 0xFF_B0_00;
const ETIQUETA: u32 = 0xA0_A0_B0;
const TEXTO: u32 = 0xFF_FF_FF;
const BARRA_BG: u32 = 0x30_30_40;
const BARRA_POS: u32 = 0x44_CC_66;
const BARRA_NEG: u32 = 0xCC_44_44;

// --- Lienzo en la memoria lineal del módulo. ---
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- Estado: índice del par activo + último scancode procesado para
//     filtrar repeticiones (igual idea que la "anti-rebote" de
//     asistente — sólo el flanco de subida cuenta). ---
static mut PAR_ACTUAL: usize = 0;
static mut SCANCODE_PREVIO: u32 = 0;

// --- Dimensión del vector de embedding. 64 es de sobra para una demo y
//     mantiene el cálculo barato (cosine sobre 64 f32 ≈ instantáneo). ---
const DIM: usize = 64;

// --- Pares de textos baked-in. Todos en mayúsculas porque la
//     mini-tipografía sólo trae mayúsculas (la misma decisión que
//     `asistente`). El par #2 es deliberadamente idéntico para que el
//     demo muestre cosine = 1.000. ---
const PARES: &[(&[u8], &[u8])] = &[
    (b"EL CONDOR CRUZA EL CIELO", b"THE CONDOR CROSSES THE SKY"),
    (b"RIMAY", b"RIMAY"),
    (b"LA MAR EN CALMA", b"FUNCION DE BESSEL DIVERGE"),
    (b"VERBO DETERMINISTA", b"PROVEEDOR DE EMBEDDINGS"),
    (b"WAWA OS", b"BARE METAL SASOS"),
];

#[no_mangle]
pub extern "C" fn init() {
    repintar();
}

#[no_mangle]
pub extern "C" fn tick() {
    let sc = unsafe { sys_get_scancode() };
    let prev = unsafe { SCANCODE_PREVIO };
    // Sólo el flanco de subida cuenta — si el host repite el mismo
    // scancode varios ticks, lo ignoramos hasta que cambie.
    if sc != prev {
        unsafe { SCANCODE_PREVIO = sc };
        match sc {
            0x39 => {
                // SPACE — siguiente par.
                unsafe {
                    PAR_ACTUAL = (PAR_ACTUAL + 1) % PARES.len();
                }
                repintar();
            }
            0x1C => {
                // ENTER — volver al primero.
                unsafe { PAR_ACTUAL = 0 };
                repintar();
            }
            _ => {}
        }
    }
    // Sin cambio: el lienzo no necesita revolcado — el host puede
    // re-componer el mismo fotograma. Pero los apps del repo siempre
    // vuelcan en cada tick, así que mantenemos el contrato.
    volcar();
}

// =============================================================================
//  Composición del fotograma
// =============================================================================

fn repintar() {
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    // Fondo.
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    // Título centrado (escala 3).
    let titulo = b"RIMAY :: VERBO";
    let titulo_ancho = ancho_texto(titulo, 3);
    dibujar_texto(lienzo, titulo, (ANCHO - titulo_ancho) / 2, 20, 3, TITULO);

    let par = unsafe { PAR_ACTUAL };
    let (a, b) = PARES[par];

    // Etiquetas + textos. Escala 2 para los textos, escala 1 para
    // etiquetas — quedan jerárquicamente distinguibles sin tipografía
    // de verdad.
    dibujar_texto(lienzo, b"TEXTO A", 30, 80, 2, ETIQUETA);
    dibujar_texto(lienzo, a, 30, 110, 2, TEXTO);

    dibujar_texto(lienzo, b"TEXTO B", 30, 170, 2, ETIQUETA);
    dibujar_texto(lienzo, b, 30, 200, 2, TEXTO);

    // Cálculo: el motor del demo. Dos hashes → dos vectores → coseno.
    let va = embed(a);
    let vb = embed(b);
    let cos = cosine(&va, &vb);

    dibujar_texto(lienzo, b"COSENO", 30, 270, 2, ETIQUETA);

    // Número escala 4 — 0.000 a 1.000 (o -1.000 a 1.000 cuando aplica).
    let mut buf = [b' '; 7];
    let n = formatear_cosine(cos, &mut buf);
    dibujar_texto(lienzo, &buf[..n], 30, 300, 4, TEXTO);

    // Barra: ancho del fill proporcional a |cosine|; color según signo.
    let bar_x = 30;
    let bar_y = 410;
    let bar_w = ANCHO - 60;
    let bar_h = 30;
    rellenar_rect(lienzo, bar_x, bar_y, bar_w, bar_h, BARRA_BG);
    let frac = if cos.is_nan() { 0.0 } else { cos.abs().min(1.0) };
    let fill_w = (frac * bar_w as f32) as usize;
    let color = if cos >= 0.0 { BARRA_POS } else { BARRA_NEG };
    if fill_w > 0 {
        rellenar_rect(lienzo, bar_x, bar_y, fill_w, bar_h, color);
    }

    // Pie con la convención del kernel: SPACE / ENTER.
    dibujar_texto(lienzo, b"SPACE SIGUIENTE  ENTER REINICIAR", 30, 510, 1, ETIQUETA);
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    // SEGURIDAD: el (ptr, len) describe nuestra propia memoria lineal y
    // el host lo verifica sin piedad.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, BYTES_FOTOGRAMA);
    }
}

// =============================================================================
//  Backend de embedding mock — copiado de `rimay-verbo-mock`
// -----------------------------------------------------------------------------
//  Misma FNV-1a + mismo LCG, sólo que pegado a `[f32; DIM]` en vez de
//  `Vec<f32>` para evitar alloc en la jaula WASM. La equivalencia
//  vectorial es bit-a-bit con el host para `text < 2^32 bytes` (es
//  decir, siempre).
// =============================================================================

fn fnv1a(text: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in text {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn embed(text: &[u8]) -> [f32; DIM] {
    let mut state = fnv1a(text);
    let mut out = [0.0f32; DIM];
    let mut i = 0;
    while i < DIM {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let unit = (state >> 40) as f32 / (1u64 << 24) as f32;
        out[i] = unit * 2.0 - 1.0;
        i += 1;
    }
    out
}

fn cosine(a: &[f32; DIM], b: &[f32; DIM]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    let mut i = 0;
    while i < DIM {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
        i += 1;
    }
    let denom = sqrt_aprox(na) * sqrt_aprox(nb);
    if denom == 0.0 {
        return 0.0;
    }
    (dot / denom).clamp(-1.0, 1.0)
}

/// Raíz cuadrada por Newton. En `wasm32-unknown-unknown` no hay
/// `f32::sqrt` sin libm; cinco iteraciones convergen sobrado para una
/// norma sobre 64 valores en [-1, 1].
fn sqrt_aprox(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut r = x;
    let mut i = 0;
    while i < 8 {
        r = 0.5 * (r + x / r);
        i += 1;
    }
    r
}

// =============================================================================
//  Formateo del coseno a ASCII
// -----------------------------------------------------------------------------
//  Coseno en [-1, 1]. Lo emitimos como "[-]N.NNN" (5-6 bytes), suficiente
//  para distinguir 1.000 de 0.999 de 0.000 visualmente.
// =============================================================================

fn formatear_cosine(cos: f32, buf: &mut [u8; 7]) -> usize {
    // Clamp duro: si llega algo raro (NaN), mostramos "0.000".
    let v = if cos.is_nan() { 0.0 } else { cos.clamp(-1.0, 1.0) };
    let negativo = v < 0.0;
    let abs = if negativo { -v } else { v };
    // Entero y tres decimales: redondeo a milésimas.
    let escala = (abs * 1000.0 + 0.5) as u32; // 0..=1000
    let entero = escala / 1000;
    let frac = escala % 1000;
    let d1 = (frac / 100) as u8;
    let d2 = ((frac / 10) % 10) as u8;
    let d3 = (frac % 10) as u8;

    let mut i = 0;
    if negativo {
        buf[i] = b'-';
        i += 1;
    }
    buf[i] = b'0' + entero as u8;
    i += 1;
    buf[i] = b'.';
    i += 1;
    buf[i] = b'0' + d1;
    i += 1;
    buf[i] = b'0' + d2;
    i += 1;
    buf[i] = b'0' + d3;
    i += 1;
    i
}

// =============================================================================
//  Mini-tipografía 5x7 — adaptada de `asistente`
// -----------------------------------------------------------------------------
//  Mismo formato (bits altos del byte por columna, 7 bytes por glifo),
//  los caracteres que la app necesita y nada más. Caracteres no
//  declarados caen al fallback `?` para que un teclazo rebelde no
//  pinte basura aleatoria.
// =============================================================================

const FA: usize = 5;
const FH: usize = 7;

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
        b'6' => [0x0E, 0x10, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x01, 0x0E],
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
        // Bloque sólido para cualquier byte fuera del mapa.
        _ => [0x1F; 7],
    }
}

fn ancho_texto(texto: &[u8], escala: usize) -> usize {
    if texto.is_empty() {
        return 0;
    }
    texto.len() * (FA + 1) * escala - escala
}

fn dibujar_texto(
    lienzo: &mut [u32],
    texto: &[u8],
    x: usize,
    y: usize,
    escala: usize,
    color: u32,
) {
    let mut cursor_x = x;
    for &c in texto {
        let g = glifo(c);
        let mut fila = 0;
        while fila < FH {
            let bits = g[fila];
            let mut col = 0;
            while col < FA {
                if bits & (1 << (FA - 1 - col)) != 0 {
                    let px0 = cursor_x + col * escala;
                    let py0 = y + fila * escala;
                    rellenar_rect(lienzo, px0, py0, escala, escala, color);
                }
                col += 1;
            }
            fila += 1;
        }
        cursor_x += (FA + 1) * escala;
    }
}

fn rellenar_rect(lienzo: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    let x1 = (x + w).min(ANCHO);
    let y1 = (y + h).min(ALTO);
    if x >= ANCHO || y >= ALTO {
        return;
    }
    let mut fila = y;
    while fila < y1 {
        let base = fila * ANCHO;
        let mut col = x;
        while col < x1 {
            lienzo[base + col] = color;
            col += 1;
        }
        fila += 1;
    }
}
