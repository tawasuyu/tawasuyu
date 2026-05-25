// =============================================================================
//  renaser :: apps/cronista — Fase 6.1c :: el primer escriba del userspace
// -----------------------------------------------------------------------------
//  Las apps de fases anteriores eran efimeras: su mundo se borraba al apagar la
//  maquina. `cronista` es la primera que deja HUELLA. En cada arranque:
//
//    1. pregunta al kernel por la RAIZ del grafo de objetos —la cabeza de la
//       cadena de arranques anteriores—;
//    2. lee de ella el numero del ultimo arranque;
//    3. graba un objeto nuevo —sus datos: el numero de arranque; su hijo: la
//       raiz anterior— y lo corona como raiz;
//    4. recorre la cadena entera para verificar que el DAG persiste integro;
//    5. pinta una celda por cada arranque registrado.
//
//  La cuenta NO vive en la RAM: vive en el disco, en el grafo direccionado por
//  contenido. Sobrevive a los reinicios. Cada vez que renaser despierta, la
//  cronista añade un eslabon a la cadena y una celda a su rejilla.
//
//  Sus unicas vias hacia el mundo son las capacidades `sys_object_*` que el
//  kernel le inyecta. No conoce el disco, ni el bus PCI, ni el format en
//  sectores: solo objetos, hashes y aristas.
// =============================================================================

#![no_std]

// --- Las capacidades que el kernel `renaser` inyecta a esta aplicacion. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    /// Compone un bufer de pixeles (de ESTA memoria lineal) en la region que el
    /// kernel asigno a esta aplicacion.
    fn sys_render_frame(ptr: u32, len: u32);

    /// Graba un objeto en el grafo: `datos` es su carga util; `hijos` apunta a
    /// un arreglo de `hijos_cnt` hashes de 32 bytes —las aristas—. El hash
    /// resultante se escribe en `salida`. Devuelve 0 si todo fue bien.
    fn sys_object_put(datos: u32, datos_len: u32, hijos: u32, hijos_cnt: u32, salida: u32) -> i32;

    /// Copia la carga util del objeto `hash` en `salida`. Devuelve el numero de
    /// bytes copiados, o un valor negativo si fallo.
    fn sys_object_datos(hash: u32, salida: u32, capacidad: u32) -> i32;

    /// Devuelve el numero de hijos del objeto `hash` y, si `indice` es valido,
    /// escribe el hash de ese hijo en `salida`. Negativo si el objeto no existe.
    fn sys_object_hijo(hash: u32, indice: u32, salida: u32) -> i32;

    /// Escribe en `salida` el hash de la raiz del grafo. Devuelve 1 si hay
    /// raiz, 0 si el grafo aun esta vacio.
    fn sys_object_raiz(salida: u32) -> i32;

    /// Corona el objeto `hash` como raiz del grafo. Devuelve 0 si lo logro.
    fn sys_object_fijar_raiz(hash: u32) -> i32;
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

/// Indigo casi negro: el fondo del lienzo de la cronista.
const FONDO: u32 = 0x0E_14_22;
/// Ambar calido: una celda por arranque registrado.
const CELDA: u32 = 0xF2_B2_33;
/// Verde: el DAG se recorrio integro de la raiz al primer eslabon.
const VERDE: u32 = 0x35_C4_6A;
/// Rojo: la cadena se rompio — un objeto no resolvio.
const ROJO: u32 = 0xD4_1E_2C;

/// El lienzo de la aplicacion, en SU propia memoria lineal.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- Buferes de intercambio con las capacidades `sys_object_*`. El kernel lee
//     y escribe hashes y datos AQUI, siempre dentro de esta memoria lineal. ---
/// El hash de la raiz anterior — la cabeza de la cadena al arrancar.
static mut HASH_RAIZ: [u8; 32] = [0; 32];
/// El hash del objeto que esta cronista graba en este arranque.
static mut HASH_NUEVO: [u8; 32] = [0; 32];
/// Hash de trabajo para recorrer la cadena del DAG.
static mut HASH_AUX: [u8; 32] = [0; 32];
/// Los ocho bytes del numero de arranque, de ida y de vuelta del grafo.
static mut DATOS_IO: [u8; 8] = [0; 8];

/// Preparacion: el kernel la invoca UNA sola vez. Aqui ocurre toda la cronica —
/// leer el grafo, grabar el eslabon nuevo, verificar el DAG y pintar.
#[no_mangle]
pub extern "C" fn init() {
    // 1. ¿Hay ya una raiz? Es la cabeza de la cadena de arranques anteriores.
    let tiene_raiz = unsafe { sys_object_raiz(core::ptr::addr_of_mut!(HASH_RAIZ) as u32) } == 1;

    // 2. El numero del ultimo arranque vive en los `datos` de esa raiz.
    let mut previo: u64 = 0;
    if tiene_raiz {
        let leidos = unsafe {
            sys_object_datos(
                core::ptr::addr_of!(HASH_RAIZ) as u32,
                core::ptr::addr_of_mut!(DATOS_IO) as u32,
                8,
            )
        };
        if leidos == 8 {
            // SEGURIDAD: lectura de un escalar `Copy` de un estatico propio.
            previo = u64::from_le_bytes(unsafe { *core::ptr::addr_of!(DATOS_IO) });
        }
    }
    let cuenta = previo + 1;

    // 3. Grabar el objeto de ESTE arranque. Sus `datos` son el numero de
    //    arranque; su unico hijo, la raiz anterior — el eslabon nuevo del DAG.
    // SEGURIDAD: escritura de un escalar `Copy` a un estatico propio.
    unsafe {
        *core::ptr::addr_of_mut!(DATOS_IO) = cuenta.to_le_bytes();
    }
    let (hijos_ptr, hijos_cnt) = if tiene_raiz {
        (core::ptr::addr_of!(HASH_RAIZ) as u32, 1u32)
    } else {
        (0u32, 0u32)
    };
    let grabado = unsafe {
        sys_object_put(
            core::ptr::addr_of!(DATOS_IO) as u32,
            8,
            hijos_ptr,
            hijos_cnt,
            core::ptr::addr_of_mut!(HASH_NUEVO) as u32,
        )
    };

    // 4. Coronar el objeto nuevo como raiz y verificar la integridad del DAG.
    let mut integro = false;
    if grabado == 0
        && unsafe { sys_object_fijar_raiz(core::ptr::addr_of!(HASH_NUEVO) as u32) } == 0
    {
        integro = verificar_cadena(cuenta);
    }

    // 5. Pintar la cronica: una celda por arranque, un testigo de integridad.
    pintar(cuenta, integro);
}

/// Un fotograma de trabajo. El numero de arranque no cambia durante una sesion:
/// la cronica que `init` pinto persiste en el lienzo del kernel. `tick` solo
/// cede el control, fiel al ABI cooperativo — no toda app necesita redibujar.
#[no_mangle]
pub extern "C" fn tick() {}

/// Recorre la cadena del DAG desde el objeto recien grabado, descendiendo por
/// el hijo 0, y comprueba que su profundidad coincide con el numero de
/// arranque. Si coincide, el grafo entero se leyo de vuelta del disco integro.
fn verificar_cadena(cuenta: u64) -> bool {
    // Partir del objeto recien grabado.
    // SEGURIDAD: copia de un arreglo `Copy` entre dos estaticos propios.
    unsafe {
        *core::ptr::addr_of_mut!(HASH_AUX) = *core::ptr::addr_of!(HASH_NUEVO);
    }
    let mut profundidad: u64 = 0;
    loop {
        profundidad += 1;
        // `sys_object_hijo` lee el hash de HASH_AUX y, si hay hijo, escribe el
        // del hijo 0 en el MISMO bufer: la cadena desciende un eslabon.
        let hijos = unsafe {
            sys_object_hijo(
                core::ptr::addr_of!(HASH_AUX) as u32,
                0,
                core::ptr::addr_of_mut!(HASH_AUX) as u32,
            )
        };
        // Sin hijos: fin de la cadena — el primer arranque de todos. Un valor
        // negativo seria un objeto que no resolvio: la cadena estaria rota.
        if hijos <= 0 || profundidad >= 4096 {
            break;
        }
    }
    profundidad == cuenta
}

/// Pinta la cronica: el fondo, una celda ambar por arranque y, en la esquina,
/// el testigo de integridad del grafo.
fn pintar(cuenta: u64, integro: bool) {
    // SEGURIDAD: durante `init` esta es la unica via de acceso a LIENZO, y el
    // kernel jamas reentra el modulo mientras `init` corre.
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    // Una celda ambar por cada arranque registrado, dispuestas en rejilla.
    let celdas = (cuenta as usize).min(MAX_CELDAS);
    for i in 0..celdas {
        let x = (i % POR_FILA) * PASO + 2;
        let y = (i / POR_FILA) * PASO + 2;
        rellenar(lienzo, x, y, LADO, LADO, CELDA);
    }

    // El testigo de integridad: verde si la cadena se recorrio entera de la
    // raiz al primer eslabon, rojo si algo se rompio.
    let testigo = if integro { VERDE } else { ROJO };
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
