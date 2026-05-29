// =============================================================================
//  ayni :: apps/ayni — P6 :: el chat soberano, ahora dentro de wawa
// -----------------------------------------------------------------------------
//  Esta app es la PRUEBA de la promesa que `ayni-core` hizo en su día cero: ser
//  `#![no_std] + alloc` para que el MISMO modelo de la conversación —el DAG de
//  mensajes firmados, direccionado por contenido— corra sin reescribirse dentro
//  del SO bare-metal wawa, como un módulo WASM aislado por wasmi.
//
//  Y hace algo más bonito que reusar código: ata DOS grafos direccionados por
//  contenido que comparten la MISMA función hash (`format::hash`, BLAKE3). Cada
//  nodo de la conversación se persiste como un OBJETO del grafo de akasha
//  (`sys_object_put`), encadenado al anterior — una espina dorsal que el kernel
//  custodia en disco. Al arrancar, la app recorre esa espina, reconstruye la
//  `Conversacion` con `ayni-core::desde_nodos`, añade un mensaje firmado de este
//  arranque, lo graba y corona la nueva cabeza como raíz. La conversación
//  sobrevive a los reinicios porque vive en el disco de objetos, no en la RAM:
//  local-first de verdad, sobre el SO soberano.
//
//  Sus únicas vías hacia el mundo son las capacidades `sys_*` que el kernel
//  inyecta —render y grafo de objetos—. No conoce el disco, ni el bus, ni los
//  sectores: sólo objetos, hashes y aristas. La firma Ed25519 la pone el mismo
//  `ed25519-compact` que el kernel; `ayni-core` se mantiene cripto-agnóstico.
// =============================================================================

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use ayni_core::{AgoraId, Carga, Conversacion, Firma, Hash, MensajeNodo};
use ed25519_compact::{KeyPair, PublicKey, Seed, Signature};
use font8x8::legacy::BASIC_LEGACY;
use linked_list_allocator::LockedHeap;

// --- El heap de la app. A diferencia de la mayoría de apps de wawa (no_std
//     puro), ésta funda su propio asignador: `ayni-core` necesita `alloc`
//     (Vec/BTreeMap/String/postcard para el grafo). Es el MISMO allocator que
//     el kernel, sobre una arena estática de ESTA memoria lineal. ---
#[global_allocator]
static ASIGNADOR: LockedHeap = LockedHeap::empty();

/// 512 KiB de arena para el grafo de la conversación y los búferes de postcard.
/// Cabe de sobra bajo el techo de 4 MiB de una app de genesis; un chat de
/// cientos de mensajes breves no se acerca.
const TAM_ARENA: usize = 512 * 1024;
static mut ARENA: [u8; TAM_ARENA] = [0; TAM_ARENA];

/// Funda el heap. Debe correr ANTES del primer uso de cualquier `alloc::*`.
fn fundar_heap() {
    // SEGURIDAD: la arena es estática, de uso exclusivo del asignador, vive
    // tanto como el módulo, y `fundar_heap` se invoca una sola vez en `init`.
    unsafe {
        ASIGNADOR
            .lock()
            .init(core::ptr::addr_of_mut!(ARENA) as *mut u8, TAM_ARENA);
    }
}

// --- Las capacidades que el kernel inyecta. Las mismas `sys_object_*` que usa
//     `cronista` para encadenar su crónica, más `sys_render_frame`. Esta app
//     declara en el manifiesto PERMISO_GRAFO_ESCRITURA | PERMISO_RAIZ: sin esos
//     bits, estas funciones NO se registran en el Linker y no existen. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_object_put(datos: u32, datos_len: u32, hijos: u32, hijos_cnt: u32, salida: u32) -> i32;
    fn sys_object_datos(hash: u32, salida: u32, capacidad: u32) -> i32;
    fn sys_object_hijo(hash: u32, indice: u32, salida: u32) -> i32;
    fn sys_object_raiz(salida: u32) -> i32;
    fn sys_object_fijar_raiz(hash: u32) -> i32;
}

/// Sin sistema operativo bajo nosotros, un pánico sólo puede detenerse en seco.
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometría de la escena. Debe coincidir con la región del manifiesto. ---
const ANCHO: usize = 480;
const ALTO: usize = 400;

/// Tope de bytes de un nodo serializado que leemos del grafo. Un mensaje de
/// texto cabe holgado en 2 KiB; nodos más grandes (adjuntos, cifrados) se
/// truncarían — el MVP de P6 escribe sólo texto.
const MAX_NODO: usize = 2048;

/// La semilla de la identidad local — la persona "yo" de este wawa. En el host
/// vendría del keystore agora; aquí es fija (es una demo de un solo dueño, sin
/// red todavía). Lo importante: los nodos quedan firmados Ed25519 de verdad y
/// `verificar_firmas` los valida.
const SEMILLA_YO: [u8; 32] = *b"ayni::wawa::identidad-local::p6!";

// --- Paleta ---
const FONDO: u32 = 0x0E_14_22; // indigo casi negro
const TITULO: u32 = 0xF2_B2_33; // ámbar
const TEXTO: u32 = 0xD7_DD_E8; // gris claro
const FIRMA_OK: u32 = 0x35_C4_6A; // verde: todas las firmas validan
const FIRMA_MAL: u32 = 0xD4_1E_2C; // rojo: una firma no valida

/// El lienzo, en la memoria lineal de la app.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- Búferes de intercambio con `sys_object_*`. El kernel lee/escribe hashes
//     aquí, siempre dentro de esta memoria lineal. ---
static mut HASH_RAIZ: [u8; 32] = [0; 32];
static mut HASH_AUX: [u8; 32] = [0; 32];
static mut HASH_NUEVO: [u8; 32] = [0; 32];
static mut BUF_NODO: [u8; MAX_NODO] = [0; MAX_NODO];

/// Preparación: el kernel la invoca UNA vez. Aquí ocurre toda la ronda — fundar
/// el heap, reconstruir la conversación del grafo, añadir el mensaje firmado de
/// este arranque, persistirlo y pintar el hilo.
#[no_mangle]
pub extern "C" fn init() {
    fundar_heap();

    // 1. La identidad local y su firmante/verificador Ed25519.
    let kp = KeyPair::from_seed(Seed::new(SEMILLA_YO));
    let mut autor: AgoraId = [0u8; 32];
    autor.copy_from_slice(&*kp.pk);

    // 2. Reconstruir la conversación recorriendo la espina dorsal de akasha
    //    desde la raíz, descendiendo por el hijo 0 (el nodo anterior).
    let tiene_raiz = unsafe { sys_object_raiz(core::ptr::addr_of_mut!(HASH_RAIZ) as u32) } == 1;
    let mut conv = cargar_conversacion(tiene_raiz);

    // 3. Redactar el mensaje de ESTE arranque: toma las cabezas actuales como
    //    padres (cose el hilo), firmado con la clave local. El `ts` declarado es
    //    el ordinal del mensaje — este núcleo no_std no lee ningún reloj.
    let ordinal = (conv.len() + 1) as u64;
    let texto = texto_arranque(ordinal);
    let nodo = conv.redactar(autor, Carga::Texto(texto), ordinal, firmar_con(&kp.sk));
    // Insertar en el grafo local antes de persistir y pintar.
    let _ = conv.agregar(nodo.clone());

    // 4. Persistir el nodo nuevo como objeto de akasha, encadenado a la raíz
    //    anterior, y coronarlo como nueva raíz: la espina crece un eslabón.
    persistir_nodo(&nodo, tiene_raiz);

    // 5. ¿Validan TODAS las firmas del hilo reconstruido + el nuevo? Es el
    //    testigo de integridad: que el grafo viajó del disco intacto y firmado.
    let integro = conv.verificar_firmas(verificar_firma).is_ok();

    // 6. Pintar el hilo.
    pintar(&conv, integro);
}

/// El ABI cooperativo: el hilo que `init` pintó persiste en el lienzo del
/// kernel; no toda app necesita redibujar cada fotograma.
#[no_mangle]
pub extern "C" fn tick() {}

/// Recorre la espina dorsal del grafo de akasha desde la raíz, recogiendo el
/// `postcard` de cada nodo, y reconstruye la `Conversacion`. `desde_nodos`
/// tolera cualquier orden y reconstruye el DAG por punto fijo.
fn cargar_conversacion(tiene_raiz: bool) -> Conversacion {
    let mut nodos: Vec<MensajeNodo> = Vec::new();
    if tiene_raiz {
        // SEGURIDAD: copia entre estáticos propios — empezar por la raíz.
        unsafe {
            *core::ptr::addr_of_mut!(HASH_AUX) = *core::ptr::addr_of!(HASH_RAIZ);
        }
        let mut profundidad = 0usize;
        loop {
            if let Some(bytes) = leer_datos_aux() {
                if let Ok(nodo) = MensajeNodo::deserializar(&bytes) {
                    nodos.push(nodo);
                }
            }
            // Descender al nodo anterior (hijo 0). El kernel reescribe HASH_AUX.
            let hijos = unsafe {
                sys_object_hijo(
                    core::ptr::addr_of!(HASH_AUX) as u32,
                    0,
                    core::ptr::addr_of_mut!(HASH_AUX) as u32,
                )
            };
            profundidad += 1;
            if hijos <= 0 || profundidad >= 4096 {
                break;
            }
        }
    }
    Conversacion::desde_nodos(nodos)
}

/// Lee la carga útil del objeto cuyo hash está en `HASH_AUX` a un `Vec` del heap.
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
    // SEGURIDAD: el kernel acaba de escribir `n` (≤ MAX_NODO) bytes en BUF_NODO,
    // un estático propio; construimos un slice de esa extensión y lo copiamos.
    let ptr = core::ptr::addr_of!(BUF_NODO) as *const u8;
    let slice = unsafe { core::slice::from_raw_parts(ptr, n as usize) };
    Some(slice.to_vec())
}

/// Graba el nodo como objeto de akasha (datos = su `postcard`; su único hijo, la
/// raíz anterior — el eslabón nuevo de la espina) y lo corona como raíz.
fn persistir_nodo(nodo: &MensajeNodo, tiene_raiz: bool) {
    let bytes = nodo.serializar();
    if bytes.is_empty() || bytes.len() > MAX_NODO {
        return; // un nodo que no cabe en el búfer de lectura no se persiste
    }
    let (hijos_ptr, hijos_cnt) = if tiene_raiz {
        (core::ptr::addr_of!(HASH_RAIZ) as u32, 1u32)
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
        }
    }
}

/// Firmante: cierra sobre la clave secreta y firma los 32 bytes del id. Es el
/// closure que `ayni-core` pide — el núcleo nunca enlaza la primitiva.
fn firmar_con(sk: &ed25519_compact::SecretKey) -> impl FnOnce(&Hash) -> Firma + '_ {
    move |id: &Hash| {
        let sig = sk.sign(id, None);
        let mut firma: Firma = [0u8; 64];
        firma.copy_from_slice(&*sig);
        firma
    }
}

/// Verificador Ed25519 genérico — el closure `(autor, id, firma) -> bool`.
fn verificar_firma(autor: &AgoraId, id: &Hash, firma: &Firma) -> bool {
    let Ok(pk) = PublicKey::from_slice(autor) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(firma) else {
        return false;
    };
    pk.verify(id, &sig).is_ok()
}

/// El texto del mensaje de un arranque. Sin `format!` (arrastra maquinaria de
/// formateo) — componemos a mano sobre un `Vec<u8>` del heap.
fn texto_arranque(ordinal: u64) -> alloc::string::String {
    let mut s = alloc::string::String::from("ayni en wawa - arranque #");
    empujar_u64(&mut s, ordinal);
    s
}

/// Anexa la representación decimal de `n` a la cadena, sin asignar de más.
fn empujar_u64(s: &mut alloc::string::String, n: u64) {
    if n >= 10 {
        empujar_u64(s, n / 10);
    }
    s.push((b'0' + (n % 10) as u8) as char);
}

/// Pinta el hilo: el título, una línea por mensaje en orden topológico, y en la
/// esquina el testigo de integridad de las firmas.
fn pintar(conv: &Conversacion, integro: bool) {
    // SEGURIDAD: durante `init` ésta es la única vía de acceso a LIENZO, y el
    // kernel jamás reentra el módulo mientras `init` corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    // Título.
    pintar_texto(lienzo, b"AYNI :: chat soberano en wawa", 6, 6, TITULO);

    // Las líneas del hilo, en orden topológico (padres antes que hijos). Si hay
    // más mensajes que líneas en pantalla, se muestran los ÚLTIMOS.
    let orden = conv
        .orden_topologico()
        .unwrap_or_else(|| conv.nodos().map(|(id, _)| *id).collect());
    let alto_linea = 10usize;
    let inicio_y = 22usize;
    let max_lineas = (ALTO - inicio_y - 4) / alto_linea;
    let total = orden.len();
    let desde = total.saturating_sub(max_lineas);

    let mut y = inicio_y;
    for id in orden.iter().skip(desde) {
        if let Some(nodo) = conv.obtener(id) {
            let mut linea: Vec<u8> = Vec::new();
            // Prefijo: 4 hex del autor — un asomo de "quién habla".
            linea.extend_from_slice(&hex4(nodo.autor()));
            linea.extend_from_slice(b": ");
            match nodo.contenido.carga.texto() {
                Some(t) => linea.extend_from_slice(t.as_bytes()),
                None => linea.extend_from_slice(b"<adjunto/cifrado>"),
            }
            pintar_texto(lienzo, &linea, 6, y, TEXTO);
        }
        y += alto_linea;
    }

    // El testigo de integridad de las firmas, esquina superior derecha.
    let testigo = if integro { FIRMA_OK } else { FIRMA_MAL };
    rellenar(lienzo, ANCHO - 14, 6, 10, 10, testigo);

    // SEGURIDAD: `sys_render_frame` recibe (ptr, len) de NUESTRA memoria lineal;
    // el host lo verifica sin piedad.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

/// Cuatro dígitos hex de los dos primeros bytes de un AgoraId — etiqueta corta.
fn hex4(autor: &AgoraId) -> [u8; 4] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [
        HEX[(autor[0] >> 4) as usize],
        HEX[(autor[0] & 0xf) as usize],
        HEX[(autor[1] >> 4) as usize],
        HEX[(autor[1] & 0xf) as usize],
    ]
}

/// Pinta una cadena de bytes con la tipografía 8x8, sin escalar (8 px/celda).
fn pintar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, color: u32) {
    let mut cx = x;
    for &c in texto {
        if cx + 8 > ANCHO {
            break;
        }
        pintar_glifo(lienzo, c, cx, y, color);
        cx += 8;
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
                let px = x + col;
                let py = y + row;
                if px < ANCHO && py < ALTO {
                    lienzo[py * ANCHO + px] = color;
                }
            }
        }
    }
}

/// Rellena un rectángulo, recortado a los límites del lienzo.
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
