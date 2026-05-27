// =============================================================================
//  renaser :: apps/bitacora — editor de notas personales que recuerda
// -----------------------------------------------------------------------------
//  La fase 7c le dio a las apps memoria mas alla del arranque: `sys_estado_*`
//  ancla la huella de un app en el grafo, y al reiniciar el kernel se la
//  devuelve. `memoriosa` lo demostro contando teclas. `bitacora` lo lleva al
//  siguiente paso natural: ofrecer un editor de texto. Lo que la app sabe hacer
//  hoy:
//
//    1. **Ocho notas independientes**. `F1`..`F8` cambian la nota activa.
//       Cada una tiene su propio buffer (8 KiB), su propio cursor y su propio
//       scroll. El header pinta `1 2 3 4 5 6 7 8` con la activa en tinta y
//       el resto atenuado.
//
//    2. **Cursor editable in-situ**. Flechas izquierda/derecha mueven el
//       cursor un byte; arriba/abajo lo llevan a la misma columna de la
//       linea anterior/siguiente (separadas por `\n`, sin wrap visual).
//       `Home`/`End` saltan al inicio/fin de la linea logica.
//       `Backspace`/`Delete` borran atras/adelante. Cualquier tecla de
//       texto se INSERTA en la posicion del cursor (memmove del resto).
//
//    3. **Shift sostenido**. Tracking de LShift (`0x2A`/`0xAA`) y RShift
//       (`0x36`/`0xB6`) ANTES del filtro general de key-up — asi una
//       mayuscula sostenida no se "suelta" al pulsar otra tecla. Con
//       Shift: A-Z, `!@#$%^&*()` en la fila numerica, y `<>?:"_+{}|~` en
//       el resto. Sin Shift: minusculas + `-=[]\` y backtick.
//
//    4. **Seleccion + portapapeles interno**. Con Shift sostenido las
//       flechas y `Home`/`End` EXTIENDEN una seleccion en lugar de mover
//       el cursor solo. Cualquier movimiento sin Shift colapsa la
//       seleccion. `Ctrl+C` copia, `Ctrl+X` corta, `Ctrl+V` pega,
//       `Ctrl+A` selecciona todo. El portapapeles vive en la memoria
//       lineal de la app (un buffer estatico) — sirve para mover texto
//       entre las ocho notas sin tocar al kernel. Cualquier insercion o
//       borrado reemplaza primero la seleccion existente.
//
//    5. **Scroll vertical**. `PageUp`/`PageDown` desplazan el viewport
//       FILAS lineas visuales sin mover el cursor — util para releer el
//       principio de una nota larga. `Ctrl+Home`/`Ctrl+End` saltan al
//       principio/fin del documento (cursor + scroll a la vez). Cualquier
//       movimiento de cursor mantiene el caret visible: si se sale del
//       viewport, el scroll lo sigue.
//
//    6. **Persistencia v3**. Cada cambio (texto, cursor, scroll, nota
//       activa) se persiste de inmediato con un formato firmado por el
//       magic `BTC2`. Soporta v2 (Fase 18, sin scroll) y v3 (esta) sin
//       romper notas viejas. Si el buffer que el kernel devuelve no
//       lleva el magic, se asume formato Fase 17 (un buffer plano) y se
//       carga en la nota 1 con el cursor al final.
//
//  Tipografia: la 8x8 clasica (font8x8), escalada x2 a 16x16. Cabe en la
//  memoria lineal de la app y se renderiza pixel a pixel — no toca el
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
/// Fondo del rectangulo de seleccion — azul profundo bajo TINTA.
const SELECCION: u32 = 0x24_3C_72;

/// Centinela "sin seleccion" en `sel_anchor`. Se elige fuera del rango
/// representable [0, CAP_NOTA] para no chocar con posiciones validas.
const SIN_SEL: u32 = u32::MAX;

/// Una nota: su buffer de texto, el cursor, el ancla de seleccion (o
/// `SIN_SEL`) y el scroll vertical (en filas visuales). Sin `String`, sin
/// `Vec`: arrays estaticos puros, asi la app no necesita allocator.
#[derive(Copy, Clone)]
struct Nota {
    cuerpo: [u8; CAP_NOTA],
    len: u32,
    cursor: u32,
    sel_anchor: u32,
    scroll: u32,
}

const NOTA_VACIA: Nota = Nota {
    cuerpo: [0; CAP_NOTA],
    len: 0,
    cursor: 0,
    sel_anchor: SIN_SEL,
    scroll: 0,
};

static mut NOTAS: [Nota; NUM_NOTAS] = [NOTA_VACIA; NUM_NOTAS];
static mut ACTIVA: usize = 0;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// Estado de las teclas modificadoras. Se actualiza con make/break ANTES del
/// filtro general de key-up — asi un modifier sostenido se mantiene activo
/// entre pulsaciones de otras teclas.
static mut SHIFT: bool = false;
static mut CTRL: bool = false;
/// Recuerda si el scancode anterior fue el prefijo extendido `0xE0`. Las
/// flechas, Home/End, Delete, PageUp/Down, RCtrl, etc. llegan como pareja
/// `0xE0 <sc>`; ponemos el flag al ver el `0xE0` y lo bajamos al consumir el
/// byte siguiente.
static mut EXT_PREFIX: bool = false;

/// Portapapeles interno: vive en la memoria lineal de la app. Cualquier
/// `Ctrl+C` o `Ctrl+X` lo reemplaza; `Ctrl+V` lo inserta en cursor. Sirve
/// para mover texto entre notas dentro de la misma sesion.
static mut CLIPBOARD: [u8; CAP_NOTA] = [0; CAP_NOTA];
static mut CLIP_LEN: usize = 0;

/// Buffer de E/S del estado persistido. Formato v3:
///   [magic 4B "BTC2"][version 1B = 0x03][activa 1B][pad 2B = 0]
///   for i in 0..NUM_NOTAS:
///     [len_i u32 LE][cursor_i u32 LE][scroll_i u32 LE][bytes_i: len_i bytes]
const HDR: usize = 4 + 1 + 1 + 2;
const NOTA_HDR_V3: usize = 4 + 4 + 4;
const ESTADO_CAP: usize = HDR + NUM_NOTAS * (NOTA_HDR_V3 + CAP_NOTA);
static mut ESTADO_IO: [u8; ESTADO_CAP] = [0; ESTADO_CAP];

const MAGIC: [u8; 4] = *b"BTC2";
const VERSION: u8 = 0x03;

// --- ABI del userspace --------------------------------------------------------

#[no_mangle]
pub extern "C" fn init() {
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(ESTADO_IO) };
    let n = unsafe { sys_estado_cargar(buf.as_mut_ptr() as u32, ESTADO_CAP as u32) };
    if n > 0 {
        let n = (n as usize).min(ESTADO_CAP);
        if !cargar_btc2(buf, n) {
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

        // Modifiers. Se siguen ANTES del filtro general de key-up. Ctrl tiene
        // make `0x1D` (LCtrl no-ext / RCtrl ext) y break `0x9D` con o sin ext.
        match (sc, ext) {
            (0x2A, false) | (0x36, false) => { unsafe { SHIFT = true; } continue; }
            (0xAA, false) | (0xB6, false) => { unsafe { SHIFT = false; } continue; }
            (0x1D, _) => { unsafe { CTRL = true; } continue; }
            (0x9D, _) => { unsafe { CTRL = false; } continue; }
            _ => {}
        }

        // Resto de key-ups (bit 7): ignorar.
        if sc & 0x80 != 0 {
            continue;
        }

        let shift = unsafe { SHIFT };
        let ctrl = unsafe { CTRL };

        // F1..F8 → cambiar nota activa. Sin ext, sin Ctrl.
        if !ext && !ctrl {
            if let Some(idx) = tecla_funcion(sc) {
                if idx < NUM_NOTAS {
                    unsafe { ACTIVA = idx; }
                    cambio_ui = true;
                    continue;
                }
            }
        }

        if ext {
            if ctrl {
                match sc {
                    0x47 => { ctrl_home(); cambio_ui = true; continue; } // Ctrl+Home
                    0x4F => { ctrl_end();  cambio_ui = true; continue; } // Ctrl+End
                    _ => {}
                }
            }
            match sc {
                0x4B => { mover_izquierda();  cambio_ui = true; }
                0x4D => { mover_derecha();    cambio_ui = true; }
                0x48 => { mover_arriba();     cambio_ui = true; }
                0x50 => { mover_abajo();      cambio_ui = true; }
                0x47 => { mover_inicio_linea(); cambio_ui = true; }
                0x4F => { mover_fin_linea();    cambio_ui = true; }
                0x49 => { page_up();   cambio_ui = true; } // PageUp
                0x51 => { page_down(); cambio_ui = true; } // PageDown
                0x53 => { // Delete
                    if borrar_seleccion() || borrar_adelante() {
                        cambio_texto = true;
                    }
                }
                _ => {}
            }
            continue;
        }

        if ctrl {
            match sc {
                0x2E => { // Ctrl+C
                    copiar_seleccion();
                    cambio_ui = true;
                }
                0x2D => { // Ctrl+X
                    copiar_seleccion();
                    if borrar_seleccion() {
                        cambio_texto = true;
                    }
                }
                0x2F => { // Ctrl+V
                    if pegar() {
                        cambio_texto = true;
                    }
                }
                0x1E => { // Ctrl+A
                    seleccionar_todo();
                    cambio_ui = true;
                }
                _ => {}
            }
            continue;
        }

        match sc {
            0x0E => { // Backspace
                if borrar_seleccion() || borrar_atras() {
                    cambio_texto = true;
                }
            }
            0x1C => {
                insertar(b'\n');
                cambio_texto = true;
            }
            otro => {
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

/// Aplica `f` a la nota activa.
fn con_activa<F: FnOnce(&mut Nota)>(f: F) {
    unsafe {
        let idx = ACTIVA.min(NUM_NOTAS - 1);
        let notas = &mut *core::ptr::addr_of_mut!(NOTAS);
        f(&mut notas[idx]);
    }
}

/// Rango ordenado `[lo, hi)` de la seleccion activa, o `None` si no hay.
fn rango_sel(n: &Nota) -> Option<(usize, usize)> {
    if n.sel_anchor == SIN_SEL {
        return None;
    }
    let a = n.sel_anchor as usize;
    let c = n.cursor as usize;
    if a == c {
        None
    } else if a < c {
        Some((a, c))
    } else {
        Some((c, a))
    }
}

/// Antes de un movimiento de cursor: si Shift esta sostenido, fijamos el
/// ancla en la posicion actual del cursor (si aun no habia ancla); si no,
/// colapsamos la seleccion previa.
fn antes_de_mover() {
    let shift = unsafe { SHIFT };
    con_activa(|n| {
        if shift {
            if n.sel_anchor == SIN_SEL {
                n.sel_anchor = n.cursor;
            }
        } else {
            n.sel_anchor = SIN_SEL;
        }
    });
}

/// Garantiza que el caret quede dentro del viewport vertical actual; mueve
/// `scroll` lo minimo necesario.
fn asegurar_cursor_visible() {
    con_activa(|n| {
        let fila = fila_visual(&n.cuerpo, n.len as usize, n.cursor as usize) as u32;
        if fila < n.scroll {
            n.scroll = fila;
        } else if fila >= n.scroll + FILAS as u32 {
            n.scroll = fila - FILAS as u32 + 1;
        }
    });
}

/// Inserta `c` en la posicion del cursor; mueve el resto un byte a la derecha.
/// Si hay seleccion activa, la sustituye. Si la nota esta llena, descarta los
/// primeros `DESCARTE` bytes y reajusta cursor (pierde posicion solo si caia
/// dentro del fragmento descartado).
fn insertar(c: u8) {
    let _ = borrar_seleccion();
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
        n.cuerpo.copy_within(cursor..len, cursor + 1);
        n.cuerpo[cursor] = c;
        n.len = (len + 1) as u32;
        n.cursor = (cursor + 1) as u32;
    });
    asegurar_cursor_visible();
}

/// Borra el byte inmediatamente atras del cursor — `Backspace` clasico.
/// Devuelve `true` si hubo algo que borrar.
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
    if cambio { asegurar_cursor_visible(); }
    cambio
}

/// Borra el byte EN la posicion del cursor — `Delete`.
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

/// Borra el rango de la seleccion actual y colapsa el ancla. Devuelve `true`
/// si habia algo que borrar.
fn borrar_seleccion() -> bool {
    let mut cambio = false;
    con_activa(|n| {
        if let Some((lo, hi)) = rango_sel(n) {
            let len = n.len as usize;
            n.cuerpo.copy_within(hi..len, lo);
            n.len = (len - (hi - lo)) as u32;
            n.cursor = lo as u32;
            n.sel_anchor = SIN_SEL;
            cambio = true;
        }
    });
    if cambio { asegurar_cursor_visible(); }
    cambio
}

/// Copia la seleccion (si la hay) al portapapeles interno. No toca el buffer.
fn copiar_seleccion() {
    con_activa(|n| {
        if let Some((lo, hi)) = rango_sel(n) {
            let len = hi - lo;
            unsafe {
                let clip = &mut *core::ptr::addr_of_mut!(CLIPBOARD);
                clip[..len].copy_from_slice(&n.cuerpo[lo..hi]);
                CLIP_LEN = len;
            }
        }
    });
}

/// Inserta el contenido del portapapeles en el cursor (o sustituye la
/// seleccion, si la hay). Si no cabe entero, descarta bytes del principio en
/// pasos de DESCARTE hasta hacer hueco.
fn pegar() -> bool {
    let _ = borrar_seleccion();
    let n_clip = unsafe { CLIP_LEN };
    if n_clip == 0 {
        return false;
    }
    let mut cambio = false;
    con_activa(|n| {
        while (n.len as usize) + n_clip > CAP_NOTA {
            if n.len == 0 {
                break;
            }
            let descartado = DESCARTE.min(n.len as usize);
            n.cuerpo.copy_within(descartado.., 0);
            n.len -= descartado as u32;
            n.cursor = n.cursor.saturating_sub(descartado as u32);
        }
        let espacio = CAP_NOTA - (n.len as usize);
        let bytes = n_clip.min(espacio);
        if bytes == 0 {
            return;
        }
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        n.cuerpo.copy_within(cursor..len, cursor + bytes);
        unsafe {
            let clip = &*core::ptr::addr_of!(CLIPBOARD);
            n.cuerpo[cursor..cursor + bytes].copy_from_slice(&clip[..bytes]);
        }
        n.len = (len + bytes) as u32;
        n.cursor = (cursor + bytes) as u32;
        cambio = true;
    });
    if cambio { asegurar_cursor_visible(); }
    cambio
}

/// Selecciona todo el buffer de la nota activa.
fn seleccionar_todo() {
    con_activa(|n| {
        if n.len == 0 {
            return;
        }
        n.sel_anchor = 0;
        n.cursor = n.len;
    });
    asegurar_cursor_visible();
}

// --- Movimientos de cursor ----------------------------------------------------

fn mover_izquierda() {
    antes_de_mover();
    con_activa(|n| {
        if n.cursor > 0 {
            n.cursor -= 1;
        }
    });
    asegurar_cursor_visible();
}

fn mover_derecha() {
    antes_de_mover();
    con_activa(|n| {
        if (n.cursor as usize) < (n.len as usize) {
            n.cursor += 1;
        }
    });
    asegurar_cursor_visible();
}

/// Devuelve `(inicio_linea, columna)` para la posicion `cursor` dentro de
/// `buf[..len]` — separadas por `\n`, sin contar el wrap visual.
fn linea_actual(buf: &[u8], _len: usize, cursor: usize) -> (usize, usize) {
    let mut inicio = 0usize;
    let mut i = cursor;
    while i > 0 {
        i -= 1;
        if buf[i] == b'\n' {
            inicio = i + 1;
            break;
        }
    }
    (inicio, cursor - inicio)
}

/// Indice del siguiente `\n` desde `pos`, o `len` si no hay.
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
    antes_de_mover();
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let (inicio_actual, col) = linea_actual(&n.cuerpo, len, cursor);
        if inicio_actual == 0 {
            return;
        }
        let fin_anterior = inicio_actual - 1;
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
    asegurar_cursor_visible();
}

fn mover_abajo() {
    antes_de_mover();
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let (_, col) = linea_actual(&n.cuerpo, len, cursor);
        let fin_actual = fin_linea(&n.cuerpo, len, cursor);
        if fin_actual >= len {
            return;
        }
        let inicio_siguiente = fin_actual + 1;
        let fin_siguiente = fin_linea(&n.cuerpo, len, inicio_siguiente);
        let largo_siguiente = fin_siguiente - inicio_siguiente;
        n.cursor = (inicio_siguiente + col.min(largo_siguiente)) as u32;
    });
    asegurar_cursor_visible();
}

fn mover_inicio_linea() {
    antes_de_mover();
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let (inicio, _) = linea_actual(&n.cuerpo, len, cursor);
        n.cursor = inicio as u32;
    });
    asegurar_cursor_visible();
}

fn mover_fin_linea() {
    antes_de_mover();
    con_activa(|n| {
        let len = n.len as usize;
        let cursor = (n.cursor as usize).min(len);
        let fin = fin_linea(&n.cuerpo, len, cursor);
        n.cursor = fin as u32;
    });
    asegurar_cursor_visible();
}

/// PageUp: scroll FILAS lineas visuales arriba. NO mueve el cursor — util
/// para releer el principio mientras se deja el caret en su sitio.
fn page_up() {
    con_activa(|n| {
        n.scroll = n.scroll.saturating_sub(FILAS as u32);
    });
}

/// PageDown: scroll FILAS lineas visuales abajo, sin pasarse del total.
fn page_down() {
    con_activa(|n| {
        let total = filas_totales(&n.cuerpo, n.len as usize) as u32;
        let max_scroll = total.saturating_sub(1);
        n.scroll = (n.scroll + FILAS as u32).min(max_scroll);
    });
}

/// Ctrl+Home: cursor y viewport al principio absoluto.
fn ctrl_home() {
    antes_de_mover();
    con_activa(|n| {
        n.cursor = 0;
        n.scroll = 0;
    });
}

/// Ctrl+End: cursor al final, viewport al ultimo bloque visible.
fn ctrl_end() {
    antes_de_mover();
    con_activa(|n| {
        n.cursor = n.len;
    });
    asegurar_cursor_visible();
}

/// `0x3B..=0x42` = F1..F8. F9..F12 no se mapean (NUM_NOTAS = 8).
fn tecla_funcion(sc: u8) -> Option<usize> {
    if (0x3B..=0x42).contains(&sc) {
        Some((sc - 0x3B) as usize)
    } else {
        None
    }
}

/// Fila visual del byte `pos` en `buf[..len]`, contando wraps por `COLUMNAS`.
fn fila_visual(buf: &[u8], len: usize, pos: usize) -> usize {
    let pos = pos.min(len);
    let mut fila = 0usize;
    let mut col = 0usize;
    for i in 0..pos {
        if buf[i] == b'\n' {
            fila += 1;
            col = 0;
        } else {
            if col >= COLUMNAS {
                fila += 1;
                col = 0;
            }
            col += 1;
        }
    }
    fila
}

/// Numero total de filas visuales que ocupa `buf[..len]`.
fn filas_totales(buf: &[u8], len: usize) -> usize {
    let mut filas = 1usize;
    let mut col = 0usize;
    for i in 0..len {
        if buf[i] == b'\n' {
            filas += 1;
            col = 0;
        } else {
            if col >= COLUMNAS {
                filas += 1;
                col = 0;
            }
            col += 1;
        }
    }
    filas
}

// --- Persistencia: serializar/deserializar el estado completo -----------------

fn guardar() {
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
        buf[off + 8..off + 12].copy_from_slice(&n.scroll.to_le_bytes());
        off += NOTA_HDR_V3;
        buf[off..off + len].copy_from_slice(&n.cuerpo[..len]);
        off += len;
    }
    unsafe {
        let _ = sys_estado_guardar(buf.as_ptr() as u32, off as u32);
    }
}

/// Carga formato BTC2 (v2 = Fase 18 sin scroll, v3 = esta con scroll).
/// Devuelve `false` si no es BTC2.
fn cargar_btc2(buf: &[u8], n: usize) -> bool {
    if n < HDR || buf[0..4] != MAGIC {
        return false;
    }
    let ver = buf[4];
    if ver != 0x02 && ver != 0x03 {
        return false;
    }
    let activa = buf[5] as usize;
    let notas = unsafe { &mut *core::ptr::addr_of_mut!(NOTAS) };
    let nota_hdr = if ver == 0x03 { NOTA_HDR_V3 } else { 4 + 4 };
    let mut off = HDR;
    for nota in notas.iter_mut() {
        if off + nota_hdr > n {
            break;
        }
        let len = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let cursor = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let scroll = if ver == 0x03 {
            u32::from_le_bytes([buf[off + 8], buf[off + 9], buf[off + 10], buf[off + 11]])
        } else {
            0
        };
        off += nota_hdr;
        let len = len.min(CAP_NOTA).min(n.saturating_sub(off));
        nota.cuerpo[..len].copy_from_slice(&buf[off..off + len]);
        nota.len = len as u32;
        nota.cursor = cursor.min(len as u32);
        nota.scroll = scroll;
        nota.sel_anchor = SIN_SEL;
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
    notas[0].scroll = 0;
    notas[0].sel_anchor = SIN_SEL;
    unsafe { ACTIVA = 0; }
}

// --- Renderizado --------------------------------------------------------------

fn pintar() {
    let lienzo = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
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

    let notas = unsafe { &*core::ptr::addr_of!(NOTAS) };
    let nota = &notas[activa.min(NUM_NOTAS - 1)];
    let len = (nota.len as usize).min(CAP_NOTA);
    let cursor = (nota.cursor as usize).min(len);
    let scroll = nota.scroll as usize;
    let sel = rango_sel(nota);
    let buffer = &nota.cuerpo[..len];

    // Una sola pasada: cuenta filas mientras pinta, capturando el caret en
    // su posicion visual EXACTA y rellenando el fondo de la seleccion antes
    // de cada glifo. `skip = scroll` (fila visual del tope del viewport).
    let skip = scroll;
    let mut fila_actual = 0usize;
    let mut col2 = 0usize;
    let mut caret_x: Option<usize> = None;
    let mut caret_y: usize = Y_TEXTO;
    for i in 0..len {
        let c = buffer[i];
        let visible = fila_actual >= skip && (fila_actual - skip) < FILAS;
        let dentro_sel = sel.map(|(lo, hi)| i >= lo && i < hi).unwrap_or(false);

        if c == b'\n' {
            // Cursor justo antes del newline: queda al final de la linea actual.
            if i == cursor && caret_x.is_none() && visible {
                let rfila = fila_actual - skip;
                caret_x = Some(MARGEN_X + col2 * PASO);
                caret_y = Y_TEXTO + rfila * PASO;
            }
            // Seleccion incluye el newline: pinta un cuadrado al final.
            if dentro_sel && visible {
                let rfila = fila_actual - skip;
                let x = MARGEN_X + col2 * PASO;
                let y = Y_TEXTO + rfila * PASO;
                rellenar(lienzo, x, y, PASO, PASO, SELECCION);
            }
            fila_actual += 1;
            col2 = 0;
            continue;
        }
        if col2 >= COLUMNAS {
            fila_actual += 1;
            col2 = 0;
        }
        let visible = fila_actual >= skip && (fila_actual - skip) < FILAS;
        if i == cursor && caret_x.is_none() && visible {
            let rfila = fila_actual - skip;
            caret_x = Some(MARGEN_X + col2 * PASO);
            caret_y = Y_TEXTO + rfila * PASO;
        }
        if visible {
            let rfila = fila_actual - skip;
            let x = MARGEN_X + col2 * PASO;
            let y = Y_TEXTO + rfila * PASO;
            if dentro_sel {
                rellenar(lienzo, x, y, PASO, PASO, SELECCION);
            }
            pintar_glifo(lienzo, c, x, y, TINTA);
        }
        col2 += 1;
    }
    // Caret al final del buffer si no se capturo antes.
    if caret_x.is_none() && cursor == len {
        let visible = fila_actual >= skip && (fila_actual - skip) < FILAS;
        if visible {
            let rfila = fila_actual - skip;
            caret_x = Some(MARGEN_X + col2 * PASO);
            caret_y = Y_TEXTO + rfila * PASO;
        }
    }
    if let Some(cx) = caret_x {
        pintar_caret(lienzo, cx, caret_y);
    }

    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

/// Caret: barra vertical de 2 px de ancho por PASO de alto en color CARET.
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

/// Rellena un rectangulo recortado a los limites del lienzo.
fn rellenar(lienzo: &mut [u32], x: usize, y: usize, ancho: usize, alto: usize, color: u32) {
    let x1 = (x + ancho).min(ANCHO);
    let y1 = (y + alto).min(ALTO);
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

/// Pinta una cadena ASCII en (x, y_base).
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

/// Pinta un solo glifo 8x8 escalado a 16x16. No ASCII se renderiza como `?`.
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
/// Shift. Devuelve 0 para los scancodes que no producen texto.
fn scancode_a_caracter(sc: u8, shift: bool) -> u8 {
    if shift {
        match sc {
            0x02 => b'!', 0x03 => b'@', 0x04 => b'#', 0x05 => b'$', 0x06 => b'%',
            0x07 => b'^', 0x08 => b'&', 0x09 => b'*', 0x0A => b'(', 0x0B => b')',
            0x10 => b'Q', 0x11 => b'W', 0x12 => b'E', 0x13 => b'R', 0x14 => b'T',
            0x15 => b'Y', 0x16 => b'U', 0x17 => b'I', 0x18 => b'O', 0x19 => b'P',
            0x1E => b'A', 0x1F => b'S', 0x20 => b'D', 0x21 => b'F', 0x22 => b'G',
            0x23 => b'H', 0x24 => b'J', 0x25 => b'K', 0x26 => b'L',
            0x2C => b'Z', 0x2D => b'X', 0x2E => b'C', 0x2F => b'V', 0x30 => b'B',
            0x31 => b'N', 0x32 => b'M',
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
