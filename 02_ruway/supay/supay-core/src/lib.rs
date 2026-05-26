//! `supay-core` — bindings Rust al motor de doomgeneric.
//!
//! Doomgeneric es un fork de Chocolate Doom que aísla el motor
//! (~10k LOC C) de cualquier renderer concreto. El host:
//!
//! 1. Llama [`DoomEngine::new`] con args al estilo `argv`
//!    (típicamente `["doomgeneric", "-iwad", "doom1.wad"]`).
//! 2. En cada frame de su event loop llama [`DoomEngine::tick`].
//!    Doom corre su lógica interna a 35 Hz; el tick puede ser más
//!    rápido (vsync) — doomgeneric maneja su clock internamente
//!    consultando [`DG_GetTicksMs`].
//! 3. Tras cada `tick`, lee [`DoomEngine::framebuffer`] y pinta los
//!    320×200 píxeles ARGB donde quiera (textura GPU, Image de
//!    Llimphi, framebuffer Wawa, etc.).
//! 4. En su `on_key` traduce sus eventos a [códigos de tecla
//!    Doom](`KEY_*`) y los empuja con [`DoomEngine::push_key`].
//!
//! ## Modo stub
//!
//! Si `vendor/doomgeneric/` no existe, `build.rs` emite
//! `cfg(doomgeneric_stub)` y el motor real se reemplaza por no-ops.
//! `DoomEngine::tick` no hace nada, `framebuffer` devuelve negro.
//! Útil para mantener `cargo check --workspace` verde mientras se
//! desarrolla el host antes de tener el código C.

#![forbid(unsafe_op_in_unsafe_fn)]

use std::ffi::CString;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub use supay_scene::{
    interpolate, PlayerSnap, SceneSnapshot, SectorSnap, SegSnap, SnapshotPair, SpriteSnap,
    SubsectorSnap, WallSeg, NO_SECTOR, NO_SKY_PIC,
};

// doomgeneric default es 640×400 (auto-scaling factor 2 sobre los
// 320×200 del Doom clásico). Está hardcoded en `doomgeneric.h` como
// `DOOMGENERIC_RESX`/`DOOMGENERIC_RESY`; si quisiéramos 320×200 puro
// habría que pasar `-DDOOMGENERIC_RESX=320 -DDOOMGENERIC_RESY=200` al
// `cc` build.
pub const DOOM_WIDTH: usize = 640;
pub const DOOM_HEIGHT: usize = 400;
pub const DOOM_PIXELS: usize = DOOM_WIDTH * DOOM_HEIGHT;

/// Códigos de tecla de doomgeneric (subset). Espejo de los `KEY_*`
/// que el motor reconoce. El host traduce sus eventos a estos.
#[allow(non_upper_case_globals)]
pub mod keys {
    pub const KEY_RIGHTARROW: u8 = 0xae;
    pub const KEY_LEFTARROW: u8 = 0xac;
    pub const KEY_UPARROW: u8 = 0xad;
    pub const KEY_DOWNARROW: u8 = 0xaf;
    pub const KEY_STRAFE_L: u8 = 0xb8;
    pub const KEY_STRAFE_R: u8 = 0xb9;
    pub const KEY_USE: u8 = 0xa2;
    pub const KEY_FIRE: u8 = 0xa3;
    pub const KEY_ESCAPE: u8 = 27;
    pub const KEY_ENTER: u8 = 13;
    pub const KEY_TAB: u8 = 9;
    pub const KEY_RSHIFT: u8 = 0x36;
    pub const KEY_SPACE: u8 = b' ';
    pub const KEY_Y: u8 = b'y';
    pub const KEY_N: u8 = b'n';
}

// =====================================================================
// HostState — singleton que los callbacks C consultan
// =====================================================================

// Algunos campos sólo se leen desde los callbacks que están
// cfgados off en modo stub — `allow(dead_code)` evita warnings ahí.
#[allow(dead_code)]
struct HostState {
    start: Instant,
    /// FIFO de eventos de teclado pendientes. `DG_GetKey` los consume
    /// uno por llamada.
    key_queue: std::collections::VecDeque<(bool, u8)>,
    /// Copia del framebuffer ARGB tras el último `DG_DrawFrame`.
    /// 320×200 u32; cada u32 = 0xAARRGGBB.
    framebuffer: Vec<u32>,
    /// Título que el motor pidió poner a la ventana.
    title: String,
}

fn state() -> &'static Mutex<HostState> {
    static STATE: OnceLock<Mutex<HostState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(HostState {
            start: Instant::now(),
            key_queue: std::collections::VecDeque::with_capacity(32),
            framebuffer: vec![0; DOOM_PIXELS],
            title: String::from("supay-doom"),
        })
    })
}

// =====================================================================
// Callbacks que doomgeneric llama (extern "C" #[no_mangle])
// Solo se exportan cuando el motor real está linkeado.
// =====================================================================

#[cfg(not(doomgeneric_stub))]
mod callbacks {
    use super::*;
    use std::ffi::{c_char, c_int, CStr};
    use std::os::raw::c_uint;

    extern "C" {
        /// Puntero al framebuffer interno de doomgeneric.
        /// 320×200 u32 ARGB. Lo asigna `D_DoomMain` al arrancar.
        pub(super) static mut DG_ScreenBuffer: *mut u32;
    }

    #[no_mangle]
    pub extern "C" fn DG_Init() {
        // Nada — toda la inicialización del host vive en `DoomEngine::new`.
    }

    /// Doomgeneric llamó esto: el framebuffer interno ya tiene el
    /// frame nuevo, lo copiamos a `HostState`.
    #[no_mangle]
    pub extern "C" fn DG_DrawFrame() {
        // SAFETY: `DG_ScreenBuffer` lo asigna doomgeneric al arrancar
        // y apunta a un buffer estático de 320×200 u32. Ronda válido
        // durante toda la vida del proceso.
        let slice = unsafe {
            if DG_ScreenBuffer.is_null() {
                return;
            }
            std::slice::from_raw_parts(DG_ScreenBuffer as *const u32, DOOM_PIXELS)
        };
        if let Ok(mut s) = state().lock() {
            s.framebuffer.copy_from_slice(slice);
        }
    }

    /// Si nuestro host ya tiene su propio scheduler (Llimphi corre
    /// `tick()` desde su event loop), ignoramos el sleep.
    #[no_mangle]
    pub extern "C" fn DG_SleepMs(_ms: c_uint) {}

    #[no_mangle]
    pub extern "C" fn DG_GetTicksMs() -> c_uint {
        state()
            .lock()
            .map(|s| s.start.elapsed().as_millis() as c_uint)
            .unwrap_or(0)
    }

    /// Saca un evento de teclado pendiente de la queue. `pressed`
    /// 1 = press, 0 = release. Devuelve 1 si había evento, 0 si la
    /// queue está vacía.
    #[no_mangle]
    pub extern "C" fn DG_GetKey(pressed: *mut c_int, doom_key: *mut u8) -> c_int {
        let Ok(mut s) = state().lock() else { return 0 };
        let Some((p, k)) = s.key_queue.pop_front() else {
            return 0;
        };
        // SAFETY: caller (doomgeneric) garantiza que los punteros
        // apuntan a variables válidas en su stack.
        unsafe {
            *pressed = if p { 1 } else { 0 };
            *doom_key = k;
        }
        1
    }

    #[no_mangle]
    pub extern "C" fn DG_SetWindowTitle(title: *const c_char) {
        if title.is_null() {
            return;
        }
        // SAFETY: doomgeneric pasa C-strings null-terminated.
        let s = unsafe { CStr::from_ptr(title) }
            .to_string_lossy()
            .into_owned();
        if let Ok(mut g) = state().lock() {
            g.title = s;
        }
    }
}

#[cfg(not(doomgeneric_stub))]
extern "C" {
    fn doomgeneric_Create(argc: std::ffi::c_int, argv: *mut *mut std::ffi::c_char);
    fn doomgeneric_Tick();
}

// =====================================================================
// FFI scene-export (Fase 2) — implementación en `src/scene_export.c`.
// Sólo existe cuando el motor real está linkeado.
// =====================================================================

#[cfg(not(doomgeneric_stub))]
extern "C" {
    fn supay_scene_player(
        x: *mut f32,
        y: *mut f32,
        z: *mut f32,
        angle: *mut f32,
        view_height: *mut f32,
    ) -> std::ffi::c_int;
    fn supay_scene_num_walls() -> std::ffi::c_int;
    fn supay_scene_wall(
        i: std::ffi::c_int,
        x1: *mut f32,
        y1: *mut f32,
        x2: *mut f32,
        y2: *mut f32,
        front: *mut u32,
        back: *mut u32,
        flags: *mut u32,
    ) -> std::ffi::c_int;
    fn supay_scene_num_sectors() -> std::ffi::c_int;
    fn supay_scene_sector(
        i: std::ffi::c_int,
        floor: *mut f32,
        ceiling: *mut f32,
        light: *mut u8,
        floor_pic: *mut u16,
        ceiling_pic: *mut u16,
    ) -> std::ffi::c_int;
    fn supay_scene_num_sprites() -> std::ffi::c_int;
    fn supay_scene_sprite(
        i: std::ffi::c_int,
        x: *mut f32,
        y: *mut f32,
        z: *mut f32,
        angle: *mut f32,
        sprite: *mut u16,
        frame: *mut u8,
        sector: *mut u32,
    ) -> std::ffi::c_int;
    // Fase 3.2: BSP subsectors + segs + sky flat number.
    fn supay_scene_num_subsectors() -> std::ffi::c_int;
    fn supay_scene_subsector(
        i: std::ffi::c_int,
        sector: *mut u32,
        first_seg: *mut u32,
        num_segs: *mut u32,
    ) -> std::ffi::c_int;
    fn supay_scene_num_segs() -> std::ffi::c_int;
    fn supay_scene_seg(
        i: std::ffi::c_int,
        x1: *mut f32,
        y1: *mut f32,
        x2: *mut f32,
        y2: *mut f32,
    ) -> std::ffi::c_int;
    fn supay_scene_sky_pic() -> u16;
    /// Resuelve un `pic_idx` (índice de flat) al nombre del lump.
    /// `out` debe apuntar a un buffer de ≥ 9 bytes. Devuelve 1 si OK.
    fn supay_scene_flat_name(pic_idx: u16, out: *mut std::ffi::c_char) -> std::ffi::c_int;
}

// =====================================================================
// API pública safe
// =====================================================================

pub struct DoomEngine {
    // CStrings dueñas de la memoria que argv apunta — debemos
    // mantenerlas vivas mientras el motor corre. doomgeneric guarda
    // `myargv = argv` y consulta los args con `M_CheckParm` durante
    // toda la partida; si los liberamos, segfault a los pocos
    // segundos cuando el motor consulta `-nosound` o similar.
    _args: Vec<CString>,
    /// Vec<*mut c_char> que doomgeneric guardó en `myargv`. También
    /// vive lo que vive el engine — debemos preservarlo o `myargv`
    /// queda dangling.
    _argv: Vec<*mut std::ffi::c_char>,
    /// `true` si vendor/doomgeneric/ se compiló y el motor real está
    /// linkeado. `false` en modo stub.
    pub real: bool,
}

// SAFETY: `*mut c_char` no es Send + Sync por defecto, pero los
// punteros que `_argv` mantiene apuntan a memoria dueña de `_args`
// (CString) que sí es Send; los pointers nunca se desreferencian
// desde Rust después de `new`. El motor C los consulta desde el
// thread del tick siempre con el mismo address space.
unsafe impl Send for DoomEngine {}
unsafe impl Sync for DoomEngine {}

impl DoomEngine {
    /// Inicializa doomgeneric con `args` estilo `argv`. El primer
    /// elemento es típicamente `"doomgeneric"` (el "programa") y los
    /// demás son flags al motor (`-iwad doom1.wad`, `-warp 1 1`, etc.).
    ///
    /// En modo stub no hace nada útil — devuelve un engine que pinta
    /// pantalla negra y consume input sin reaccionar.
    pub fn new(args: Vec<String>) -> Self {
        // Inicializa el singleton.
        let _ = state();
        let cstrings: Vec<CString> = args
            .into_iter()
            .filter_map(|s| CString::new(s).ok())
            .collect();
        let mut argv: Vec<*mut std::ffi::c_char> =
            cstrings.iter().map(|c| c.as_ptr() as *mut _).collect();
        argv.push(std::ptr::null_mut());
        #[cfg(not(doomgeneric_stub))]
        {
            // SAFETY: doomgeneric_Create lee argc + argv como sería C
            // y guarda los punteros en `myargv` globales. Los
            // mantenemos vivos en `_args` (CStrings) y `_argv` (Vec
            // de ptrs) durante toda la vida del engine.
            unsafe {
                doomgeneric_Create(cstrings.len() as std::ffi::c_int, argv.as_mut_ptr());
            }
        }
        Self {
            _args: cstrings,
            _argv: argv,
            real: cfg!(not(doomgeneric_stub)),
        }
    }

    /// Avanza un tick del motor. En modo stub: no-op.
    pub fn tick(&mut self) {
        #[cfg(not(doomgeneric_stub))]
        unsafe {
            doomgeneric_Tick();
        }
    }

    /// Encola un evento de teclado para que el motor lo consuma en
    /// su próximo tick. Usar las constantes de [`keys`].
    pub fn push_key(&mut self, pressed: bool, doom_key: u8) {
        if let Ok(mut s) = state().lock() {
            s.key_queue.push_back((pressed, doom_key));
        }
    }

    /// Devuelve el framebuffer del último frame (320×200 u32 ARGB).
    /// El primer u32 es la esquina superior izquierda; el segundo es
    /// el píxel a su derecha, etc.
    pub fn framebuffer(&self) -> Vec<u32> {
        state()
            .lock()
            .map(|s| s.framebuffer.clone())
            .unwrap_or_else(|_| vec![0; DOOM_PIXELS])
    }

    /// Resuelve un índice de flat al nombre del lump (e.g. `"FLOOR4_8"`).
    /// En modo stub o si el mapa todavía no cargó, devuelve `None`.
    /// El renderer 3.3+ lo usa para mapear `sector.floor_pic` al color
    /// real del flat parseando el lump desde el WAD aparte.
    pub fn flat_name(&self, pic_idx: u16) -> Option<String> {
        #[cfg(doomgeneric_stub)]
        {
            let _ = pic_idx;
            None
        }
        #[cfg(not(doomgeneric_stub))]
        {
            let mut buf = [0i8; 9];
            // SAFETY: buf vive en este stack frame; la fn C escribe
            // hasta 9 bytes (8 chars + nul).
            let ok = unsafe { supay_scene_flat_name(pic_idx, buf.as_mut_ptr()) };
            if ok == 0 {
                return None;
            }
            // Convertir [i8; 9] null-terminated a String.
            let mut end = buf.len();
            for (i, &c) in buf.iter().enumerate() {
                if c == 0 {
                    end = i;
                    break;
                }
            }
            let bytes: Vec<u8> = buf[..end].iter().map(|&c| c as u8).collect();
            String::from_utf8(bytes).ok()
        }
    }

    /// Título que el motor solicitó (último `DG_SetWindowTitle`).
    pub fn title(&self) -> String {
        state()
            .lock()
            .map(|s| s.title.clone())
            .unwrap_or_default()
    }

    /// Captura un snapshot del estado visible del motor para el tick
    /// dado. El renderer (Fase 3) acumula dos snapshots consecutivos y
    /// los interpola para correr más rápido que 35 Hz.
    ///
    /// En modo stub (sin vendor doomgeneric) devuelve un snapshot
    /// sintético: una sala 8×8 con el jugador caminando en círculo y
    /// un sprite siguiéndolo. Útil para desarrollar el renderer antes
    /// de tener el motor real linkeado.
    ///
    /// En modo real lee el estado del motor C vía los getters de
    /// `scene_export.c`. **Debe llamarse desde el mismo thread que
    /// invoca `tick()`** — el cache interno de mobjs no es
    /// thread-safe (en la práctica, el host corre todo desde el event
    /// loop de Llimphi así que esto se cumple naturalmente).
    pub fn capture_scene(&self, tick: u64) -> SceneSnapshot {
        #[cfg(doomgeneric_stub)]
        {
            return synth_snapshot(tick);
        }
        #[cfg(not(doomgeneric_stub))]
        {
            capture_scene_real(tick)
        }
    }
}

// =====================================================================
// Captura real desde doomgeneric (Fase 2)
// =====================================================================

#[cfg(not(doomgeneric_stub))]
fn capture_scene_real(tick: u64) -> SceneSnapshot {
    use std::sync::Arc;

    // Player: si el motor todavía no cargó el mapa, devolvemos snapshot
    // vacío en lugar de coordenadas inválidas.
    let mut player = PlayerSnap::default();
    // SAFETY: los punteros apuntan a stack locales válidos; la fn C
    // sólo escribe en ellos si retorna != 0.
    let player_ok = unsafe {
        supay_scene_player(
            &mut player.x,
            &mut player.y,
            &mut player.z,
            &mut player.angle,
            &mut player.view_height,
        ) != 0
    };
    if !player_ok {
        return SceneSnapshot::empty(tick);
    }

    // Walls.
    // SAFETY: getter sin side-effects, lee globales del motor.
    let n_walls = unsafe { supay_scene_num_walls() }.max(0) as usize;
    let mut walls = Vec::with_capacity(n_walls);
    for i in 0..n_walls {
        let mut x1 = 0.0_f32;
        let mut y1 = 0.0_f32;
        let mut x2 = 0.0_f32;
        let mut y2 = 0.0_f32;
        let mut front = 0_u32;
        let mut back = 0_u32;
        let mut flags = 0_u32;
        // SAFETY: i en rango, punteros a locales válidos.
        let ok = unsafe {
            supay_scene_wall(
                i as std::ffi::c_int,
                &mut x1,
                &mut y1,
                &mut x2,
                &mut y2,
                &mut front,
                &mut back,
                &mut flags,
            )
        };
        if ok != 0 {
            walls.push(WallSeg {
                x1,
                y1,
                x2,
                y2,
                front_sector: front,
                back_sector: back,
                flags,
            });
        }
    }

    // Sectors.
    // SAFETY: idem.
    let n_sectors = unsafe { supay_scene_num_sectors() }.max(0) as usize;
    let mut sects = Vec::with_capacity(n_sectors);
    for i in 0..n_sectors {
        let mut floor = 0.0_f32;
        let mut ceiling = 0.0_f32;
        let mut light = 0_u8;
        let mut floor_pic = 0_u16;
        let mut ceiling_pic = 0_u16;
        // SAFETY: idem.
        let ok = unsafe {
            supay_scene_sector(
                i as std::ffi::c_int,
                &mut floor,
                &mut ceiling,
                &mut light,
                &mut floor_pic,
                &mut ceiling_pic,
            )
        };
        if ok != 0 {
            sects.push(SectorSnap {
                floor_height: floor,
                ceiling_height: ceiling,
                light_level: light,
                floor_pic,
                ceiling_pic,
            });
        }
    }

    // Sprites (mobjs). `num_sprites` reconstruye el cache interno C —
    // hay que llamarlo siempre antes de iterar `sprite(i)`.
    // SAFETY: idem.
    let n_sprites = unsafe { supay_scene_num_sprites() }.max(0) as usize;
    let mut sprs = Vec::with_capacity(n_sprites);
    for i in 0..n_sprites {
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut z = 0.0_f32;
        let mut angle = 0.0_f32;
        let mut sprite = 0_u16;
        let mut frame = 0_u8;
        let mut sector = 0_u32;
        // SAFETY: idem.
        let ok = unsafe {
            supay_scene_sprite(
                i as std::ffi::c_int,
                &mut x,
                &mut y,
                &mut z,
                &mut angle,
                &mut sprite,
                &mut frame,
                &mut sector,
            )
        };
        if ok != 0 {
            sprs.push(SpriteSnap {
                x,
                y,
                z,
                angle,
                sprite,
                frame,
                sector,
            });
        }
    }

    // Subsectors + segs (Fase 3.2).
    // SAFETY: getters sin side-effects.
    let n_subs = unsafe { supay_scene_num_subsectors() }.max(0) as usize;
    let mut subs = Vec::with_capacity(n_subs);
    for i in 0..n_subs {
        let mut sector = 0_u32;
        let mut first_seg = 0_u32;
        let mut num_segs = 0_u32;
        // SAFETY: i en rango, punteros válidos.
        let ok = unsafe {
            supay_scene_subsector(
                i as std::ffi::c_int,
                &mut sector,
                &mut first_seg,
                &mut num_segs,
            )
        };
        if ok != 0 {
            subs.push(SubsectorSnap {
                sector,
                first_seg,
                num_segs,
            });
        }
    }
    // SAFETY: idem.
    let n_segs = unsafe { supay_scene_num_segs() }.max(0) as usize;
    let mut segs_vec = Vec::with_capacity(n_segs);
    for i in 0..n_segs {
        let mut x1 = 0.0_f32;
        let mut y1 = 0.0_f32;
        let mut x2 = 0.0_f32;
        let mut y2 = 0.0_f32;
        // SAFETY: idem.
        let ok = unsafe {
            supay_scene_seg(
                i as std::ffi::c_int,
                &mut x1,
                &mut y1,
                &mut x2,
                &mut y2,
            )
        };
        if ok != 0 {
            segs_vec.push(SegSnap { x1, y1, x2, y2 });
        }
    }
    // SAFETY: idem.
    let sky_pic = unsafe { supay_scene_sky_pic() };

    SceneSnapshot {
        tick,
        player,
        walls: Arc::from(walls),
        sectors: Arc::from(sects),
        sprites: Arc::from(sprs),
        subsectors: Arc::from(subs),
        segs: Arc::from(segs_vec),
        sky_pic,
    }
}

// =====================================================================
// Captura sintética para modo stub (Fase 2)
// =====================================================================

/// Snapshot sintético: una sala 256×256 (≈ 4 celdas Doom de 64) con
/// el jugador caminando en círculo de radio 64 y un sprite trailing
/// 96 unidades por detrás. Unidades Doom-realistas para que el near
/// plane (4.0) del renderer no recorte paredes — Fase 3 lo consume sin
/// modificaciones.
#[cfg(doomgeneric_stub)]
fn synth_snapshot(tick: u64) -> SceneSnapshot {
    use std::sync::Arc;
    // 35 Hz → 1 vuelta cada ~6 s con coef 0.03.
    let t = tick as f32 * 0.03;
    let center = 128.0_f32;
    let r = 64.0_f32;
    let player = PlayerSnap {
        x: center + t.cos() * r,
        y: center + t.sin() * r,
        z: 0.0,
        // El jugador mira tangente al círculo, hacia donde camina.
        angle: t + std::f32::consts::FRAC_PI_2,
        view_height: 41.0,
    };
    // Cuatro paredes de la sala. Winding CW (mirando desde +Z): así
    // el "front side" de cada linedef en convención Doom — donde
    // `(v2-v1) × (pt-v1)_z < 0` — queda hacia adentro de la sala, y
    // el jugador parado en (128, 128) ve `front_sector = 0`.
    let walls: Vec<WallSeg> = vec![
        // Oeste: (0,0)→(0,256).
        WallSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 256.0,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
        },
        // Norte: (0,256)→(256,256).
        WallSeg {
            x1: 0.0,
            y1: 256.0,
            x2: 256.0,
            y2: 256.0,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
        },
        // Este: (256,256)→(256,0).
        WallSeg {
            x1: 256.0,
            y1: 256.0,
            x2: 256.0,
            y2: 0.0,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
        },
        // Sur: (256,0)→(0,0).
        WallSeg {
            x1: 256.0,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
        },
    ];
    let sectors: Vec<SectorSnap> = vec![SectorSnap {
        floor_height: 0.0,
        ceiling_height: 192.0,
        // Brightness pulsando suave — sirve para probar interpolación
        // de luz en el renderer.
        light_level: (192.0 + (t * 0.5).sin() * 32.0).clamp(0.0, 255.0) as u8,
        floor_pic: 0,
        ceiling_pic: 0,
    }];
    // Un sprite siguiendo al jugador a 96 unidades por detrás.
    let trail_angle = t - std::f32::consts::FRAC_PI_2;
    let sprites: Vec<SpriteSnap> = vec![SpriteSnap {
        x: player.x - trail_angle.cos() * 96.0,
        y: player.y - trail_angle.sin() * 96.0,
        z: 0.0,
        angle: trail_angle,
        sprite: 0,
        // Ciclo de 4 frames a 35/4 ≈ 8.75 Hz.
        frame: ((tick / 4) % 4) as u8,
        sector: 0,
    }];
    SceneSnapshot {
        tick,
        player,
        walls: Arc::from(walls),
        sectors: Arc::from(sectors),
        sprites: Arc::from(sprites),
        // Stub: sin BSP. El renderer cae al modo fake-floor de 3.1.
        subsectors: Arc::from(Vec::<SubsectorSnap>::new()),
        segs: Arc::from(Vec::<SegSnap>::new()),
        sky_pic: NO_SKY_PIC,
    }
}
