// =============================================================================
//  renaser :: apps/memoriosa — Fase 7c :: la app que recuerda entre vidas
// -----------------------------------------------------------------------------
//  La cronista deja huella, sí — pero en la RAIZ del grafo, un ancla unica que
//  solo una app puede usar. `memoriosa` estrena la persistencia POR-APP: cada
//  aplicacion tiene, en su `EntradaApp` del Manifiesto de Genesis, una ranura
//  propia —`estado`— donde guardar lo suyo. Mil apps, mil memorias; ninguna
//  pisa a otra.
//
//  Lo que memoriosa recuerda es simple y visible: cuantas teclas se han pulsado
//  EN TODA SU HISTORIA. En cada `tick` drena su canal de teclado; cada
//  pulsacion suma uno y graba el recuento con `sys_estado_guardar`. El kernel lo
//  ancla en el manifiesto. Al reiniciar, `init` lo relee con `sys_estado_cargar`
//  y la app despierta con su cuenta intacta — una celda por tecla, como las dejo.
//
//  El testigo de la esquina lo cuenta todo de un vistazo: verde si nacio limpia
//  —disco recien sembrado—, ambar si desperto con memoria de vidas pasadas.
// =============================================================================

#![no_std]

// --- Las capacidades que el kernel `renaser` inyecta a esta aplicacion. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    /// Compone un bufer de pixeles (de ESTA memoria lineal) en la region que el
    /// kernel asigno a esta aplicacion.
    fn sys_render_frame(ptr: u32, len: u32);

    /// Extrae, sin bloquear, el siguiente scancode del canal de teclado propio
    /// de esta app. Devuelve 0 si el canal esta vacio.
    fn sys_get_scancode() -> u32;

    /// Copia el estado persistido de ESTA app en `salida`. Devuelve los bytes
    /// copiados, 0 si no hay estado previo, o un valor negativo si fallo.
    fn sys_estado_cargar(salida: u32, capacidad: u32) -> i32;

    /// Graba `datos` como el nuevo estado persistido de ESTA app. El kernel lo
    /// ancla en el Manifiesto de Genesis: sobrevivira al reinicio. Devuelve 0
    /// si todo fue bien, un valor negativo si fallo.
    fn sys_estado_guardar(datos: u32, datos_len: u32) -> i32;
}

/// Sin sistema operativo bajo nosotros, un panico solo puede detenerse en seco.
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria de la escena. El ancho y el alto DEBEN coincidir con la region
//     que el kernel asigna a esta app. ---
const ANCHO: usize = 360;
const ALTO: usize = 80;

/// Lado de paso de la rejilla de celdas, en pixeles.
const PASO: usize = 20;
/// Lado de una celda, en pixeles.
const LADO: usize = 16;
/// Celdas que caben en una fila.
const POR_FILA: usize = ANCHO / PASO;
/// Celdas que caben en la rejilla entera — el techo de lo que se pinta.
const MAX_CELDAS: usize = POR_FILA * (ALTO / PASO);

/// Indigo casi negro: el fondo del lienzo de memoriosa.
const FONDO: u32 = 0x10_0E_22;
/// Violeta: una celda por cada tecla pulsada en toda la historia de la app.
const CELDA: u32 = 0x8C_6E_F2;
/// Verde: la app nacio limpia — disco recien sembrado, sin estado previo.
const TESTIGO_FRESCO: u32 = 0x35_C4_6A;
/// Ambar: la app desperto con memoria — leyo su estado de una vida anterior.
const TESTIGO_RECORDADO: u32 = 0xF2_B2_33;

/// El lienzo de la aplicacion, en SU propia memoria lineal.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// Los ocho bytes del recuento, de ida y de vuelta de la ranura de estado.
static mut ESTADO_IO: [u8; 8] = [0; 8];

/// El recuento vivo de pulsaciones — el estado que memoriosa persiste.
static mut CONTADOR: u64 = 0;

/// ¿Desperto la app con un estado previo? Decide el color del testigo.
static mut RECORDADO: bool = false;

/// Preparacion: el kernel la invoca UNA sola vez. memoriosa relee aqui su
/// estado persistido —el recuento de teclas de vidas anteriores— y pinta la
/// rejilla tal como la dejo la ultima vez que la maquina estuvo viva.
#[no_mangle]
pub extern "C" fn init() {
    // Cargar el estado persistido. `8` => habia un recuento anclado; la app
    // despierta con memoria. `0` => disco recien sembrado, sin estado previo.
    let leidos = unsafe { sys_estado_cargar(core::ptr::addr_of_mut!(ESTADO_IO) as u32, 8) };
    if leidos == 8 {
        // SEGURIDAD: lectura de un escalar `Copy` de un estatico propio.
        unsafe {
            CONTADOR = u64::from_le_bytes(*core::ptr::addr_of!(ESTADO_IO));
            RECORDADO = true;
        }
    }
    pintar();
}

/// Un fotograma de trabajo. Drena el canal de teclado: cada tecla pulsada suma
/// uno al recuento. Si hubo alguna pulsacion, persiste el nuevo total —el
/// kernel lo ancla en el manifiesto— y redibuja la rejilla.
#[no_mangle]
pub extern "C" fn tick() {
    let mut cambio = false;
    loop {
        let scancode = unsafe { sys_get_scancode() };
        if scancode == 0 {
            break; // Canal vacio: nada mas que atender este fotograma.
        }
        // Make codes del set 1: `0x01..=0x7F` (bit 7 a cero) — una tecla
        // PULSADA. Los break codes —tecla soltada— llevan el bit 7 y se
        // ignoran: asi cada pulsacion fisica cuenta exactamente una vez.
        if (1..=0x7F).contains(&scancode) {
            // SEGURIDAD: incremento de un escalar estatico propio; el kernel
            // jamas reentra el modulo mientras `tick` corre.
            unsafe {
                CONTADOR += 1;
            }
            cambio = true;
        }
    }

    if cambio {
        // Persistir el nuevo recuento. El kernel lo graba como un objeto del
        // grafo y reescribe el manifiesto: sobrevivira al proximo apagon.
        // SEGURIDAD: escritura de un escalar `Copy` a un estatico propio.
        unsafe {
            *core::ptr::addr_of_mut!(ESTADO_IO) = CONTADOR.to_le_bytes();
            sys_estado_guardar(core::ptr::addr_of!(ESTADO_IO) as u32, 8);
        }
        pintar();
    }
}

/// Pinta la cronica de pulsaciones: el fondo, una celda violeta por cada tecla
/// pulsada en toda la historia de la app y, en la esquina, el testigo que
/// delata si la app desperto con memoria o nacio limpia.
fn pintar() {
    // SEGURIDAD: el kernel jamas reentra el modulo mientras `init` o `tick`
    // corren; esta es la unica via de acceso a LIENZO durante esa ventana.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    // Una celda violeta por cada pulsacion registrada, dispuestas en rejilla.
    // SEGURIDAD: lectura de escalares `Copy` de estaticos propios.
    let contador = unsafe { *core::ptr::addr_of!(CONTADOR) };
    let celdas = (contador as usize).min(MAX_CELDAS);
    for i in 0..celdas {
        let x = (i % POR_FILA) * PASO + 2;
        let y = (i / POR_FILA) * PASO + 2;
        rellenar(lienzo, x, y, LADO, LADO, CELDA);
    }

    // El testigo: ambar si la app leyo un estado de una vida anterior, verde
    // si nacio en un disco recien sembrado, sin memoria que heredar.
    let recordado = unsafe { *core::ptr::addr_of!(RECORDADO) };
    let testigo = if recordado {
        TESTIGO_RECORDADO
    } else {
        TESTIGO_FRESCO
    };
    rellenar(lienzo, ANCHO - 14, 4, 10, 10, testigo);

    // SEGURIDAD: `sys_render_frame` es una capacidad del host; el (ptr, len)
    // describe nuestra propia memoria lineal y el host lo verifica sin piedad.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

/// Rellena un rectangulo, recortado con firmeza a los limites del lienzo.
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
