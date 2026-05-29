// =============================================================================
//  renaser :: apps/testigo — Fase C4 :: el testigo del motor `tinkuy`
// -----------------------------------------------------------------------------
//  Esta app cierra el bucle de la capa 2 de tinkuy: el reactor `wasmi` del
//  kernel carga `assets/tinkuy.wasm` (motor de particulas DOD), expone las
//  capacidades `sys_tinkuy_*` a esta app, y la app las ejerce paso a paso.
//
//  AL ARRANQUE (`init`):
//    1. `sys_tinkuy_sim_new()` reserva una sim con dominio fijo [-50, +50]^3.
//    2. Para cada particula del cubo lattice 4×4×4 (= 64) llama
//       `sys_tinkuy_sim_spawn(slot, x, y, z, vx, vy, vz, masa, carga)` con
//       velocidades pequeñas pseudo-aleatorias (xorshift32 deterministic).
//    3. `sys_tinkuy_sim_rebuild_grid` cose la grilla espacial.
//
//  EN CADA `tick`:
//    1. `sys_tinkuy_sim_step_lj(slot, 4, dt, ε, σ, cutoff)` avanza 4 substeps.
//    2. `sys_tinkuy_sim_observables(slot, out_24)` lee step / KE / T.
//    3. `sys_tinkuy_sim_snapshot_cid(slot, out_32)` lee el CID BLAKE3.
//    4. Pinta tres lineas: "step    KE     T" + "CID 0123abcd…" + un mini
//       grafico tipo barra del KE para que el ojo vea el sistema termalizar.
//
//  El renderer es la 8x8 escalada x2 a 16x16, identica a `bitacora`/`rimay`.
// =============================================================================

#![no_std]

use font8x8::legacy::BASIC_LEGACY;

// --- Capacidades del kernel ---------------------------------------------------
#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_get_scancode() -> u32;
    fn sys_tinkuy_sim_new() -> i32;
    fn sys_tinkuy_sim_spawn(
        slot: u32,
        x: f32,
        y: f32,
        z: f32,
        vx: f32,
        vy: f32,
        vz: f32,
        masa: f32,
        carga: f32,
    ) -> i32;
    fn sys_tinkuy_sim_rebuild_grid(slot: u32) -> i32;
    fn sys_tinkuy_sim_step_lj(
        slot: u32,
        n_steps: u32,
        dt: f32,
        eps: f32,
        sigma: f32,
        cutoff: f32,
    ) -> i32;
    fn sys_tinkuy_sim_observables(slot: u32, out_24_ptr: u32) -> i32;
    fn sys_tinkuy_sim_snapshot_cid(slot: u32, out_32_ptr: u32) -> i32;
    fn sys_tinkuy_sim_positions(slot: u32, out_ptr: u32, cap_count: u32) -> i32;
    #[allow(dead_code)]
    fn sys_tinkuy_sim_free(slot: u32) -> i32;
    #[allow(dead_code)]
    fn sys_tinkuy_sim_len(slot: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria del lienzo -----------------------------------------------------
const ANCHO: usize = 480;
const ALTO: usize = 240;
const PASO: usize = 16;
const GLIFO: usize = 8;
const ESCALA: usize = 2;
const MARGEN_X: usize = 12;

// Paleta — palos sobre fondo grafito, indigo de cabeza y cyan calido para
// los numeros (igual que `pulso`/`rimay`).
const FONDO: u32 = 0x10_18_28;
const TINTA: u32 = 0xD8_DC_E4;
const ETIQUETA: u32 = 0x6A_78_92;
const HIGHLIGHT: u32 = 0x8B_5C_F6;
const BARRA: u32 = 0x2E_50_C8;
const ERROR: u32 = 0xC8_50_50;
const OK: u32 = 0x60_C8_82;

static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- Estado de la sim --------------------------------------------------------
const N_LADO: u32 = 4;
// N_PARTICULAS = N_LADO^3 = 64; queda implicito en los tres bucles del init.
const DT: f32 = 0.005;
const EPS: f32 = 1.0;
const SIGMA: f32 = 1.0;
const CUTOFF: f32 = 2.5;
const SUBSTEPS_POR_TICK: u32 = 4;

/// Estado interno de testigo: el slot que el kernel le entrego en `init` y
/// el ultimo paquete de observables / CID + posiciones para el visor 3D. Si
/// la sim no se pudo crear (PERMISO_TINKUY ausente, motor no instalado),
/// `slot < 0` y la app pinta un cartel de error en lugar de los numeros.
const CAP_PARTICULAS: usize = 64;
struct Estado {
    slot: i32,
    step: u64,
    ke: f64,
    temp: f64,
    cid: [u8; 32],
    ok: bool,
    /// Codigo del ultimo `sys_tinkuy_*` no-cero; lo pinta el cartel rojo
    /// cuando el motor del kernel rechaza una llamada.
    ultimo_codigo: i32,
    /// AoS de hasta CAP_PARTICULAS particulas: 3 f32 por particula.
    /// La syscall `sys_tinkuy_sim_positions` lo reescribe cada `tick`.
    posiciones: [f32; CAP_PARTICULAS * 3],
    /// Cantidad real de particulas en `posiciones` (0..=CAP_PARTICULAS).
    n_particulas: u32,
}

static mut ESTADO: Estado = Estado {
    slot: -1,
    step: 0,
    ke: 0.0,
    temp: 0.0,
    cid: [0u8; 32],
    ok: false,
    ultimo_codigo: 0,
    posiciones: [0.0; CAP_PARTICULAS * 3],
    n_particulas: 0,
};

#[no_mangle]
pub extern "C" fn init() {
    let estado = estado_mut();
    // 1. Reservar la sim.
    let slot = unsafe { sys_tinkuy_sim_new() };
    if slot < 0 {
        estado.ok = false;
        estado.ultimo_codigo = slot;
        pintar(estado);
        return;
    }
    estado.slot = slot;

    // 2. Sembrar 4×4×4 particulas en un cubo lattice centrado en el
    //    origen del dominio. Espaciado 1.5σ — ni colapsadas ni dispersas.
    let espacio = 1.5f32;
    let origen_grid = -(N_LADO as f32 - 1.0) * 0.5 * espacio;
    let mut rng = XorShift32 { state: 0xC0FFEEu32 };
    for kz in 0..N_LADO {
        for ky in 0..N_LADO {
            for kx in 0..N_LADO {
                let x = origen_grid + kx as f32 * espacio;
                let y = origen_grid + ky as f32 * espacio;
                let z = origen_grid + kz as f32 * espacio;
                // Velocidades pequeñas en [-0.1, 0.1] para semillar T > 0.
                let vx = (rng.next_unit() - 0.5) * 0.2;
                let vy = (rng.next_unit() - 0.5) * 0.2;
                let vz = (rng.next_unit() - 0.5) * 0.2;
                let rc = unsafe {
                    sys_tinkuy_sim_spawn(slot as u32, x, y, z, vx, vy, vz, 1.0, 0.0)
                };
                if rc != 0 {
                    estado.ok = false;
                    estado.ultimo_codigo = rc;
                    pintar(estado);
                    return;
                }
            }
        }
    }

    // 3. Cocer la grilla espacial.
    let rc = unsafe { sys_tinkuy_sim_rebuild_grid(slot as u32) };
    if rc != 0 {
        estado.ok = false;
        estado.ultimo_codigo = rc;
        pintar(estado);
        return;
    }

    estado.ok = true;
    pintar(estado);
}

#[no_mangle]
pub extern "C" fn tick() {
    let estado = estado_mut();
    if estado.slot < 0 {
        pintar(estado);
        return;
    }

    // Consumir scancodes pendientes (no usamos ninguno todavia, pero
    // mantener la cola drenada evita acumulacion).
    let _ = unsafe { sys_get_scancode() };

    let slot = estado.slot as u32;

    // 1. Avanzar SUBSTEPS_POR_TICK substeps de Velocity-Verlet + LJ.
    let rc = unsafe {
        sys_tinkuy_sim_step_lj(slot, SUBSTEPS_POR_TICK, DT, EPS, SIGMA, CUTOFF)
    };
    if rc != 0 {
        estado.ultimo_codigo = rc;
        estado.ok = false;
        pintar(estado);
        return;
    }

    // 2. Leer observables (step + KE + T) en un buffer plano de 24 B.
    let mut obs = [0u8; 24];
    let rc = unsafe { sys_tinkuy_sim_observables(slot, obs.as_mut_ptr() as u32) };
    if rc == 0 {
        let mut buf8 = [0u8; 8];
        buf8.copy_from_slice(&obs[0..8]);
        estado.step = u64::from_le_bytes(buf8);
        buf8.copy_from_slice(&obs[8..16]);
        estado.ke = f64::from_le_bytes(buf8);
        buf8.copy_from_slice(&obs[16..24]);
        estado.temp = f64::from_le_bytes(buf8);
    } else {
        estado.ultimo_codigo = rc;
    }

    // 3. CID BLAKE3 del estado en `cid` (32 B).
    let mut cid = [0u8; 32];
    let rc = unsafe { sys_tinkuy_sim_snapshot_cid(slot, cid.as_mut_ptr() as u32) };
    if rc == 0 {
        estado.cid = cid;
    } else {
        estado.ultimo_codigo = rc;
    }

    // 4. Posiciones para el visor 3D — AoS f32[N*3].
    let rc = unsafe {
        sys_tinkuy_sim_positions(
            slot,
            estado.posiciones.as_mut_ptr() as u32,
            CAP_PARTICULAS as u32,
        )
    };
    if rc >= 0 {
        estado.n_particulas = rc as u32;
    } else {
        estado.ultimo_codigo = rc;
        estado.n_particulas = 0;
    }

    estado.ok = true;
    pintar(estado);
}

// ─── Render ────────────────────────────────────────────────────────────────

// --- Visor 3D :: geometria del viewport y del dominio ----------------------
const VISOR_X0: usize = 240;
const VISOR_Y0: usize = 32;
const VISOR_ANCHO: usize = 228;
const VISOR_ALTO: usize = 184;
const DOMINIO_MIN: f32 = -50.0;
const DOMINIO_MAX: f32 = 50.0;
/// Color frio (z chico) -> color caliente (z grande). Lerp lineal sobre
/// los canales R/G/B en bytes (la app no usa premultiplicado).
const FRIO_R: i32 = 0x40;
const FRIO_G: i32 = 0x80;
const FRIO_B: i32 = 0xC8;
const CALIENTE_R: i32 = 0xC8;
const CALIENTE_G: i32 = 0x60;
const CALIENTE_B: i32 = 0x40;

fn pintar(estado: &Estado) {
    let lienzo: &mut [u32] = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for pixel in lienzo.iter_mut() {
        *pixel = FONDO;
    }

    // Linea 0: cabecera.
    texto(lienzo, MARGEN_X, 8, "testigo :: motor tinkuy embebido", HIGHLIGHT);

    if estado.slot < 0 || !estado.ok {
        texto(lienzo, MARGEN_X, 48, "ERROR :: el motor tinkuy rechazo una llamada", ERROR);
        let mut buf = [0u8; 32];
        let txt = render_codigo(estado.ultimo_codigo, &mut buf);
        texto(lienzo, MARGEN_X, 76, "codigo:", ETIQUETA);
        texto(lienzo, MARGEN_X + 7 * GLIFO * ESCALA + 8, 76, txt, ERROR);
        texto(
            lienzo,
            MARGEN_X,
            120,
            "comprueba PERMISO_TINKUY en el manifiesto",
            ETIQUETA,
        );
        volcar(lienzo);
        return;
    }

    // --- Panel izquierdo :: lectura de observables -------------------------
    // Una columna estrecha (12..220) con step/T/KE como etiqueta+valor en
    // tres lineas, mas el CID (primeros 4 B en hex) y la mini-barra de KE.
    texto(lienzo, MARGEN_X, 36, "step", ETIQUETA);
    let mut buf = [0u8; 32];
    let txt_step = render_u64(estado.step, &mut buf);
    texto(lienzo, MARGEN_X + 72, 36, txt_step, TINTA);

    texto(lienzo, MARGEN_X, 56, "T", ETIQUETA);
    let mut buf_t = [0u8; 32];
    let txt_t = render_f64_fixed(estado.temp, 4, &mut buf_t);
    texto(lienzo, MARGEN_X + 72, 56, txt_t, TINTA);

    texto(lienzo, MARGEN_X, 76, "KE", ETIQUETA);
    let mut buf_ke = [0u8; 32];
    let txt_ke = render_f64_fixed(estado.ke, 3, &mut buf_ke);
    texto(lienzo, MARGEN_X + 72, 76, txt_ke, TINTA);

    // CID — 4 bytes (8 nibbles) caben en el panel izquierdo (128 px).
    texto(lienzo, MARGEN_X, 96, "CID", ETIQUETA);
    let mut hex = [0u8; 8];
    for i in 0..4 {
        let b = estado.cid[i];
        hex[i * 2] = nibble_a_hex(b >> 4);
        hex[i * 2 + 1] = nibble_a_hex(b & 0x0F);
    }
    texto_bytes(lienzo, MARGEN_X + 60, 96, &hex, TINTA);

    // Mini-barra de KE: el ancho es proporcional a la energia cinetica,
    // saturada a un techo razonable. Ayuda al ojo a ver la termalizacion.
    let techo_ke: f64 = 30.0;
    let frac = (estado.ke / techo_ke).clamp(0.0, 1.0);
    let ancho_max_barra = 200usize;
    let ancho_barra = (ancho_max_barra as f64 * frac) as usize;
    banda(lienzo, MARGEN_X, MARGEN_X + ancho_barra, 124, 138, BARRA);
    texto(lienzo, MARGEN_X, 148, "ke 0..30", ETIQUETA);

    // Linea de status.
    texto(lienzo, MARGEN_X, 220, "OK", OK);
    texto(lienzo, MARGEN_X + 40, 220, "LJ N=64", ETIQUETA);

    // --- Panel derecho :: visor 3D axonometrico ---------------------------
    pintar_visor(lienzo, estado);

    volcar(lienzo);
}

/// Proyeccion axonometrica fija (mismo helper que `tinkuy-llimphi::visor`):
/// `(x + 0.6·z, y + 0.4·z)`. Sin camara orbital — la sim cabe holgada y la
/// vista 3/4 da sensacion espacial inmediata.
#[inline]
fn proyectar(p: [f32; 3]) -> (f32, f32) {
    (p[0] + 0.6 * p[2], p[1] + 0.4 * p[2])
}

/// Convierte una coord proyectada del DOMINIO al pixel del viewport. La
/// escala se elige una sola vez con el dominio del kernel — caja
/// `[-50, +50]^3` proyectada: x' ∈ [-80, +80], y' ∈ [-70, +70].
#[inline]
fn ndc_a_pixel(xp: f32, yp: f32) -> (i32, i32) {
    const X_MIN: f32 = DOMINIO_MIN + 0.6 * DOMINIO_MIN;
    const X_MAX: f32 = DOMINIO_MAX + 0.6 * DOMINIO_MAX;
    const Y_MIN: f32 = DOMINIO_MIN + 0.4 * DOMINIO_MIN;
    const Y_MAX: f32 = DOMINIO_MAX + 0.4 * DOMINIO_MAX;
    let u = (xp - X_MIN) / (X_MAX - X_MIN);
    let v = (yp - Y_MIN) / (Y_MAX - Y_MIN);
    let px = VISOR_X0 as i32 + (u * VISOR_ANCHO as f32) as i32;
    let py = VISOR_Y0 as i32 + (v * VISOR_ALTO as f32) as i32;
    (px, py)
}

fn pintar_visor(lienzo: &mut [u32], estado: &Estado) {
    // Fondo del viewport: una sombra discreta.
    banda(
        lienzo,
        VISOR_X0,
        VISOR_X0 + VISOR_ANCHO,
        VISOR_Y0,
        VISOR_Y0 + VISOR_ALTO,
        0x18_22_38,
    );
    // Borde fino.
    contorno(
        lienzo,
        VISOR_X0,
        VISOR_Y0,
        VISOR_X0 + VISOR_ANCHO,
        VISOR_Y0 + VISOR_ALTO,
        ETIQUETA,
    );

    // Wireframe del cubo `[-50, +50]^3` — 12 aristas.
    pintar_caja(lienzo);

    if estado.n_particulas == 0 {
        return;
    }
    let n = (estado.n_particulas as usize).min(CAP_PARTICULAS);

    // Ordenar por depth_key = z + 0.3·x (back-to-front), para que las
    // particulas mas "cerca" del ojo pisen a las mas lejanas. Insertion-sort
    // por indice — N=64 → trivialmente rapido y cero alloc.
    let mut orden = [0u8; CAP_PARTICULAS];
    for i in 0..n {
        orden[i] = i as u8;
    }
    let mut i = 1;
    while i < n {
        let mut j = i;
        while j > 0 {
            let a = orden[j - 1] as usize;
            let b = orden[j] as usize;
            let ka = depth_key(estado, a);
            let kb = depth_key(estado, b);
            if ka <= kb {
                break;
            }
            orden.swap(j - 1, j);
            j -= 1;
        }
        i += 1;
    }

    // Pintar cada particula como disco de 3 px coloreado por z.
    for k in 0..n {
        let idx = orden[k] as usize;
        let p = [
            estado.posiciones[idx * 3],
            estado.posiciones[idx * 3 + 1],
            estado.posiciones[idx * 3 + 2],
        ];
        let (xp, yp) = proyectar(p);
        let (px, py) = ndc_a_pixel(xp, yp);
        let z_norm = ((p[2] - DOMINIO_MIN) / (DOMINIO_MAX - DOMINIO_MIN))
            .clamp(0.0, 1.0);
        let color = lerp_color(z_norm);
        disco(lienzo, px, py, 3, color);
    }
}

#[inline]
fn depth_key(estado: &Estado, i: usize) -> f32 {
    let x = estado.posiciones[i * 3];
    let z = estado.posiciones[i * 3 + 2];
    z + 0.3 * x
}

fn lerp_color(t: f32) -> u32 {
    let r = FRIO_R + ((CALIENTE_R - FRIO_R) as f32 * t) as i32;
    let g = FRIO_G + ((CALIENTE_G - FRIO_G) as f32 * t) as i32;
    let b = FRIO_B + ((CALIENTE_B - FRIO_B) as f32 * t) as i32;
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn pintar_caja(lienzo: &mut [u32]) {
    // 8 vertices del cubo `[DOMINIO_MIN, DOMINIO_MAX]^3`, proyectados.
    let mut puntos = [(0i32, 0i32); 8];
    let v: [[f32; 3]; 8] = [
        [DOMINIO_MIN, DOMINIO_MIN, DOMINIO_MIN],
        [DOMINIO_MAX, DOMINIO_MIN, DOMINIO_MIN],
        [DOMINIO_MAX, DOMINIO_MAX, DOMINIO_MIN],
        [DOMINIO_MIN, DOMINIO_MAX, DOMINIO_MIN],
        [DOMINIO_MIN, DOMINIO_MIN, DOMINIO_MAX],
        [DOMINIO_MAX, DOMINIO_MIN, DOMINIO_MAX],
        [DOMINIO_MAX, DOMINIO_MAX, DOMINIO_MAX],
        [DOMINIO_MIN, DOMINIO_MAX, DOMINIO_MAX],
    ];
    for (i, p) in v.iter().enumerate() {
        let (xp, yp) = proyectar(*p);
        puntos[i] = ndc_a_pixel(xp, yp);
    }
    // 12 aristas — mismas que `tinkuy-llimphi::visor::BOX_EDGES`.
    const ARISTAS: [(usize, usize); 12] = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];
    let color = 0x2A_3A_56;
    for (a, b) in ARISTAS {
        linea(lienzo, puntos[a].0, puntos[a].1, puntos[b].0, puntos[b].1, color);
    }
}

// --- Rasterizado helpers ---------------------------------------------------

fn pintar_pixel(lienzo: &mut [u32], x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 {
        return;
    }
    let xu = x as usize;
    let yu = y as usize;
    if xu >= ANCHO || yu >= ALTO {
        return;
    }
    lienzo[yu * ANCHO + xu] = color;
}

fn disco(lienzo: &mut [u32], cx: i32, cy: i32, r: i32, color: u32) {
    let r2 = r * r;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r2 {
                pintar_pixel(lienzo, cx + dx, cy + dy, color);
            }
        }
    }
}

/// Bresenham clasico. Cero alloc, todos los casos de pendiente.
fn linea(lienzo: &mut [u32], x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        pintar_pixel(lienzo, x, y, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn contorno(lienzo: &mut [u32], x0: usize, y0: usize, x1: usize, y1: usize, color: u32) {
    let xi0 = x0 as i32;
    let yi0 = y0 as i32;
    let xi1 = x1 as i32 - 1;
    let yi1 = y1 as i32 - 1;
    linea(lienzo, xi0, yi0, xi1, yi0, color);
    linea(lienzo, xi1, yi0, xi1, yi1, color);
    linea(lienzo, xi1, yi1, xi0, yi1, color);
    linea(lienzo, xi0, yi1, xi0, yi0, color);
}

fn volcar(lienzo: &[u32]) {
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

// ─── Glifos 8×8 escalados ×2 ────────────────────────────────────────────────

fn texto(lienzo: &mut [u32], x0: usize, y0: usize, s: &str, color: u32) {
    texto_bytes(lienzo, x0, y0, s.as_bytes(), color);
}

fn texto_bytes(lienzo: &mut [u32], mut x0: usize, y0: usize, bytes: &[u8], color: u32) {
    for &b in bytes {
        if b == b' ' {
            x0 += GLIFO * ESCALA;
            continue;
        }
        let idx = b as usize;
        if idx >= BASIC_LEGACY.len() {
            x0 += GLIFO * ESCALA;
            continue;
        }
        let glifo = &BASIC_LEGACY[idx];
        for (fila_g, &linea) in glifo.iter().enumerate() {
            for col_g in 0..GLIFO {
                if linea & (1 << col_g) != 0 {
                    let px = x0 + col_g * ESCALA;
                    let py = y0 + fila_g * ESCALA;
                    for dy in 0..ESCALA {
                        for dx in 0..ESCALA {
                            let xp = px + dx;
                            let yp = py + dy;
                            if xp < ANCHO && yp < ALTO {
                                lienzo[yp * ANCHO + xp] = color;
                            }
                        }
                    }
                }
            }
        }
        x0 += GLIFO * ESCALA;
    }
    let _ = PASO; // silencia unused
}

fn banda(lienzo: &mut [u32], x0: usize, x1: usize, y0: usize, y1: usize, color: u32) {
    let x0 = x0.min(ANCHO);
    let x1 = x1.min(ANCHO);
    let y0 = y0.min(ALTO);
    let y1 = y1.min(ALTO);
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

// ─── Render numerico (no_std, sin format!) ─────────────────────────────────

fn render_u64<'a>(mut n: u64, buf: &'a mut [u8; 32]) -> &'a str {
    if n == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[0..1]).unwrap_or("0");
    }
    let mut tmp = [0u8; 32];
    let mut i = 0;
    while n > 0 {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    core::str::from_utf8(&buf[0..i]).unwrap_or("?")
}

fn render_codigo<'a>(c: i32, buf: &'a mut [u8; 32]) -> &'a str {
    if c >= 0 {
        return render_u64(c as u64, buf);
    }
    buf[0] = b'-';
    let mut tail = [0u8; 32];
    let s = render_u64((-(c as i64)) as u64, &mut tail);
    let s_bytes = s.as_bytes();
    let mut i = 1;
    for &b in s_bytes {
        if i >= buf.len() {
            break;
        }
        buf[i] = b;
        i += 1;
    }
    core::str::from_utf8(&buf[0..i]).unwrap_or("?")
}

/// Render f64 con un numero fijo de decimales. Tolera NaN/Inf -> "?".
fn render_f64_fixed<'a>(x: f64, decimales: u32, buf: &'a mut [u8; 32]) -> &'a str {
    if !x.is_finite() {
        buf[0] = b'?';
        return core::str::from_utf8(&buf[0..1]).unwrap_or("?");
    }
    let neg = x < 0.0;
    let mut x = if neg { -x } else { x };
    // Trunca/redondea al numero de decimales.
    let mut escala: f64 = 1.0;
    for _ in 0..decimales {
        escala *= 10.0;
    }
    // `floor` no esta en `core` para f64; pero `x` es no-negativo aqui (ya
    // tomamos abs arriba), asi que truncar por cast a u64 equivale a floor.
    let cuantos = (x * escala + 0.5) as u64;
    x = cuantos as f64 / escala;
    let parte_entera = x as u64;
    let mut buf_ent = [0u8; 32];
    let ent_txt = render_u64(parte_entera, &mut buf_ent);
    let ent_bytes = ent_txt.as_bytes();
    let mut i = 0;
    if neg && i < buf.len() {
        buf[i] = b'-';
        i += 1;
    }
    for &b in ent_bytes {
        if i >= buf.len() {
            break;
        }
        buf[i] = b;
        i += 1;
    }
    if decimales > 0 && i < buf.len() {
        buf[i] = b'.';
        i += 1;
        let frac = ((x - parte_entera as f64) * escala + 0.5) as u64;
        // Padding con ceros a la izquierda hasta `decimales`.
        let mut tmp = [0u8; 16];
        let mut fi = 0usize;
        let mut f = frac;
        while f > 0 && fi < tmp.len() {
            tmp[fi] = b'0' + (f % 10) as u8;
            f /= 10;
            fi += 1;
        }
        // Padding.
        while fi < decimales as usize && fi < tmp.len() {
            tmp[fi] = b'0';
            fi += 1;
        }
        // Volcar invertido.
        for j in 0..fi {
            if i >= buf.len() {
                break;
            }
            buf[i] = tmp[fi - 1 - j];
            i += 1;
        }
    }
    core::str::from_utf8(&buf[0..i]).unwrap_or("?")
}

fn nibble_a_hex(n: u8) -> u8 {
    match n {
        0..=9 => b'0' + n,
        _ => b'a' + (n - 10),
    }
}

// ─── PRNG sin std ──────────────────────────────────────────────────────────

struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    fn next(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }
    fn next_unit(&mut self) -> f32 {
        (self.next() & 0x00FF_FFFF) as f32 / 16_777_216.0
    }
}

// ─── Helper ────────────────────────────────────────────────────────────────

fn estado_mut() -> &'static mut Estado {
    unsafe { &mut *core::ptr::addr_of_mut!(ESTADO) }
}
