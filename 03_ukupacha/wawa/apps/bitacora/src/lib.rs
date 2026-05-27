// =============================================================================
//  renaser :: apps/bitacora — editor de notas personales que recuerda
// -----------------------------------------------------------------------------
//  La fase 7c le dio a las apps memoria mas alla del arranque: `sys_estado_*`
//  ancla la huella de un app en el grafo, y al reiniciar el kernel se la
//  devuelve. `memoriosa` lo demostro contando teclas. `bitacora` lo lleva al
//  siguiente paso natural: ofrecer un editor de texto. Esta iteracion la sube
//  a "app de notas personales":
//
//    1. **Ocho notas independientes**. `F1`..`F8` cambian la nota activa.
//       Cada una tiene su propio buffer (8 KiB) y su propio cursor. El
//       header pinta `1 2 3 4 5 6 7 8` con la activa resaltada en tinta y
//       el resto atenuado.
//
//    2. **Cursor editable in-situ**. Flechas izquierda/derecha mueven el
//       cursor un byte; arriba/abajo lo llevan a la misma columna de la
//       linea anterior/siguiente (separadas por `\n`, sin wrap visual).
//       `Home`/`End` saltan al inicio/fin de linea. `Backspace` borra el
//       byte de atras del cursor; `Delete` el de adelante. Cualquier tecla
//       de texto se INSERTA en la posicion del cursor (memmove del resto)
//       en vez de anexarse al final.
//
//    3. **Shift sostenido**. Tracking de LShift (`0x2A`/`0xAA`) y RShift
//       (`0x36`/`0xB6`) ANTES del filtro general de key-up — asi una
//       mayuscula sostenida no se "suelta" al pulsar otra tecla. Con Shift:
//       A-Z, `!@#$%^&*()` en la fila numerica, y `<>?:"_+{}|~` en el resto
//       de puntuacion del layout US. Sin Shift, todo en minuscula mas
//       `-=[]\` y backtick.
//
//    4. **Persistencia de las ocho notas + cursores**. Cada cambio (texto,
//       cursor, nota activa) se persiste de inmediato con un formato
//       cabecera+payload firmado por el magic `BTC2`. Si lo que el kernel
//       devuelve no lleva el magic, asumimos formato viejo (Fase 17, un
//       solo buffer plano) y lo cargamos en la nota 1 — la nota del usuario
//       sobrevive a la actualizacion. La apagada brusca no pierde nada.
//
//  Tipografia: la 8x8 clasica (font8x8), escalada x2 a 16x16. Cabe en su
//  propia memoria lineal y se renderiza pixel a pixel — el app no toca el
//  lienzo del kernel, solo entrega su propio fotograma.
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

/// Numero de notas independientes. Se cambian con F1..F8.
const NUM_NOTAS: usize = 8;
/// Capacidad de texto por nota. Al desbordarse se descartan `DESCARTE` bytes
/// del principio (amortiza el coste; no es una mudanza por cada pulsacion).
const CAP_NOTA: usize = 8192;
/// Bytes que se descartan al desbordar la nota actual.
const DESCARTE: usize = 512;

const FONDO: u32 = 0x0A_18_30;
const TINTA: u32 = 0xE8_EC_F4;
const ETIQUETA: u32 = 0x8B_5C_F6;
/// Numero de nota inactivo en el header — atenuado respecto a ETIQUETA.
const INACTIVO: u32 = 0x4A_3A_7C;
/// Caret del cursor — un poco mas brillante que la tinta para verlo.
const CARET: u32 = 0xF8_E8_8A;

/// Una nota: su buffer de texto, cuanto se lleva escrito y donde esta el
/// cursor dentro de el. Sin `String`, sin `Vec`: arrays estaticos puros, asi
/// la app no necesita allocator.
#[derive(Copy, Clone)]
struct Nota {
    cuerpo: [u8; CAP_NOTA],
    len: u32,
    cursor: u32,
}

const NOTA_VACIA: Nota = Nota {
    cuerpo: [0; CAP_NOTA],
    len: 0,
    cursor: 0,
};

static mut NOTAS: [Nota; NUM_NOTAS] = [NOTA_VACIA; NUM_NOTAS];
static mut ACTIVA: usize = 0;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// Estado de las teclas modificadoras. Se actualiza con make/break de los
/// scancodes `0x2A` (LShift) y `0x36` (RShift) ANTES del filtro general de
/// key-up, asi una mayuscula sostenida se mantiene activa entre pulsaciones.
static mut SHIFT: bool = false;
/// Recuerda si el scancode anterior fue el prefijo extendido `0xE0`. Las
/// flechas, Home/End, Delete, etc. llegan como pareja `0xE0 <sc>`; ponemos
/// el flag al ver el `0xE0` y lo bajamos al consumir el byte siguiente.
static mut EXT_PREFIX: bool = false;

/// Buffer de E/S del estado persistido. Formato:
///   [magic 4B "BTC2"]
///   [version 1B = 0x02]
///   [activa 1B] (0..NUM_NOTAS)
///   [pad 2B = 0]
///   for i in 0..NUM_NOTAS:
///     [len_i u32 LE]
///     [cursor_i u32 LE]
///     [bytes_i: len_i bytes]
const HDR: usize = 4 + 1 + 1 + 2;
const NOTA_HDR: usize = 4 + 4;
const ESTADO_CAP: usize = HDR + NUM_NOTAS * (NOTA_HDR + CAP_NOTA);
static mut ESTADO_IO: [u8; ESTADO_CAP] = [0; ESTADO_CAP];

const MAGIC: [u8; 4] = *b"BTC2";
const VERSION: u8 = 0x02;

// --- ABI del userspace --------------------------------------------------------

#[no_mangle]
pub extern "C" fn init() {
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(ESTADO_IO) };
    let n = unsafe { sys_estado_cargar(buf.as_mut_ptr() as u32, ESTADO_CAP as u32) };
    if n > 0 {
        let n = (n as usize).min(ESTADO_CAP);
        // SEGURIDAD: leemos del buffer de IO que pertenece a esta linear memory.
        if !cargar_v2(buf, n) {
            // Compatibilidad: si no es BTC2, asumimos formato Fase 17 (un
            // solo buffer plano sin cabecera). Lo cargamos en la nota 1.
            cargar_legacy(buf, n);
        }
    }
    pintar();
}

#[no_mangle]
pub extern "C" fn tick() {
    let mut cambio_texto = false;
    let mut cambio_ui = false;
    // Drenar TODOS los scancodes acumulados desde el ultimo fotograma. La cola
    // es propia de este app — la inscribio la fase 5 en la IRQ1.
    loop {
        let sc = unsafe { sys_get_scancode() } as u8;
        if sc == 0 {
            break;
        }
        // Prefijo extendido: solo marca, no produce evento por si solo.
        if sc == 0xE0 {
            unsafe { EXT_PREFIX = true; }
            continue;
        }
        let ext = unsafe { EXT_PREFIX };
        unsafe { EXT_PREFIX = false; }

        // Shift se sigue ANTES del filtro general — necesitamos saber cuando
        // se suelta para volver a minuscula.
        if !ext {
            match sc {
                0x2A | 0x36 => { unsafe { SHIFT = true; } continue; }
                0xAA | 0xB6 => { unsafe { SHIFT = false; } continue; }
                _ => {}
            }
        }

        // Resto de key-ups (bit 7): ignorar.
        if sc & 0x80 != 0 {
            continue;
        }

        // F1..F8 → cambiar nota activa. Sin extended prefix.
        if !ext {
            if let Some(idx) = tecla_funcion(sc) {
                if idx < NUM_NOTAS {
                    unsafe { ACTIVA = idx; }
                    cambio_ui = true;
                    continue;
                }
            }
        }

        if ext {
            // Movimientos del cursor (extended set 1).
            match sc {
                0x4B => { mover_izquierda(); cambio_ui = true; }
                0x4D => { mover_derecha();   cambio_ui = true; }
                0x48 => { mover_arriba();    cambio_ui = true; }
                0x50 => { mover_abajo();     cambio_ui = true; }
                0x47 => { mover_inicio();    cambio_ui = true; }
                0x4F => { mover_fin();       cambio_ui = true; }
                0x53 => { if borrar_adelante() { cambio_texto = true; } }
                _ => {}
            }
            continue;
        }

        match sc {
            0x0E => { if borrar_atras() { cambio_texto = true; } }
            0x1C => { insertar(b'\n'); cambio_texto = true; }
            otro => {
                let shift = unsafe { SHIFT };
                let c = scancode_a_caracter(otro, shift);
                if c != 0 {
                    insertar(c);
                    cambio_texto = true;
                }
            }
        }
    }
    if cambio_texto || cambio_ui {
        guardar();
    }
    pintar();
}

// --- Edicion del buffer activo ------------------------------------------------

/// Devuelve referencias `&mut` a la nota activa. La envoltura paga el coste
/// de lectura/escritura de `ACTIVA` una sola vez y los callers tratan con
/// indices directos al array.
fn con_activa<F: FnOnce(&mut Nota)>(f: F) {
    unsafe {
        let idx = ACTIVA.min(NUM_NOTAS - 1);
        let notas = &mut *core::ptr::addr_of_mut!(NOTAS);
        f(&mut notas[idx]);
    }
}

/// Inserta `c` en la posicion del cursor; mueve el resto un byte a la derecha.
/// Si la nota esta llena, descarta los primeros `DESCARTE` bytes y reajusta el
/// cursor (pierde posicion solo si caia dentro del fragmento descartado).
fn insertar(c: u8) {
    con_activa(|n| {
        let mut len = n.len as usize;
        let mut cursor = n.cursor as usize;
        if len >= CAP_NOTA {
            n.cuerpo.copy_within(DESCARTE.., 0);
            len = CAP_NOTA - DESCARTE;
            cursor = cursor.saturating_sub(DESCARTE);
        }
        if cursor > len {
            cursor = len;
        }
        // Empuja `[cursor..len]` un byte a la derecha y escribe en `cursor`.
        n.cuerpo.copy_within(cursor..len, cursor + 1);
        n.cuerpo[cursor] = c;
        n.len = (len + 1) as u32;
        n.cursor = (cursor + 1) as u32;
    });
}

/// Borra el byte inmediatamente atras del cursor. Devuelve `true` si hubo algo
/// que borrar. Reproduce `Backspace` clasico.
fn borrar_atras() -> bool {
    let mut cambio = false;
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = n.cursor as usize;
        if cursor == 0 || len == 0 {
            return;
        }
        let nuevo = cursor - 1;
        n.cuerpo.copy_within(cursor..len, nuevo);
        n.len = (len - 1) as u32;
        n.cursor = nuevo as u32;
        cambio = true;
    });
    cambio
}

/// Borra el byte EN la posicion del cursor — `Delete`. Devuelve `true` si lo
/// hubo (no estamos al final del buffer).
fn borrar_adelante() -> bool {
    let mut cambio = false;
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = n.cursor as usize;
        if cursor >= len {
            return;
        }
        n.cuerpo.copy_within((cursor + 1)..len, cursor);
        n.len = (len - 1) as u32;
        cambio = true;
    });
    cambio
}

fn mover_izquierda() {
    con_activa(|n| {
        if n.cursor > 0 {
            n.cursor -= 1;
        }
    });
}

fn mover_derecha() {
    con_activa(|n| {
        if (n.cursor as usize) < (n.len as usize) {
            n.cursor += 1;
        }
    });
}

/// Devuelve `(inicio_linea, columna)` para la posicion `cursor` dentro de
/// `buf[..len]` — separadas por `\n`, sin contar el wrap visual.
fn linea_actual(buf: &[u8], len: usize, cursor: usize) -> (usize, usize) {
    let mut inicio = 0usize;
    let mut i = cursor;
    while i > 0 {
        i -= 1;
        if buf[i] == b'\n' {
            inicio = i + 1;
            break;
        }
    }
    let _ = len; // tolera len fuera de borde sin tocar memoria.
    (inicio, cursor - inicio)
}

/// Devuelve el indice del siguiente `\n` desde `pos`, o `len` si no hay.
fn fin_linea(buf: &[u8], len: usize, pos: usize) -> usize {
    let mut i = pos;
    while i < len {
        if buf[i] == b'\n' {
            return i;
        }
        i += 1;
    }
    len
}

fn mover_arriba() {
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let (inicio_actual, col) = linea_actual(&n.cuerpo, len, cursor);
        if inicio_actual == 0 {
            return; // ya estamos en la primera linea.
        }
        let fin_anterior = inicio_actual - 1; // el `\n` que la cierra.
        let mut inicio_anterior = 0usize;
        let mut i = fin_anterior;
        while i > 0 {
            i -= 1;
            if n.cuerpo[i] == b'\n' {
                inicio_anterior = i + 1;
                break;
            }
        }
        let largo_anterior = fin_anterior - inicio_anterior;
        n.cursor = (inicio_anterior + col.min(largo_anterior)) as u32;
    });
}

fn mover_abajo() {
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let (_, col) = linea_actual(&n.cuerpo, len, cursor);
        let fin_actual = fin_linea(&n.cuerpo, len, cursor);
        if fin_actual >= len {
            return; // ya estamos en la ultima linea.
        }
        let inicio_siguiente = fin_actual + 1;
        let fin_siguiente = fin_linea(&n.cuerpo, len, inicio_siguiente);
        let largo_siguiente = fin_siguiente - inicio_siguiente;
        n.cursor = (inicio_siguiente + col.min(largo_siguiente)) as u32;
    });
}

fn mover_inicio() {
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let (inicio, _) = linea_actual(&n.cuerpo, len, cursor);
        n.cursor = inicio as u32;
    });
}

fn mover_fin() {
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let fin = fin_linea(&n.cuerpo, len, cursor);
        n.cursor = fin as u32;
    });
}

/// `0x3B..=0x42` = F1..F8. `0x43`/`0x44` (F9/F10) y `0x57`/`0x58` (F11/F12)
/// no se mapean: NUM_NOTAS=8.
fn tecla_funcion(sc: u8) -> Option<usize> {
    if (0x3B..=0x42).contains(&sc) {
        Some((sc - 0x3B) as usize)
    } else {
        None
    }
}

// --- Persistencia: serializar/deserializar el estado completo -----------------

fn guardar() {
    // Serializa NOTAS + ACTIVA al buffer de IO y llama al syscall.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(ESTADO_IO) };
    let notas = unsafe { &*core::ptr::addr_of!(NOTAS) };
    let activa = unsafe { ACTIVA } as u8;

    buf[0..4].copy_from_slice(&MAGIC);
    buf[4] = VERSION;
    buf[5] = activa;
    buf[6] = 0;
    buf[7] = 0;
    let mut off = HDR;
    for n in notas.iter() {
        let len = (n.len as usize).min(CAP_NOTA);
        buf[off..off + 4].copy_from_slice(&(len as u32).to_le_bytes());
        buf[off + 4..off + 8].copy_from_slice(&n.cursor.to_le_bytes());
        off += NOTA_HDR;
        buf[off..off + len].copy_from_slice(&n.cuerpo[..len]);
        off += len;
    }
    unsafe {
        // SEGURIDAD: (ptr, len) describen nuestra propia memoria; el host lo
        // verifica y nunca lee fuera del rango entregado.
        let _ = sys_estado_guardar(buf.as_ptr() as u32, off as u32);
    }
}

fn cargar_v2(buf: &[u8], n: usize) -> bool {
    if n < HDR || buf[0..4] != MAGIC || buf[4] != VERSION {
        return false;
    }
    let activa = buf[5] as usize;
    let notas = unsafe { &mut *core::ptr::addr_of_mut!(NOTAS) };
    let mut off = HDR;
    for nota in notas.iter_mut() {
        if off + NOTA_HDR > n {
            break;
        }
        let len = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let cursor = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        off += NOTA_HDR;
        let len = len.min(CAP_NOTA).min(n.saturating_sub(off));
        nota.cuerpo[..len].copy_from_slice(&buf[off..off + len]);
        nota.len = len as u32;
        nota.cursor = cursor.min(len as u32);
        off += len;
    }
    unsafe { ACTIVA = activa.min(NUM_NOTAS - 1); }
    true
}

/// Fase 17: el buffer plano era el unico estado. Lo cargamos como nota 1 con
/// el cursor al final — el usuario ve su texto donde lo dejo y puede seguir.
fn cargar_legacy(buf: &[u8], n: usize) {
    let len = n.min(CAP_NOTA);
    let notas = unsafe { &mut *core::ptr::addr_of_mut!(NOTAS) };
    notas[0].cuerpo[..len].copy_from_slice(&buf[..len]);
    notas[0].len = len as u32;
    notas[0].cursor = len as u32;
    unsafe { ACTIVA = 0; }
}

// --- Renderizado --------------------------------------------------------------

fn pintar() {
    let lienzo = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    // Fondo limpio.
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }
    // Header: prefijo + lista de notas con la activa en tinta.
    let prefijo = b"bitacora :: ";
    pintar_texto(lienzo, prefijo, MARGEN_X, Y_LABEL, ETIQUETA);
    let mut cx = MARGEN_X + prefijo.len() * PASO;
    let activa = unsafe { ACTIVA };
    for i in 0..NUM_NOTAS {
        let digito = b'1' + i as u8;
        let color = if i == activa { TINTA } else { INACTIVO };
        pintar_glifo(lienzo, digito, cx, Y_LABEL, color);
        cx += PASO;
        pintar_glifo(lienzo, b' ', cx, Y_LABEL, color);
        cx += PASO;
    }
    // Linea sutil bajo el titulo.
    let y_linea = Y_LABEL + PASO + 4;
    for x in MARGEN_X..(ANCHO - MARGEN_X) {
        lienzo[y_linea * ANCHO + x] = ETIQUETA;
        lienzo[(y_linea + 1) * ANCHO + x] = ETIQUETA;
    }

    // Cuerpo: mostrar las ultimas `FILAS` lineas de la nota activa, con wrap
    // en `COLUMNAS`. Dos pasadas: contar filas para saber cuanto saltar y,
    // mientras pintamos, marcar donde cae el cursor para dibujar el caret.
    let notas = unsafe { &*core::ptr::addr_of!(NOTAS) };
    let nota = &notas[activa.min(NUM_NOTAS - 1)];
    let len = (nota.len as usize).min(CAP_NOTA);
    let cursor = (nota.cursor as usize).min(len);
    let buffer = &nota.cuerpo[..len];

    // Pasada 1: contar filas totales (con wrap).
    let mut filas_total = 1usize;
    let mut col = 0usize;
    for &c in buffer {
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

    // Pasada 2: renderizar desde la fila `skip`. Caret en cualquier punto del
    // buffer cuyo byte_index coincida con `cursor`. Si el cursor cae al final
    // (`cursor == len`), pintamos el caret tras el ultimo glifo emitido.
    let mut fila_actual = 0usize;
    let mut col2 = 0usize;
    let mut caret_x: Option<usize> = None;
    let mut caret_y: usize = Y_TEXTO;
    for i in 0..len {
        let c = buffer[i];
        if c == b'\n' {
            // Cursor justo antes del newline: queda al final de la linea actual.
            if i == cursor && caret_x.is_none() && fila_actual >= skip {
                let rfila = fila_actual - skip;
                if rfila < FILAS {
                    caret_x = Some(MARGEN_X + col2 * PASO);
                    caret_y = Y_TEXTO + rfila * PASO;
                }
            }
            fila_actual += 1;
            col2 = 0;
            continue;
        }
        if col2 >= COLUMNAS {
            fila_actual += 1;
            col2 = 0;
        }
        // Cursor sobre este glifo: capturar DESPUES del wrap-check para que el
        // caret se pinte en la posicion visual real del char.
        if i == cursor && caret_x.is_none() && fila_actual >= skip {
            let rfila = fila_actual - skip;
            if rfila < FILAS {
                caret_x = Some(MARGEN_X + col2 * PASO);
                caret_y = Y_TEXTO + rfila * PASO;
            }
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
    // Caret al final del buffer si no se capturo antes.
    if caret_x.is_none() && cursor == len {
        if fila_actual >= skip {
            let rfila = fila_actual - skip;
            if rfila < FILAS {
                caret_x = Some(MARGEN_X + col2 * PASO);
                caret_y = Y_TEXTO + rfila * PASO;
            }
        }
    }
    if let Some(cx) = caret_x {
        pintar_caret(lienzo, cx, caret_y);
    }

    // SEGURIDAD: `sys_render_frame` valida (ptr, len) contra nuestra memoria.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

/// Caret: barra vertical de 2 px de ancho por PASO de alto, en color CARET.
fn pintar_caret(lienzo: &mut [u32], x: usize, y: usize) {
    let x_max = (x + 2).min(ANCHO);
    let y_max = (y + PASO).min(ALTO);
    let mut fila = y;
    while fila < y_max {
        let base = fila * ANCHO;
        let mut col = x;
        while col < x_max {
            lienzo[base + col] = CARET;
            col += 1;
        }
        fila += 1;
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

/// Traduce un MAKE-code del set 1 (US layout) a su caracter ASCII, respetando
/// el modificador Shift. Devuelve 0 para los scancodes que no producen texto
/// —modificadores, extendidos, etc.: el llamante los descarta sin gritar.
fn scancode_a_caracter(sc: u8, shift: bool) -> u8 {
    if shift {
        match sc {
            // Fila numerica: simbolos del US layout.
            0x02 => b'!', 0x03 => b'@', 0x04 => b'#', 0x05 => b'$', 0x06 => b'%',
            0x07 => b'^', 0x08 => b'&', 0x09 => b'*', 0x0A => b'(', 0x0B => b')',
            // Letras: mayusculas.
            0x10 => b'Q', 0x11 => b'W', 0x12 => b'E', 0x13 => b'R', 0x14 => b'T',
            0x15 => b'Y', 0x16 => b'U', 0x17 => b'I', 0x18 => b'O', 0x19 => b'P',
            0x1E => b'A', 0x1F => b'S', 0x20 => b'D', 0x21 => b'F', 0x22 => b'G',
            0x23 => b'H', 0x24 => b'J', 0x25 => b'K', 0x26 => b'L',
            0x2C => b'Z', 0x2D => b'X', 0x2E => b'C', 0x2F => b'V', 0x30 => b'B',
            0x31 => b'N', 0x32 => b'M',
            // Puntuacion con shift.
            0x33 => b'<', 0x34 => b'>', 0x35 => b'?',
            0x27 => b':', 0x28 => b'"',
            0x0C => b'_', 0x0D => b'+',
            0x1A => b'{', 0x1B => b'}',
            0x2B => b'|', 0x29 => b'~',
            0x39 => b' ',
            _ => 0,
        }
    } else {
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
            0x0C => b'-', 0x0D => b'=',
            0x1A => b'[', 0x1B => b']',
            0x2B => b'\\', 0x29 => b'`',
            0x39 => b' ',
            _ => 0,
        }
    }
}
