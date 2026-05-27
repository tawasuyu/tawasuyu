// =============================================================================
//  renaser :: apps/ide — Fase 28/29 :: IDE semantico con emisor Forth->WASM
// -----------------------------------------------------------------------------
//  El IDE habita en su jaula de 4 MiB y trata el codigo como NODOS DEL GRAFO
//  direccionado por contenido. Tres paneles internos sobre el lienzo natural
//  480x400 (ALFA editor / BETA AST / GAMMA consola).
//
//  FASE 29 :: F1 deja de registrar un modulo hardcoded; ahora COMPILA en
//  caliente el buffer del editor con un emisor Forth->WASM `#![no_std]`
//  zero-alloc. Reconoce expresiones de pila simples ("5 10 +") y produce
//  un modulo WebAssembly entero (magic, type, func, export, code) sobre
//  buffers en pila. Si la sintaxis es correcta el IDE graba el texto
//  fuente como objeto del grafo (sys_object_put -> HASH_FUENTE) y a
//  continuacion registra el binario (sys_subsistema_registrar_ejecutable
//  -> HASH_BINARIO). BETA muestra los DOS hashes apilados, conectados por
//  una linea acento vertical: el grafo semantico explicito.
//
//  Hotkeys:
//
//   F1  ->  Compila el texto del editor, graba fuente, registra binario.
//           Camino feliz: GAMMA dice "OK BINARIO EMITIDO" y BETA actualiza.
//           Camino de error: GAMMA muestra PayloadInvalido (-7).
//
//   F2  ->  Registra el TEXTO TECLEADO crudo (sin compilar). El kernel
//           rechaza con PayloadInvalido porque el texto no es WASM.
//           Demo de la validacion semantica del syscall.
//
//   F3  ->  sys_object_datos sobre el ultimo binario registrado.
//           Demuestra el SWAP SEMANTICO: el IDE no retiene el bytecode
//           en su memoria lineal; lo aspira del disco cuando lo necesita.
// =============================================================================

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_object_put(
        datos_ptr: u32,
        datos_len: u32,
        hijos_ptr: u32,
        hijos_cnt: u32,
        salida: u32,
    ) -> i32;
    fn sys_object_datos(hash_ptr: u32, salida: u32, capacidad: u32) -> i32;
    fn sys_subsistema_registrar_ejecutable(
        ptr: u32,
        len: u32,
        salida_hash_ptr: u32,
    ) -> i32;
    /// FASE 31 :: vincula causa->efecto. El kernel inscribe `padre_hash`
    /// como el PRIMER HIJO del nodo binario; la arista queda materializada
    /// en el grafo direccionado por contenido sin que el userspace tenga
    /// que escribir dos veces.
    fn sys_subsistema_registrar_ejecutable_v2(
        ptr: u32,
        len: u32,
        padre_hash_ptr: u32,
        salida_hash_ptr: u32,
    ) -> i32;
}

#[cfg(not(test))]
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// =============================================================================
//  Geometria
// =============================================================================

const ANCHO: usize = 480;
const ALTO: usize = 400;

const ALFA_Y: usize = 24;
const ALFA_ALTO: usize = 200;
const BETA_Y: usize = ALFA_Y + ALFA_ALTO + 4;
const BETA_ALTO: usize = 90;
const GAMMA_Y: usize = BETA_Y + BETA_ALTO + 4;
const GAMMA_ALTO: usize = ALTO - GAMMA_Y - 4;

// =============================================================================
//  Estado en estaticos
// =============================================================================

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];
static mut IDIOMA: u16 = 0;

const BUFFER_CAP: usize = 256;
static mut FUENTE: [u8; BUFFER_CAP] = [0; BUFFER_CAP];
static mut FUENTE_LEN: usize = 0;

/// Hash del objeto FUENTE (texto del editor) tras un F1 exitoso. Cero si
/// nadie ha compilado aun.
static mut HASH_FUENTE: [u8; 32] = [0; 32];
static mut HASH_FUENTE_VALIDO: bool = false;

/// Hash del objeto BINARIO (modulo WASM materializado) tras un F1 exitoso.
static mut HASH_BINARIO: [u8; 32] = [0; 32];
static mut HASH_BINARIO_VALIDO: bool = false;

/// FASE 31 :: la ARISTA CAUSAL esta SINCRONIZADA cuando el binario y la
/// fuente del editor siguen siendo el mismo par que se inscribio. Si el
/// usuario teclea (o borra) un solo caracter despues de F1, la sincronia
/// se rompe: el conector vertical entre FUENTE y BINARIO en el Panel BETA
/// se desvanece al gris mate, demostrando en tiempo real que la causa
/// escrita y el efecto ejecutable han divergido.
static mut ARISTA_SINCRONIZADA: bool = false;

/// Buffer de recuperacion de F3.
static mut RECUPERADO: [u8; 32] = [0; 32];
static mut RECUPERADO_LEN: usize = 0;

static mut ULTIMO_CODIGO: i32 = i32::MIN;
static mut ULTIMA_ETIQUETA: [u8; 8] = *b"INICIO  ";

static mut F1_PREV: bool = false;
static mut F2_PREV: bool = false;
static mut F3_PREV: bool = false;
static mut SCAN_PREV: u32 = 0;

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
//  Teclado
// =============================================================================

fn atender_scancode(scancode: u32) {
    match scancode {
        0x3B => {
            if !unsafe { F1_PREV } {
                accion_compilar();
            }
            unsafe { F1_PREV = true };
            return;
        }
        0x3C => {
            if !unsafe { F2_PREV } {
                accion_registrar_texto_crudo();
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
        _ => {}
    }
    unsafe {
        F1_PREV = scancode == 0x3B;
        F2_PREV = scancode == 0x3C;
        F3_PREV = scancode == 0x3D;
    }

    let escribir = match scancode {
        0x0E => {
            if unsafe { FUENTE_LEN } > 0 {
                unsafe { FUENTE_LEN -= 1 };
                // Romper la arista causal: la fuente ya no coincide con la
                // que engendro el binario inscrito en el grafo.
                unsafe { ARISTA_SINCRONIZADA = false };
            }
            return;
        }
        0x1C => Some(b'\n'),
        0x39 => Some(b' '),
        _ => mapear_caracter(scancode as u8),
    };
    if let Some(b) = escribir {
        let mut len = unsafe { FUENTE_LEN };
        if len < BUFFER_CAP {
            unsafe {
                FUENTE[len] = b;
            }
            len += 1;
            unsafe { FUENTE_LEN = len };
            // Romper la arista causal: cualquier edicion posterior a F1
            // divorcia la causa escrita del efecto ejecutable.
            unsafe { ARISTA_SINCRONIZADA = false };
        }
    }
}

/// Mapeo de scancodes PS/2 set 1 a ASCII. Cubre la fila principal de
/// letras + digitos, y todo el numpad (incluyendo sus operadores). El
/// numpad es la VIA POR DEFECTO de introducir `+ - *` sin Shift, lo que
/// hace que la expresion Forth "5 10 +" sea componible en cualquier
/// teclado fisico sin tener que captar modificadores.
fn mapear_caracter(scan: u8) -> Option<u8> {
    Some(match scan {
        // Fila principal: digitos y letras minusculas.
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
        0x0C => b'-', // tecla '-' unshifted en la fila de digitos
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
        // Numpad: digitos.
        0x47 => b'7',
        0x48 => b'8',
        0x49 => b'9',
        0x4B => b'4',
        0x4C => b'5',
        0x4D => b'6',
        0x4F => b'1',
        0x50 => b'2',
        0x51 => b'3',
        0x52 => b'0',
        // Numpad: operadores Forth.
        0x37 => b'*',
        0x4A => b'-',
        0x4E => b'+',
        _ => return None,
    })
}

// =============================================================================
//  Emisor Forth -> WASM (Fase 29 / Fase 30)
// -----------------------------------------------------------------------------
//  Fase 30 :: el motor de tokenizacion y emision LEB128 vive ahora en
//  la crate aislada `shared/forth-emisor`, donde el toolchain del host
//  audita su contrato con una suite de tests nativos. El IDE solo
//  consume `ForthCompiler::compilar_bytes(&buffer, &mut salida)` —el
//  comportamiento es identico al de la Fase 29 pero la lectura del
//  codigo cabe ahora en una crate por separado y la deuda formal queda
//  saldada.
// =============================================================================

// `forth_emisor` re-exportado para el wrapper local que conserva la
// interfaz que ya usa el resto del IDE (`emisor::compilar(...)`). Eso
// nos ahorra tocar las dos invocaciones de `accion_*`.
mod emisor {
    use forth_emisor::ForthCompiler;
    pub fn compilar(fuente: &[u8], destino: &mut [u8]) -> Option<usize> {
        ForthCompiler::compilar_bytes(fuente, destino)
    }
}


// =============================================================================
//  Acciones de las hotkeys
// =============================================================================

fn accion_compilar() {
    let len = unsafe { FUENTE_LEN };
    if len == 0 {
        unsafe {
            ULTIMO_CODIGO = -7;
            ULTIMA_ETIQUETA = *b"F1 VACIO";
            ARISTA_SINCRONIZADA = false;
        }
        return;
    }
    // Compilar Forth -> WASM en un buffer en pila.
    let fuente = unsafe { &FUENTE[..len] };
    let mut binario = [0u8; 768];
    let bin_len = match emisor::compilar(fuente, &mut binario) {
        Some(n) => n,
        None => {
            unsafe {
                ULTIMO_CODIGO = -7;
                ULTIMA_ETIQUETA = *b"F1 EMSR ";
                HASH_FUENTE_VALIDO = false;
                HASH_BINARIO_VALIDO = false;
                ARISTA_SINCRONIZADA = false;
            }
            return;
        }
    };

    // 1) Grabar el TEXTO FUENTE como objeto del grafo. Sin hijos.
    //    Devuelve HASH_FUENTE — la causa direccionada por contenido.
    let codigo_fuente = unsafe {
        sys_object_put(
            core::ptr::addr_of!(FUENTE) as u32,
            len as u32,
            0u32,
            0u32,
            core::ptr::addr_of_mut!(HASH_FUENTE) as u32,
        )
    };
    if codigo_fuente != 0 {
        unsafe {
            ULTIMO_CODIGO = codigo_fuente;
            ULTIMA_ETIQUETA = *b"F1 PUT  ";
            HASH_FUENTE_VALIDO = false;
            HASH_BINARIO_VALIDO = false;
            ARISTA_SINCRONIZADA = false;
        }
        return;
    }
    unsafe { HASH_FUENTE_VALIDO = true };

    // 2) Registrar el BINARIO WASM via la syscall PRIVILEGIADA `v2` (Fase 31):
    //    el kernel inscribe HASH_FUENTE como PRIMER HIJO del nodo binario, de
    //    modo que la arista causa->efecto queda materializada en el grafo en
    //    una sola transicion. El IDE no escribe la arista a mano; la firma
    //    criptografica del enlace la pone el almacen direccionado por
    //    contenido al mezclar el HASH_FUENTE con el bytecode del binario.
    let codigo_bin = unsafe {
        sys_subsistema_registrar_ejecutable_v2(
            binario.as_ptr() as u32,
            bin_len as u32,
            core::ptr::addr_of!(HASH_FUENTE) as u32,
            core::ptr::addr_of_mut!(HASH_BINARIO) as u32,
        )
    };
    unsafe {
        ULTIMO_CODIGO = codigo_bin;
        ULTIMA_ETIQUETA = *b"F1 V2   ";
        HASH_BINARIO_VALIDO = codigo_bin == 0;
        // La arista solo queda SINCRONIZADA si ambas escrituras llegaron a
        // disco sin novedad. Saturado (-6) deja el binario sin registrar y
        // la arista huerfana hasta el proximo tick.
        ARISTA_SINCRONIZADA = codigo_bin == 0;
    }
}

fn accion_registrar_texto_crudo() {
    let len = unsafe { FUENTE_LEN };
    if len == 0 {
        unsafe {
            ULTIMO_CODIGO = -7;
            ULTIMA_ETIQUETA = *b"F2 VACIO";
        }
        return;
    }
    let codigo = unsafe {
        sys_subsistema_registrar_ejecutable(
            core::ptr::addr_of!(FUENTE) as u32,
            len as u32,
            core::ptr::addr_of_mut!(HASH_BINARIO) as u32,
        )
    };
    unsafe {
        ULTIMO_CODIGO = codigo;
        ULTIMA_ETIQUETA = *b"F2 TXT  ";
        // Si milagrosamente el texto cuadrara con magic WASM, marcarlo
        // como hash valido — en cualquier otro caso, no.
        HASH_BINARIO_VALIDO = codigo == 0;
    }
}

fn accion_recuperar_ultimo() {
    if !unsafe { HASH_BINARIO_VALIDO } {
        unsafe {
            ULTIMO_CODIGO = -1;
            ULTIMA_ETIQUETA = *b"F3 SHASH";
        }
        return;
    }
    let codigo = unsafe {
        sys_object_datos(
            core::ptr::addr_of!(HASH_BINARIO) as u32,
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
//  Pintado
// =============================================================================

fn pintar() {
    let paleta = unsafe { PALETA };
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    let fondo = color_u32(paleta, 2);
    let tinta = color_u32(paleta, 3);
    let acento = color_u32(paleta, 4);
    let secundario = color_u32(paleta, 1);

    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, fondo);

    rellenar_rect(lienzo, 0, 0, ANCHO, ALFA_Y - 4, secundario);
    dibujar_texto(lienzo, b"IDE WAWA  FASE 31", 8, 6, 2, tinta);
    rellenar_rect(lienzo, 0, ALFA_Y - 4, ANCHO, 2, acento);

    pintar_panel_alfa(lienzo, tinta, acento);
    pintar_panel_beta(lienzo, tinta, acento, secundario);
    pintar_panel_gamma(lienzo, tinta, acento, secundario);
}

fn pintar_panel_alfa(lienzo: &mut [u32], tinta: u32, acento: u32) {
    dibujar_texto(lienzo, b"ALFA  EDITOR", 8, ALFA_Y, 1, acento);
    rellenar_rect(lienzo, 0, ALFA_Y + 10, ANCHO, 1, acento);

    let escala = 2usize;
    let avance = 6 * escala;
    let cols = (ANCHO - 16) / avance;
    let mut linea_y = ALFA_Y + 16;
    let mut cursor_col = 0usize;
    let len = unsafe { FUENTE_LEN };
    let mut buf_local = [b' '; 64];
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
        let renderable = c.is_ascii_alphanumeric()
            || c == b' '
            || c == b'+'
            || c == b'-'
            || c == b'*';
        if renderable {
            buf_local[buf_len] = c.to_ascii_uppercase();
            buf_len += 1;
            cursor_col += 1;
        }
    }
    if buf_len > 0 {
        comprimir_y_pintar(lienzo, linea_y, &buf_local[..buf_len]);
    }
    let cursor_x = 8 + buf_len * avance;
    rellenar_rect(lienzo, cursor_x, linea_y, 2 * escala, 7 * escala, acento);
}

fn pintar_panel_beta(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"BETA  GRAFO SEMANTICO", 8, BETA_Y, 1, acento);
    rellenar_rect(lienzo, 0, BETA_Y + 10, ANCHO, 1, acento);

    let fila_y_fuente = BETA_Y + 18;
    let fila_y_bin = BETA_Y + 56;
    let bloque_fondo = color_atenuar_u32(secundario, 0xC0);

    // --- Nodo FUENTE (texto del editor) ---
    rellenar_rect(lienzo, 8, fila_y_fuente, ANCHO - 16, 24, bloque_fondo);
    dibujar_texto(lienzo, b"FUENTE", 12, fila_y_fuente + 2, 1, acento);
    let hash_f = unsafe { HASH_FUENTE };
    let mut hex_f = [b'-'; 64];
    if unsafe { HASH_FUENTE_VALIDO } {
        for i in 0..32 {
            hex_f[i * 2] = nibble_hex(hash_f[i] >> 4);
            hex_f[i * 2 + 1] = nibble_hex(hash_f[i] & 0x0F);
        }
    }
    dibujar_texto(lienzo, &hex_f[..32], 60, fila_y_fuente + 2, 1, tinta);
    dibujar_texto(lienzo, &hex_f[32..], 60, fila_y_fuente + 12, 1, tinta);

    // --- LA ARISTA CAUSAL (Fase 31). Linea acento vertical de 5 px que une
    // los nodos FUENTE y BINARIO. Cuando el ultimo F1 cerro con exito
    // (codigo 0 y ambos hashes inscritos por `v2`), la pintamos con el
    // color ACENTO de la paleta activa: la causa escrita y el efecto
    // ejecutable estan consolidados en el hardware. Si el usuario teclea
    // un solo caracter despues, la sincronia se rompe y el conector se
    // tiñe de GRIS MATE — el divorcio entre lo que el editor muestra y
    // lo que el grafo guarda es visible en tiempo real.
    let conector_x = 32;
    let conector_y0 = fila_y_fuente + 24;
    let conector_y1 = fila_y_bin;
    let color_conector = if unsafe { ARISTA_SINCRONIZADA } {
        acento
    } else {
        // Gris mate: la arista existe en el grafo (si los hashes son
        // validos) pero ya no refleja el texto que el usuario edita.
        color_atenuar_u32(secundario, 0x60)
    };
    rellenar_rect(
        lienzo,
        conector_x,
        conector_y0,
        5,
        conector_y1 - conector_y0,
        color_conector,
    );

    // --- Nodo BINARIO (modulo WASM emitido).
    rellenar_rect(lienzo, 8, fila_y_bin, ANCHO - 16, 24, bloque_fondo);
    dibujar_texto(lienzo, b"BINARIO", 12, fila_y_bin + 2, 1, acento);
    let hash_b = unsafe { HASH_BINARIO };
    let mut hex_b = [b'-'; 64];
    if unsafe { HASH_BINARIO_VALIDO } {
        for i in 0..32 {
            hex_b[i * 2] = nibble_hex(hash_b[i] >> 4);
            hex_b[i * 2 + 1] = nibble_hex(hash_b[i] & 0x0F);
        }
    }
    dibujar_texto(lienzo, &hex_b[..32], 60, fila_y_bin + 2, 1, tinta);
    dibujar_texto(lienzo, &hex_b[32..], 60, fila_y_bin + 12, 1, tinta);
}

fn pintar_panel_gamma(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"GAMMA  CONSOLA", 8, GAMMA_Y, 1, acento);
    rellenar_rect(lienzo, 0, GAMMA_Y + 10, ANCHO, 1, acento);

    rellenar_rect(lienzo, 8, GAMMA_Y + 16, ANCHO - 16, 12, color_atenuar_u32(secundario, 0xC0));

    let etiqueta = unsafe { ULTIMA_ETIQUETA };
    let codigo = unsafe { ULTIMO_CODIGO };
    let mut linea1 = [b' '; 48];
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

    let leyenda: &[u8] = match codigo {
        0 => b"OK  BINARIO EMITIDO CON EXITO",
        -1 => b"AUSENTE  OBJETO NO HALLADO",
        -2 => b"CAPACIDAD INSUFICIENTE",
        -3 => b"ALMACENAMIENTO FALLO",
        -4 => b"SIN FOCO",
        -5 => b"ENVIO FALLO",
        -6 => b"SATURADO  REINTENTAR PROXIMO TICK",
        -7 => b"PAYLOAD INVALIDO  SINTAXIS FORTH AJENA",
        x if x > 0 => b"OK  BYTES COPIADOS",
        _ => b"INICIO  TECLEA 5 10 + Y PULSA F1",
    };
    dibujar_texto(lienzo, leyenda, 12, GAMMA_Y + 32, 1, tinta);

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

    let hotkeys = b"F1 COMPILA   F2 CRUDO   F3 RECUPERA   BS  ENTER";
    dibujar_texto(lienzo, hotkeys, 8, GAMMA_Y + GAMMA_ALTO - 10, 1, acento);
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

// =============================================================================
//  Helpers de color y dibujo
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
//  Mini-tipografia 5x7
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
        b'+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        b'*' => [0x00, 0x0A, 0x04, 0x1F, 0x04, 0x0A, 0x00],
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

// El emisor Forth->WASM se verifica empiricamente al construir el .wasm
// para `wasm32-unknown-unknown` y comprobar interactivamente con el IDE
// (`5 10 +` + F1) que el binario se acepta por `wasmi`. Los tests unitarios
// en host requeririan extraer el emisor a una crate aparte (los syscalls
// `sys_*` solo existen en wasm32 y rompen el target de pruebas std).
