// =============================================================================
//  wawa :: apps/asistente — Fase 60 v3+v4 :: scaffolding del asistente WASM
// -----------------------------------------------------------------------------
//  Asistente conversacional dentro de wawa. Vamos por capas:
//
//  - v1 :: UI puro: pinta el fondo, el titulo, el roadmap.
//  - v2 :: input de texto local (sys_get_scancode + traduccion +
//          buffer QUERY). Sin red todavia: Enter no manda nada.
//  - v3 :: sys_red_enviar / sys_red_recibir sobre `CANAL_ASISTENTE`.
//  - v4 :: presentar propuestas y disparar la firma humana via
//          `daemon-firma` cuando aplique.
//
//  Este archivo cubre v1+v2.
// =============================================================================

#![no_std]

// --- Capacidades del kernel `wawa` que esta app usa. v3 ya monta las de
//     red — la app necesitara PERMISO_RED en su EntradaApp cuando se
//     siembre en GENESIS. v4 sumara PERMISO_RAIZ para disparar la firma
//     humana de re-anclas. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    /// Devuelve el ultimo scancode pulsado en bruto, o 0 si la cola del
    /// teclado de la app esta vacia. Es la misma syscall que `mudanza`
    /// usa para anti-rebote del SPACE.
    fn sys_get_scancode() -> u32;
    /// Copia los 6 bytes de la MAC de la tarjeta de red en `salida`.
    /// Gateada por PERMISO_RED. Si la red no esta montada o el slot no
    /// existe, el contador devuelve `< 0`.
    fn sys_net_mac(salida: u32) -> i32;
    /// Envia un frame Ethernet crudo (cabecera + payload, sin CRC). El
    /// (ptr, len) es de NUESTRA memoria lineal; el host verifica. Gateada
    /// por PERMISO_RED.
    fn sys_net_enviar(ptr: u32, len: u32) -> i32;
    /// Saca el siguiente frame de la cola del usuario hacia `(salida,
    /// capacidad)`. Bytes copiados >0 si habia frame, 0 si la cola esta
    /// vacia, valor negativo si no hay red. El kernel demuxa Akasha
    /// (0x88B5) en su propio reino; lo que llega aqui es trafico de
    /// EtherType ajeno (incluido el del asistente, 0x88B6).
    fn sys_net_recibir(salida: u32, capacidad: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria. DEBE encajar con la region que el manifiesto reserve
//     para esta app cuando se siembre en GENESIS. ---
const ANCHO: usize = 480;
const ALTO: usize = 240;

// --- Paleta. v1 usa colores hardcoded (alineados con la paleta del
//     compositor: indigo oscuro de fondo, slate de panel, indigo
//     brillante de acento, blanco suave de tinta). v2 leera la paleta
//     activa via `sys_config_paleta` cuando integre el sistema de temas. ---
const FONDO: u32 = 0x12_16_20;
const PANEL: u32 = 0x1B_21_30;
const ACENTO: u32 = 0x6E_8C_DC;
const TINTA: u32 = 0xE8_EC_F4;
const SUTIL: u32 = 0x8C_98_AA;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- Estado de v2: el operador escribe una query en vivo. ---

/// Cota dura de la query — caracteres ASCII. Por encima, los keystrokes
/// se descartan en silencio (el operador ve que el texto no crece).
const QUERY_MAX: usize = 64;
static mut QUERY: [u8; QUERY_MAX] = [0; QUERY_MAX];
static mut QUERY_LEN: usize = 0;

/// Anti-rebote: el ultimo scancode procesado. Solo el flanco
/// scancode_actual != scancode_previo cuenta como pulsacion (igual
/// patron que `mudanza::SPACE_PREV`).
static mut SCANCODE_PREV: u32 = 0;

/// FASE 60 v2 :: el ultimo carcater visible para que el operador sepa
/// que el input lo esta viendo. Es un byte ASCII o 0 si no hay nada
/// reciente. Util para validacion visual del scaffolding antes de que
/// haya red.
static mut ULTIMO_CHAR: u8 = 0;

// --- Estado de v3: red. ---

/// MAC propia, cargada en `init()`. `MAC_LISTA` distingue "no cargada"
/// de "cargada con ceros".
static mut MAC: [u8; 6] = [0; 6];
static mut MAC_LISTA: bool = false;

/// Contador monotono de consultas — el `id` que se envia al puente. El
/// puente lo correlaciona en sus respuestas; la app lo compara contra
/// `ULTIMA_CONSULTA_ID` para descartar respuestas a consultas viejas
/// que cruzaron la red despues de timeout.
static mut CONSULTA_ID_SIGUIENTE: u64 = 1;
static mut ULTIMA_CONSULTA_ID: u64 = 0;

/// Estado de v3 hasta que llegue una propuesta.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EstadoRed {
    /// Nada en vuelo. El operador puede tipear.
    Reposo,
    /// Hay una consulta esperando respuesta del puente.
    EsperandoPropuesta,
    /// Llego una propuesta del tipo indicado (codigo de `TipoCable`).
    /// El texto/hash esta en `RESPUESTA_BUFFER[..RESPUESTA_LEN]`.
    Propuesta(u16),
    /// Llego un Error.
    Error,
}

static mut ESTADO_RED: EstadoRed = EstadoRed::Reposo;

/// Buffer para la representacion legible de la ultima respuesta. Hasta
/// 256 bytes — caben textos cortos del LLM. Si una propuesta trae mas,
/// se trunca y la app pinta una pista visual ("...").
const RESPUESTA_MAX: usize = 256;
static mut RESPUESTA_BUFFER: [u8; RESPUESTA_MAX] = [0; RESPUESTA_MAX];
static mut RESPUESTA_LEN: usize = 0;

/// Buffer de transmision — un frame Ethernet entero. Cabecera 14 +
/// cabecera_cable 12 + prompt hasta QUERY_MAX = 64 + holgura.
const TX_MAX: usize = 256;
static mut TX_BUFFER: [u8; TX_MAX] = [0; TX_MAX];

/// Buffer de recepcion — MTU clasico.
const RX_MAX: usize = 1518;
static mut RX_BUFFER: [u8; RX_MAX] = [0; RX_MAX];

// =============================================================================
//  Espejo del protocolo cable del asistente
// -----------------------------------------------------------------------------
//  Las constantes y helpers que siguen son ESPEJO de lo definido en
//  `shared/format/src/lib.rs` (`CANAL_ASISTENTE`, `ETHERTYPE_ASISTENTE`,
//  `TipoCable`, `TAM_CABECERA_CABLE`, `escribir_cabecera_cable`,
//  `leer_cabecera_cable`). Los duplicamos AQUI porque importar `format`
//  arrastra el `extern crate alloc` que esta app `no_std` puro no provee.
//  Los tests de format (`tipo_cable_codigos_estables`, etc.) defienden
//  contra que los discriminantes diverjan; si alguien cambia uno, el
//  test del lado kernel/puente lo caza y este archivo hay que actualizarlo
//  a mano.
// =============================================================================

const CANAL_ASISTENTE: u16 = 0x4153;
const ETHERTYPE_ASISTENTE: u16 = 0x88B6;
const TAM_CABECERA_CABLE: usize = 12;

// Espejo de `format::TipoCable` discriminantes:
const TIPO_CABLE_CONSULTA: u16 = 1;
const TIPO_CABLE_PROPUESTA_NOTAR: u16 = 2;
const TIPO_CABLE_PROPUESTA_LANZAR: u16 = 3;
const TIPO_CABLE_PROPUESTA_INSTALAR: u16 = 4;
const TIPO_CABLE_PROPUESTA_CAMBIAR_CONFIG: u16 = 5;
const TIPO_CABLE_ERROR: u16 = 6;

/// EtherType del asistente en big endian.
const ETHERTYPE_ASISTENTE_BE: [u8; 2] = [
    (ETHERTYPE_ASISTENTE >> 8) as u8,
    ETHERTYPE_ASISTENTE as u8,
];

/// Cabecera Ethernet: dest (6) + src (6) + ethertype (2) = 14 bytes.
const TAM_CAB_ETH: usize = 14;

/// Escribe la cabecera del cable en `out` (debe tener al menos 12 bytes).
/// Espejo de `format::escribir_cabecera_cable`.
fn escribir_cabecera_cable(out: &mut [u8], tipo: u16, id: u64) -> Option<usize> {
    if out.len() < TAM_CABECERA_CABLE {
        return None;
    }
    out[0..2].copy_from_slice(&CANAL_ASISTENTE.to_be_bytes());
    out[2..4].copy_from_slice(&tipo.to_be_bytes());
    out[4..12].copy_from_slice(&id.to_be_bytes());
    Some(TAM_CABECERA_CABLE)
}

/// Lee y valida la cabecera del cable. Espejo de
/// `format::leer_cabecera_cable`. Devuelve `(tipo, id)` o `None`.
fn leer_cabecera_cable(bytes: &[u8]) -> Option<(u16, u64)> {
    if bytes.len() < TAM_CABECERA_CABLE {
        return None;
    }
    let canal = u16::from_be_bytes([bytes[0], bytes[1]]);
    if canal != CANAL_ASISTENTE {
        return None;
    }
    let tipo = u16::from_be_bytes([bytes[2], bytes[3]]);
    let id = u64::from_be_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
    ]);
    Some((tipo, id))
}

/// El kernel invoca esta funcion UNA sola vez, al instanciar el modulo.
/// Pinta el primer fotograma de modo que la ventana no nazca vacia.
#[no_mangle]
pub extern "C" fn init() {
    cargar_mac();
    pintar();
    volcar();
}

/// Un fotograma de trabajo. v3 :: drena scancodes, drena la red,
/// y recompone el fotograma. Enter dispara una `Consulta` por el cable
/// (CANAL_ASISTENTE sobre EtherType 0x88B6).
#[no_mangle]
pub extern "C" fn tick() {
    procesar_teclado();
    drenar_red();
    pintar();
    volcar();
}

/// Lee el scancode pendiente y, si es un flanco de subida nuevo,
/// actualiza el estado: append a `QUERY` si es printable, pop si es
/// backspace, Enter es no-op (v3 lo conectara). Make codes (bit 7
/// limpio) son los unicos que producen efecto; los break codes
/// (bit 7 puesto) se ignoran — la pulsacion ya quedo contada en su make.
fn procesar_teclado() {
    let actual = unsafe { sys_get_scancode() };
    let prev = unsafe { SCANCODE_PREV };
    // Solo el flanco sirve: si llega el mismo scancode dos ticks
    // seguidos sin cambiar, no lo re-procesamos.
    if actual == prev {
        return;
    }
    unsafe { SCANCODE_PREV = actual };
    if actual == 0 || actual >= 0x80 {
        // Cola vacia o break code; ignorar.
        return;
    }
    let sc = actual as u8;
    // Backspace (scancode 0x0E en set 1).
    if sc == 0x0E {
        unsafe {
            if QUERY_LEN > 0 {
                QUERY_LEN -= 1;
                QUERY[QUERY_LEN] = 0;
            }
        }
        return;
    }
    // Enter (scancode 0x1C en set 1) — dispara la consulta al puente.
    // El send vacia QUERY y mueve el estado a EsperandoPropuesta; las
    // pulsaciones siguientes vuelven a construir una query nueva.
    if sc == 0x1C {
        unsafe { ULTIMO_CHAR = b'\n' };
        enviar_consulta();
        return;
    }
    // Letra/cifra/espacio: append si cabe.
    if let Some(byte) = traducir_scancode_a_ascii(sc) {
        unsafe {
            if QUERY_LEN < QUERY_MAX {
                QUERY[QUERY_LEN] = byte;
                QUERY_LEN += 1;
                ULTIMO_CHAR = byte;
            }
        }
    }
}

/// Mapa minimo de scancodes set 1 a ASCII MAYUSCULA — la app usa la
/// fuente que solo tiene mayusculas, asi que no perdemos info al subir
/// a uppercase. Sin shift detection: el operador escribe mayusculas
/// siempre (consistente con el resto de las apps del kernel).
fn traducir_scancode_a_ascii(sc: u8) -> Option<u8> {
    // Cifras '1'-'9' en 0x02..0x0A, '0' en 0x0B.
    if (0x02..=0x0A).contains(&sc) {
        return Some(b'1' + (sc - 0x02));
    }
    if sc == 0x0B {
        return Some(b'0');
    }
    // Espacio en 0x39.
    if sc == 0x39 {
        return Some(b' ');
    }
    // Letras QWERTY en set 1. Tabla escrita a mano — chiquita y
    // determinista, sin alocacion.
    let letra = match sc {
        0x10 => b'Q', 0x11 => b'W', 0x12 => b'E', 0x13 => b'R',
        0x14 => b'T', 0x15 => b'Y', 0x16 => b'U', 0x17 => b'I',
        0x18 => b'O', 0x19 => b'P',
        0x1E => b'A', 0x1F => b'S', 0x20 => b'D', 0x21 => b'F',
        0x22 => b'G', 0x23 => b'H', 0x24 => b'J', 0x25 => b'K',
        0x26 => b'L',
        0x2C => b'Z', 0x2D => b'X', 0x2E => b'C', 0x2F => b'V',
        0x30 => b'B', 0x31 => b'N', 0x32 => b'M',
        _ => return None,
    };
    Some(letra)
}

// =============================================================================
//  Red — v3: cargar MAC, enviar Consulta, drenar Propuestas/Errores
// =============================================================================

/// Carga la MAC propia desde el kernel. La app necesita PERMISO_RED
/// para que esta capacidad este enlazada; si el kernel devuelve `< 0`,
/// dejamos `MAC_LISTA = false` y los envios fallaran silenciosamente
/// (la app sigue funcionando como UI sin red).
fn cargar_mac() {
    unsafe {
        let codigo = sys_net_mac(core::ptr::addr_of_mut!(MAC) as u32);
        MAC_LISTA = codigo == 0;
    }
}

/// Construye un frame Ethernet con el contenido de `QUERY` como prompt
/// del cable y lo envia. Vacia `QUERY` y pasa el estado a
/// `EsperandoPropuesta`. No-op si `MAC_LISTA == false` (sin red) o si
/// `QUERY_LEN == 0` (no hay nada que preguntar).
fn enviar_consulta() {
    unsafe {
        if !MAC_LISTA || QUERY_LEN == 0 {
            return;
        }
        let id = CONSULTA_ID_SIGUIENTE;
        CONSULTA_ID_SIGUIENTE = CONSULTA_ID_SIGUIENTE.wrapping_add(1);
        ULTIMA_CONSULTA_ID = id;

        // Construir frame Ethernet en `TX_BUFFER`. Sin asignacion.
        // dest = broadcast (FF:FF:FF:FF:FF:FF), src = MAC, ethertype.
        let tx = &mut *core::ptr::addr_of_mut!(TX_BUFFER);
        tx[0..6].copy_from_slice(&[0xFF; 6]);
        // SEGURIDAD: lectura de MAC en contexto single-threaded —
        // `init()` la setea una sola vez y `enviar_consulta` corre dentro
        // de `tick()`, jamas concurrente.
        let mac_ref: &[u8; 6] = &*core::ptr::addr_of!(MAC);
        tx[6..12].copy_from_slice(mac_ref);
        tx[12..14].copy_from_slice(&ETHERTYPE_ASISTENTE_BE);

        // Cabecera cable (12 bytes) + payload.
        let cab_dst = &mut tx[TAM_CAB_ETH..TAM_CAB_ETH + TAM_CABECERA_CABLE];
        let _ = escribir_cabecera_cable(cab_dst, TIPO_CABLE_CONSULTA, id);

        let payload_inicio = TAM_CAB_ETH + TAM_CABECERA_CABLE;
        let n = QUERY_LEN.min(TX_MAX - payload_inicio);
        tx[payload_inicio..payload_inicio + n].copy_from_slice(&QUERY[..n]);
        let total = payload_inicio + n;

        let _ = sys_net_enviar(tx.as_ptr() as u32, total as u32);

        // Limpiar QUERY despues de mandar; mover estado.
        for i in 0..QUERY_LEN {
            QUERY[i] = 0;
        }
        QUERY_LEN = 0;
        ESTADO_RED = EstadoRed::EsperandoPropuesta;
    }
}

/// Drena la cola del usuario hasta vaciarla. Filtra por EtherType
/// (queremos solo 0x88B6) y por canal en la cabecera del cable. Cuando
/// llega un frame valido, lo decodifica segun TipoCable y actualiza
/// `ESTADO_RED` + `RESPUESTA_BUFFER`. Frames ajenos al asistente se
/// descartan en silencio — el demuxer del kernel envia todo lo no-Akasha
/// a esta cola y la app filtra.
fn drenar_red() {
    unsafe {
        loop {
            let rx = &mut *core::ptr::addr_of_mut!(RX_BUFFER);
            let n = sys_net_recibir(rx.as_mut_ptr() as u32, RX_MAX as u32);
            if n <= 0 {
                return;
            }
            let n = n as usize;
            if n < TAM_CAB_ETH + TAM_CABECERA_CABLE {
                continue; // demasiado corto para el asistente
            }
            // Filtrar por EtherType.
            if rx[12..14] != ETHERTYPE_ASISTENTE_BE {
                continue;
            }
            // Leer cabecera del cable.
            let cable = &rx[TAM_CAB_ETH..TAM_CAB_ETH + TAM_CABECERA_CABLE];
            let Some((tipo, id)) = leer_cabecera_cable(cable) else {
                continue;
            };
            // Solo aceptamos respuestas a NUESTRA ultima consulta. Esto
            // evita procesar respuestas dirigidas a otros nodos y
            // tambien respuestas viejas (id != ULTIMA_CONSULTA_ID).
            if id != ULTIMA_CONSULTA_ID {
                continue;
            }
            let payload = &rx[TAM_CAB_ETH + TAM_CABECERA_CABLE..n];
            absorber_propuesta(tipo, payload);
        }
    }
}

/// Aplica la propuesta recibida al estado de la app. Solo lo que la app
/// necesita para pintar — el resto (firma humana de InstalarApp /
/// CambiarConfig) es trabajo de v4.
fn absorber_propuesta(tipo: u16, payload: &[u8]) {
    unsafe {
        ESTADO_RED = match tipo {
            TIPO_CABLE_PROPUESTA_NOTAR => {
                copiar_a_respuesta(payload);
                EstadoRed::Propuesta(tipo)
            }
            TIPO_CABLE_PROPUESTA_LANZAR => {
                // Payload es u32 BE con el indice de plantilla.
                if payload.len() >= 4 {
                    let idx = u32::from_be_bytes([
                        payload[0], payload[1], payload[2], payload[3],
                    ]);
                    let mut buf = [0u8; 16];
                    let len = formatear_u32(idx, &mut buf);
                    copiar_a_respuesta(&buf[..len]);
                    EstadoRed::Propuesta(tipo)
                } else {
                    copiar_a_respuesta(b"INDICE TRUNCADO");
                    EstadoRed::Error
                }
            }
            TIPO_CABLE_PROPUESTA_INSTALAR | TIPO_CABLE_PROPUESTA_CAMBIAR_CONFIG => {
                // Payload son 32 bytes de hash. v3 pinta solo los
                // primeros 4 bytes en hex como pista; v4 lo mandara al
                // daemon-firma para confirmacion humana.
                if payload.len() >= 4 {
                    let mut buf = [b'-'; 16];
                    buf[0] = hex_nibble(payload[0] >> 4);
                    buf[1] = hex_nibble(payload[0] & 0x0F);
                    buf[2] = hex_nibble(payload[1] >> 4);
                    buf[3] = hex_nibble(payload[1] & 0x0F);
                    buf[4] = hex_nibble(payload[2] >> 4);
                    buf[5] = hex_nibble(payload[2] & 0x0F);
                    buf[6] = hex_nibble(payload[3] >> 4);
                    buf[7] = hex_nibble(payload[3] & 0x0F);
                    copiar_a_respuesta(&buf[..8]);
                    EstadoRed::Propuesta(tipo)
                } else {
                    copiar_a_respuesta(b"HASH TRUNCADO");
                    EstadoRed::Error
                }
            }
            TIPO_CABLE_ERROR => {
                copiar_a_respuesta(payload);
                EstadoRed::Error
            }
            TIPO_CABLE_CONSULTA => {
                // Otro nodo hablando — no es para nosotros. Silencio.
                return;
            }
            _ => {
                // Tipo desconocido — ignorar silenciosamente.
                return;
            }
        };
    }
}

/// Copia hasta `RESPUESTA_MAX` bytes de `src` a `RESPUESTA_BUFFER`.
/// Convierte ASCII no-printable a `?` y bytes >= 0x80 a `*` (fuente
/// solo soporta mayusculas y digitos). Resto pasa tal cual.
fn copiar_a_respuesta(src: &[u8]) {
    unsafe {
        let n = src.len().min(RESPUESTA_MAX);
        for i in 0..n {
            let b = src[i];
            RESPUESTA_BUFFER[i] = if b.is_ascii_alphanumeric() || b == b' ' || b == b'.' || b == b':' || b == b'-' {
                b.to_ascii_uppercase()
            } else if b == b'\n' || b == b'\r' || b == b'\t' {
                b' '
            } else {
                b'?'
            };
        }
        RESPUESTA_LEN = n;
    }
}

/// Formatea un u32 a ASCII decimal en `out`, devolviendo la longitud.
/// Mas barato que `core::fmt` — no usa el formatter.
fn formatear_u32(mut n: u32, out: &mut [u8]) -> usize {
    if n == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut len = 0;
    while n > 0 && len < tmp.len() {
        tmp[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    let mut pos = 0;
    for i in (0..len).rev() {
        if pos < out.len() {
            out[pos] = tmp[i];
            pos += 1;
        }
    }
    pos
}

fn hex_nibble(n: u8) -> u8 {
    let n = n & 0x0F;
    if n < 10 {
        b'0' + n
    } else {
        b'A' + (n - 10)
    }
}

// =============================================================================
//  Pintado del fotograma
// =============================================================================

fn pintar() {
    // SEGURIDAD: durante `init` y `tick` esta es la unica via de acceso al
    // LIENZO; el kernel jamas reentra el modulo mientras una de ellas corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    // Fondo plano + barra de titulo.
    rellenar_rect(lienzo, 0, 0, ANCHO, ALTO, FONDO);
    rellenar_rect(lienzo, 0, 0, ANCHO, 36, PANEL);
    dibujar_texto(lienzo, b"ASISTENTE", 18, 10, 2, ACENTO);
    rellenar_rect(lienzo, 0, 36, ANCHO, 2, ACENTO);

    // FASE 60 v2 :: la zona de input. Caja con el prompt y el contenido
    // de QUERY. Vacio cuando el operador no escribio nada todavia.
    let mut y = 56;
    dibujar_texto(lienzo, b"PROMPT:", 18, y, 1, SUTIL);
    y += 14;
    rellenar_rect(lienzo, 18, y, ANCHO - 36, 24, PANEL);
    rellenar_rect(lienzo, 18, y, 2, 24, ACENTO); // borde izq del input
    // El texto de la query, en mayusculas (la fuente solo tiene mayus).
    // SEGURIDAD: lectura de mutable static en contexto single-threaded
    // — solo `tick` muta `QUERY`/`QUERY_LEN`, y no reentra mientras
    // `pintar` corre.
    let (query, query_len): (&[u8], usize) = unsafe { (&QUERY[..QUERY_LEN], QUERY_LEN) };
    dibujar_texto(lienzo, query, 28, y + 8, 1, TINTA);
    // Cursor al final — un guion bajo grueso.
    let cursor_x = 28 + query_len * 6;
    if cursor_x < ANCHO - 12 {
        rellenar_rect(lienzo, cursor_x, y + 16, 4, 2, ACENTO);
    }
    y += 32;

    // FASE 60 v3 :: estado de red. Linea fija con el estado del puente
    // + el contenido de la respuesta cuando llega.
    let (etiqueta, tinta) = match unsafe { ESTADO_RED } {
        EstadoRed::Reposo => {
            let lista = unsafe { MAC_LISTA };
            if lista {
                (b"RED LISTA  ENTER PARA CONSULTAR".as_slice(), SUTIL)
            } else {
                (b"SIN RED  REVISA PERMISO_RED".as_slice(), TINTA)
            }
        }
        EstadoRed::EsperandoPropuesta => (b"ESPERANDO PROPUESTA DEL PUENTE".as_slice(), ACENTO),
        EstadoRed::Propuesta(tipo) => match tipo {
            TIPO_CABLE_PROPUESTA_NOTAR => (b"NOTA DEL PUENTE:".as_slice(), TINTA),
            TIPO_CABLE_PROPUESTA_LANZAR => (b"LANZAR APP IDX:".as_slice(), ACENTO),
            TIPO_CABLE_PROPUESTA_INSTALAR => {
                (b"INSTALAR MANIFIESTO HASH:".as_slice(), ACENTO)
            }
            TIPO_CABLE_PROPUESTA_CAMBIAR_CONFIG => {
                (b"CAMBIAR CONFIG HASH:".as_slice(), ACENTO)
            }
            _ => (b"PROPUESTA RECIBIDA".as_slice(), SUTIL),
        },
        EstadoRed::Error => (b"ERROR DEL PUENTE:".as_slice(), TINTA),
    };
    dibujar_texto(lienzo, etiqueta, 18, y, 1, tinta);
    y += 14;

    // Contenido de la respuesta — primeros caracteres legibles.
    let (resp, resp_len): (&[u8], usize) =
        unsafe { (&RESPUESTA_BUFFER[..RESPUESTA_LEN], RESPUESTA_LEN) };
    if resp_len > 0 {
        // Hasta 60 chars caben en el ancho a escala 1.
        let visible_len = resp_len.min(60);
        dibujar_texto(lienzo, &resp[..visible_len], 18, y, 1, TINTA);
        y += 14;
        if resp_len > visible_len {
            dibujar_texto(lienzo, b"...", 18, y, 1, SUTIL);
        }
    }

    // Pie: una franja sutil que marca el limite de la region.
    rellenar_rect(lienzo, 0, ALTO - 2, ANCHO, 2, ACENTO);

    // Suprime unused warning de ULTIMO_CHAR — sigue siendo util para
    // diagnostico aunque ya no la pintemos (v3 prioriza el estado de red).
    let _ = unsafe { ULTIMO_CHAR };
}

/// Entrega el lienzo completo al kernel. (ptr, len) apunta SIEMPRE dentro
/// de nuestra memoria lineal; el host lo verifica sin piedad.
fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    // SEGURIDAD: `sys_render_frame` es una capacidad del host; el (ptr,
    // len) describe nuestra propia memoria lineal.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

// =============================================================================
//  Primitivas de pintado — sin asignacion, sin dependencias
// =============================================================================

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

// =============================================================================
//  Mini-tipografia 5x7 — solo los caracteres que esta app usa
// =============================================================================

const FA: usize = 5; // ancho del glifo
const FH: usize = 7; // alto del glifo

fn glifo(c: u8) -> [u8; FH] {
    match c {
        b' ' => [0; 7],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        b'3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
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
        cursor_x += (FA + 1) * escala;
    }
}
