// =============================================================================
//  renaser :: apps/tonada — Fase 12 :: una melodia visual para estrenar la voz
// -----------------------------------------------------------------------------
//  Con la Fase 11 el userspace gano un reloj; con la Fase 12, una voz: la
//  bocina del PC, capacidad `sys_tono`. `tonada` las junta. Toca una escala de
//  Do mayor, una y otra vez, y la dibuja como una escalera de barras — la nota
//  que suena, encendida.
//
//  Su escena y su sonido son una FUNCION PURA del reloj del host: no guarda
//  estado entre fotogramas. Y la bocina pertenece a la ventana ENFOCADA —como
//  el teclado—: `tonada` pide su tono en cada fotograma, pero solo se oye
//  cuando tiene el foco. Mira la melodia siempre; escuchala cuando la enfocas.
// =============================================================================

#![no_std]

// --- Las capacidades que el kernel `renaser` inyecta a esta aplicacion. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    /// Compone un bufer de pixeles (de ESTA memoria lineal) en la region que el
    /// kernel asigno a esta aplicacion.
    fn sys_render_frame(ptr: u32, len: u32);
    /// El reloj monotono del sistema: milisegundos desde el arranque.
    fn sys_tiempo_mono() -> u64;
    /// Hace sonar la bocina del PC a una frecuencia (un 0 la silencia). Solo se
    /// oye si esta ventana tiene el foco — lo decide el host.
    fn sys_tono(frecuencia_hz: u32);
}

/// Sin sistema operativo bajo nosotros, un panico solo puede detenerse en seco.
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria de la escena. El ancho y el alto DEBEN coincidir con la region
//     que el kernel asigna a esta app. ---
const ANCHO: usize = 360;
const ALTO: usize = 120;

/// La melodia: una escala de Do mayor ascendente. Frecuencias en Hz —de Do4 a
/// Do5—. El indice de la nota es, ademas, el indice de su barra en pantalla.
const MELODIA: [u32; 8] = [262, 294, 330, 349, 392, 440, 494, 523];

/// Duracion de cada nota, en milisegundos.
const NOTA_MS: u64 = 420;

/// Azul nocturno: el fondo del lienzo.
const FONDO: u32 = 0x0A_18_30;
/// Una nota en reposo — una barra de la escalera, apagada.
const BARRA: u32 = 0x24_30_4E;
/// La nota que suena AHORA — el indigo brillante del foco del compositor.
const BARRA_VIVA: u32 = 0x8B_5C_F6;

/// Margen lateral, en pixeles.
const MARGEN: usize = 22;
/// La linea base sobre la que se alzan las barras.
const BASE_Y: usize = 100;
/// Altura de la barra mas grave, y cuanto crece cada peldaño de la escala.
const ALTO_MIN: usize = 13;
const ALTO_PASO: usize = 11;

/// El lienzo de la aplicacion, en SU propia memoria lineal. El kernel jamas lo
/// ve directamente: solo recibe el (ptr, len) que cada fotograma le entrega.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// Preparacion: el kernel la invoca UNA sola vez, al cargar el modulo.
#[no_mangle]
pub extern "C" fn init() {
    pintar();
}

/// Un fotograma de trabajo: pide el tono de la nota actual, redibuja la
/// escalera y RETORNA, cediendo la CPU al kernel y a las apps vecinas.
#[no_mangle]
pub extern "C" fn tick() {
    pintar();
}

/// Pinta —y suena— la melodia en el instante ACTUAL. No guarda estado alguno:
/// la nota viva es una funcion pura del reloj del host.
fn pintar() {
    // SEGURIDAD: durante `init` y `tick` esta es la unica via de acceso a
    // LIENZO, y el kernel jamas reentra el modulo mientras una de ellas corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    // La nota actual: se deriva SOLO del reloj del host.
    // SEGURIDAD: `sys_tiempo_mono` es una capacidad del host; no toca memoria.
    let tiempo = unsafe { sys_tiempo_mono() };
    let ciclo = NOTA_MS * MELODIA.len() as u64;
    let actual = ((tiempo % ciclo) / NOTA_MS) as usize;

    // Pedir el tono de la nota actual. El host solo lo hara sonar si esta
    // ventana tiene el foco — la app pide sin saberlo.
    // SEGURIDAD: `sys_tono` es una capacidad del host; no toca memoria.
    unsafe {
        sys_tono(MELODIA[actual]);
    }

    // Fondo limpio.
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    // Una barra por nota: la escalera de la melodia. La que suena, encendida.
    let util = ANCHO - 2 * MARGEN;
    let ranura = util / MELODIA.len();
    let barra_ancho = ranura * 3 / 4;
    let mut i = 0;
    while i < MELODIA.len() {
        let altura = ALTO_MIN + i * ALTO_PASO;
        let x0 = MARGEN + i * ranura + (ranura - barra_ancho) / 2;
        let color = if i == actual { BARRA_VIVA } else { BARRA };
        columna(lienzo, x0, barra_ancho, altura, color);
        i += 1;
    }

    volcar(lienzo);
}

/// Pinta una barra vertical: `ancho` pixeles desde la columna `x0`, `altura`
/// pixeles alzandose sobre la linea base `BASE_Y`. Se recorta al lienzo.
fn columna(lienzo: &mut [u32], x0: usize, ancho: usize, altura: usize, color: u32) {
    let x1 = (x0 + ancho).min(ANCHO);
    let y1 = BASE_Y.min(ALTO);
    let y0 = y1.saturating_sub(altura);
    let mut fila = y0;
    while fila < y1 {
        let base = fila * ANCHO;
        let mut col = x0;
        while col < x1 {
            lienzo[base + col] = color;
            col += 1;
        }
        fila += 1;
    }
}

/// Entrega el lienzo completo al kernel. El (ptr, len) apunta SIEMPRE dentro de
/// nuestra memoria lineal, y su tamaño es, exactamente, el de la region.
fn volcar(lienzo: &[u32]) {
    // SEGURIDAD: `sys_render_frame` es una capacidad del host; el (ptr, len)
    // describe nuestra propia memoria lineal y el host lo verifica sin piedad.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}
