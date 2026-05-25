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

    /// Título que el motor solicitó (último `DG_SetWindowTitle`).
    pub fn title(&self) -> String {
        state()
            .lock()
            .map(|s| s.title.clone())
            .unwrap_or_default()
    }
}
