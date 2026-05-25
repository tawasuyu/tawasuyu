// =============================================================================
//  renaser :: apps/pulso — Fase 11 :: el compas visual del reloj del host
// -----------------------------------------------------------------------------
//  Hasta la Fase 10 una aplicacion solo sabia CUANTAS veces la habian llamado
//  —un `tick` tras otro—, no CUANTO tiempo habia pasado. La Fase 11 le da al
//  userspace un reloj: la capacidad `sys_tiempo_mono`, los milisegundos
//  monotonos desde el arranque.
//
//  `pulso` es su testigo. Dibuja un compas —una cabeza brillante que recorre
//  una pista y vuelve— cuya posicion es una FUNCION PURA del reloj del host.
//  No guarda estado entre fotogramas: no le hace falta. Y de ahi su prueba:
//  dos instancias de `pulso`, nazca una al arrancar y otra mucho despues con
//  un `Alt+N`, laten EXACTAMENTE al unisono — porque ambas leen el mismo
//  reloj, no un contador propio de fotogramas.
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

/// El periodo del compas, en milisegundos: la cabeza recorre la pista entera y
/// vuelve a empezar cada `PERIODO`.
const PERIODO: u64 = 6000;

/// Azul nocturno: el fondo del lienzo.
const FONDO: u32 = 0x0A_18_30;
/// La pista por donde corre el compas — un surco tenue.
const PISTA: u32 = 0x1E_2A_44;
/// La estela ya recorrida en este ciclo.
const ESTELA: u32 = 0x2E_50_C8;
/// La cabeza del compas — el indigo brillante del foco del compositor.
const CABEZA: u32 = 0x8B_5C_F6;

/// Margen lateral de la pista, en pixeles.
const MARGEN: usize = 24;
/// Filas que ocupa la pista — un surco horizontal centrado en el lienzo.
const PISTA_Y0: usize = 46;
const PISTA_Y1: usize = 74;
/// Anchura de la cabeza del compas, en pixeles.
const CABEZA_ANCHO: usize = 7;

/// El lienzo de la aplicacion, en SU propia memoria lineal. El kernel jamas lo
/// ve directamente: solo recibe el (ptr, len) que cada fotograma le entrega.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// Preparacion: el kernel la invoca UNA sola vez, al cargar el modulo. Pinta el
/// primer fotograma — el compas en el instante de nacer.
#[no_mangle]
pub extern "C" fn init() {
    pintar();
}

/// Un fotograma de trabajo: vuelve a pintar el compas en el instante actual y
/// RETORNA, cediendo la CPU al kernel y a las apps vecinas.
#[no_mangle]
pub extern "C" fn tick() {
    pintar();
}

/// Pinta el compas en el instante ACTUAL. No guarda estado alguno: la escena es
/// una funcion pura del reloj del host. Por eso dos instancias de `pulso`,
/// nazcan cuando nazcan, laten EXACTAMENTE al unisono.
fn pintar() {
    // SEGURIDAD: durante `init` y `tick` esta es la unica via de acceso a
    // LIENZO, y el kernel jamas reentra el modulo mientras una de ellas corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };

    // La fase del compas: 0 al inicio del periodo, casi 1 al final. Se deriva
    // SOLO del reloj del host — ni un contador propio, ni un origen guardado.
    // SEGURIDAD: `sys_tiempo_mono` es una capacidad del host; no toca memoria.
    let tiempo = unsafe { sys_tiempo_mono() };
    let fase = tiempo % PERIODO;

    // El ancho util de la pista y hasta donde ha llegado la cabeza.
    let util = ANCHO - 2 * MARGEN;
    let avance = (fase as usize * util) / PERIODO as usize;

    // Fondo limpio.
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }
    // La pista entera, tenue.
    banda(lienzo, MARGEN, MARGEN + util, PISTA);
    // La estela ya recorrida en este ciclo.
    banda(lienzo, MARGEN, MARGEN + avance, ESTELA);
    // La cabeza del compas: una barra brillante al frente de la estela.
    let cabeza0 = MARGEN + avance;
    banda(lienzo, cabeza0, cabeza0 + CABEZA_ANCHO, CABEZA);

    volcar(lienzo);
}

/// Rellena la pista —las filas [`PISTA_Y0`, `PISTA_Y1`)— entre las columnas
/// [x0, x1) con un color, recortado con firmeza al ancho del lienzo.
fn banda(lienzo: &mut [u32], x0: usize, x1: usize, color: u32) {
    let x0 = x0.min(ANCHO);
    let x1 = x1.min(ANCHO);
    let mut fila = PISTA_Y0;
    while fila < PISTA_Y1 {
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
