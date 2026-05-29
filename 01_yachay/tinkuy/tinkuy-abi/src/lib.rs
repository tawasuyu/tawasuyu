//! `tinkuy-abi` — superficie C-friendly del motor.
//!
//! Diseño:
//!   - Una opaca `TkSim` agrupa `World`, `Grid3D`, params del integrador y los
//!     outboxes worker-local. El caller solo maneja `*mut TkSim` (token de
//!     `Box::into_raw`). Esto reduce la superficie ABI a un puñado de funciones
//!     y deja al motor la libertad de mover su estructura interna sin romper
//!     hosts (Wawa, JS, futuros).
//!   - Códigos de retorno `i32` (`TK_OK` = 0, negativos = error). Nada de
//!     panics atravesando la frontera FFI: cada validación retorna error.
//!   - Buffers (snapshot serializado) se entregan como `(*mut u8, usize)`
//!     vivos hasta `tk_buf_free`. El allocador es el del crate (Rust `Vec`),
//!     así que el caller NO debe `free` con allocator del host.
//!   - Misma superficie funciona bajo `cpu` (rayon) o `wasm` (single-thread);
//!     el backend se elige en compile-time por feature del crate raíz.
//!
//! No-mangle global: todos los símbolos van con prefijo `tk_` para evitar
//! colisiones con cualquier otro crate vecino en el cdylib.

#![allow(clippy::missing_safety_doc)]

use core::ptr::{self, NonNull};

use tinkuy_core::{
    kinetic_energy as core_ke, reflect_walls, temperature as core_temp,
    total_momentum as core_p, velocity_verlet_step, Grid3D, IntegratorParams,
    Outbox, Snapshot, World,
};
use tinkuy_forces::{clear_accelerations, lennard_jones, LjParams};

// ─── Códigos de error ───────────────────────────────────────────────────────

pub const TK_OK:           i32 = 0;
pub const TK_ERR_NULL:     i32 = -1;
pub const TK_ERR_INVALID:  i32 = -2;
pub const TK_ERR_OOM:      i32 = -3;

// ─── Handle opaco ───────────────────────────────────────────────────────────

/// Estado completo de una simulación. Opaco para el caller FFI: nunca debe
/// derreferenciar a `*const TkSim` directamente; usar las funciones `tk_sim_*`.
#[repr(C)]
pub struct TkSim {
    world: World,
    grid: Grid3D,
    outboxes: Vec<Outbox>,
}

// ─── Helpers internos ───────────────────────────────────────────────────────

/// Convierte un `*mut TkSim` no nulo en `&mut TkSim` con bounds-check de null.
/// SAFETY: el puntero debe haber sido producido por `tk_sim_new` y no
/// haberse liberado todavía.
#[inline]
unsafe fn sim_mut<'a>(p: *mut TkSim) -> Option<&'a mut TkSim> {
    if p.is_null() { None } else { unsafe { Some(&mut *p) } }
}

#[inline]
unsafe fn sim_ref<'a>(p: *const TkSim) -> Option<&'a TkSim> {
    if p.is_null() { None } else { unsafe { Some(&*p) } }
}

// ─── Constructores / destructores ───────────────────────────────────────────

/// Crea una `TkSim` vacía. El caller debe llamar a `tk_sim_free` para liberarla.
///
/// `origin` y `dims_xyz` son punteros a 3 floats / 3 u32. `cap` reserva
/// capacidad inicial para partículas (puede crecer dinámicamente luego).
///
/// Errores:
///   - `TK_ERR_NULL` si `out`, `origin` o `dims_xyz` son null.
///   - `TK_ERR_INVALID` si `cell_size <= 0` o alguna dim es 0.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_new(
    cap: u32,
    origin: *const f32,
    cell_size: f32,
    dims_xyz: *const u32,
    out: *mut *mut TkSim,
) -> i32 {
    if out.is_null() || origin.is_null() || dims_xyz.is_null() {
        return TK_ERR_NULL;
    }
    if !(cell_size > 0.0) {
        return TK_ERR_INVALID;
    }
    let origin = unsafe { [*origin.add(0), *origin.add(1), *origin.add(2)] };
    let dims = unsafe { [*dims_xyz.add(0), *dims_xyz.add(1), *dims_xyz.add(2)] };
    if dims[0] == 0 || dims[1] == 0 || dims[2] == 0 {
        return TK_ERR_INVALID;
    }

    let cap = cap as usize;
    let world = World::with_capacity(cap);
    let grid = Grid3D::new(origin, cell_size, dims, cap);
    // Un outbox por hilo bajo `cpu`; uno solo bajo `wasm` (single-thread).
    let n_workers = workers_default();
    let outboxes = (0..n_workers).map(|_| Outbox::default()).collect();

    let boxed = Box::new(TkSim { world, grid, outboxes });
    unsafe { *out = Box::into_raw(boxed); }
    TK_OK
}

/// Libera la simulación. Tolera puntero null (no-op).
#[no_mangle]
pub unsafe extern "C" fn tk_sim_free(sim: *mut TkSim) {
    if sim.is_null() { return; }
    drop(unsafe { Box::from_raw(sim) });
}

// ─── Spawning / consulta ────────────────────────────────────────────────────

/// Añade una partícula. Escribe su índice en `out_idx` si no es null.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_spawn(
    sim: *mut TkSim,
    x: f32, y: f32, z: f32,
    vx: f32, vy: f32, vz: f32,
    mass: f32, charge: f32,
    out_idx: *mut u32,
) -> i32 {
    let Some(s) = (unsafe { sim_mut(sim) }) else { return TK_ERR_NULL; };
    let h = s.world.spawn([x, y, z], [vx, vy, vz], mass, charge);
    if !out_idx.is_null() {
        unsafe { *out_idx = h.idx; }
    }
    TK_OK
}

/// Número de partículas vivas.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_len(sim: *const TkSim) -> u32 {
    match unsafe { sim_ref(sim) } {
        Some(s) => s.world.len() as u32,
        None    => 0,
    }
}

/// Reconstruye la grilla a partir de las posiciones actuales. Llamar después
/// de spawning masivo o tras `tk_sim_import_state` (futuro).
#[no_mangle]
pub unsafe extern "C" fn tk_sim_rebuild_grid(sim: *mut TkSim) -> i32 {
    let Some(s) = (unsafe { sim_mut(sim) }) else { return TK_ERR_NULL; };
    s.grid.rebuild(&s.world);
    TK_OK
}

// ─── Step de simulación ─────────────────────────────────────────────────────

/// Un step Velocity-Verlet con fuerza Lennard-Jones + paredes reflectivas.
///
/// `bmin` y `bmax` son punteros a 3 floats cada uno (esquinas del dominio).
/// Una sola llamada hace: clear_accel → LJ → kick_drift → merge → finish_kick
/// → reflect_walls.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_step_lj(
    sim: *mut TkSim,
    dt: f32,
    epsilon: f32,
    sigma: f32,
    cutoff: f32,
    bmin: *const f32,
    bmax: *const f32,
) -> i32 {
    let Some(s) = (unsafe { sim_mut(sim) }) else { return TK_ERR_NULL; };
    if bmin.is_null() || bmax.is_null() { return TK_ERR_NULL; }
    let bmin_a = unsafe { [*bmin.add(0), *bmin.add(1), *bmin.add(2)] };
    let bmax_a = unsafe { [*bmax.add(0), *bmax.add(1), *bmax.add(2)] };

    let params = IntegratorParams { dt, bounds_min: bmin_a, bounds_max: bmax_a };
    let lj = LjParams { epsilon, sigma, cutoff };

    velocity_verlet_step(
        &mut s.world, &mut s.grid, &params, &mut s.outboxes,
        |w, g| { clear_accelerations(w); lennard_jones(w, g, &lj); },
    );
    reflect_walls(&mut s.world, bmin_a, bmax_a);
    TK_OK
}

// ─── Observables ────────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn tk_sim_kinetic_energy(sim: *const TkSim) -> f64 {
    match unsafe { sim_ref(sim) } {
        Some(s) => core_ke(&s.world),
        None    => 0.0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn tk_sim_temperature(sim: *const TkSim, kb: f64) -> f64 {
    match unsafe { sim_ref(sim) } {
        Some(s) => core_temp(&s.world, kb),
        None    => 0.0,
    }
}

/// Escribe el momentum total (3×f64) en `out_xyz`. Caller asegura espacio para
/// 24 bytes.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_total_momentum(
    sim: *const TkSim,
    out_xyz: *mut f64,
) -> i32 {
    let Some(s) = (unsafe { sim_ref(sim) }) else { return TK_ERR_NULL; };
    if out_xyz.is_null() { return TK_ERR_NULL; }
    let p = core_p(&s.world);
    unsafe {
        *out_xyz.add(0) = p[0];
        *out_xyz.add(1) = p[1];
        *out_xyz.add(2) = p[2];
    }
    TK_OK
}

// ─── Posiciones (AoS para el caller — render 3D) ───────────────────────────

/// Copia las posiciones `(x, y, z)` de TODAS las partículas vivas como un
/// arreglo plano `f32[N*3]` en `out`. El layout interno es SoA; este export
/// hace la transposición a AoS para que el caller — típicamente un renderer
/// que proyecta a 2D — itere de un tirón sin saltos entre buffers.
///
/// `cap_count` es la capacidad del buffer en NÚMERO DE PARTÍCULAS (no en
/// floats ni bytes). Si `cap_count < world.len()`, se devuelve
/// `TK_ERR_INVALID` sin copiar nada.
///
/// Devuelve `n` (cantidad copiada) como `i32`, o un código negativo.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_positions(
    sim: *const TkSim,
    out: *mut f32,
    cap_count: u32,
) -> i32 {
    let Some(s) = (unsafe { sim_ref(sim) }) else { return TK_ERR_NULL; };
    if out.is_null() { return TK_ERR_NULL; }
    let n = s.world.len();
    if (cap_count as usize) < n {
        return TK_ERR_INVALID;
    }
    let xs = &s.world.xs.0;
    let ys = &s.world.ys.0;
    let zs = &s.world.zs.0;
    for i in 0..n {
        unsafe {
            *out.add(i * 3) = xs[i];
            *out.add(i * 3 + 1) = ys[i];
            *out.add(i * 3 + 2) = zs[i];
        }
    }
    n as i32
}

// ─── Snapshots ──────────────────────────────────────────────────────────────

/// Escribe los 32 B del CID BLAKE3 del estado actual en `out_32`.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_snapshot_cid(
    sim: *const TkSim,
    out_32: *mut u8,
) -> i32 {
    let Some(s) = (unsafe { sim_ref(sim) }) else { return TK_ERR_NULL; };
    if out_32.is_null() { return TK_ERR_NULL; }
    let snap = Snapshot::capture(&s.world);
    unsafe { ptr::copy_nonoverlapping(snap.cid.as_ptr(), out_32, 32); }
    TK_OK
}

/// Exporta el estado completo (formato definido en `snapshot.rs`) como buffer
/// recién alocado. Caller libera con `tk_buf_free(ptr, len)`.
#[no_mangle]
pub unsafe extern "C" fn tk_sim_snapshot_export(
    sim: *const TkSim,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let Some(s) = (unsafe { sim_ref(sim) }) else { return TK_ERR_NULL; };
    if out_ptr.is_null() || out_len.is_null() { return TK_ERR_NULL; }
    let mut bytes = Snapshot::capture(&s.world).bytes;
    bytes.shrink_to_fit();
    let len = bytes.len();
    let cap = bytes.capacity();
    debug_assert_eq!(len, cap, "shrink_to_fit dejó capacidad extra");
    let ptr = bytes.as_mut_ptr();
    core::mem::forget(bytes);
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
    TK_OK
}

/// Libera un buffer producido por `tk_sim_snapshot_export`. `len` DEBE ser el
/// mismo valor escrito por export (es la `capacity` del Vec original).
#[no_mangle]
pub unsafe extern "C" fn tk_buf_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 { return; }
    // SAFETY: ptr/len fueron producidos por un Vec::into_raw_parts equivalente
    // dentro de `tk_sim_snapshot_export`; reconstruir el Vec con misma (len, cap)
    // es la única forma correcta de devolver la memoria a Rust.
    let _ = unsafe { Vec::<u8>::from_raw_parts(ptr, len, len) };
    let _ = NonNull::new(ptr); // silencia la sospecha del optimizador
}

// ─── Tuning interno ─────────────────────────────────────────────────────────

#[cfg(feature = "cpu")]
fn workers_default() -> usize {
    rayon::current_num_threads().max(1)
}

#[cfg(all(feature = "wasm", not(feature = "cpu")))]
fn workers_default() -> usize { 1 }

#[cfg(not(any(feature = "cpu", feature = "wasm")))]
fn workers_default() -> usize {
    compile_error!("tinkuy-abi necesita feature `cpu` o `wasm`");
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(all(test, any(feature = "cpu", feature = "wasm")))]
mod tests {
    use super::*;

    fn fresh_sim() -> *mut TkSim {
        let origin = [-50.0f32, -50.0, -50.0];
        let dims = [34u32, 34, 34]; // 34·3 = 102 ≥ 100 = dominio
        let mut out: *mut TkSim = ptr::null_mut();
        // cell_size 3.0 ≥ cutoff LJ 2.5 (precondición del kernel de fuerzas).
        let rc = unsafe {
            tk_sim_new(64, origin.as_ptr(), 3.0, dims.as_ptr(), &mut out)
        };
        assert_eq!(rc, TK_OK);
        assert!(!out.is_null());
        out
    }

    #[test]
    fn lifecycle_new_spawn_len_free() {
        let s = fresh_sim();
        for k in 0..4 {
            let f = k as f32 * 0.5;
            let mut idx = 0u32;
            let rc = unsafe { tk_sim_spawn(s, f, 0.0, 0.0, 0.1, 0.0, 0.0, 1.0, 0.0, &mut idx) };
            assert_eq!(rc, TK_OK);
            assert_eq!(idx as i32, k);
        }
        assert_eq!(unsafe { tk_sim_len(s) }, 4);
        unsafe { tk_sim_free(s) };
    }

    #[test]
    fn step_lj_advances_and_keeps_momentum_bounded() {
        let s = fresh_sim();
        // 8 partículas en un cubo 2×2×2 espaciado a 1.5σ.
        for k in 0..2 { for j in 0..2 { for i in 0..2 {
            let p = [i as f32 * 1.5, j as f32 * 1.5, k as f32 * 1.5];
            let mut idx = 0u32;
            let rc = unsafe { tk_sim_spawn(s, p[0], p[1], p[2], 0.0, 0.0, 0.0, 1.0, 0.0, &mut idx) };
            assert_eq!(rc, TK_OK);
        }}}
        let rc = unsafe { tk_sim_rebuild_grid(s) };
        assert_eq!(rc, TK_OK);

        let bmin = [-50.0f32, -50.0, -50.0];
        let bmax = [ 50.0f32,  50.0,  50.0];
        for _ in 0..50 {
            let rc = unsafe { tk_sim_step_lj(s, 0.005, 1.0, 1.0, 2.5, bmin.as_ptr(), bmax.as_ptr()) };
            assert_eq!(rc, TK_OK);
        }

        // Momento total acotado (drift mínimo en sistema cerrado con Newton-3 efectivo).
        let mut p = [0.0f64; 3];
        let rc = unsafe { tk_sim_total_momentum(s, p.as_mut_ptr()) };
        assert_eq!(rc, TK_OK);
        let mag = (p[0]*p[0] + p[1]*p[1] + p[2]*p[2]).sqrt();
        assert!(mag < 1.0, "momentum drift = {} (esperado pequeño)", mag);

        unsafe { tk_sim_free(s) };
    }

    #[test]
    fn cid_is_deterministic_across_two_sims() {
        let build = || {
            let s = fresh_sim();
            for k in 0..3 {
                let f = k as f32 * 0.4;
                let mut idx = 0u32;
                unsafe { tk_sim_spawn(s, f, f * 0.5, f * 0.25, 0.01, 0.0, 0.0, 1.0, 0.0, &mut idx); }
            }
            unsafe { tk_sim_rebuild_grid(s); }
            s
        };
        let a = build();
        let b = build();
        let mut ca = [0u8; 32];
        let mut cb = [0u8; 32];
        unsafe {
            tk_sim_snapshot_cid(a, ca.as_mut_ptr());
            tk_sim_snapshot_cid(b, cb.as_mut_ptr());
        }
        assert_eq!(ca, cb, "CIDs distintos para estados idénticos");
        unsafe { tk_sim_free(a); tk_sim_free(b); }
    }

    #[test]
    fn snapshot_export_then_buf_free_roundtrip() {
        let s = fresh_sim();
        for k in 0..5 {
            let f = k as f32 * 0.5;
            let mut idx = 0u32;
            unsafe { tk_sim_spawn(s, f, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, &mut idx); }
        }
        let mut ptr: *mut u8 = ptr::null_mut();
        let mut len: usize = 0;
        let rc = unsafe { tk_sim_snapshot_export(s, &mut ptr, &mut len) };
        assert_eq!(rc, TK_OK);
        assert!(!ptr.is_null());
        // header u64 + 5 partículas × 11 campos × 4 B = 8 + 220 = 228 B
        assert_eq!(len, 8 + 5 * 11 * 4);
        unsafe { tk_buf_free(ptr, len); }
        unsafe { tk_sim_free(s); }
    }

    #[test]
    fn positions_are_copied_aos() {
        let s = fresh_sim();
        // Tres partículas con posiciones únicas para identificar el orden.
        let pts = [
            [1.0f32,  2.0,  3.0],
            [4.0,     5.0,  6.0],
            [-7.0,    8.0, -9.0],
        ];
        for p in &pts {
            let mut idx = 0u32;
            let rc = unsafe { tk_sim_spawn(s, p[0], p[1], p[2], 0., 0., 0., 1., 0., &mut idx) };
            assert_eq!(rc, TK_OK);
        }
        let mut buf = [0.0f32; 16 * 3];
        let n = unsafe { tk_sim_positions(s, buf.as_mut_ptr(), 16) };
        assert_eq!(n, 3);
        for (i, p) in pts.iter().enumerate() {
            assert_eq!(buf[i * 3], p[0]);
            assert_eq!(buf[i * 3 + 1], p[1]);
            assert_eq!(buf[i * 3 + 2], p[2]);
        }
        // Capacidad insuficiente: rechazo limpio sin tocar el buffer.
        let mut buf2 = [42.0f32; 6];
        let rc = unsafe { tk_sim_positions(s, buf2.as_mut_ptr(), 2) };
        assert_eq!(rc, TK_ERR_INVALID);
        assert!(buf2.iter().all(|&v| v == 42.0));
        unsafe { tk_sim_free(s); }
    }

    #[test]
    fn null_pointers_are_rejected() {
        let mut out: *mut TkSim = ptr::null_mut();
        let bad = unsafe { tk_sim_new(0, ptr::null(), 1.0, ptr::null(), &mut out) };
        assert_eq!(bad, TK_ERR_NULL);

        let bad2 = unsafe { tk_sim_spawn(ptr::null_mut(), 0., 0., 0., 0., 0., 0., 1., 0., ptr::null_mut()) };
        assert_eq!(bad2, TK_ERR_NULL);

        let bad3 = unsafe { tk_sim_kinetic_energy(ptr::null()) };
        assert_eq!(bad3, 0.0);
    }
}
