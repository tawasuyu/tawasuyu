// =============================================================================
//  renaser :: apps/cuaderno — Fase 33 :: motor celular de calculo persistente
// -----------------------------------------------------------------------------
//  Un cuaderno de celulas: cada CELDA enlaza una FUENTE Forth, un BINARIO
//  WASM emitido por forth-emisor y el RETORNO de su ultima ejecucion. Las
//  tres piezas se inscriben en el grafo direccionado por contenido del
//  kernel — el cuaderno deja de ser un buffer volatil y se vuelve un nodo
//  inmutable que sobrevive al apagado, al panico y a la mudanza.
//
//  El usuario teclea en el PANEL EDITOR (parte alta del lienzo natural);
//  F5 dispara una rafaga de syscalls que:
//    1. Compila la fuente con forth-emisor.
//    2. Inscribe la FUENTE en el grafo (`sys_object_put`).
//    3. Inscribe el BINARIO con la arista causal de la Fase 31
//       (`sys_subsistema_registrar_ejecutable_v2`).
//    4. Ejecuta el binario en una sub-jaula efimera de la Fase 32
//       (`sys_subsistema_ejecutar_dinamico`) y captura el i32 retornado.
//    5. Consolida la celda en el cuaderno via la nueva syscall de la
//       Fase 33 (`sys_cuaderno_registrar_celda`).
//
//  Si CUALQUIER paso de la cadena falla (trap, fuel agotado, almacenamiento),
//  la celda se inscribe igual — con `retorno` negativo del rango reservado de
//  `CodigoError`— y se pinta de AMARILLO PALIDO. Las celdas vecinas siguen
//  ofreciendo sus retornos en verde, el compositor jamas pierde un fotograma.
//
//  Alt+Enter, mencionado en la directiva original, lo intercepta el kernel
//  como mando del compositor (promueve la ventana enfocada a maestra). El
//  cuaderno escogio F5 (scancode 0x3F) como hotkey pragmatica de ejecucion
//  celular — alineada con la convencion de los IDEs notebook clasicos sin
//  colisionar con la matriz de mandos del WM.
// =============================================================================

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_object_put(
        datos_ptr: u32,
        datos_len: u32,
        hijos_ptr: u32,
        hijos_cnt: u32,
        salida: u32,
    ) -> i32;
    fn sys_subsistema_registrar_ejecutable_v2(
        ptr: u32,
        len: u32,
        padre_hash_ptr: u32,
        salida_hash_ptr: u32,
    ) -> i32;
    fn sys_subsistema_ejecutar_dinamico(binario_hash_ptr: u32) -> i32;
    fn sys_cuaderno_registrar_celda(
        fuente_hash_ptr: u32,
        binario_hash_ptr: u32,
        retorno: i32,
        salida_cuaderno_hash_ptr: u32,
    ) -> i32;
}

#[cfg(not(test))]
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// =============================================================================
//  Geometria del lienzo natural (480 x 400)
// =============================================================================

const ANCHO: usize = 480;
const ALTO: usize = 400;

const EDITOR_Y: usize = 24;
const EDITOR_ALTO: usize = 60;
const CELDAS_Y: usize = EDITOR_Y + EDITOR_ALTO + 6;
const CELDAS_ALTO: usize = 270;
const LEYENDA_Y: usize = ALTO - 16;

const MAX_CELDAS: usize = 3;
const CELDA_ALTO: usize = CELDAS_ALTO / MAX_CELDAS;

// =============================================================================
//  Estado celular
// =============================================================================

/// Tope del buffer fuente (compartido por todas las celdas y el editor).
const FUENTE_CAP: usize = 64;

/// Una celda del cuaderno en su forma viva en RAM. La forma serializada
/// canonica vive en el grafo: este struct es el cache de pintado.
#[derive(Clone, Copy)]
struct Celda {
    fuente: [u8; FUENTE_CAP],
    fuente_len: usize,
    hash_fuente: [u8; 32],
    hash_binario: [u8; 32],
    retorno: i32,
    /// `true` cuando la celda recorrio la cadena completa al menos una vez.
    /// Distingue celdas habitadas de las ranuras vacias del cuaderno.
    valida: bool,
    /// `true` cuando el retorno fue legitimo. `false` cuando la cadena
    /// fallo (cualquier eslabon devolvio un codigo de error). El pintado
    /// usa este bit para teñir la celda de amarillo palido sin enterrar
    /// el resultado.
    exito: bool,
}

impl Celda {
    const fn vacia() -> Self {
        Celda {
            fuente: [0; FUENTE_CAP],
            fuente_len: 0,
            hash_fuente: [0; 32],
            hash_binario: [0; 32],
            retorno: 0,
            valida: false,
            exito: false,
        }
    }
}

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];

static mut EDITOR: [u8; FUENTE_CAP] = [0; FUENTE_CAP];
static mut EDITOR_LEN: usize = 0;

static mut CELDAS: [Celda; MAX_CELDAS] = [Celda::vacia(); MAX_CELDAS];
static mut PROXIMA_CELDA: usize = 0;

/// Hash del cuaderno tras la ultima consolidacion exitosa. Es la "raiz"
/// movil del cuaderno: cada celda inscrita engendra un cuaderno nuevo y
/// este se reancla aqui. No se compara con el grafo del kernel — solo
/// rotula la cabecera del editor para que el humano vea que el cuaderno
/// quedo persistido (`HASH cuaderno: XX..YY`).
static mut HASH_CUADERNO: [u8; 32] = [0; 32];
static mut HASH_CUADERNO_VALIDO: bool = false;

static mut F5_PREV: bool = false;
static mut SCAN_PREV: u32 = 0;

// =============================================================================
//  ABI obligatorio del userspace renaser
// =============================================================================

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
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

// =============================================================================
//  Teclado
// =============================================================================

fn atender_scancode(scancode: u32) {
    if scancode == 0x3F {
        // F5 :: ejecutar celda. Solo dispara una vez por pulsacion.
        if !unsafe { F5_PREV } {
            ejecutar_celda_actual();
        }
        unsafe { F5_PREV = true };
        return;
    }
    unsafe { F5_PREV = scancode == 0x3F };

    if scancode == 0x0E {
        // Backspace.
        if unsafe { EDITOR_LEN } > 0 {
            unsafe { EDITOR_LEN -= 1 };
        }
        return;
    }
    let escribir = match scancode {
        0x39 => Some(b' '),
        _ => mapear_caracter(scancode as u8),
    };
    if let Some(b) = escribir {
        let len = unsafe { EDITOR_LEN };
        if len < FUENTE_CAP {
            unsafe {
                EDITOR[len] = b;
                EDITOR_LEN = len + 1;
            }
        }
    }
}

fn mapear_caracter(scan: u8) -> Option<u8> {
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
        0x0C => b'-',
        // Numpad: digitos y operadores Forth (sin Shift).
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
        0x37 => b'*',
        0x4A => b'-',
        0x4E => b'+',
        _ => return None,
    })
}

// =============================================================================
//  Cadena de ejecucion celular (F5)
// =============================================================================

/// Recorre la cadena FUENTE -> BINARIO -> EJECUTAR -> CUADERNO sobre el
/// buffer del editor. Cualquier eslabon fallido se conserva como celda
/// con `exito = false`, para que el humano vea el motivo en el panel sin
/// que el cuaderno entero se invalide.
fn ejecutar_celda_actual() {
    let len = unsafe { EDITOR_LEN };
    if len == 0 {
        return;
    }

    let mut celda = Celda::vacia();
    celda.fuente_len = len;
    celda.fuente[..len].copy_from_slice(unsafe { &EDITOR[..len] });
    celda.valida = true;

    // 1. Compilar Forth -> WASM en un buffer en pila.
    let mut binario = [0u8; 512];
    let bin_len = match forth_emisor::ForthCompiler::compilar_bytes(
        &celda.fuente[..celda.fuente_len],
        &mut binario,
    ) {
        Some(n) => n,
        None => {
            celda.retorno = -7; // PayloadInvalido (sintaxis Forth ajena).
            celda.exito = false;
            registrar_celda_local(celda);
            return;
        }
    };

    // 2. Grabar la FUENTE como objeto del grafo. Sin hijos.
    let cod_fuente = unsafe {
        sys_object_put(
            celda.fuente.as_ptr() as u32,
            celda.fuente_len as u32,
            0u32,
            0u32,
            celda.hash_fuente.as_mut_ptr() as u32,
        )
    };
    if cod_fuente != 0 {
        celda.retorno = cod_fuente;
        celda.exito = false;
        registrar_celda_local(celda);
        return;
    }

    // 3. Registrar el BINARIO con la arista causal hacia la FUENTE.
    let cod_bin = unsafe {
        sys_subsistema_registrar_ejecutable_v2(
            binario.as_ptr() as u32,
            bin_len as u32,
            celda.hash_fuente.as_ptr() as u32,
            celda.hash_binario.as_mut_ptr() as u32,
        )
    };
    if cod_bin != 0 {
        celda.retorno = cod_bin;
        celda.exito = false;
        registrar_celda_local(celda);
        return;
    }

    // 4. Ejecutar el binario en sub-jaula efimera. El i32 retornado por
    //    `"run"` viaja tal cual; codigos negativos [-7..-1] reservados
    //    son fallas controladas (trap, fuel agotado, etc.).
    let retorno = unsafe { sys_subsistema_ejecutar_dinamico(celda.hash_binario.as_ptr() as u32) };
    let es_falla = retorno <= -1 && retorno >= -7;
    celda.retorno = retorno;
    celda.exito = !es_falla;

    // 5. Consolidar la celda como nodo cuaderno en el grafo.
    let cod_cuaderno = unsafe {
        sys_cuaderno_registrar_celda(
            celda.hash_fuente.as_ptr() as u32,
            celda.hash_binario.as_ptr() as u32,
            celda.retorno,
            core::ptr::addr_of_mut!(HASH_CUADERNO) as u32,
        )
    };
    unsafe { HASH_CUADERNO_VALIDO = cod_cuaderno == 0 };

    registrar_celda_local(celda);

    // Si la ejecucion fue exitosa, dejamos el editor cargado para que el
    // humano pueda iterar; si fallo, tampoco se borra — el usuario corrige
    // y vuelve a F5. Vaciar el editor cada vez seria un anti-MVP.
}

/// Inserta `celda` en el array circular. Cuando el cuaderno se llena, la
/// celda mas antigua se sobreescribe — el grafo guarda todas las
/// historias, pero el panel solo retiene las ultimas tres.
fn registrar_celda_local(celda: Celda) {
    unsafe {
        let slot = PROXIMA_CELDA;
        CELDAS[slot] = celda;
        PROXIMA_CELDA = (slot + 1) % MAX_CELDAS;
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

    // Cabecera con el hash del cuaderno (si ya hubo una consolidacion).
    rellenar_rect(lienzo, 0, 0, ANCHO, EDITOR_Y - 4, secundario);
    dibujar_texto(lienzo, b"CUADERNO WAWA  FASE 33", 8, 6, 1, tinta);
    if unsafe { HASH_CUADERNO_VALIDO } {
        let h = unsafe { HASH_CUADERNO };
        let mut etiqueta = [b' '; 8];
        etiqueta[0] = nibble_hex(h[0] >> 4);
        etiqueta[1] = nibble_hex(h[0] & 0x0F);
        etiqueta[2] = nibble_hex(h[1] >> 4);
        etiqueta[3] = nibble_hex(h[1] & 0x0F);
        etiqueta[4] = b'.';
        etiqueta[5] = b'.';
        etiqueta[6] = nibble_hex(h[31] >> 4);
        etiqueta[7] = nibble_hex(h[31] & 0x0F);
        dibujar_texto(lienzo, &etiqueta, ANCHO - 8 - 8 * 6, 6, 1, acento);
    }
    rellenar_rect(lienzo, 0, EDITOR_Y - 4, ANCHO, 2, acento);

    pintar_editor(lienzo, tinta, acento, secundario);
    pintar_celdas(lienzo, tinta, acento, secundario);

    let leyenda = b"F5 EJECUTAR CELDA   BS BORRAR";
    dibujar_texto(lienzo, leyenda, 8, LEYENDA_Y, 1, acento);
}

fn pintar_editor(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"EDITOR  TECLEA FORTH", 8, EDITOR_Y, 1, acento);
    rellenar_rect(
        lienzo,
        8,
        EDITOR_Y + 12,
        ANCHO - 16,
        EDITOR_ALTO - 16,
        color_atenuar_u32(secundario, 0xC0),
    );

    let len = unsafe { EDITOR_LEN };
    let mut buf = [b' '; FUENTE_CAP];
    let mut n = 0;
    for i in 0..len {
        let c = unsafe { EDITOR[i] };
        if es_renderable(c) {
            buf[n] = c.to_ascii_uppercase();
            n += 1;
        }
    }
    dibujar_texto(lienzo, &buf[..n], 14, EDITOR_Y + 18, 2, tinta);

    // Cursor parpadeando — un rectanguillo al final del buffer renderizado.
    let cursor_x = 14 + n * 12;
    rellenar_rect(lienzo, cursor_x, EDITOR_Y + 18, 4, 14, acento);
}

fn pintar_celdas(lienzo: &mut [u32], tinta: u32, acento: u32, secundario: u32) {
    dibujar_texto(lienzo, b"CELDAS  GRAFO", 8, CELDAS_Y - 12, 1, acento);

    for i in 0..MAX_CELDAS {
        let y = CELDAS_Y + i * CELDA_ALTO;
        let celda = unsafe { CELDAS[i] };
        // Fondo de la celda: amarillo palido cuando la cadena fallo,
        // gris atenuado en exito o ranura vacia.
        let fondo_celda = if celda.valida && !celda.exito {
            AMARILLO_PALIDO
        } else {
            color_atenuar_u32(secundario, 0xA0)
        };
        rellenar_rect(lienzo, 8, y, ANCHO - 16, CELDA_ALTO - 4, fondo_celda);

        // Etiqueta de slot (CELDA 1/2/3).
        let etiqueta = [b'C', b'E', b'L', b'D', b'A', b' ', b'1' + i as u8];
        dibujar_texto(lienzo, &etiqueta, 14, y + 4, 1, acento);

        if !celda.valida {
            dibujar_texto(lienzo, b"VACIA", ANCHO - 14 - 5 * 6, y + 4, 1, tinta);
            continue;
        }

        // Linea 1: fragmento de la fuente en minusculas (hasta 24 chars).
        let mut linea_src = [b' '; 24];
        let cap = celda.fuente_len.min(linea_src.len());
        for k in 0..cap {
            let c = celda.fuente[k];
            linea_src[k] = if es_renderable(c) { c.to_ascii_uppercase() } else { b' ' };
        }
        dibujar_texto(lienzo, &linea_src, 14, y + 18, 1, tinta);

        // Linea 2: hash del binario, primeros 16 nibbles.
        let mut linea_hash = [b'-'; 16];
        for k in 0..8 {
            let b = celda.hash_binario[k];
            linea_hash[k * 2] = nibble_hex(b >> 4);
            linea_hash[k * 2 + 1] = nibble_hex(b & 0x0F);
        }
        dibujar_texto(lienzo, b"BIN ", 14, y + 32, 1, acento);
        dibujar_texto(lienzo, &linea_hash, 14 + 4 * 6, y + 32, 1, tinta);

        // Linea 3 (grande): OUT  <retorno> o OUT TRAP cuando fallo.
        let mut linea_out = [b' '; 24];
        let prefix = b"OUT  ";
        linea_out[..prefix.len()].copy_from_slice(prefix);
        if celda.exito {
            let (dec, dlen) = formatear_i32(celda.retorno);
            let mut p = prefix.len();
            for &c in &dec[..dlen] {
                if p < linea_out.len() {
                    linea_out[p] = c;
                    p += 1;
                }
            }
            dibujar_texto(lienzo, &linea_out[..prefix.len() + dlen], 14, y + 48, 2, tinta);
        } else {
            let etiqueta = match celda.retorno {
                -7 => b"TRAP    ".as_slice(),
                -6 => b"SATURADO".as_slice(),
                -2 => b"CAP INSF".as_slice(),
                -3 => b"DISCO   ".as_slice(),
                -1 => b"AUSENTE ".as_slice(),
                _ => b"ERROR   ".as_slice(),
            };
            let p = prefix.len();
            let cap = etiqueta.len().min(linea_out.len() - p);
            linea_out[p..p + cap].copy_from_slice(&etiqueta[..cap]);
            dibujar_texto(lienzo, &linea_out[..p + cap], 14, y + 48, 2, tinta);
        }
    }
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

// =============================================================================
//  Helpers de color y dibujo
// =============================================================================

const AMARILLO_PALIDO: u32 = 0x00FFEEA0;

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

fn es_renderable(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b' ' || c == b'+' || c == b'-' || c == b'*'
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
//  Mini-tipografia 5x7 (copia local — el ABI del userspace renaser no expone
//  glifos compartidos por ahora; cada app trae su tabla).
// =============================================================================

const FA: usize = 5;
const FH: usize = 7;
const FAV: usize = 6;

fn glifo(c: u8) -> [u8; FH] {
    match c {
        b' ' => [0; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
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
