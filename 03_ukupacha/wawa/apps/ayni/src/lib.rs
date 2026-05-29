// =============================================================================
//  ayni :: apps/ayni — P6+ :: el chat soberano que habla por akasha
// -----------------------------------------------------------------------------
//  La app P6 dejó de ser un monólogo: ahora TECLEÁS un mensaje y, al pulsar
//  Enter, se firma (Ed25519), se persiste como objeto del grafo de akasha
//  —encadenado al anterior en la espina dorsal que el kernel custodia en disco—
//  y se DIFUNDE por la red del SO: un frame Ethernet de EtherType propio
//  (0x88B7), sin TCP/IP, al puro estilo akasha. Otra instancia de wawa en el
//  mismo segmento absorbe ese frame, verifica e integra el nodo en SU grafo, y
//  lo pinta. Dos wawas convergen su conversación sin servidor — chat
//  persona-a-persona sobre el SO soberano.
//
//  El MISMO `ayni-core` (no_std + alloc) de Linux sostiene el grafo; aquí sólo
//  se añaden las capacidades del kernel: teclado (`sys_get_scancode`), red
//  (`sys_net_*`, PERMISO_RED) y grafo de objetos (`sys_object_*`,
//  PERMISO_GRAFO_ESCRITURA | PERMISO_RAIZ). La firma la pone `ed25519-compact`,
//  el mismo del kernel; `ayni-core` se mantiene cripto-agnóstico.
//
//  Lo que todavía NO hace (honestidad): anti-entropía completa sobre L2 (un peer
//  recién arrancado ve los mensajes NUEVOS en vivo, pero no recibe el historial
//  hasta que alguien reemita) y cifrado de sesión. Ambos son el siguiente paso.
// =============================================================================

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use ayni_core::{AgoraId, Carga, Conversacion, Firma, Hash, MensajeNodo};
use ed25519_compact::{KeyPair, PublicKey, Seed, Signature};
use font8x8::legacy::BASIC_LEGACY;
use linked_list_allocator::LockedHeap;

// --- Heap propio (el grafo de la conversación necesita `alloc`). -------------
#[global_allocator]
static ASIGNADOR: LockedHeap = LockedHeap::empty();
const TAM_ARENA: usize = 512 * 1024;
static mut ARENA: [u8; TAM_ARENA] = [0; TAM_ARENA];

fn fundar_heap() {
    unsafe {
        ASIGNADOR
            .lock()
            .init(core::ptr::addr_of_mut!(ARENA) as *mut u8, TAM_ARENA);
    }
}

// --- Capacidades del kernel. Render + teclado + red + grafo de objetos. ------
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_net_mac(salida: u32) -> i32;
    fn sys_net_enviar(ptr: u32, len: u32) -> i32;
    fn sys_net_recibir(salida: u32, capacidad: u32) -> i32;
    fn sys_object_put(datos: u32, datos_len: u32, hijos: u32, hijos_cnt: u32, salida: u32) -> i32;
    fn sys_object_datos(hash: u32, salida: u32, capacidad: u32) -> i32;
    fn sys_object_hijo(hash: u32, indice: u32, salida: u32) -> i32;
    fn sys_object_raiz(salida: u32) -> i32;
    fn sys_object_fijar_raiz(hash: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometría ---------------------------------------------------------------
const ANCHO: usize = 480;
const ALTO: usize = 400;
const MAX_NODO: usize = 2048;

/// EtherType propio del transporte de Ayni sobre akasha (experimental). Vecino
/// de los de `pregon` (0x88B5) y `asistente` (0x88B6).
const ETHER_TYPE_AYNI: u16 = 0x88B7;
const MAX_FRAME: usize = 1514;
const CAB_ETH: usize = 14; // dst(6) + src(6) + ethertype(2)

/// Semilla de la identidad local (demo: un dueño por wawa; en prod, keystore agora).
const SEMILLA_YO: [u8; 32] = *b"ayni::wawa::identidad-local::p6!";

// --- Paleta ---
const FONDO: u32 = 0x0E_14_22;
const TITULO: u32 = 0xF2_B2_33;
const TEXTO_MIO: u32 = 0x78_DC_AA;
const TEXTO_AJENO: u32 = 0x96_B9_EB;
const TEXTO_ENTRADA: u32 = 0xD7_DD_E8;
const BORDE: u32 = 0x24_2A_37;

// --- Estado vivo (single-thread cooperativo: el kernel nunca reentra). -------
static mut CONV: Option<Conversacion> = None;
static mut KP: Option<KeyPair> = None;
static mut ENTRADA: Option<String> = None;
static mut MAC: [u8; 6] = [0; 6];
/// La cabeza ACTUAL de la espina dorsal de akasha (último objeto-nodo grabado).
static mut CABEZA: [u8; 32] = [0; 32];
static mut HAY_CABEZA: bool = false;
static mut SHIFT: bool = false;
static mut EXT: bool = false;
static mut SUCIO: bool = true;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// Búferes de intercambio con el kernel (siempre dentro de esta memoria lineal).
static mut HASH_RAIZ: [u8; 32] = [0; 32];
static mut HASH_AUX: [u8; 32] = [0; 32];
static mut HASH_NUEVO: [u8; 32] = [0; 32];
static mut BUF_NODO: [u8; MAX_NODO] = [0; MAX_NODO];
static mut BUF_RX: [u8; MAX_FRAME] = [0; MAX_FRAME];
static mut BUF_TX: [u8; MAX_FRAME] = [0; MAX_FRAME];

// --- Helpers de acceso a los estáticos vivos. --------------------------------
fn conv() -> &'static mut Conversacion {
    // SEGURIDAD: inicializado en `init` antes de cualquier `tick`; acceso
    // exclusivo en este hilo cooperativo.
    unsafe { (*core::ptr::addr_of_mut!(CONV)).as_mut().unwrap() }
}
fn entrada() -> &'static mut String {
    unsafe { (*core::ptr::addr_of_mut!(ENTRADA)).as_mut().unwrap() }
}
fn kp() -> &'static KeyPair {
    unsafe { (*core::ptr::addr_of!(KP)).as_ref().unwrap() }
}

/// Preparación: funda el heap, la identidad, el teclado, la red y RECONSTRUYE la
/// conversación recorriendo la espina dorsal de akasha. No autoredacta nada: el
/// humano teclea.
#[no_mangle]
pub extern "C" fn init() {
    fundar_heap();
    unsafe {
        KP = Some(KeyPair::from_seed(Seed::new(SEMILLA_YO)));
        ENTRADA = Some(String::new());
        let _ = sys_net_mac(core::ptr::addr_of_mut!(MAC) as u32);
    }

    let tiene_raiz = unsafe { sys_object_raiz(core::ptr::addr_of_mut!(HASH_RAIZ) as u32) } == 1;
    let conversacion = cargar_conversacion(tiene_raiz);
    unsafe {
        CABEZA = *core::ptr::addr_of!(HASH_RAIZ);
        HAY_CABEZA = tiene_raiz;
        CONV = Some(conversacion);
        SUCIO = true;
    }
    pintar();
}

/// Cada fotograma: drena el teclado (compone/envía), drena la red (absorbe), y
/// repinta si algo cambió. Fiel al ABI cooperativo: trabajo acotado por tick.
#[no_mangle]
pub extern "C" fn tick() {
    drenar_teclado();
    drenar_red();
    if unsafe { SUCIO } {
        pintar();
        unsafe { SUCIO = false };
    }
}

// === Teclado =================================================================
fn drenar_teclado() {
    loop {
        let sc = unsafe { sys_get_scancode() } as u8;
        if sc == 0 {
            break;
        }
        if sc == 0xE0 {
            unsafe { EXT = true };
            continue;
        }
        let ext = unsafe { EXT };
        unsafe { EXT = false };

        match sc {
            0x2A | 0x36 => {
                unsafe { SHIFT = true };
                continue;
            }
            0xAA | 0xB6 => {
                unsafe { SHIFT = false };
                continue;
            }
            _ => {}
        }
        if sc & 0x80 != 0 {
            continue; // key-up
        }
        if ext {
            continue; // teclas extendidas: irrelevantes para componer
        }

        match sc {
            0x1C => enviar_entrada(),    // Enter
            0x0E => {
                entrada().pop();
                unsafe { SUCIO = true };
            } // Backspace
            _ => {
                if let Some(ch) = scancode_a_char(sc, unsafe { SHIFT }) {
                    entrada().push(ch);
                    unsafe { SUCIO = true };
                }
            }
        }
    }
}

/// Toma el contenido del input, redacta un nodo firmado, lo integra, lo
/// persiste en akasha y lo difunde por la red. Limpia el input.
fn enviar_entrada() {
    let texto = entrada().trim();
    if texto.is_empty() {
        return;
    }
    let autor = autor_local();
    let ts = (conv().len() + 1) as u64;
    let nodo = conv().redactar(autor, Carga::Texto(texto.into()), ts, firmar);
    integrar(&nodo, true);
    entrada().clear();
    unsafe { SUCIO = true };
}

// === Red (akasha sobre Ethernet propio) ======================================
fn drenar_red() {
    loop {
        let n = unsafe {
            sys_net_recibir(
                core::ptr::addr_of_mut!(BUF_RX) as u32,
                MAX_FRAME as u32,
            )
        };
        if n <= 0 {
            break;
        }
        procesar_frame(n as usize);
    }
}

fn procesar_frame(len: usize) {
    if len < CAB_ETH {
        return;
    }
    // SEGURIDAD: el kernel escribió `len` (≤ MAX_FRAME) bytes en BUF_RX.
    let frame = unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(BUF_RX) as *const u8, len) };
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    if ethertype != ETHER_TYPE_AYNI {
        return;
    }
    let payload = &frame[CAB_ETH..];
    if let Ok(nodo) = MensajeNodo::deserializar(payload) {
        // La firma se verifica ANTES de aceptar: un nodo del cable que no valide
        // contra su autor declarado se descarta sin tocar el grafo.
        if nodo.verificar(verificar_firma) && !conv().contiene(&nodo.id()) {
            integrar(&nodo, false);
            unsafe { SUCIO = true };
        }
    }
}

/// Difunde el `postcard` de un nodo en un frame Ethernet broadcast de EtherType
/// propio. Los nodos que no caben (adjuntos/cifrados grandes) no se emiten (MVP).
fn difundir(bytes: &[u8]) {
    if bytes.len() > MAX_FRAME - CAB_ETH {
        return;
    }
    let total = CAB_ETH + bytes.len();
    // SEGURIDAD: escritura acotada a BUF_TX (≤ MAX_FRAME).
    let tx = unsafe { &mut *core::ptr::addr_of_mut!(BUF_TX) };
    tx[0..6].copy_from_slice(&[0xff; 6]); // destino: broadcast
    tx[6..12].copy_from_slice(unsafe { &*core::ptr::addr_of!(MAC) });
    tx[12..14].copy_from_slice(&ETHER_TYPE_AYNI.to_be_bytes());
    tx[CAB_ETH..total].copy_from_slice(bytes);
    unsafe {
        let _ = sys_net_enviar(tx.as_ptr() as u32, total as u32);
    }
}

/// Integra un nodo en el grafo, lo persiste en la espina de akasha y —si es
/// propio o recién llegado y debe propagarse— lo difunde.
fn integrar(nodo: &MensajeNodo, difundir_lo: bool) {
    if conv().agregar(nodo.clone()).is_err() {
        return;
    }
    let bytes = nodo.serializar();
    persistir_spine(&bytes);
    if difundir_lo {
        difundir(&bytes);
    }
}

// === Persistencia en el grafo de objetos (espina dorsal) =====================
/// Reconstruye la conversación recorriendo la espina desde la raíz (hijo 0 = nodo
/// anterior), igual que P6.
fn cargar_conversacion(tiene_raiz: bool) -> Conversacion {
    let mut nodos: Vec<MensajeNodo> = Vec::new();
    if tiene_raiz {
        unsafe {
            *core::ptr::addr_of_mut!(HASH_AUX) = *core::ptr::addr_of!(HASH_RAIZ);
        }
        let mut prof = 0usize;
        loop {
            if let Some(bytes) = leer_datos_aux() {
                if let Ok(nodo) = MensajeNodo::deserializar(&bytes) {
                    nodos.push(nodo);
                }
            }
            let hijos = unsafe {
                sys_object_hijo(
                    core::ptr::addr_of!(HASH_AUX) as u32,
                    0,
                    core::ptr::addr_of_mut!(HASH_AUX) as u32,
                )
            };
            prof += 1;
            if hijos <= 0 || prof >= 4096 {
                break;
            }
        }
    }
    Conversacion::desde_nodos(nodos)
}

fn leer_datos_aux() -> Option<Vec<u8>> {
    let n = unsafe {
        sys_object_datos(
            core::ptr::addr_of!(HASH_AUX) as u32,
            core::ptr::addr_of_mut!(BUF_NODO) as u32,
            MAX_NODO as u32,
        )
    };
    if n <= 0 {
        return None;
    }
    let ptr = core::ptr::addr_of!(BUF_NODO) as *const u8;
    let slice = unsafe { core::slice::from_raw_parts(ptr, n as usize) };
    Some(slice.to_vec())
}

/// Graba los `bytes` del nodo como objeto de akasha encadenado a la cabeza
/// ACTUAL de la espina, y avanza la cabeza. Así varios mensajes de una misma
/// sesión se encadenan correctamente (no todos contra la raíz de arranque).
fn persistir_spine(bytes: &[u8]) {
    if bytes.is_empty() || bytes.len() > MAX_NODO {
        return;
    }
    let (hijos_ptr, hijos_cnt) = if unsafe { HAY_CABEZA } {
        (core::ptr::addr_of!(CABEZA) as u32, 1u32)
    } else {
        (0u32, 0u32)
    };
    let grabado = unsafe {
        sys_object_put(
            bytes.as_ptr() as u32,
            bytes.len() as u32,
            hijos_ptr,
            hijos_cnt,
            core::ptr::addr_of_mut!(HASH_NUEVO) as u32,
        )
    };
    if grabado == 0 {
        unsafe {
            let _ = sys_object_fijar_raiz(core::ptr::addr_of!(HASH_NUEVO) as u32);
            CABEZA = *core::ptr::addr_of!(HASH_NUEVO);
            HAY_CABEZA = true;
        }
    }
}

// === Cripto (closures que `ayni-core` pide) ==================================
fn autor_local() -> AgoraId {
    let mut a: AgoraId = [0u8; 32];
    a.copy_from_slice(&*kp().pk);
    a
}

fn firmar(id: &Hash) -> Firma {
    let sig = kp().sk.sign(id, None);
    let mut f: Firma = [0u8; 64];
    f.copy_from_slice(&*sig);
    f
}

fn verificar_firma(autor: &AgoraId, id: &Hash, firma: &Firma) -> bool {
    let Ok(pk) = PublicKey::from_slice(autor) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(firma) else {
        return false;
    };
    pk.verify(id, &sig).is_ok()
}

// === Render ==================================================================
fn pintar() {
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for p in lienzo.iter_mut() {
        *p = FONDO;
    }

    pintar_texto(lienzo, b"AYNI :: chat soberano por akasha", 6, 6, TITULO);

    // El hilo, en orden topológico; los últimos que quepan.
    let c = conv();
    let orden = c
        .orden_topologico()
        .unwrap_or_else(|| c.nodos().map(|(id, _)| *id).collect());
    let alto_linea = 10usize;
    let inicio_y = 22usize;
    let fin_y = ALTO - 28; // deja sitio para la línea de entrada
    let max_lineas = (fin_y - inicio_y) / alto_linea;
    let desde = orden.len().saturating_sub(max_lineas);

    let yo = autor_local();
    let mut y = inicio_y;
    for id in orden.iter().skip(desde) {
        if let Some(nodo) = c.obtener(id) {
            let propio = *nodo.autor() == yo;
            let color = if propio { TEXTO_MIO } else { TEXTO_AJENO };
            let mut linea: Vec<u8> = Vec::new();
            linea.extend_from_slice(&hex4(nodo.autor()));
            linea.extend_from_slice(b": ");
            match nodo.contenido.carga.texto() {
                Some(t) => linea.extend_from_slice(t.as_bytes()),
                None => linea.extend_from_slice(b"<no-texto>"),
            }
            pintar_texto(lienzo, &linea, 6, y, color);
        }
        y += alto_linea;
    }

    // La línea de entrada, separada por un borde.
    let by = ALTO - 22;
    rellenar(lienzo, 0, by - 4, ANCHO, 1, BORDE);
    let mut prompt: Vec<u8> = Vec::new();
    prompt.extend_from_slice(b"> ");
    prompt.extend_from_slice(entrada().as_bytes());
    prompt.push(b'_');
    pintar_texto(lienzo, &prompt, 6, by, TEXTO_ENTRADA);

    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

fn hex4(autor: &AgoraId) -> [u8; 4] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [
        HEX[(autor[0] >> 4) as usize],
        HEX[(autor[0] & 0xf) as usize],
        HEX[(autor[1] >> 4) as usize],
        HEX[(autor[1] & 0xf) as usize],
    ]
}

fn pintar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, color: u32) {
    let mut cx = x;
    for &ch in texto {
        if cx + 8 > ANCHO {
            break;
        }
        pintar_glifo(lienzo, ch, cx, y, color);
        cx += 8;
    }
}

fn pintar_glifo(lienzo: &mut [u32], ch: u8, x: usize, y: usize, color: u32) {
    let glifo = if (ch as usize) < 128 {
        BASIC_LEGACY[ch as usize]
    } else {
        BASIC_LEGACY[b'?' as usize]
    };
    for row in 0..8 {
        for col in 0..8 {
            if glifo[row] & (1 << col) != 0 {
                let px = x + col;
                let py = y + row;
                if px < ANCHO && py < ALTO {
                    lienzo[py * ANCHO + px] = color;
                }
            }
        }
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

// === Teclado: scancode → carácter (US, subconjunto para componer) ============
fn scancode_a_char(sc: u8, shift: bool) -> Option<char> {
    let c = if shift {
        match sc {
            0x02 => b'!', 0x03 => b'@', 0x04 => b'#', 0x05 => b'$', 0x06 => b'%',
            0x07 => b'^', 0x08 => b'&', 0x09 => b'*', 0x0A => b'(', 0x0B => b')',
            0x10 => b'Q', 0x11 => b'W', 0x12 => b'E', 0x13 => b'R', 0x14 => b'T',
            0x15 => b'Y', 0x16 => b'U', 0x17 => b'I', 0x18 => b'O', 0x19 => b'P',
            0x1E => b'A', 0x1F => b'S', 0x20 => b'D', 0x21 => b'F', 0x22 => b'G',
            0x23 => b'H', 0x24 => b'J', 0x25 => b'K', 0x26 => b'L',
            0x2C => b'Z', 0x2D => b'X', 0x2E => b'C', 0x2F => b'V', 0x30 => b'B',
            0x31 => b'N', 0x32 => b'M',
            0x0C => b'_', 0x0D => b'+', 0x27 => b':', 0x28 => b'"',
            0x33 => b'<', 0x34 => b'>', 0x35 => b'?', 0x39 => b' ',
            _ => return None,
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
            0x0C => b'-', 0x0D => b'=', 0x27 => b';', 0x28 => b'\'',
            0x33 => b',', 0x34 => b'.', 0x35 => b'/', 0x39 => b' ',
            _ => return None,
        }
    };
    Some(c as char)
}
