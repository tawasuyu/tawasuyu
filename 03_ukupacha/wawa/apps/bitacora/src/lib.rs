// =============================================================================
//  renaser :: apps/bitacora — editor de notas personales que recuerda
// -----------------------------------------------------------------------------
//  Editor de texto WASM que persiste entre arranques. Lo que sabe hacer hoy:
//
//    1. **Ocho notas independientes**. `F1`..`F8` cambian la nota activa.
//       Cada una tiene su propio buffer (8 KiB), cursor y scroll. El header
//       pinta `1 2 3 4 5 6 7 8` con la activa en tinta y el resto atenuado.
//
//    2. **Cursor editable in-situ**. Flechas izquierda/derecha mueven un
//       byte; arriba/abajo van a la misma columna de la linea anterior /
//       siguiente (separadas por `\n`). `Home`/`End` saltan al inicio /
//       fin de la linea logica. `Backspace`/`Delete` borran atras /
//       adelante. Cualquier tecla de texto se INSERTA en cursor.
//
//    3. **Shift sostenido**. Tracking de LShift (`0x2A`/`0xAA`) y RShift
//       (`0x36`/`0xB6`) ANTES del filtro general de key-up. Con Shift:
//       A-Z, `!@#$%^&*()` en la fila numerica, `<>?:"_+{}|~`. Sin Shift:
//       minusculas + `-=[]\` y backtick.
//
//    4. **Seleccion + portapapeles interno**. Con Shift sostenido las
//       flechas y `Home`/`End` EXTIENDEN una seleccion. Cualquier
//       movimiento sin Shift colapsa la seleccion. `Ctrl+C`/`Ctrl+X`/
//       `Ctrl+V`/`Ctrl+A`. El portapapeles vive en la memoria lineal de
//       la app — sirve para mover texto entre notas sin tocar al kernel.
//
//    5. **Scroll vertical**. `PageUp`/`PageDown` desplazan el viewport
//       FILAS lineas visuales sin mover el cursor. `Ctrl+Home`/`Ctrl+End`
//       saltan al principio/fin del documento. Cualquier movimiento de
//       cursor mantiene el caret visible.
//
//    6. **Wrap por palabra**. El render parte las lineas en el ultimo
//       espacio antes de COLUMNAS en lugar de en cualquier byte. Las
//       palabras mas largas que COLUMNAS siguen rompiendose por la
//       fuerza. Los movimientos `↑↓` siguen operando sobre lineas
//       LOGICAS (separadas por `\n`); las lineas VISUALES (wrap)
//       afectan solo al scroll.
//
//    7. **Modo busqueda**. `Ctrl+F` entra a modo busqueda; aparece una
//       barra abajo con el query. Tecleas para refinar, `Backspace`
//       borra, `Enter` salta al siguiente match, `Esc` sale. El render
//       destaca los matches con fondo amarillo (distinto del azul de
//       seleccion). Case-insensitive.
//
//    8. **Modo indice (F9)**. Overlay a pantalla completa: lista las 8
//       notas con su numero, primera linea como titulo y bytes usados.
//       Flechas `↑↓` navegan; `Enter` o `F1`..`F8` eligen y vuelven a
//       NORMAL. `Esc` cierra sin cambiar.
//
//    9. **Persistencia v3**. Cada cambio se persiste con un formato
//       firmado por el magic `BTC2`. Soporta v2 (sin scroll), v3 (con
//       scroll) y el formato Fase 17 (un buffer plano) — la nota del
//       usuario sobrevive a cada upgrade sin perdida.
//
//  Tipografia: la 8x8 clasica (font8x8), escalada x2 a 16x16. Cabe en la
//  memoria lineal de la app y se renderiza pixel a pixel — la app entrega
//  su propio fotograma; el kernel solo lo compone en su region.
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

const ANCHO: usize = 480;
const ALTO: usize = 280;
const PASO: usize = 16;
const MARGEN_X: usize = 16;
const Y_LABEL: usize = 6;
const Y_TEXTO: usize = 38;
const COLUMNAS: usize = (ANCHO - 2 * MARGEN_X) / PASO;
/// Filas visibles del cuerpo. La fila bajo este bloque (`Y_BARRA`) se reserva
/// para la barra de busqueda; en modo normal queda en blanco.
const FILAS: usize = 14;
const Y_BARRA: usize = Y_TEXTO + FILAS * PASO;

// --- Modos --------------------------------------------------------------------

const MODO_NORMAL: u8 = 0;
const MODO_BUSCAR: u8 = 1;
const MODO_INDICE: u8 = 2;
static mut MODO: u8 = MODO_NORMAL;

// --- Estado de busqueda -------------------------------------------------------

const CAP_QUERY: usize = 64;
static mut QUERY: [u8; CAP_QUERY] = [0; CAP_QUERY];
static mut QUERY_LEN: usize = 0;
/// Posiciones de los matches encontrados sobre la nota activa para la query
/// actual. Recalculadas a demanda (cuando query o nota cambian, o cada render
/// del modo busqueda). Cap practico para una nota de 8 KiB con query corta.
const CAP_MATCHES: usize = 256;
static mut MATCH_POS: [u32; CAP_MATCHES] = [0; CAP_MATCHES];
static mut MATCH_LEN: usize = 0;

// --- Estado del indice --------------------------------------------------------

/// Nota focada en el panel indice (NO la activa: solo la que tiene el
/// resaltado de navegacion). Enter la convierte en activa.
static mut INDICE_FOCO: usize = 0;

// --- Estado de notas ----------------------------------------------------------

const NUM_NOTAS: usize = 8;
const CAP_NOTA: usize = 8192;
const DESCARTE: usize = 512;
const SIN_SEL: u32 = u32::MAX;

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

static mut SHIFT: bool = false;
static mut CTRL: bool = false;
static mut EXT_PREFIX: bool = false;

static mut CLIPBOARD: [u8; CAP_NOTA] = [0; CAP_NOTA];
static mut CLIP_LEN: usize = 0;

// --- Colores ------------------------------------------------------------------

const FONDO: u32 = 0x0A_18_30;
const TINTA: u32 = 0xE8_EC_F4;
const ETIQUETA: u32 = 0x8B_5C_F6;
const INACTIVO: u32 = 0x4A_3A_7C;
const CARET: u32 = 0xF8_E8_8A;
const SELECCION: u32 = 0x24_3C_72;
/// Fondo del match en modo busqueda — amarillo oscuro.
const MATCH_BG: u32 = 0x6A_60_18;
/// Fondo de la tile focada en el panel indice — morado profundo.
const FOCO_BG: u32 = 0x3A_2A_5A;

// --- Persistencia: cabecera ---------------------------------------------------

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
        if sc == 0xE0 {
            unsafe { EXT_PREFIX = true; }
            continue;
        }
        let ext = unsafe { EXT_PREFIX };
        unsafe { EXT_PREFIX = false; }

        // Modifiers ANTES del filtro de key-up. Ctrl tiene make `0x1D` (LCtrl
        // no-ext / RCtrl ext) y break `0x9D` con o sin ext.
        match (sc, ext) {
            (0x2A, false) | (0x36, false) => { unsafe { SHIFT = true; } continue; }
            (0xAA, false) | (0xB6, false) => { unsafe { SHIFT = false; } continue; }
            (0x1D, _) => { unsafe { CTRL = true; } continue; }
            (0x9D, _) => { unsafe { CTRL = false; } continue; }
            _ => {}
        }

        if sc & 0x80 != 0 {
            continue;
        }

        let shift = unsafe { SHIFT };
        let ctrl = unsafe { CTRL };
        let modo = unsafe { MODO };

        match modo {
            MODO_NORMAL => {
                dispatch_normal(sc, ext, shift, ctrl, &mut cambio_texto, &mut cambio_ui);
            }
            MODO_BUSCAR => {
                dispatch_buscar(sc, ext, shift, &mut cambio_ui);
            }
            MODO_INDICE => {
                dispatch_indice(sc, ext, &mut cambio_ui);
            }
            _ => {}
        }
    }
    if cambio_texto || cambio_ui {
        guardar();
    }
    pintar();
}

// --- Dispatch por modo --------------------------------------------------------

fn dispatch_normal(sc: u8, ext: bool, shift: bool, ctrl: bool, cambio_texto: &mut bool, cambio_ui: &mut bool) {
    // F1..F8 (sin ext, sin ctrl) cambian de nota activa.
    if !ext && !ctrl {
        if let Some(idx) = tecla_funcion(sc) {
            if idx < NUM_NOTAS {
                unsafe { ACTIVA = idx; }
                *cambio_ui = true;
                return;
            }
        }
        // F9 abre el panel indice.
        if sc == 0x43 {
            unsafe {
                INDICE_FOCO = ACTIVA;
                MODO = MODO_INDICE;
            }
            *cambio_ui = true;
            return;
        }
    }

    if ext {
        if ctrl {
            match sc {
                0x47 => { ctrl_home(); *cambio_ui = true; return; }
                0x4F => { ctrl_end();  *cambio_ui = true; return; }
                _ => {}
            }
        }
        match sc {
            0x4B => { mover_izquierda();  *cambio_ui = true; }
            0x4D => { mover_derecha();    *cambio_ui = true; }
            0x48 => { mover_arriba();     *cambio_ui = true; }
            0x50 => { mover_abajo();      *cambio_ui = true; }
            0x47 => { mover_inicio_linea(); *cambio_ui = true; }
            0x4F => { mover_fin_linea();    *cambio_ui = true; }
            0x49 => { page_up();   *cambio_ui = true; }
            0x51 => { page_down(); *cambio_ui = true; }
            0x53 => {
                if borrar_seleccion() || borrar_adelante() {
                    *cambio_texto = true;
                }
            }
            _ => {}
        }
        return;
    }

    if ctrl {
        match sc {
            0x2E => { copiar_seleccion(); *cambio_ui = true; }       // Ctrl+C
            0x2D => {                                                 // Ctrl+X
                copiar_seleccion();
                if borrar_seleccion() {
                    *cambio_texto = true;
                }
            }
            0x2F => { if pegar() { *cambio_texto = true; } }          // Ctrl+V
            0x1E => { seleccionar_todo(); *cambio_ui = true; }        // Ctrl+A
            0x21 => {                                                 // Ctrl+F
                unsafe {
                    MODO = MODO_BUSCAR;
                    QUERY_LEN = 0;
                }
                *cambio_ui = true;
            }
            _ => {}
        }
        return;
    }

    match sc {
        0x0E => {
            if borrar_seleccion() || borrar_atras() {
                *cambio_texto = true;
            }
        }
        0x1C => {
            insertar(b'\n');
            *cambio_texto = true;
        }
        otro => {
            let c = scancode_a_caracter(otro, shift);
            if c != 0 {
                insertar(c);
                *cambio_texto = true;
            }
        }
    }
}

fn dispatch_buscar(sc: u8, ext: bool, shift: bool, cambio_ui: &mut bool) {
    // Esc sale del modo busqueda.
    if !ext && sc == 0x01 {
        unsafe { MODO = MODO_NORMAL; }
        *cambio_ui = true;
        return;
    }
    // F1..F8 cambian de nota y salen del modo busqueda.
    if !ext {
        if let Some(idx) = tecla_funcion(sc) {
            if idx < NUM_NOTAS {
                unsafe {
                    ACTIVA = idx;
                    MODO = MODO_NORMAL;
                    QUERY_LEN = 0;
                }
                *cambio_ui = true;
                return;
            }
        }
    }
    // Enter salta al siguiente match (Shift+Enter al anterior).
    if !ext && sc == 0x1C {
        if shift {
            ir_a_anterior_match();
        } else {
            ir_a_siguiente_match();
        }
        *cambio_ui = true;
        return;
    }
    // Backspace borra del query.
    if !ext && sc == 0x0E {
        unsafe {
            if QUERY_LEN > 0 {
                QUERY_LEN -= 1;
            }
        }
        *cambio_ui = true;
        return;
    }
    // Texto anexado al query.
    if !ext {
        let c = scancode_a_caracter(sc, shift);
        if c != 0 {
            unsafe {
                if QUERY_LEN < CAP_QUERY {
                    let q = &mut *core::ptr::addr_of_mut!(QUERY);
                    q[QUERY_LEN] = c;
                    QUERY_LEN += 1;
                }
            }
            *cambio_ui = true;
        }
    }
    // Flechas en ext se ignoran en modo busqueda — la atencion del usuario
    // esta en la barra; tras Esc el cursor sigue donde estaba.
}

fn dispatch_indice(sc: u8, ext: bool, cambio_ui: &mut bool) {
    // Esc cierra el indice sin cambiar la activa.
    if !ext && sc == 0x01 {
        unsafe { MODO = MODO_NORMAL; }
        *cambio_ui = true;
        return;
    }
    // Enter elige la nota focada.
    if !ext && sc == 0x1C {
        unsafe {
            ACTIVA = INDICE_FOCO.min(NUM_NOTAS - 1);
            MODO = MODO_NORMAL;
        }
        *cambio_ui = true;
        return;
    }
    // F1..F8 eligen directamente.
    if !ext {
        if let Some(idx) = tecla_funcion(sc) {
            if idx < NUM_NOTAS {
                unsafe {
                    ACTIVA = idx;
                    MODO = MODO_NORMAL;
                }
                *cambio_ui = true;
                return;
            }
        }
    }
    // Flechas mueven el foco.
    if ext {
        match sc {
            0x48 => unsafe {
                if INDICE_FOCO > 0 { INDICE_FOCO -= 1; }
                *cambio_ui = true;
            },
            0x50 => unsafe {
                if INDICE_FOCO + 1 < NUM_NOTAS { INDICE_FOCO += 1; }
                *cambio_ui = true;
            },
            _ => {}
        }
    }
}

// --- Edicion del buffer activo ------------------------------------------------

fn con_activa<F: FnOnce(&mut Nota)>(f: F) {
    unsafe {
        let idx = ACTIVA.min(NUM_NOTAS - 1);
        let notas = &mut *core::ptr::addr_of_mut!(NOTAS);
        f(&mut notas[idx]);
    }
}

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

fn page_up() {
    con_activa(|n| {
        n.scroll = n.scroll.saturating_sub(FILAS as u32);
    });
}

fn page_down() {
    con_activa(|n| {
        let total = filas_totales(&n.cuerpo, n.len as usize) as u32;
        let max_scroll = total.saturating_sub(1);
        n.scroll = (n.scroll + FILAS as u32).min(max_scroll);
    });
}

fn ctrl_home() {
    antes_de_mover();
    con_activa(|n| {
        n.cursor = 0;
        n.scroll = 0;
    });
}

fn ctrl_end() {
    antes_de_mover();
    con_activa(|n| {
        n.cursor = n.len;
    });
    asegurar_cursor_visible();
}

fn tecla_funcion(sc: u8) -> Option<usize> {
    if (0x3B..=0x42).contains(&sc) {
        Some((sc - 0x3B) as usize)
    } else {
        None
    }
}

// --- Wrap visual (por palabra) ------------------------------------------------

/// Indice del primer byte de la SIGUIENTE linea visual desde `inicio`.
/// Reglas:
///   - Si encuentra un `\n` antes de COLUMNAS, wrap justo despues del `\n`.
///   - Si la linea cabe entera sin wrap, devuelve `len`.
///   - Si excede COLUMNAS, busca el ULTIMO espacio en `[inicio, inicio+COLUMNAS]`
///     y wrap justo despues. El espacio queda en la linea anterior; la
///     siguiente arranca con el primer no-espacio. Si no hay espacio (palabra
///     mas larga que COLUMNAS), wrap duro en COLUMNAS.
fn fin_linea_visual(buf: &[u8], len: usize, inicio: usize) -> usize {
    if inicio >= len {
        return len;
    }
    let mut i = inicio;
    let tope = (inicio + COLUMNAS).min(len);
    // 1) Buscar `\n` dentro de la primera COLUMNAS.
    while i < tope {
        if buf[i] == b'\n' {
            return i + 1;
        }
        i += 1;
    }
    if tope == len {
        return len;
    }
    if i >= len {
        return len;
    }
    // 2) No hay `\n` en la primera COLUMNAS. Buscar el ultimo espacio para
    //    wrappear en limite de palabra.
    let mut j = tope;
    while j > inicio {
        if buf[j - 1] == b' ' {
            return j;
        }
        j -= 1;
    }
    // 3) Sin espacio: wrap duro en COLUMNAS.
    tope
}

/// Fila visual del byte `pos` en `buf[..len]`, segun el wrap por palabra.
fn fila_visual(buf: &[u8], len: usize, pos: usize) -> usize {
    let pos = pos.min(len);
    let mut fila = 0usize;
    let mut inicio = 0usize;
    while inicio < pos {
        let fin = fin_linea_visual(buf, len, inicio);
        if fin > pos {
            break;
        }
        if fin == inicio {
            break; // proteccion ante una iteracion sin avance.
        }
        fila += 1;
        inicio = fin;
    }
    fila
}

/// Total de filas visuales que ocupa `buf[..len]`. Al menos 1 (linea vacia).
fn filas_totales(buf: &[u8], len: usize) -> usize {
    if len == 0 {
        return 1;
    }
    let mut filas = 0usize;
    let mut inicio = 0usize;
    while inicio < len {
        let fin = fin_linea_visual(buf, len, inicio);
        if fin == inicio {
            break;
        }
        filas += 1;
        inicio = fin;
    }
    // Si el ultimo char es `\n`, hay una linea visual mas (la vacia).
    if len > 0 && buf[len - 1] == b'\n' {
        filas += 1;
    }
    if filas == 0 { 1 } else { filas }
}

// --- Busqueda -----------------------------------------------------------------

fn igual_ci(a: u8, b: u8) -> bool {
    let na = if a.is_ascii_uppercase() { a + 32 } else { a };
    let nb = if b.is_ascii_uppercase() { b + 32 } else { b };
    na == nb
}

/// Recalcula `MATCH_POS` y `MATCH_LEN` con todas las ocurrencias case-insens
/// de `query` en `buf[..len]`. No solapadas. Hasta CAP_MATCHES posiciones.
fn buscar_matches(buf: &[u8], len: usize, query: &[u8]) {
    let q_len = query.len();
    unsafe { MATCH_LEN = 0; }
    if q_len == 0 || q_len > len {
        return;
    }
    let matches = unsafe { &mut *core::ptr::addr_of_mut!(MATCH_POS) };
    let mut n = 0usize;
    let mut i = 0usize;
    while i + q_len <= len && n < CAP_MATCHES {
        let mut ok = true;
        for j in 0..q_len {
            if !igual_ci(buf[i + j], query[j]) {
                ok = false;
                break;
            }
        }
        if ok {
            matches[n] = i as u32;
            n += 1;
            i += q_len;
        } else {
            i += 1;
        }
    }
    unsafe { MATCH_LEN = n; }
}

/// Trae el cursor al inicio del siguiente match, contando desde la posicion
/// actual del cursor. Si no hay matches o el cursor esta despues del ultimo
/// match, vuelve al primer match (wrap-around).
fn ir_a_siguiente_match() {
    // Asegurarse de que MATCH_POS este al dia respecto a la query y nota.
    refrescar_matches();
    let n = unsafe { MATCH_LEN };
    if n == 0 {
        return;
    }
    let matches = unsafe { &*core::ptr::addr_of!(MATCH_POS) };
    con_activa(|nota| {
        let cursor = nota.cursor;
        let mut elegido = matches[0];
        let mut hubo = false;
        for k in 0..n {
            if matches[k] > cursor {
                elegido = matches[k];
                hubo = true;
                break;
            }
        }
        if !hubo {
            elegido = matches[0]; // wrap-around al principio.
        }
        nota.cursor = elegido;
        nota.sel_anchor = SIN_SEL;
    });
    asegurar_cursor_visible();
}

fn ir_a_anterior_match() {
    refrescar_matches();
    let n = unsafe { MATCH_LEN };
    if n == 0 {
        return;
    }
    let matches = unsafe { &*core::ptr::addr_of!(MATCH_POS) };
    con_activa(|nota| {
        let cursor = nota.cursor;
        let mut elegido = matches[n - 1];
        let mut hubo = false;
        let mut k = n;
        while k > 0 {
            k -= 1;
            if matches[k] < cursor {
                elegido = matches[k];
                hubo = true;
                break;
            }
        }
        if !hubo {
            elegido = matches[n - 1];
        }
        nota.cursor = elegido;
        nota.sel_anchor = SIN_SEL;
    });
    asegurar_cursor_visible();
}

fn refrescar_matches() {
    let q_len = unsafe { QUERY_LEN };
    if q_len == 0 {
        unsafe { MATCH_LEN = 0; }
        return;
    }
    let q = unsafe { &*core::ptr::addr_of!(QUERY) };
    let activa = unsafe { ACTIVA }.min(NUM_NOTAS - 1);
    let notas = unsafe { &*core::ptr::addr_of!(NOTAS) };
    let nota = &notas[activa];
    let len = (nota.len as usize).min(CAP_NOTA);
    buscar_matches(&nota.cuerpo[..len], len, &q[..q_len]);
}

// --- Persistencia ------------------------------------------------------------

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

// --- Render -------------------------------------------------------------------

fn pintar() {
    let lienzo = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }
    let modo = unsafe { MODO };
    if modo == MODO_INDICE {
        pintar_indice(lienzo);
    } else {
        pintar_header(lienzo);
        if modo == MODO_BUSCAR {
            refrescar_matches();
        }
        pintar_cuerpo(lienzo);
        if modo == MODO_BUSCAR {
            pintar_barra_busqueda(lienzo);
        }
    }
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

fn pintar_header(lienzo: &mut [u32]) {
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
    let y_linea = Y_LABEL + PASO + 4;
    for x in MARGEN_X..(ANCHO - MARGEN_X) {
        lienzo[y_linea * ANCHO + x] = ETIQUETA;
        lienzo[(y_linea + 1) * ANCHO + x] = ETIQUETA;
    }
}

/// `true` si `i` cae dentro de algun match cacheado en MATCH_POS.
fn dentro_match(i: usize, q_len: usize) -> bool {
    if q_len == 0 {
        return false;
    }
    let n = unsafe { MATCH_LEN };
    let matches = unsafe { &*core::ptr::addr_of!(MATCH_POS) };
    // Busqueda binaria por el primer match cuyo inicio sea <= i.
    // n esta acotado a 256, busqueda lineal hacia adelante es OK; matches estan
    // en orden creciente. Optimizable, pero MVP.
    for k in 0..n {
        let p = matches[k] as usize;
        if i < p { return false; }
        if i < p + q_len { return true; }
    }
    false
}

fn pintar_cuerpo(lienzo: &mut [u32]) {
    let activa = unsafe { ACTIVA }.min(NUM_NOTAS - 1);
    let notas = unsafe { &*core::ptr::addr_of!(NOTAS) };
    let nota = &notas[activa];
    let len = (nota.len as usize).min(CAP_NOTA);
    let cursor = (nota.cursor as usize).min(len);
    let scroll = nota.scroll as usize;
    let sel = rango_sel(nota);
    let buffer = &nota.cuerpo[..len];

    let modo = unsafe { MODO };
    let q_len = if modo == MODO_BUSCAR { unsafe { QUERY_LEN } } else { 0 };

    // Iterar por lineas visuales.
    let mut inicio_visual = 0usize;
    let mut fila_visual_n = 0usize;
    let mut caret_x: Option<usize> = None;
    let mut caret_y: usize = Y_TEXTO;
    let mut ultima_col = 0usize;
    let mut ultima_fila = 0usize;
    let mut ultima_termina_en_nl = false;

    while inicio_visual < len {
        let fin = fin_linea_visual(buffer, len, inicio_visual);
        if fin == inicio_visual {
            break;
        }
        let visible = fila_visual_n >= scroll && (fila_visual_n - scroll) < FILAS;
        let rfila = fila_visual_n.saturating_sub(scroll);
        let mut col = 0usize;

        for i in inicio_visual..fin {
            let c = buffer[i];
            let dentro_sel = sel.map(|(lo, hi)| i >= lo && i < hi).unwrap_or(false);
            let en_match = q_len > 0 && dentro_match(i, q_len);

            // Capturar caret antes de pintar este byte.
            if i == cursor && caret_x.is_none() && visible {
                caret_x = Some(MARGEN_X + col * PASO);
                caret_y = Y_TEXTO + rfila * PASO;
            }

            if c == b'\n' {
                // Newline en la seleccion: rectangulo al final de la linea.
                if dentro_sel && visible {
                    let x = MARGEN_X + col * PASO;
                    let y = Y_TEXTO + rfila * PASO;
                    rellenar(lienzo, x, y, PASO, PASO, SELECCION);
                }
                // No incrementa col, no se pinta como glifo.
            } else {
                if visible {
                    let x = MARGEN_X + col * PASO;
                    let y = Y_TEXTO + rfila * PASO;
                    if dentro_sel {
                        rellenar(lienzo, x, y, PASO, PASO, SELECCION);
                    } else if en_match {
                        rellenar(lienzo, x, y, PASO, PASO, MATCH_BG);
                    }
                    pintar_glifo(lienzo, c, x, y, TINTA);
                }
                col += 1;
            }
        }

        ultima_col = col;
        ultima_fila = fila_visual_n;
        ultima_termina_en_nl = fin > 0 && buffer[fin - 1] == b'\n';

        fila_visual_n += 1;
        inicio_visual = fin;
    }

    // Caret al final del buffer (cursor == len) o sobre buffer vacio.
    if caret_x.is_none() && cursor == len {
        let fila_caret = if len == 0 {
            0
        } else if ultima_termina_en_nl {
            fila_visual_n
        } else {
            ultima_fila
        };
        let col_caret = if len == 0 {
            0
        } else if ultima_termina_en_nl {
            0
        } else {
            ultima_col
        };
        if fila_caret >= scroll && (fila_caret - scroll) < FILAS {
            let rfila = fila_caret - scroll;
            caret_x = Some(MARGEN_X + col_caret * PASO);
            caret_y = Y_TEXTO + rfila * PASO;
        }
    }
    if let Some(cx) = caret_x {
        pintar_caret(lienzo, cx, caret_y);
    }
}

fn pintar_barra_busqueda(lienzo: &mut [u32]) {
    // Fondo de la barra.
    rellenar(lienzo, 0, Y_BARRA, ANCHO, PASO, 0x14_24_44);
    let prefijo = b"[buscar]: ";
    pintar_texto(lienzo, prefijo, MARGEN_X, Y_BARRA, ETIQUETA);
    let q_len = unsafe { QUERY_LEN };
    let q = unsafe { &*core::ptr::addr_of!(QUERY) };
    let mut cx = MARGEN_X + prefijo.len() * PASO;
    for k in 0..q_len {
        if cx + PASO > ANCHO - 80 {
            break;
        }
        pintar_glifo(lienzo, q[k], cx, Y_BARRA, TINTA);
        cx += PASO;
    }
    // Caret blinkless al final del query.
    if cx + 2 <= ANCHO {
        pintar_caret(lienzo, cx, Y_BARRA);
    }
    // Conteo de matches en el extremo derecho.
    let n = unsafe { MATCH_LEN };
    let texto = formatear_conteo(n);
    let largo = texto.iter().take_while(|&&c| c != 0).count();
    let x = ANCHO.saturating_sub(largo * PASO + 4);
    pintar_texto(lienzo, &texto[..largo], x, Y_BARRA, ETIQUETA);
}

/// Formatea el conteo `N matches` en un buffer fijo (sin alloc). Devuelve los
/// bytes utilizables; el resto es `0` (centinela).
fn formatear_conteo(n: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    if n == 0 {
        let s = b"sin matches";
        out[..s.len()].copy_from_slice(s);
        return out;
    }
    let mut digitos = [0u8; 6];
    let mut nd = 0usize;
    let mut x = n;
    while x > 0 && nd < digitos.len() {
        digitos[nd] = b'0' + (x % 10) as u8;
        x /= 10;
        nd += 1;
    }
    let mut pos = 0usize;
    let mut k = nd;
    while k > 0 {
        k -= 1;
        if pos < out.len() {
            out[pos] = digitos[k];
            pos += 1;
        }
    }
    for &b in b" matches".iter() {
        if pos < out.len() {
            out[pos] = b;
            pos += 1;
        }
    }
    out
}

fn pintar_indice(lienzo: &mut [u32]) {
    // Header — corto para caber holgado en 480 px.
    pintar_texto(lienzo, b"bitacora :: indice", MARGEN_X, Y_LABEL, ETIQUETA);
    let y_linea = Y_LABEL + PASO + 4;
    for x in MARGEN_X..(ANCHO - MARGEN_X) {
        lienzo[y_linea * ANCHO + x] = ETIQUETA;
        lienzo[(y_linea + 1) * ANCHO + x] = ETIQUETA;
    }

    let foco = unsafe { INDICE_FOCO }.min(NUM_NOTAS - 1);
    let activa = unsafe { ACTIVA };
    let notas = unsafe { &*core::ptr::addr_of!(NOTAS) };
    let alto_tile = 26usize;
    let y0 = Y_TEXTO;
    for i in 0..NUM_NOTAS {
        let y = y0 + i * alto_tile;
        if i == foco {
            rellenar(lienzo, 4, y - 2, ANCHO - 8, alto_tile, FOCO_BG);
        }
        // Numero.
        let mut etiqueta = [b'F', b'0', b' ', b' '];
        etiqueta[1] = b'1' + i as u8;
        // Marcador de activa: `*` en lugar del segundo espacio.
        if i == activa {
            etiqueta[3] = b'*';
        }
        let color = if i == foco { TINTA } else { ETIQUETA };
        pintar_texto(lienzo, &etiqueta, MARGEN_X, y, color);
        // Titulo (primera linea, hasta `\n` o 28 chars).
        let nota = &notas[i];
        let nlen = (nota.len as usize).min(CAP_NOTA);
        let mut tlen = 0usize;
        while tlen < nlen && tlen < 28 && nota.cuerpo[tlen] != b'\n' {
            tlen += 1;
        }
        let cx = MARGEN_X + etiqueta.len() * PASO + 8;
        if tlen == 0 {
            pintar_texto(lienzo, b"(vacia)", cx, y, INACTIVO);
        } else {
            pintar_texto(lienzo, &nota.cuerpo[..tlen], cx, y, TINTA);
        }
        // Bytes en el extremo derecho.
        let buf = formatear_bytes(nlen);
        let largo = buf.iter().take_while(|&&c| c != 0).count();
        let x_d = ANCHO.saturating_sub(largo * PASO + 8);
        pintar_texto(lienzo, &buf[..largo], x_d, y, INACTIVO);
    }
    // Pie con ayuda corta — cabe en 480 px (29 chars * 16 = 464).
    let y_pie = Y_TEXTO + NUM_NOTAS * 26;
    if y_pie + PASO <= ALTO {
        pintar_texto(lienzo, b"Enter elegir   Esc volver", MARGEN_X, y_pie, INACTIVO);
    }
}

fn formatear_bytes(n: usize) -> [u8; 12] {
    let mut out = [0u8; 12];
    let mut digitos = [0u8; 6];
    let mut nd = 0usize;
    let mut x = n;
    if x == 0 {
        digitos[0] = b'0';
        nd = 1;
    } else {
        while x > 0 && nd < digitos.len() {
            digitos[nd] = b'0' + (x % 10) as u8;
            x /= 10;
            nd += 1;
        }
    }
    let mut pos = 0usize;
    let mut k = nd;
    while k > 0 {
        k -= 1;
        if pos < out.len() {
            out[pos] = digitos[k];
            pos += 1;
        }
    }
    let resto = b" B";
    for &b in resto.iter() {
        if pos < out.len() {
            out[pos] = b;
            pos += 1;
        }
    }
    out
}

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
