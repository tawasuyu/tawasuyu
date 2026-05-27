// =============================================================================
//  renaser :: apps/ide — Fase 28 :: el IDE semantico
// -----------------------------------------------------------------------------
//  Tres paneles internos sobre el lienzo natural 480x400, sin pretensiones
//  graficas: una fila por panel, cromo minimo, fuente 5x7 embebida.
//
//   Panel ALFA  (editor)         altura 200 px
//   Panel BETA  (AST / hash)     altura  90 px
//   Panel GAMMA (consola)        altura 110 px
//
//  El usuario teclea sobre un BUFFER lineal acotado a 256 bytes. Al pulsar:
//
//   F1  ->  sys_subsistema_registrar_ejecutable con un MODULO WASM MINIMO
//           hardcoded (`\0asm\1\0\0\0`). Demuestra el OK path: el hash
//           devuelto aparece en el Panel BETA y la consola dice "REGISTRADO".
//
//   F2  ->  sys_subsistema_registrar_ejecutable con el TEXTO QUE TECLEASTE.
//           El kernel rechaza con `PayloadInvalido` (-7) porque el texto
//           no tiene magia WebAssembly. La consola muestra el codigo y la
//           leccion: validacion semantica funcionando.
//
//   F3  ->  sys_object_datos sobre el ultimo hash registrado. Trae los
//           bytes de vuelta desde el grafo —demostracion del SWAP SEMANTICO:
//           el IDE no retiene el bytecode en su memoria lineal, lo aspira
//           del disco cuando lo necesita—. El primer byte recuperado se
//           muestra en la consola.
//
//  Las hotkeys evidencian el patron sin construir un parser real: el motor
//  del IDE de produccion vendra cuando un lexer/parser `#![no_std]`
//  porte tree-sitter o un equivalente al wasm32 cdylib.
// =============================================================================

#![no_std]

#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_subsistema_registrar_ejecutable(
        ptr: u32,
        len: u32,
        salida_hash_ptr: u32,
    ) -> i32;
    fn sys_object_datos(hash_ptr: u32, salida: u32, capacidad: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// =============================================================================
//  Geometria
// =============================================================================

const ANCHO: usize = 480;
const ALTO: usize = 400;

const ALFA_Y: usize = 24; // tras la cabecera global del IDE
const ALFA_ALTO: usize = 200;
const BETA_Y: usize = ALFA_Y + ALFA_ALTO + 4;
const BETA_ALTO: usize = 90;
const GAMMA_Y: usize = BETA_Y + BETA_ALTO + 4;
const GAMMA_ALTO: usize = ALTO - GAMMA_Y - 4;

// =============================================================================
//  Estado en estaticos — todo en la memoria lineal de la app
// =============================================================================

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];
static mut IDIOMA: u16 = 0;

const BUFFER_CAP: usize = 256;
static mut FUENTE: [u8; BUFFER_CAP] = [0; BUFFER_CAP];
static mut FUENTE_LEN: usize = 0;

/// Ultimo hash registrado con exito por F1 o (futuramente) F2. Cero si nadie
/// ha registrado nada todavia — para que el panel BETA muestre solo guiones.
static mut HASH_ULTIMO: [u8; 32] = [0; 32];
static mut HASH_VALIDO: bool = false;

/// Buffer de recuperacion de F3 — el byte que `sys_object_datos` trajo del
/// grafo y la longitud que recibimos.
static mut RECUPERADO: [u8; 32] = [0; 32];
static mut RECUPERADO_LEN: usize = 0;

/// Codigo de la ultima syscall importante; el panel GAMMA lo rotula como
/// el "estado del compilador".
static mut ULTIMO_CODIGO: i32 = i32::MIN;
/// Etiqueta que acompaña al codigo: "F1 ::", "F2 ::", "F3 ::". Sin alocar.
static mut ULTIMA_ETIQUETA: [u8; 8] = *b"INICIO  ";

// =============================================================================
//  Hotkeys + flancos
// =============================================================================

static mut F1_PREV: bool = false;
static mut F2_PREV: bool = false;
static mut F3_PREV: bool = false;
/// Estado anterior del scancode generico, para detectar flancos de tecla
/// (cualquier press se considera "ENTRADA NUEVA").
static mut SCAN_PREV: u32 = 0;

/// Modulo WASM minimo VALIDO: magia `\0asm` + version 0x01_00_00_00. Sin
/// secciones — wasmi lo aceptaria como modulo vacio en tiempo de instanciar,
/// pero `almacen::almacenar` solo necesita que sea bien formado a nivel
/// de cabecera. El IDE lo lleva hardcoded como "test fixture" del registro.
const WASM_MINIMO: [u8; 8] = [0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];

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
    if scancode != 0 && scancode != unsafe { SCAN_PREV } {
        atender_scancode(scancode);
    }
    unsafe { SCAN_PREV = scancode };

    pintar();
    volcar();
}

fn refrescar_contexto() {
    let idioma = unsafe { sys_config_idioma() } as u16;
    unsafe { IDIOMA = idioma };
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

// =============================================================================
//  Manejo del teclado
// =============================================================================

fn atender_scancode(scancode: u32) {
    // Hotkeys de funcion primero — tienen prioridad y NO se incorporan al
    // buffer de texto.
    match scancode {
        0x3B => {
            if !unsafe { F1_PREV } {
                accion_registrar_modulo_minimo();
            }
            unsafe { F1_PREV = true };
            return;
        }
        0x3C => {
            if !unsafe { F2_PREV } {
                accion_registrar_texto_buffer();
            }
            unsafe { F2_PREV = true };
            return;
        }
        0x3D => {
            if !unsafe { F3_PREV } {
                accion_recuperar_ultimo();
            }
            unsafe { F3_PREV = true };
            return;
        }
        // Make codes "break" (bit alto puesto) NO los vemos —el driver
        // PS/2 del kernel solo entrega make codes y break codes con 0x80—.
        // Los flancos los gestionamos por nuestra cuenta abajo.
        _ => {}
    }

    // Resetear flancos de hotkeys al recibir cualquier OTRA tecla — asi
    // pulsar F1 dos veces (con otra tecla en medio) cuenta como dos.
    unsafe {
        F1_PREV = scancode == 0x3B;
        F2_PREV = scancode == 0x3C;
        F3_PREV = scancode == 0x3D;
    }

    // Edicion: backspace, enter, o letra/digito/espacio.
    let escribir = match scancode {
        0x0E => {
            // Backspace.
            if unsafe { FUENTE_LEN } > 0 {
                unsafe { FUENTE_LEN -= 1 };
            }
            return;
        }
        0x1C => Some(b'\n'),
        0x39 => Some(b' '),
        _ => mapear_letra_o_digito(scancode as u8),
    };
    if let Some(b) = escribir {
        let mut len = unsafe { FUENTE_LEN };
        if len < BUFFER_CAP {
            unsafe {
                FUENTE[len] = b;
            }
            len += 1;
            unsafe { FUENTE_LEN = len };
        }
    }
}

/// Mapea los make codes mas comunes de PS/2 set 1 a ASCII en minusculas.
/// Solo los caracteres que un editor minimo necesita; el resto se ignora.
fn mapear_letra_o_digito(scan: u8) -> Option<u8> {
    Some(match scan {
        0x02 => b'1',
        0x03 => b'2',
        0x04 => b'3',
        0x05 => b'4',
        0x06 => b'5',
        0x07 => b'6',
        0x08 => b'7',
        0x09 => b'8',
        0x0A => b'9',
        0x0B => b'0',
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1E => b'a',
        0x1F => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        _ => return None,
    })
}

// =============================================================================
//  Acciones de las hotkeys
// =============================================================================

fn accion_registrar_modulo_minimo() {
    let codigo = unsafe {
        sys_subsistema_registrar_ejecutable(
            WASM_MINIMO.as_ptr() as u32,
            WASM_MINIMO.len() as u32,
            core::ptr::addr_of_mut!(HASH_ULTIMO) as u32,
        )
    };
    unsafe {
        ULTIMO_CODIGO = codigo;
        ULTIMA_ETIQUETA = *b"F1 MIN  ";
        HASH_VALIDO = codigo == 0;
    }
}

fn accion_registrar_texto_buffer() {
    let len = unsafe { FUENTE_LEN };
    if len == 0 {
        unsafe {
            ULTIMO_CODIGO = -7; // PayloadInvalido sin tocar el kernel
            ULTIMA_ETIQUETA = *b"F2 VACIO";
        }
        return;
    }
    let codigo = unsafe {
        sys_subsistema_registrar_ejecutable(
            core::ptr::addr_of!(FUENTE) as u32,
            len as u32,
            core::ptr::addr_of_mut!(HASH_ULTIMO) as u32,
        )
    };
    unsafe {
        ULTIMO_CODIGO = codigo;
        ULTIMA_ETIQUETA = *b"F2 TXT  ";
        if codigo == 0 {
            HASH_VALIDO = true;
        }
    }
}

fn accion_recuperar_ultimo() {
    if !unsafe { HASH_VALIDO } {
        unsafe {
            ULTIMO_CODIGO = -1; // Ausente — no hay hash que recuperar
            ULTIMA_ETIQUETA = *b"F3 SHASH";
        }
        return;
    }
    let codigo = unsafe {
        sys_object_datos(
            core::ptr::addr_of!(HASH_ULTIMO) as u32,
            core::ptr::addr_of_mut!(RECUPERADO) as u32,
            32,
        )
    };
    unsafe {
        ULTIMO_CODIGO = codigo;
        ULTIMA_ETIQUETA = *b"F3 GET  ";
        if codigo > 0 {
            RECUPERADO_LEN = codigo as usize;
        }
    }
}

// =============================================================================
//  Pintado de los tres paneles
// =============================================================================

fn pintar() {
    let paleta = unsafe { PALETA };
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    let fondo = color_u32(paleta, 2);
    let tinta = color_u32(paleta, 3);
    let acento = color_u32(paleta, 4);
    let secundario = color_u32(paleta, 1);

    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, fondo);

    // Cabecera global con titulo y la firma del idioma activo.
    rellenar_rect(lienzo, 0, 0, ANCHO, ALFA_Y - 4, secundario);
    dibujar_texto(lienzo, b"IDE WAWA  FASE 28", 8, 6, 2, tinta);
    rellenar_rect(lienzo, 0, ALFA_Y - 4, ANCHO, 2, acento);

    pintar_panel_alfa(lienzo, tinta, acento);
    pintar_panel_beta(lienzo, tinta, acento, secundario);
    pintar_panel_gamma(lienzo, tinta, acento, secundario);
}

fn pintar_panel_alfa(lienzo: &mut [u32], tinta: u32, acento: u32) {
    // Marquito y rotulo.
    dibujar_texto(lienzo, b"ALFA  EDITOR", 8, ALFA_Y, 1, acento);
    rellenar_rect(lienzo, 0, ALFA_Y + 10, ANCHO, 1, acento);

    // Render del buffer FUENTE con wrap simple: 5x7 a escala 2 = 10 px ancho
    // por caracter, ~46 caracteres por linea de 460 px utiles.
    let escala = 2usize;
    let avance = 6 * escala;
    let cols = (ANCHO - 16) / avance;
    let mut linea_y = ALFA_Y + 16;
    let mut cursor_col = 0usize;
    let len = unsafe { FUENTE_LEN };
    let mut buf_local = [b' '; 64]; // bufer de UNA linea, escrito sobre pila.
    let mut buf_len = 0usize;
    let comprimir_y_pintar = |lienzo: &mut [u32], y: usize, buf: &[u8]| {
        dibujar_texto(lienzo, buf, 8, y, escala, tinta);
    };
    for i in 0..len {
        let c = unsafe { FUENTE[i] };
        let salto = c == b'\n' || cursor_col >= cols || buf_len == buf_local.len();
        if salto {
            comprimir_y_pintar(lienzo, linea_y, &buf_local[..buf_len]);
            linea_y += 12;
            buf_len = 0;
            cursor_col = 0;
            if linea_y + 10 > ALFA_Y + ALFA_ALTO {
                return;
            }
            if c == b'\n' {
                continue;
            }
        }
        if c.is_ascii_alphanumeric() || c == b' ' {
            // La mini-fuente solo cubre mayusculas; subimos minusculas.
            buf_local[buf_len] = c.to_ascii_uppercase();
            buf_len += 1;
            cursor_col += 1;
        }
    }
    if buf_len > 0 {
        // Cursor: un bloque solido al final de la ultima linea.
        comprimir_y_pintar(lienzo, linea_y, &buf_local[..buf_len]);
    }
    // Bloque-cursor parpadeante: dibujado siempre por simplicidad. El
    // parpadeo lo daria un timer; aqui es un cuadrado fijo.
    let cursor_x = 8 + buf_len * avance;
    rellenar_rect(lienzo, cursor_x, linea_y, 2 * escala, 7 * escala, acento);
}

fn pintar_panel_beta(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"BETA  HASH AST", 8, BETA_Y, 1, acento);
    rellenar_rect(lienzo, 0, BETA_Y + 10, ANCHO, 1, acento);

    // Pintar el hash del ultimo objeto registrado como 32 bytes hex sobre
    // dos lineas, con cromo discreto de fondo. Si no hay hash, guiones.
    let hash = unsafe { HASH_ULTIMO };
    let valido = unsafe { HASH_VALIDO };
    let mut hex = [0u8; 64];
    for i in 0..32 {
        let b = if valido { hash[i] } else { 0 };
        hex[i * 2] = nibble_hex(b >> 4);
        hex[i * 2 + 1] = nibble_hex(b & 0x0F);
    }
    if !valido {
        for slot in hex.iter_mut() {
            *slot = b'-';
        }
    }
    rellenar_rect(lienzo, 8, BETA_Y + 18, ANCHO - 16, 28, color_atenuar_u32(secundario, 0xC0));
    dibujar_texto(lienzo, &hex[..32], 12, BETA_Y + 22, 1, tinta);
    dibujar_texto(lienzo, &hex[32..], 12, BETA_Y + 34, 1, tinta);

    // Etiqueta inferior: ¿hay arbol AST? Por ahora, hash unico = unica
    // hoja del arbol. Cuando llegue el parser, se mostraran tantos hashes
    // como bloques sintacticos haya en el grafo.
    let nota: &[u8] = if valido {
        b"AST  1 HOJA  HASH UNICO REGISTRADO"
    } else {
        b"AST  VACIO  REGISTRA UN MODULO CON F1"
    };
    dibujar_texto(lienzo, nota, 8, BETA_Y + 60, 1, acento);
}

fn pintar_panel_gamma(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"GAMMA  CONSOLA", 8, GAMMA_Y, 1, acento);
    rellenar_rect(lienzo, 0, GAMMA_Y + 10, ANCHO, 1, acento);

    rellenar_rect(lienzo, 8, GAMMA_Y + 16, ANCHO - 16, 12, color_atenuar_u32(secundario, 0xC0));

    // Linea 1: etiqueta del ultimo gesto + codigo.
    let etiqueta = unsafe { ULTIMA_ETIQUETA };
    let codigo = unsafe { ULTIMO_CODIGO };
    let mut linea1 = [b' '; 32];
    linea1[..8].copy_from_slice(&etiqueta);
    linea1[8] = b' ';
    let (dec, dlen) = formatear_i32(codigo);
    let cola = b"CODIGO  ";
    linea1[9..9 + cola.len()].copy_from_slice(cola);
    let mut n = 9 + cola.len();
    for &c in &dec[..dlen] {
        if n < linea1.len() {
            linea1[n] = c;
            n += 1;
        }
    }
    dibujar_texto(lienzo, &linea1[..n], 12, GAMMA_Y + 18, 1, tinta);

    // Linea 2: leyenda humana del codigo.
    let leyenda: &[u8] = match codigo {
        0 => b"OK  REGISTRADO EN GRAFO",
        -1 => b"AUSENTE  OBJETO NO HALLADO",
        -2 => b"CAPACIDAD INSUFICIENTE",
        -3 => b"ALMACENAMIENTO FALLO",
        -4 => b"SIN FOCO",
        -5 => b"ENVIO FALLO",
        -6 => b"SATURADO  REINTENTAR PROXIMO TICK",
        -7 => b"PAYLOAD INVALIDO  MAGIA WASM AJENA",
        x if x > 0 => b"OK  BYTES COPIADOS",
        _ => b"INICIO  PULSA F1 PARA REGISTRAR",
    };
    dibujar_texto(lienzo, leyenda, 12, GAMMA_Y + 32, 1, tinta);

    // Linea 3 (opcional): primer byte recuperado por F3.
    let rl = unsafe { RECUPERADO_LEN };
    if rl > 0 {
        let mut linea3 = [b' '; 32];
        let prefix = b"BYTE0  ";
        linea3[..prefix.len()].copy_from_slice(prefix);
        let b0 = unsafe { RECUPERADO[0] };
        linea3[prefix.len()] = nibble_hex(b0 >> 4);
        linea3[prefix.len() + 1] = nibble_hex(b0 & 0x0F);
        let len_total = prefix.len() + 2;
        dibujar_texto(lienzo, &linea3[..len_total], 12, GAMMA_Y + 46, 1, acento);
    }

    // Pie con hotkeys.
    let hotkeys = b"F1 MODULO MIN   F2 TEXTO   F3 RECUPERA   BS DEL   ENTER";
    dibujar_texto(lienzo, hotkeys, 8, GAMMA_Y + GAMMA_ALTO - 10, 1, acento);
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

fn color_atenuar_u32(color: u32, factor: u32) -> u32 {
    let b = (color & 0xFF) * factor >> 8;
    let g = ((color >> 8) & 0xFF) * factor >> 8;
    let r = ((color >> 16) & 0xFF) * factor >> 8;
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

fn nibble_hex(n: u8) -> u8 {
    if n < 10 {
        b'0' + n
    } else {
        b'A' + (n - 10)
    }
}

fn formatear_i32(n: i32) -> ([u8; 12], usize) {
    let mut buf = [0u8; 12];
    if n == 0 {
        buf[0] = b'0';
        return (buf, 1);
    }
    let (mut absoluto, negativo) = if n < 0 {
        (n.unsigned_abs(), true)
    } else {
        (n as u32, false)
    };
    let mut digits = [0u8; 11];
    let mut nd = 0;
    while absoluto > 0 && nd < digits.len() {
        digits[nd] = b'0' + (absoluto % 10) as u8;
        absoluto /= 10;
        nd += 1;
    }
    let mut out = 0;
    if negativo {
        buf[out] = b'-';
        out += 1;
    }
    while nd > 0 {
        nd -= 1;
        buf[out] = digits[nd];
        out += 1;
    }
    (buf, out)
}

// =============================================================================
//  Mini-tipografia 5x7 (mayusculas, digitos, simbolos comunes)
// =============================================================================

const FA: usize = 5;
const FH: usize = 7;
const FAV: usize = 6;

fn glifo(c: u8) -> [u8; FH] {
    match c {
        b' ' => [0; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b'/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        b'?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
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
        cursor_x += FAV * escala;
    }
}
