// =============================================================================
//  renaser :: apps/mudanza — Fase 25 :: el centro soberano de reancla
// -----------------------------------------------------------------------------
//  Mudanza es la unica app del genesis con PERMISO_RAIZ que invoca
//  `sys_manifiesto_proponer`. La syscall toma un sobre `ManifiestoFirmado`
//  (32 B hash + 32 B autor Ed25519 + 64 B firma) en formato postcard y, si
//  la firma valida contra la clave publica que el kernel lleva grabada,
//  reanca el manifiesto. Aqui el operador local autoriza esa decision con
//  Alt+Enter, despues de leer la propuesta en pantalla.
//
//  Esta version MVP no orquesta el flujo entero AnunciarCanal aun —espera
//  a que wawactl exporte el comando de firma para que el operador genere
//  propuestas reales—. Mientras tanto, sirve para PROBAR end-to-end el
//  guardarrail criptografico: pulsando SPACE construye un sobre de prueba
//  con autor `zero-key` (rechazado con `CapacidadInsuficiente`) y muestra
//  el codigo que el kernel devolvio. El cero no equivale a la clave local,
//  asi el rechazo es la respuesta correcta — la prueba consiste en que el
//  kernel rechace, no en que acepte.
// =============================================================================

#![no_std]

#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_manifiesto_proponer(mf_ptr: u32, mf_len: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria. DEBE encajar con la region que `boot` asigna a la app. ---
const ANCHO: usize = 480;
const ALTO: usize = 240;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];
static mut PALETA: [u8; 20] = [0; 20];
static mut IDIOMA: u16 = 0;

/// Anti-rebote del SPACE: solo el flanco de subida vale como pulsacion.
static mut SPACE_PREV: bool = false;

/// Ultimo codigo recibido del kernel al probar la reancla. `i32::MIN`
/// significa "aun no probado"; cualquier otro es el valor literal devuelto
/// por `sys_manifiesto_proponer` —que la UI rotula sobre la paleta activa—.
static mut ULTIMO_CODIGO: i32 = i32::MIN;

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
    let space_ahora = scancode == 0x39;
    if space_ahora && !unsafe { SPACE_PREV } {
        probar_reancla();
    }
    unsafe { SPACE_PREV = space_ahora };

    pintar();
    volcar();
}

fn refrescar_contexto() {
    let idioma = unsafe { sys_config_idioma() } as u16;
    unsafe { IDIOMA = idioma };
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

/// Construye un `ManifiestoFirmado` de prueba —autor TODO ceros, firma TODO
/// ceros, hash TODO ceros— y lo entrega al kernel. La estructura postcard
/// para una tupla de arrays fijos `[[u8;32],[u8;32],[u8;64]]` es el
/// CONCATENADO crudo de los bytes: 128 B en total, sin preludios de longitud.
/// Confiamos en este detalle del format porque la crate `format` ya lo
/// fija con su `Serialize`/`Deserialize` derivado — el contrato lo mantiene
/// el script guardian de simetria no_std.
///
/// Resultado esperado: `CodigoError::CapacidadInsuficiente` (-2). El kernel
/// rechaza autores ajenos ANTES de tocar criptografia; un autor "ceros"
/// jamas igualara `claves::AGORA_PUBLIC_KEY_LOCAL`.
fn probar_reancla() {
    let mf = [0u8; 128]; // 32 hash + 32 autor + 64 firma; todo ceros.
    let codigo = unsafe { sys_manifiesto_proponer(mf.as_ptr() as u32, mf.len() as u32) };
    unsafe { ULTIMO_CODIGO = codigo };
}

// =============================================================================
//  Pintado del fotograma
// =============================================================================

fn pintar() {
    let paleta = unsafe { PALETA };
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    let fondo = color_u32(paleta, 2);
    let tinta = color_u32(paleta, 3);
    let acento = color_u32(paleta, 4);
    let secundario = color_u32(paleta, 1);

    // Fondo + cabecera con titulo en acento.
    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, fondo);
    rellenar_rect(lienzo, 0, 0, ANCHO, 32, secundario);
    dibujar_texto(lienzo, b"MUDANZA", 16, 8, 2, acento);
    rellenar_rect(lienzo, 0, 32, ANCHO, 2, acento);

    // Cuerpo: explicacion + estado.
    let mut y = 48;
    dibujar_texto(lienzo, b"SOBRE CRYPTO ED25519", 16, y, 1, tinta);
    y += 12;
    dibujar_texto(lienzo, b"KERNEL VERIFICA FIRMA LOCAL", 16, y, 1, tinta);
    y += 18;
    dibujar_texto(lienzo, b"SPACE PARA PROBAR REANCLA", 16, y, 1, acento);
    y += 22;

    // Resultado del ultimo intento.
    let ult = unsafe { ULTIMO_CODIGO };
    if ult == i32::MIN {
        dibujar_texto(lienzo, b"ESTADO: SIN PROBAR", 16, y, 1, tinta);
    } else {
        // Imprimir "ESTADO: <num>" en ASCII. Acepta valores entre -9 y 99
        // para no llevar parser de int en el WASM.
        let mut buf = [b' '; 24];
        buf[..8].copy_from_slice(b"ESTADO: ");
        let dec = formatear_i32(ult);
        let mut n = 8;
        for &c in &dec.0[..dec.1] {
            buf[n] = c;
            n += 1;
        }
        dibujar_texto(lienzo, &buf[..n], 16, y, 1, tinta);
        y += 12;
        // Explicacion humana del codigo.
        let explica: &[u8] = match ult {
            0 => b"OK :: REANCLADO",
            -1 => b"AUSENTE :: SOBRE NO DECODIFICA",
            -2 => b"AUTOR AJENO :: RECHAZADO",
            -3 => b"FIRMA INVALIDA :: RECHAZADO",
            _ => b"CODIGO DESCONOCIDO",
        };
        dibujar_texto(lienzo, explica, 16, y, 1, secundario);
    }

    // Pie con idioma.
    let pie_y = ALTO - 20;
    rellenar_rect(lienzo, 0, pie_y - 2, ANCHO, 2, acento);
    let idioma = unsafe { IDIOMA };
    let mut pie = [b' '; 16];
    pie[0] = ((idioma & 0xFF) as u8).to_ascii_uppercase();
    pie[1] = (((idioma >> 8) & 0xFF) as u8).to_ascii_uppercase();
    let cola = b"   FASE 25";
    pie[2..2 + cola.len()].copy_from_slice(cola);
    dibujar_texto(lienzo, &pie[..2 + cola.len()], 16, pie_y + 2, 1, tinta);
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

fn color_u32(paleta: [u8; 20], n: usize) -> u32 {
    let base = n * 4;
    let r = paleta[base] as u32;
    let g = paleta[base + 1] as u32;
    let b = paleta[base + 2] as u32;
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

/// Formatea un i32 corto (entre -9 y 99) en ASCII decimal sin asignacion.
/// Devuelve (buffer, longitud). Conservador: para valores fuera de rango,
/// devuelve "?".
fn formatear_i32(n: i32) -> ([u8; 4], usize) {
    let mut buf = [0u8; 4];
    if n == 0 {
        buf[0] = b'0';
        return (buf, 1);
    }
    if (-9..=99).contains(&n) {
        let mut len = 0;
        if n < 0 {
            buf[0] = b'-';
            len = 1;
            let abs = (-n) as u8;
            buf[len] = b'0' + abs;
            len += 1;
            return (buf, len);
        }
        let n = n as u8;
        if n >= 10 {
            buf[len] = b'0' + n / 10;
            len += 1;
        }
        buf[len] = b'0' + n % 10;
        len += 1;
        (buf, len)
    } else {
        buf[0] = b'?';
        (buf, 1)
    }
}

// =============================================================================
//  Mini-tipografia 5x7 — solo los caracteres que esta app usa
// =============================================================================

const FA: usize = 5; // ancho
const FH: usize = 7; // alto
const FAV: usize = 6; // avance horizontal

fn glifo(c: u8) -> [u8; FH] {
    match c {
        b' ' => [0; 7],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b'?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
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
