//! `mirada-plugin-sdk` — el lado *guest* de los plugins WASM de mirada.
//!
//! Un plugin es un módulo WebAssembly sandboxeado por `wasmi` en
//! [`mirada-plugin-host`]. Este crate le da la ABI y dos formas de plugin:
//!
//! - **Layout** ([`LayoutPlugin`]): una función pura de teselado. El host le
//!   pasa un [`TileInput`] (ventanas teseladas + área útil) y recibe los
//!   rectángulos. **No importa nada del host** → frontera de cero superficie.
//! - **Reactor** ([`ReactorPlugin`], feature `reactor`): reacciona a cada
//!   [`BodyEvent`] y emite comandos por un [`Ctx`] cuyos métodos llaman las
//!   importaciones del host **gateadas por capacidad** (si la capacidad no se
//!   concedió, el símbolo no se registra y el módulo ni instancia).
//!
//! La memoria cruza por dos búferes estáticos reusados (el guest es mono-hilo):
//! el host llama [`alloc`](abi_alloc) para reservar, escribe el input postcard,
//! y luego invoca `mirada_tile` / `mirada_on_event`.
//!
//! Para exportar un plugin usá [`export_layout_plugin!`] o
//! [`export_reactor_plugin!`].

#![cfg_attr(target_arch = "wasm32", no_std)]
// `static mut` es el idiom del guest mono-hilo (igual que las arenas de las
// apps WASM de wawa). Accedemos por `addr_of!`/`addr_of_mut!` donde hace falta.
#![allow(static_mut_refs)]

extern crate alloc;

use alloc::boxed::Box;
#[cfg(feature = "reactor")]
use alloc::string::String;
use alloc::vec::Vec;

pub use mirada_layout;
pub use mirada_protocol::{
    self, BodyEvent, BrainCommand, Decorations, LayoutMode, LayoutParams, Rect, TileInput,
    WindowEffects, WindowId,
};

// Re-exports para que los macros funcionen en crates que no traen `alloc`.
pub use alloc::boxed::Box as SdkBox;

// ---------------------------------------------------------------------------
// Heap del guest (sólo en el binario WASM; en host usa el allocator de std).
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod heap {
    use core::ptr::addr_of_mut;
    use linked_list_allocator::LockedHeap;

    #[global_allocator]
    static ALLOC: LockedHeap = LockedHeap::empty();

    const ARENA_SIZE: usize = 1024 * 1024;
    static mut ARENA: [u8; ARENA_SIZE] = [0; ARENA_SIZE];
    static mut READY: bool = false;

    /// Inicializa el heap en el primer uso. El host llama `alloc` antes que
    /// nada, así que cualquier reserva del guest encuentra el heap fundado.
    pub fn ensure() {
        unsafe {
            if !READY {
                READY = true;
                ALLOC.lock().init(addr_of_mut!(ARENA) as *mut u8, ARENA_SIZE);
            }
        }
    }

    /// Sin SO debajo, un pánico sólo puede frenar en seco.
    #[panic_handler]
    fn al_fallar(_: &core::panic::PanicInfo) -> ! {
        core::arch::wasm32::unreachable()
    }
}

#[cfg(target_arch = "wasm32")]
fn ensure_heap() {
    heap::ensure();
}
#[cfg(not(target_arch = "wasm32"))]
fn ensure_heap() {}

// ---------------------------------------------------------------------------
// ABI de memoria — dos búferes estáticos reusados.
// ---------------------------------------------------------------------------

static mut IN_BUF: Vec<u8> = Vec::new();
static mut OUT_BUF: Vec<u8> = Vec::new();

/// Reserva `len` bytes y devuelve el puntero donde el host escribe el input.
/// Lo re-exportan los macros como `#[no_mangle] pub extern "C" fn alloc`.
#[doc(hidden)]
pub fn abi_alloc(len: u32) -> u32 {
    ensure_heap();
    unsafe {
        let mut v = Vec::<u8>::with_capacity(len as usize);
        v.resize(len as usize, 0);
        let ptr = v.as_mut_ptr() as u32;
        IN_BUF = v;
        ptr
    }
}

/// Vista de sólo lectura del input recién escrito por el host.
fn input_bytes() -> &'static [u8] {
    unsafe { &*core::ptr::addr_of!(IN_BUF) }
}

/// Guarda el output en el búfer estático y empaqueta `(ptr<<32 | len)`.
fn pack_output(bytes: Vec<u8>) -> u64 {
    unsafe {
        OUT_BUF = bytes;
        let p = OUT_BUF.as_ptr() as u64;
        let l = OUT_BUF.len() as u64;
        (p << 32) | l
    }
}

// ---------------------------------------------------------------------------
// Plugin de LAYOUT (Tier-0, sin importaciones del host).
// ---------------------------------------------------------------------------

/// Una estrategia de teselado. El host la consulta en cada `relayout`.
pub trait LayoutPlugin {
    /// Reparte las ventanas teseladas (`input.ids`, en orden) dentro de
    /// `input.work`. Devolvé un rect por id; los ids que no devuelvas conservan
    /// la geometría que el `Desktop` ya les había dado.
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)>;
}

static mut LAYOUT: Option<Box<dyn LayoutPlugin>> = None;

/// Punto de entrada que el macro [`export_layout_plugin!`] cablea a
/// `mirada_tile`. Inicializa el plugin perezosamente y despacha.
#[doc(hidden)]
pub fn layout_entry(ctor: fn() -> Box<dyn LayoutPlugin>, _ptr: u32, _len: u32) -> u64 {
    ensure_heap();
    let input: TileInput = match postcard::from_bytes(input_bytes()) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let plugin = unsafe {
        if LAYOUT.is_none() {
            LAYOUT = Some(ctor());
        }
        LAYOUT.as_mut().unwrap().as_mut()
    };
    let rects = plugin.tile(&input);
    pack_output(postcard::to_allocvec(&rects).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Plugin REACTOR (Tier-1, capacidades gateadas).
// ---------------------------------------------------------------------------

// Importaciones del host: sólo existen en el binario WASM y sólo con la feature
// `reactor`. Si la capacidad no se concedió en el host, el símbolo correspondiente
// no se registra y el módulo ni instancia — frontera física.
#[cfg(all(feature = "reactor", target_arch = "wasm32"))]
#[link(wasm_import_module = "mirada_host")]
extern "C" {
    fn host_log(ptr: u32, len: u32);
    fn host_emit_spawn(ptr: u32, len: u32);
    fn host_emit_close(id: u64);
    fn host_emit_kill(id: u64);
    fn host_emit_keys(ptr: u32, len: u32);
    fn host_emit_decor(ptr: u32, len: u32);
    fn host_emit_cursor(ptr: u32, len: u32);
    fn host_emit_effects(id: u64, opacity: u32, flags: u32);
}

// Stubs en host: el crate compila para el smoke test; nunca se llaman ahí.
#[cfg(all(feature = "reactor", not(target_arch = "wasm32")))]
#[allow(clippy::missing_safety_doc)]
mod host_imports {
    pub unsafe fn host_log(_: u32, _: u32) {}
    pub unsafe fn host_emit_spawn(_: u32, _: u32) {}
    pub unsafe fn host_emit_close(_: u64) {}
    pub unsafe fn host_emit_kill(_: u64) {}
    pub unsafe fn host_emit_keys(_: u32, _: u32) {}
    pub unsafe fn host_emit_decor(_: u32, _: u32) {}
    pub unsafe fn host_emit_cursor(_: u32, _: u32) {}
    pub unsafe fn host_emit_effects(_: u64, _: u32, _: u32) {}
}
#[cfg(all(feature = "reactor", not(target_arch = "wasm32")))]
use host_imports::*;

/// El canal por el que un reactor emite comandos al host. Cada método está
/// respaldado por una importación gateada por capacidad; usar uno cuya
/// capacidad no se concedió hace que el módulo ni instancie.
#[cfg(feature = "reactor")]
pub struct Ctx {
    _priv: (),
}

#[cfg(feature = "reactor")]
impl Ctx {
    #[doc(hidden)]
    pub fn new() -> Self {
        Ctx { _priv: () }
    }

    /// Diagnóstico (sin capacidad).
    pub fn log(&mut self, msg: &str) {
        let b = msg.as_bytes();
        unsafe { host_log(b.as_ptr() as u32, b.len() as u32) }
    }

    /// Lanza un programa (`CAP_SPAWN`). La cadena se pasa a `sh -c`.
    pub fn spawn(&mut self, cmd: &str) {
        let b = cmd.as_bytes();
        unsafe { host_emit_spawn(b.as_ptr() as u32, b.len() as u32) }
    }

    /// Cierra ordenadamente una ventana (`CAP_WINDOW_CONTROL`).
    pub fn close(&mut self, id: WindowId) {
        unsafe { host_emit_close(id) }
    }

    /// Mata al cliente de una ventana (`CAP_WINDOW_CONTROL`).
    pub fn kill(&mut self, id: WindowId) {
        unsafe { host_emit_kill(id) }
    }

    /// Registra atajos globales a interceptar (`CAP_KEYS`). El host los **une**
    /// a los del `Desktop`.
    pub fn grab_keys<S: AsRef<str>>(&mut self, keys: &[S]) {
        let v: Vec<String> = keys.iter().map(|s| s.as_ref().into()).collect();
        let b = postcard::to_allocvec(&v).unwrap_or_default();
        unsafe { host_emit_keys(b.as_ptr() as u32, b.len() as u32) }
    }

    /// Fija la decoración de ventana (`CAP_DECOR`).
    pub fn set_decorations(&mut self, d: &Decorations) {
        let b = postcard::to_allocvec(d).unwrap_or_default();
        unsafe { host_emit_decor(b.as_ptr() as u32, b.len() as u32) }
    }

    /// Cambia el cursor del puntero (`CAP_DECOR`).
    pub fn set_cursor(&mut self, name: &str) {
        let b = name.as_bytes();
        unsafe { host_emit_cursor(b.as_ptr() as u32, b.len() as u32) }
    }

    /// Fija los efectos visuales de una ventana (`CAP_EFFECTS`): opacidad
    /// (`0`=transparente, `255`=opaca) y sombra. Atenuar/sombrear según foco, etc.
    pub fn set_effects(&mut self, id: WindowId, effects: WindowEffects) {
        let flags = if effects.shadow { 1 } else { 0 };
        unsafe { host_emit_effects(id, effects.opacity as u32, flags) }
    }
}

/// Un plugin que reacciona a eventos del Cuerpo y emite comandos por el [`Ctx`].
#[cfg(feature = "reactor")]
pub trait ReactorPlugin {
    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx);
}

#[cfg(feature = "reactor")]
static mut REACTOR: Option<Box<dyn ReactorPlugin>> = None;

/// Punto de entrada que el macro [`export_reactor_plugin!`] cablea a
/// `mirada_on_event`.
#[cfg(feature = "reactor")]
#[doc(hidden)]
pub fn reactor_entry(ctor: fn() -> Box<dyn ReactorPlugin>, _ptr: u32, _len: u32) {
    ensure_heap();
    let event: BodyEvent = match postcard::from_bytes(input_bytes()) {
        Ok(v) => v,
        Err(_) => return,
    };
    let plugin = unsafe {
        if REACTOR.is_none() {
            REACTOR = Some(ctor());
        }
        REACTOR.as_mut().unwrap().as_mut()
    };
    let mut ctx = Ctx::new();
    plugin.on_event(event, &mut ctx);
}

// ---------------------------------------------------------------------------
// Macros de exportación.
// ---------------------------------------------------------------------------

/// Exporta un [`LayoutPlugin`] como módulo WASM. `$make` es una expresión que
/// construye el plugin (p. ej. `MiLayout::default()`).
#[macro_export]
macro_rules! export_layout_plugin {
    ($make:expr) => {
        #[no_mangle]
        pub extern "C" fn alloc(len: u32) -> u32 {
            $crate::abi_alloc(len)
        }

        #[no_mangle]
        pub extern "C" fn mirada_tile(ptr: u32, len: u32) -> u64 {
            fn __ctor() -> $crate::SdkBox<dyn $crate::LayoutPlugin> {
                $crate::SdkBox::new($make)
            }
            $crate::layout_entry(__ctor, ptr, len)
        }
    };
}

/// Exporta un [`ReactorPlugin`] como módulo WASM. Requiere la feature `reactor`.
#[macro_export]
macro_rules! export_reactor_plugin {
    ($make:expr) => {
        #[no_mangle]
        pub extern "C" fn alloc(len: u32) -> u32 {
            $crate::abi_alloc(len)
        }

        #[no_mangle]
        pub extern "C" fn mirada_on_event(ptr: u32, len: u32) {
            fn __ctor() -> $crate::SdkBox<dyn $crate::ReactorPlugin> {
                $crate::SdkBox::new($make)
            }
            $crate::reactor_entry(__ctor, ptr, len)
        }
    };
}
