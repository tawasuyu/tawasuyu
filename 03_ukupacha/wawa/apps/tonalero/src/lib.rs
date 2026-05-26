// =============================================================================
//  renaser :: apps/tonalero — Fase 22 :: la Configuracion como nodo del grafo
// -----------------------------------------------------------------------------
//  El testigo visual del bucle de Configuracion. Pinta cinco bandas
//  horizontales con los cinco colores de la paleta activa: primario,
//  secundario, fondo, texto y acento. Tambien rotula un caracter ASCII bajo
//  la primera banda con el codigo de idioma para que se vea cambiar al
//  vuelo.
//
//  La app jamas pregunta al kernel "cual es la paleta". No hay sondeo, no
//  hay bloqueo: en cada `tick`, el kernel ya copio idioma y paleta dentro
//  del ContextoCapacidades antes de cederle el control; la app las lee con
//  dos capacidades PASIVAS (`sys_config_idioma` y `sys_config_paleta`) que
//  son, fisicamente, leer veintidos bytes del contexto. Inyeccion
//  unidireccional, frame-lock perfecto.
//
//  La barra espaciadora propone una paleta nueva: la app rota los cinco
//  colores y llama a `sys_config_proponer`. El kernel engendra un nodo
//  nuevo del grafo, reancla el manifiesto al hash recien creado, y el
//  proximo `tick` —de esta app y de TODAS las demas— pinta con la paleta
//  nueva. Sin estados mutables globales: el "ahora" es el hash al que
//  apunta el manifiesto vivo.
// =============================================================================

#![no_std]

// --- Las capacidades que el kernel inyecta. Nada mas existe para esta app. ---
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_config_idioma() -> u32;
    fn sys_config_paleta(salida: u32) -> i32;
    fn sys_config_proponer(idioma: u32, paleta_ptr: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria. DEBE encajar con la region que `boot` asigna a esta app. ---
const ANCHO: usize = 480;
const ALTO: usize = 300;
const BANDA: usize = ALTO / 5; // 60 px por banda

/// El lienzo, en la propia memoria lineal. El kernel jamas lo ve; solo recibe
/// el (ptr, len) que vuelca `sys_render_frame`. Cuatro bytes por pixel; el
/// kernel los decodifica como BGRA.
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

/// La paleta que el kernel inyecta. 5 colores RGBA8, en orden:
/// primario, secundario, fondo, texto, acento. La rellena cada `tick`
/// desde el contexto, asi que un cambio remoto la refleja al siguiente
/// fotograma.
static mut PALETA: [u8; 20] = [0; 20];

/// El codigo de idioma activo (ISO 639-1 empaquetado en LE). Tambien se
/// refresca cada `tick` desde el contexto.
static mut IDIOMA: u16 = 0;

/// Anti-rebote del SPACE. La capacidad de teclado entrega el scancode crudo;
/// si el usuario mantiene la barra, llegaria un torrente. Aceptamos una sola
/// propuesta cada vez que se libera la tecla.
static mut SPACE_PRESS: bool = false;

#[no_mangle]
pub extern "C" fn init() {
    // `init` solo refresca contexto y vuelca el primer fotograma. El estado
    // visual entero se reconstruye en cada `tick` a partir del contexto:
    // jamas hay divergencia entre "lo que la app cree" y "lo que el kernel
    // sabe".
    refrescar_contexto();
    pintar();
    volcar();
}

#[no_mangle]
pub extern "C" fn tick() {
    refrescar_contexto();

    // Anti-rebote: contar como pulsacion solo el flanco de subida.
    let scancode = unsafe { sys_get_scancode() };
    let space_ahora = scancode == 0x39; // SPACE en scancode set 1
    if space_ahora && !unsafe { SPACE_PRESS } {
        proponer_rotacion();
    }
    unsafe { SPACE_PRESS = space_ahora };

    pintar();
    volcar();
}

/// Lee idioma y paleta del contexto de capacidades. Dos llamadas pasivas: no
/// hay E/S, no hay bloqueo, no hay sondeo del grafo. El kernel ya las dejo
/// ahi al iniciar este `tick`.
fn refrescar_contexto() {
    let idioma = unsafe { sys_config_idioma() } as u16;
    unsafe { IDIOMA = idioma };
    // SEGURIDAD: PALETA tiene 20 bytes, el kernel valida los limites del
    // puntero contra esta memoria lineal antes de copiar.
    let _ = unsafe { sys_config_paleta(core::ptr::addr_of_mut!(PALETA) as u32) };
}

/// Propone una rotacion de la paleta: los cinco colores se desplazan un
/// puesto. Si la propuesta llega (foco), el siguiente `tick` ya pinta con la
/// paleta nueva — el `refrescar_contexto` la encontrara en el contexto.
fn proponer_rotacion() {
    let actual = unsafe { PALETA };
    let mut rotada = [0u8; 20];
    rotada[0..16].copy_from_slice(&actual[4..20]);
    rotada[16..20].copy_from_slice(&actual[0..4]);
    let idioma = unsafe { IDIOMA } as u32;
    // SEGURIDAD: pasamos un puntero a un local valido durante toda la llamada;
    // el kernel solo lee 20 bytes y valida limites por su cuenta.
    let _ = unsafe { sys_config_proponer(idioma, rotada.as_ptr() as u32) };
}

/// Pinta las cinco bandas en el lienzo. Cada banda usa el color RGBA8
/// correspondiente de la paleta. El kernel decodifica el lienzo como BGRA
/// (lee byte 0 como B, 1 como G, 2 como R), asi que componemos el u32 con
/// los bytes en ese orden.
fn pintar() {
    let paleta = unsafe { PALETA };
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for banda in 0..5 {
        let r = paleta[banda * 4];
        let g = paleta[banda * 4 + 1];
        let b = paleta[banda * 4 + 2];
        // BGRA en memoria => u32 LE con B en el byte bajo.
        let color = (b as u32) | ((g as u32) << 8) | ((r as u32) << 16);
        let y_inicio = banda * BANDA;
        let y_fin = if banda == 4 { ALTO } else { y_inicio + BANDA };
        for fila in y_inicio..y_fin {
            let base = fila * ANCHO;
            for col in 0..ANCHO {
                lienzo[base + col] = color;
            }
        }
    }

    // Rotular las dos letras del idioma (ISO 639-1) en la esquina superior
    // izquierda. Cada letra ocupa una matriz 5x7 que se dibuja con bloques
    // de 4x4 pixeles: ocho colores por pixel, sin tipografias. Es fea pero
    // legible — MVP feo primero.
    let idioma = unsafe { IDIOMA };
    let letra_a = (idioma & 0xFF) as u8;
    let letra_b = ((idioma >> 8) & 0xFF) as u8;
    // El color del texto (cuarto color de la paleta).
    let tr = paleta[12];
    let tg = paleta[13];
    let tb = paleta[14];
    let tinta = (tb as u32) | ((tg as u32) << 8) | ((tr as u32) << 16);
    rotular_letra(lienzo, 16, 10, letra_a, tinta);
    rotular_letra(lienzo, 16 + 6 * 5, 10, letra_b, tinta);
}

fn volcar() {
    let lienzo: &[u32] = unsafe { &*core::ptr::addr_of!(LIENZO) };
    unsafe { sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32) };
}

/// Dibuja una letra ASCII mayuscula en (x, y) con bloques de 5 px de lado.
/// La matriz es 5x7 columnas/filas; lo justo para leer "ES", "EN" o "QU".
/// Letras no reconocidas se omiten — la rotulacion sirve al testigo visual,
/// no a la lectura general.
fn rotular_letra(lienzo: &mut [u32], x: usize, y: usize, c: u8, tinta: u32) {
    let glifo: [u8; 7] = match c {
        b'a'..=b'z' => glifo_letra(c - b'a' + b'A'),
        b'A'..=b'Z' => glifo_letra(c),
        _ => return,
    };
    for (fila, bits) in glifo.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                let px0 = x + col * 5;
                let py0 = y + fila * 5;
                for dy in 0..5 {
                    for dx in 0..5 {
                        let px = px0 + dx;
                        let py = py0 + dy;
                        if px < ANCHO && py < ALTO {
                            lienzo[py * ANCHO + px] = tinta;
                        }
                    }
                }
            }
        }
    }
}

/// La tipografia 5x7 de los caracteres ASCII A-Z, codificada como siete
/// filas; cada fila usa los cinco bits bajos. Una matriz suficiente para
/// los codigos ISO 639-1 mas comunes — solo las letras que aparecen en
/// `es`, `en` y `qu` se rellenan; el resto cae a la `E` por defecto, que
/// rara vez se vera en la practica.
fn glifo_letra(c: u8) -> [u8; 7] {
    match c {
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        _ => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
    }
}
