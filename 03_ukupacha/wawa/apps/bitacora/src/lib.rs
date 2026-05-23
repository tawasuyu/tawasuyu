// =============================================================================
//  renaser :: apps/bitacora — Fase 17 :: un editor que recuerda
// -----------------------------------------------------------------------------
//  La fase 7c le dio a las apps memoria mas alla del arranque: `sys_estado_*`
//  ancla la huella de un app en el grafo, y al reiniciar el kernel se la
//  devuelve. `memoriosa` lo demostro contando teclas. `bitacora` lo lleva al
//  siguiente paso natural: ofrecer un editor de texto.
//
//  La pantalla muestra un titulo en indigo y debajo el texto que el usuario va
//  tecleando, con salto de linea automatico al llegar al margen y con `Enter`.
//  Backspace borra el ultimo. Cada cambio se persiste de inmediato, asi que la
//  apagada brusca no pierde nada — la proxima vida del kernel retoma exacto.
//
//  Tipografia: la 8x8 clasica (font8x8), escalada x2 a 16x16. Cabe en su propia
//  memoria lineal y se renderiza pixel a pixel — el app no toca el lienzo del
//  kernel, solo entrega su propio fotograma.
// =============================================================================

#![no_std]

use font8x8::legacy::BASIC_LEGACY;

#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_estado_cargar(salida: u32, capacidad: u32) -> i32;
    fn sys_estado_guardar(datos: u32, datos_len: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria ----------------------------------------------------------------

/// Tamaño del lienzo natural — debe coincidir con `region` del manifiesto.
const ANCHO: usize = 480;
const ALTO: usize = 280;
/// Pixeles por celda de glifo (8x8 escalado x2).
const PASO: usize = 16;
/// Margen horizontal para el cuerpo del texto.
const MARGEN_X: usize = 16;
/// Y de la linea base del titulo.
const Y_LABEL: usize = 6;
/// Y de la linea base de la primera fila de texto.
const Y_TEXTO: usize = 38;
/// Cuantas columnas caben.
const COLUMNAS: usize = (ANCHO - 2 * MARGEN_X) / PASO;
/// Cuantas filas caben bajo el titulo.
const FILAS: usize = (ALTO - Y_TEXTO) / PASO;

// --- Estado -------------------------------------------------------------------

/// Capacidad del buffer de texto. Al desbordarse se descarta una porcion del
/// principio (el texto mas viejo) para dejar sitio al nuevo — un cuaderno con
/// memoria finita, no un agujero negro.
const CAPACIDAD: usize = 512;

const FONDO: u32 = 0x0A_18_30;
const TINTA: u32 = 0xE8_EC_F4;
const ETIQUETA: u32 = 0x8B_5C_F6;

static mut BUFFER: [u8; CAPACIDAD] = [0; CAPACIDAD];
static mut LEN: usize = 0;
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- ABI del userspace --------------------------------------------------------

#[no_mangle]
pub extern "C" fn init() {
    // Cargar el texto persistido — si no hay nada, `n` es 0 y empezamos vacios.
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(BUFFER) };
    // SEGURIDAD: `sys_estado_cargar` es una capacidad del host; (ptr, len) cae
    // dentro de nuestra propia memoria lineal y el host lo valida sin piedad.
    let n = unsafe { sys_estado_cargar(buffer.as_mut_ptr() as u32, CAPACIDAD as u32) };
    if n > 0 {
        // SEGURIDAD: lectura/escritura escalar; LEN es nuestro propio cursor.
        unsafe {
            LEN = (n as usize).min(CAPACIDAD);
        }
    }
    pintar();
}

#[no_mangle]
pub extern "C" fn tick() {
    let mut cambio = false;
    // Drenar TODOS los scancodes acumulados desde el ultimo fotograma. La cola
    // es propia de este app — la inscribio la fase 5 en la IRQ1 — asi que
    // mirarla aqui no le quita nada a nadie.
    loop {
        let sc = unsafe { sys_get_scancode() } as u8;
        if sc == 0 {
            break;
        }
        if sc & 0x80 != 0 {
            // Codigo de KEY-UP (release). Lo ignoramos: tecleamos al pulsar.
            continue;
        }
        match sc {
            0x0E => {
                // Backspace — borrar el ultimo caracter, si lo hay.
                unsafe {
                    if LEN > 0 {
                        LEN -= 1;
                        cambio = true;
                    }
                }
            }
            0x1C => {
                // Enter — salto de linea explicito.
                anexar(b'\n');
                cambio = true;
            }
            otro => {
                let c = scancode_a_caracter(otro);
                if c != 0 {
                    anexar(c);
                    cambio = true;
                }
            }
        }
    }
    if cambio {
        guardar();
    }
    pintar();
}

// --- Estado: buffer -----------------------------------------------------------

/// Anexa un caracter al final del buffer. Si el buffer esta lleno descarta los
/// 64 primeros bytes para hacer hueco (amortiza el coste; no es una mudanza por
/// cada pulsacion).
fn anexar(c: u8) {
    unsafe {
        if LEN >= CAPACIDAD {
            let buffer = &mut *core::ptr::addr_of_mut!(BUFFER);
            buffer.copy_within(64.., 0);
            LEN = CAPACIDAD - 64;
        }
        let buffer = &mut *core::ptr::addr_of_mut!(BUFFER);
        buffer[LEN] = c;
        LEN += 1;
    }
}

/// Persiste el buffer en el grafo. La huella sobrevive a la siguiente arrancada.
fn guardar() {
    unsafe {
        let buffer = &*core::ptr::addr_of!(BUFFER);
        // SEGURIDAD: (ptr, len) describe nuestra propia memoria; el host lo
        // verifica y nunca lee fuera del rango entregado.
        let _ = sys_estado_guardar(buffer.as_ptr() as u32, LEN as u32);
    }
}

// --- Renderizado --------------------------------------------------------------

fn pintar() {
    let lienzo = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    // Fondo limpio.
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }
    // Titulo.
    pintar_texto(lienzo, b"bitacora :: el texto persiste", MARGEN_X, Y_LABEL, ETIQUETA);
    // Linea sutil bajo el titulo.
    let y_linea = Y_LABEL + PASO + 4;
    for x in MARGEN_X..(ANCHO - MARGEN_X) {
        lienzo[y_linea * ANCHO + x] = ETIQUETA;
        lienzo[(y_linea + 1) * ANCHO + x] = ETIQUETA;
    }

    // Cuerpo: mostrar las ultimas `FILAS` lineas del buffer, con wrap en
    // `COLUMNAS`. Dos pasadas para saltarse las filas viejas con elegancia.
    let buffer = unsafe { &*core::ptr::addr_of!(BUFFER) };
    let len = unsafe { LEN };

    // Pasada 1: contar filas totales (con wrap).
    let mut filas_total = 1usize;
    let mut col = 0usize;
    for i in 0..len {
        let c = buffer[i];
        if c == b'\n' {
            filas_total += 1;
            col = 0;
        } else {
            if col >= COLUMNAS {
                filas_total += 1;
                col = 0;
            }
            col += 1;
        }
    }
    let skip = filas_total.saturating_sub(FILAS);

    // Pasada 2: renderizar solo a partir de la fila `skip`.
    let mut fila_actual = 0usize;
    let mut col2 = 0usize;
    for i in 0..len {
        let c = buffer[i];
        if c == b'\n' {
            fila_actual += 1;
            col2 = 0;
            continue;
        }
        if col2 >= COLUMNAS {
            fila_actual += 1;
            col2 = 0;
        }
        if fila_actual >= skip {
            let rfila = fila_actual - skip;
            if rfila < FILAS {
                let x = MARGEN_X + col2 * PASO;
                let y = Y_TEXTO + rfila * PASO;
                pintar_glifo(lienzo, c, x, y, TINTA);
            }
        }
        col2 += 1;
    }

    // SEGURIDAD: `sys_render_frame` valida (ptr, len) contra nuestra memoria.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

/// Pinta una cadena ASCII en (x, y_base), avanzando un PASO por glifo. Se
/// detiene si el siguiente glifo no cabria dentro del lienzo.
fn pintar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, color: u32) {
    let mut cx = x;
    for &c in texto {
        if cx + PASO > ANCHO {
            break;
        }
        pintar_glifo(lienzo, c, cx, y, color);
        cx += PASO;
    }
}

/// Pinta un solo glifo 8x8 escalado a 16x16 en (x, y). Los caracteres no ASCII
/// se renderizan como `?`.
fn pintar_glifo(lienzo: &mut [u32], c: u8, x: usize, y: usize, color: u32) {
    let glifo = if (c as usize) < 128 {
        BASIC_LEGACY[c as usize]
    } else {
        BASIC_LEGACY[b'?' as usize]
    };
    for row in 0..8 {
        for col in 0..8 {
            if glifo[row] & (1 << col) != 0 {
                let px = x + col * 2;
                let py = y + row * 2;
                if px + 1 >= ANCHO || py + 1 >= ALTO {
                    continue;
                }
                lienzo[py * ANCHO + px] = color;
                lienzo[py * ANCHO + px + 1] = color;
                lienzo[(py + 1) * ANCHO + px] = color;
                lienzo[(py + 1) * ANCHO + px + 1] = color;
            }
        }
    }
}

// --- Teclado: scancode -> caracter --------------------------------------------

/// Traduce un MAKE-code del set 1 (US layout) a su caracter ASCII en minuscula.
/// Devuelve 0 para los scancodes que no producen texto — modificadores,
/// extendidos, etc.: el llamante los descarta sin gritar.
fn scancode_a_caracter(sc: u8) -> u8 {
    match sc {
        0x02 => b'1', 0x03 => b'2', 0x04 => b'3', 0x05 => b'4', 0x06 => b'5',
        0x07 => b'6', 0x08 => b'7', 0x09 => b'8', 0x0A => b'9', 0x0B => b'0',
        0x10 => b'q', 0x11 => b'w', 0x12 => b'e', 0x13 => b'r', 0x14 => b't',
        0x15 => b'y', 0x16 => b'u', 0x17 => b'i', 0x18 => b'o', 0x19 => b'p',
        0x1E => b'a', 0x1F => b's', 0x20 => b'd', 0x21 => b'f', 0x22 => b'g',
        0x23 => b'h', 0x24 => b'j', 0x25 => b'k', 0x26 => b'l',
        0x2C => b'z', 0x2D => b'x', 0x2E => b'c', 0x2F => b'v', 0x30 => b'b',
        0x31 => b'n', 0x32 => b'm',
        0x33 => b',', 0x34 => b'.', 0x35 => b'/',
        0x27 => b';', 0x28 => b'\'',
        0x39 => b' ',
        _ => 0,
    }
}
