// =============================================================================
//  renaser :: apps/mudanza — Fase 25/48 :: el centro soberano de reancla
// -----------------------------------------------------------------------------
//  Mudanza es la unica app del genesis con PERMISO_RAIZ que invoca
//  `sys_manifiesto_proponer`. La syscall toma un sobre `ManifiestoFirmado`
//  (32 B hash + 32 B autor Ed25519 + 64 B firma) en formato postcard y, si
//  la firma valida contra el `AGORA_AUTH_RING` del binario y la criptografia
//  cierra, reanca el manifiesto.
//
//  Verificacion en DOS niveles:
//
//    1. USERSPACE (esta app, ed25519-compact). Antes de gastar un syscall
//       deserializamos el sobre y verificamos `pk.verify(hash, sig)`. Una
//       firma rota o un sobre corrupto se rechazan ANTES de Ring 0 — la
//       UI muestra "FIRMA LOCAL INVALIDA" sin molestar al kernel.
//
//    2. KERNEL (claves.rs::verificar_manifiesto_firmado). Si la app acepta,
//       el kernel re-verifica + checa que el autor habite el
//       `AGORA_AUTH_RING`. Una pubkey ajena cae con `CapacidadInsuficiente`
//       sin tocar criptografia.
//
//  El sobre `propuesta_demo.bin` viene firmado por una seed DEMO (`[42u8;32]`)
//  generada por el example `agora-channel::forjar_propuesta_mudanza_demo`. La
//  firma valida bajo SU PROPIA pubkey (lo que la app verifica OK), pero esa
//  pubkey no esta en el anillo soberano del kernel — asi el demo termina
//  con `CapacidadInsuficiente` en pantalla. Reemplazando el sobre por uno
//  firmado por una de las claves del anillo, mudanza reanca de verdad.
// =============================================================================

#![no_std]

#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_manifiesto_proponer(mf_ptr: u32, mf_len: u32) -> i32;
    /// Fase 64 :: vuelca el ultimo `AnunciarCanal` recibido (168 B fijos:
    /// canal|raiz|autor|timestamp_le|firma) o 0 si no hay. Lectura pasiva.
    fn sys_canal_anuncio(salida: u32, capacidad: u32) -> i32;
    /// Fase 64 :: acepta el anuncio retenido cuya raiz casa con `raiz_ptr` y
    /// reancla el manifiesto. El kernel re-verifica anillo + firma canonica.
    fn sys_canal_aceptar(raiz_ptr: u32) -> i32;
}

/// Sobre demo embebido en el binario. Forjado por
/// `agora-channel::forjar_propuesta_mudanza_demo`. Exactamente 128 B
/// (postcard de `ManifiestoFirmado`).
const PROPUESTA_DEMO: &[u8; 128] = include_bytes!("propuesta_demo.bin");

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

/// Resultado de la ultima propuesta intentada. Tres estados:
/// - `i32::MIN` :: aun no probado.
/// - `-100`    :: la verificacion en USERSPACE fallo (firma rota o sobre
///               corrupto). El syscall NO se llamo.
/// - cualquier otro :: el valor literal devuelto por `sys_manifiesto_proponer`.
static mut ULTIMO_CODIGO: i32 = i32::MIN;

/// Sentinel que rotula "el sobre fallo localmente, no llame al kernel".
const VERIFICACION_LOCAL_FALLO: i32 = -100;

/// Layout fijo del anuncio que `sys_canal_anuncio` vuelca: 168 bytes.
const LARGO_ANUNCIO: usize = 168;

/// Buzon del ultimo anuncio de canal recibido por red. `HAY_ANUNCIO` dice si
/// `ANUNCIO` trae datos vigentes este fotograma; `ANUNCIO_RAIZ` es la raiz
/// extraida (bytes 32..64), copiada aparte para pasar su puntero al syscall
/// de aceptacion.
static mut ANUNCIO: [u8; LARGO_ANUNCIO] = [0; LARGO_ANUNCIO];
static mut HAY_ANUNCIO: bool = false;
static mut ANUNCIO_RAIZ: [u8; 32] = [0; 32];

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

    // Sondear la red por un anuncio de canal en vivo. 168 => hay propuesta;
    // 0 => buzon vacio. Copiamos su raiz aparte para el syscall de aceptacion.
    let n = unsafe {
        sys_canal_anuncio(
            core::ptr::addr_of_mut!(ANUNCIO) as u32,
            LARGO_ANUNCIO as u32,
        )
    };
    if n == LARGO_ANUNCIO as i32 {
        unsafe {
            HAY_ANUNCIO = true;
            let a = &*core::ptr::addr_of!(ANUNCIO);
            (*core::ptr::addr_of_mut!(ANUNCIO_RAIZ)).copy_from_slice(&a[32..64]);
        }
    } else {
        unsafe { HAY_ANUNCIO = false };
    }
}

/// Parsea el sobre demo a sus tres campos sin allocar — postcard de
/// `ManifiestoFirmado` es exactamente `hash(32) || autor(32) || firma(64)`
/// (test `manifiesto_firmado_layout_es_128_bytes_raw` en agora-channel
/// protege este contrato). Verifica la firma localmente con
/// ed25519-compact (misma libreria que el kernel) y, si la firma cierra,
/// lo entrega al kernel. Si la verificacion local falla, NO se llama al
/// syscall — la UI muestra "VERIFICACION LOCAL FALLO" sin quemar un trap.
fn probar_reancla() {
    // Camino VIVO (Fase 64): si hay un anuncio de canal en red, aceptarlo. El
    // kernel re-verifica anillo + firma canonica y reancla; aqui solo pasamos
    // la raiz que el operador vio (cierra el TOCTOU contra un anuncio que se
    // reemplace entre mostrar y aceptar). Toda la criptografia vive en el
    // kernel para este camino —el anuncio no es un sobre `ManifiestoFirmado`
    // de hash pelado, sino una firma canonica que el kernel sabe verificar—.
    if unsafe { HAY_ANUNCIO } {
        let codigo = unsafe { sys_canal_aceptar(core::ptr::addr_of!(ANUNCIO_RAIZ) as u32) };
        unsafe { ULTIMO_CODIGO = codigo };
        return;
    }

    // Camino DEMO (legacy): sin anuncio en red, probamos el sobre horneado por
    // el camino del hash pelado (`sys_manifiesto_proponer`), con verificacion
    // local previa. Util para ejercitar la cadena offline sin red.
    use ed25519_compact::{PublicKey, Signature};

    let bytes: &[u8; 128] = PROPUESTA_DEMO;
    let hash: &[u8; 32] = bytes[0..32].try_into().expect("hash slice");
    let autor: &[u8; 32] = bytes[32..64].try_into().expect("autor slice");
    let firma: &[u8; 64] = bytes[64..128].try_into().expect("firma slice");

    // Verificacion en userspace: pk.verify(hash, sig). Misma logica que
    // el kernel ejecuta despues; filtrar aqui evita el syscall sobre un
    // sobre corrupto o forjado.
    let pk = match PublicKey::from_slice(autor) {
        Ok(pk) => pk,
        Err(_) => {
            unsafe { ULTIMO_CODIGO = VERIFICACION_LOCAL_FALLO };
            return;
        }
    };
    let sig = match Signature::from_slice(firma) {
        Ok(sig) => sig,
        Err(_) => {
            unsafe { ULTIMO_CODIGO = VERIFICACION_LOCAL_FALLO };
            return;
        }
    };
    if pk.verify(*hash, &sig).is_err() {
        unsafe { ULTIMO_CODIGO = VERIFICACION_LOCAL_FALLO };
        return;
    }

    // La firma cierra. Ahora si vale gastar el syscall. El kernel hara
    // su propia verificacion + el chequeo de anillo soberano.
    let codigo = unsafe {
        sys_manifiesto_proponer(PROPUESTA_DEMO.as_ptr() as u32, PROPUESTA_DEMO.len() as u32)
    };
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

    // Cuerpo: modo (red vivo vs demo) + estado.
    let mut y = 48;
    if unsafe { HAY_ANUNCIO } {
        dibujar_texto(lienzo, b"PROPUESTA EN RED (AKASHA)", 16, y, 1, acento);
        y += 12;
        // Raiz recomendada: primeros 4 bytes en hex.
        let raiz = unsafe { ANUNCIO_RAIZ };
        let mut linea = [b' '; 16];
        linea[..6].copy_from_slice(b"RAIZ: ");
        let hx = hex8(&raiz[..4]);
        linea[6..14].copy_from_slice(&hx);
        dibujar_texto(lienzo, &linea[..14], 16, y, 1, tinta);
        y += 18;
        dibujar_texto(lienzo, b"SPACE PARA ACEPTAR", 16, y, 1, acento);
        y += 22;
    } else {
        dibujar_texto(lienzo, b"SIN ANUNCIO EN RED", 16, y, 1, tinta);
        y += 12;
        dibujar_texto(lienzo, b"SPACE PRUEBA SOBRE DEMO", 16, y, 1, secundario);
        y += 18;
        dibujar_texto(lienzo, b"ESPERANDO PUBLICAR", 16, y, 1, acento);
        y += 22;
    }

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
            -100 => b"VERIFICACION LOCAL FALLO",
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
    let cola = b"   FASE 64";
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

/// Convierte hasta 4 bytes en 8 caracteres hex ASCII MAYUSCULAS (la mini-fuente
/// solo trae A-F en mayuscula). Sin asignacion.
fn hex8(bytes: &[u8]) -> [u8; 8] {
    const DIG: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [b'0'; 8];
    for (i, &b) in bytes.iter().take(4).enumerate() {
        out[i * 2] = DIG[(b >> 4) as usize];
        out[i * 2 + 1] = DIG[(b & 0x0F) as usize];
    }
    out
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
