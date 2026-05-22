// =============================================================================
//  renaser :: apps/hello_wasm — Fase 4/5 :: el primer ciudadano del userspace
// -----------------------------------------------------------------------------
//  Esta aplicacion vive DENTRO de su propia memoria lineal de WebAssembly. No
//  conoce la MMU, no conoce los anillos de privilegio de la CPU: su unica via
//  hacia el mundo son las dos capacidades que el kernel le inyecta. Lo que no
//  este importado, sencillamente, no tiene camino fisico que recorrer.
//
//  FASE 5 :: el ABI deja de ser un `run()` que se queda dentro para siempre.
//  Ahora la app exporta `init()` —preparacion, una sola vez— y `tick()` —un
//  fotograma de trabajo, y RETORNA—. Ese retorno es el punto de cesion
//  cooperativa: el kernel recupera el control y atiende a las demas apps.
// =============================================================================

#![no_std]

// --- Las dos UNICAS capacidades que el kernel `renaser` expone al modulo. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    /// Compone un bufer de pixeles (de ESTA memoria lineal) en la region que el
    /// kernel asigno a esta aplicacion.
    fn sys_render_frame(ptr: u32, len: u32);
    /// Devuelve el ultimo scancode crudo del teclado, o 0 si no hay ninguno.
    fn sys_get_scancode() -> u32;
}

/// Sin sistema operativo bajo nosotros, un panico solo puede detenerse en seco.
#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria de la escena. El ancho y el alto DEBEN coincidir con la region
//     que el kernel asigna a esta app: el host rechaza cualquier fotograma de
//     un tamaño que no sea, exactamente, el de su ventana. ---
const ANCHO: usize = 480;
const ALTO: usize = 560;
const LADO: usize = 96;
const PASO: i32 = 24;

/// Azul nocturno: el fondo del lienzo de la aplicacion.
const FONDO: u32 = 0x0A_18_30;
/// Ambar: el cuadrado que el usuario gobierna.
const CUADRO: u32 = 0xFF_B0_00;

/// El lienzo de la aplicacion, en SU propia memoria lineal. El kernel jamas lo
/// ve directamente: solo recibe el (ptr, len) que cada fotograma le entrega.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// Posicion del cuadrado. Vive entre fotogramas en la memoria lineal del modulo
/// — el estado persiste porque la instancia, en la Fase 5, ya no es efimera.
static mut POS_X: i32 = 0;
static mut POS_Y: i32 = 0;

/// Preparacion: el kernel la invoca UNA sola vez, al cargar el modulo. Pinta el
/// fondo, centra el cuadrado y vuelca el primer fotograma.
#[no_mangle]
pub extern "C" fn init() {
    // SEGURIDAD: durante `init` y `tick` esta es la unica via de acceso a
    // LIENZO, y el kernel jamas reentra el modulo mientras una de ellas corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    let x = (ANCHO / 2 - LADO / 2) as i32;
    let y = (ALTO / 2 - LADO / 2) as i32;
    rellenar(lienzo, x, y, CUADRO);
    // SEGURIDAD: escritura de escalares `Copy`; no se crea referencia alguna.
    unsafe {
        POS_X = x;
        POS_Y = y;
    }

    volcar(lienzo);
}

/// Un fotograma de trabajo: escucha el teclado, mueve el cuadrado, vuelca la
/// imagen y RETORNA. El retorno cede la CPU al kernel y a las apps vecinas.
#[no_mangle]
pub extern "C" fn tick() {
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    let (mut x, mut y) = unsafe { (POS_X, POS_Y) };

    // 1. Escuchar al teclado a traves de la capacidad del host.
    let (dx, dy) = match unsafe { sys_get_scancode() } {
        0x11 => (0, -PASO), // tecla W -> arriba
        0x1F => (0, PASO),  // tecla S -> abajo
        0x1E => (-PASO, 0), // tecla A -> izquierda
        0x20 => (PASO, 0),  // tecla D -> derecha
        _ => (0, 0),
    };

    // 2. Borrar el cuadrado anterior repintando su hueco con el fondo.
    rellenar(lienzo, x, y, FONDO);

    // 3. Moverlo, manteniendolo siempre dentro del lienzo.
    x = (x + dx).clamp(0, (ANCHO - LADO) as i32);
    y = (y + dy).clamp(0, (ALTO - LADO) as i32);

    // 4. Dibujar el cuadrado en su nueva posicion y guardar el estado.
    rellenar(lienzo, x, y, CUADRO);
    unsafe {
        POS_X = x;
        POS_Y = y;
    }

    // 5. Volcar el fotograma: el host lo compondra dentro de nuestra region.
    volcar(lienzo);
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

/// Rellena un cuadrado de lado `LADO`, con su esquina en (x, y), recortado con
/// firmeza a los limites del lienzo.
fn rellenar(lienzo: &mut [u32], x: i32, y: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x0 + LADO).min(ANCHO);
    let y1 = (y0 + LADO).min(ALTO);

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
