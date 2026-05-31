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
    interpolate, NodeSnap, PlayerOverlays, PlayerSnap, PlayerStats, SceneSnapshot, SectorSnap,
    SegSnap, SnapshotPair, SpriteSnap, SubsectorSnap, WallSeg, WeaponSpriteSnap, NF_SUBSECTOR,
    NO_SECTOR, NO_SKY_PIC,
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
    /// Resuelve un `spritenum` al string 4-char de `sprnames[]`.
    /// `out` debe apuntar a un buffer de ≥ 5 bytes (4 chars + nul).
    fn supay_scene_sprite_name(spritenum: u16, out: *mut std::ffi::c_char) -> std::ffi::c_int;
    /// Resuelve la textura de pared en (wall_idx, side, kind) al
    /// nombre del lump TEXTURE1. `out` debe apuntar a ≥ 9 bytes.
    /// side: 0=front, 1=back. kind: 0=middle, 1=upper, 2=lower.
    fn supay_scene_wall_texture(
        wall_idx: std::ffi::c_int,
        side: std::ffi::c_int,
        kind: std::ffi::c_int,
        out: *mut std::ffi::c_char,
    ) -> std::ffi::c_int;
    /// Resuelve los offsets de textura del sidedef
    /// (`textureoffset`/`rowoffset`) para `(wall_idx, side)`. side=0/1.
    /// Devuelve 1 si OK + valores en `*xoff`/`*yoff`; 0 si fuera de rango.
    fn supay_scene_wall_offsets(
        wall_idx: std::ffi::c_int,
        side: std::ffi::c_int,
        xoff: *mut f32,
        yoff: *mut f32,
    ) -> std::ffi::c_int;
    /// Fase 3.13: árbol BSP del mapa.
    fn supay_scene_num_nodes() -> std::ffi::c_int;
    fn supay_scene_node(
        i: std::ffi::c_int,
        x: *mut f32,
        y: *mut f32,
        dx: *mut f32,
        dy: *mut f32,
        child_front: *mut u16,
        child_back: *mut u16,
    ) -> std::ffi::c_int;
    /// Fase 3.15: estado del psprite del arma del jugador (pistol,
    /// shotgun, etc.). Devuelve 0 si el psprite no tiene state activo
    /// (player dead, pre-mapa). `sx`/`sy` en coords nominales 320×200.
    fn supay_scene_player_weapon(
        spritenum: *mut u16,
        frame: *mut u8,
        sx: *mut f32,
        sy: *mut f32,
    ) -> std::ffi::c_int;
    /// Fase 3.16: variante extendida con `power_strength` (berserk).
    fn supay_scene_player_overlays_ext(
        damagecount: *mut std::ffi::c_int,
        bonuscount: *mut std::ffi::c_int,
        power_invuln: *mut std::ffi::c_int,
        power_radsuit: *mut std::ffi::c_int,
        power_strength: *mut std::ffi::c_int,
    ) -> std::ffi::c_int;
    /// Fase 3.16: estado del `psprites[ps_flash]` (muzzle flash overlay
    /// sobre el arma). Inactivo la mayor parte del tiempo.
    fn supay_scene_player_flash(
        spritenum: *mut u16,
        frame: *mut u8,
        sx: *mut f32,
        sy: *mut f32,
    ) -> std::ffi::c_int;
    /// Fase 3.20: stats vitales del jugador para el HUD inferior.
    /// `ammo` y `maxammo` apuntan a buffers `[i32; 4]`; `cards` a
    /// `[u8; 6]`. Devuelve 0 si el jugador no existe (pre-mapa) — todo
    /// el buffer queda en cero y el HUD se pinta hueco.
    fn supay_scene_player_stats(
        health: *mut std::ffi::c_int,
        armor_points: *mut std::ffi::c_int,
        armor_type: *mut std::ffi::c_int,
        ready_weapon: *mut std::ffi::c_int,
        ammo: *mut std::ffi::c_int,
        maxammo: *mut std::ffi::c_int,
        cards: *mut u8,
    ) -> std::ffi::c_int;

    /// Drena hasta `max` eventos de sonido del ring buffer C
    /// (`audio_stubs.c`) al array `out`. Devuelve cuántos copió.
    fn supay_sound_poll(out: *mut SupaySndEvent, max: std::ffi::c_int) -> std::ffi::c_int;

    /// Contador de generación de música (cambia en cada PlaySong/StopSong).
    fn supay_music_gen() -> std::ffi::c_uint;
    /// Drena el estado de música: copia el lump MUS a `out` si suena.
    fn supay_music_poll(
        out: *mut u8,
        max: std::ffi::c_int,
        out_len: *mut std::ffi::c_int,
        out_play: *mut std::ffi::c_int,
        out_loop: *mut std::ffi::c_int,
    ) -> std::ffi::c_uint;
}

/// Orden de música emitida por el motor: arrancar un lump MUS (con loop)
/// o parar. Ver [`DoomEngine::poll_music`].
#[derive(Clone, Debug)]
pub enum MusicCommand {
    /// Reproducir estos bytes MUS crudos; `looping` repite al terminar.
    Play { data: Vec<u8>, looping: bool },
    /// Detener la música actual.
    Stop,
}

/// Layout C del evento de sonido (`supay_snd_event` en `audio_stubs.c`).
/// `#[repr(C)]` para que el padding coincida byte-a-byte con el lado C.
#[repr(C)]
#[derive(Clone, Copy)]
struct SupaySndEvent {
    name: [std::ffi::c_char; 9],
    vol: std::ffi::c_int,
    sep: std::ffi::c_int,
}

/// Evento de sonido emitido por el motor en un tick: qué sfx disparar y
/// con qué volumen/balance. El consumidor (`supay-audio`) resuelve el
/// lump `DS<name>` del WAD y lo mezcla. Ver [`DoomEngine::poll_sounds`].
#[derive(Clone, Debug)]
pub struct SoundEvent {
    /// Nombre base del sfx (e.g. `"pistol"`). El lump real es
    /// `DS` + uppercase (e.g. `DSPISTOL`).
    pub name: String,
    /// Volumen 0..127.
    pub vol: u8,
    /// Separación estéreo 0..255 (128 ≈ centro; 0 izquierda, 255 derecha).
    pub sep: u8,
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
    /// Fase 4.1: última generación de música vista (`supay_music_gen`).
    /// `poll_music` la compara para detectar cambios sin copiar el lump.
    last_music_gen: u32,
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
            last_music_gen: 0,
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

    /// Drena los eventos de sonido que el motor encoló desde el último
    /// tick. Cada uno es un sfx a reproducir (lump `DS<name>`) con su
    /// volumen y balance estéreo. En modo stub devuelve vacío.
    ///
    /// Debe llamarse desde el mismo thread que [`Self::tick`] (el ring
    /// buffer C no es thread-safe; el host de Llimphi lo cumple porque
    /// tick y poll viven en el event loop).
    pub fn poll_sounds(&mut self) -> Vec<SoundEvent> {
        #[cfg(doomgeneric_stub)]
        {
            Vec::new()
        }
        #[cfg(not(doomgeneric_stub))]
        {
            const MAX: usize = 64;
            let mut raw = [SupaySndEvent {
                name: [0; 9],
                vol: 0,
                sep: 0,
            }; MAX];
            // SAFETY: raw vive en este stack frame; la fn C escribe a lo
            // sumo `MAX` entradas y devuelve cuántas.
            let n = unsafe { supay_sound_poll(raw.as_mut_ptr(), MAX as std::ffi::c_int) };
            let n = (n.max(0) as usize).min(MAX);
            let mut out = Vec::with_capacity(n);
            for ev in &raw[..n] {
                // name[9] null-terminated → String.
                let mut end = ev.name.len();
                for (i, &c) in ev.name.iter().enumerate() {
                    if c == 0 {
                        end = i;
                        break;
                    }
                }
                let bytes: Vec<u8> = ev.name[..end].iter().map(|&c| c as u8).collect();
                if let Ok(name) = String::from_utf8(bytes) {
                    if !name.is_empty() {
                        out.push(SoundEvent {
                            name,
                            vol: ev.vol.clamp(0, 127) as u8,
                            sep: ev.sep.clamp(0, 255) as u8,
                        });
                    }
                }
            }
            out
        }
    }

    /// Detecta si el motor cambió de música desde el último poll. Devuelve
    /// `Some(Play|Stop)` sólo cuando hay un cambio (arranque de nivel,
    /// `idmus`, victoria, etc.); `None` el resto de los ticks. En modo
    /// stub siempre `None`. Mismo thread que [`Self::tick`].
    pub fn poll_music(&mut self) -> Option<MusicCommand> {
        #[cfg(doomgeneric_stub)]
        {
            None
        }
        #[cfg(not(doomgeneric_stub))]
        {
            // Chequeo barato: ¿cambió la generación?
            let gen = unsafe { supay_music_gen() } as u32;
            if gen == self.last_music_gen {
                return None;
            }
            self.last_music_gen = gen;
            // Cambió → drenar el estado completo.
            const MAX: usize = 256 * 1024;
            let mut buf = vec![0u8; MAX];
            let mut len: std::ffi::c_int = 0;
            let mut play: std::ffi::c_int = 0;
            let mut looping: std::ffi::c_int = 0;
            // SAFETY: buf tiene MAX bytes; la fn C copia a lo sumo MAX.
            unsafe {
                supay_music_poll(
                    buf.as_mut_ptr(),
                    MAX as std::ffi::c_int,
                    &mut len,
                    &mut play,
                    &mut looping,
                );
            }
            if play != 0 {
                buf.truncate((len.max(0) as usize).min(MAX));
                Some(MusicCommand::Play {
                    data: buf,
                    looping: looping != 0,
                })
            } else {
                Some(MusicCommand::Stop)
            }
        }
    }

    /// Resuelve la textura de pared al nombre del lump.
    /// `side`: 0=front, 1=back (back es `None` cuando one-sided).
    /// `kind`: 0=middle, 1=upper, 2=lower.
    /// Devuelve `None` si no hay sidedef, no hay textura asignada
    /// (slot vacío con id 0), o estamos en modo stub.
    pub fn wall_texture(&self, wall_idx: u32, side: u8, kind: u8) -> Option<String> {
        #[cfg(doomgeneric_stub)]
        {
            let _ = (wall_idx, side, kind);
            None
        }
        #[cfg(not(doomgeneric_stub))]
        {
            let mut buf = [0i8; 9];
            // SAFETY: buf vive en este stack frame; la fn C escribe
            // hasta 9 bytes (8 chars + nul).
            let ok = unsafe {
                supay_scene_wall_texture(
                    wall_idx as std::ffi::c_int,
                    side as std::ffi::c_int,
                    kind as std::ffi::c_int,
                    buf.as_mut_ptr(),
                )
            };
            if ok == 0 {
                return None;
            }
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

    /// Resuelve un `spritenum_t` al string 4-char del sprite (e.g.
    /// `SPR_TROO=29 → "TROO"`). El renderer combina con `frame`+ángulo
    /// para encontrar el lump del sprite (e.g. `"TROOA1"`).
    pub fn sprite_name(&self, spritenum: u16) -> Option<String> {
        #[cfg(doomgeneric_stub)]
        {
            let _ = spritenum;
            None
        }
        #[cfg(not(doomgeneric_stub))]
        {
            let mut buf = [0i8; 5];
            // SAFETY: buf vive en este stack frame; la fn C escribe
            // hasta 5 bytes (4 chars + nul).
            let ok = unsafe { supay_scene_sprite_name(spritenum, buf.as_mut_ptr()) };
            if ok == 0 {
                return None;
            }
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
            // Texturas: leemos las 6 combinaciones (side × kind) ahora
            // que el wall está aceptado. supay_scene_wall_texture
            // devuelve 0 para slots vacíos — quedan como [0; 8].
            let mut textures = [[0u8; 8]; 6];
            let mut tex_x_offsets = [0.0_f32; 2];
            let mut tex_y_offsets = [0.0_f32; 2];
            for side in 0..2_u8 {
                for kind in 0..3_u8 {
                    let mut buf = [0i8; 9];
                    // SAFETY: buf válido; fn C escribe ≤9 bytes.
                    let tok = unsafe {
                        supay_scene_wall_texture(
                            i as std::ffi::c_int,
                            side as std::ffi::c_int,
                            kind as std::ffi::c_int,
                            buf.as_mut_ptr(),
                        )
                    };
                    if tok != 0 {
                        let idx = side as usize * 3 + kind as usize;
                        for j in 0..8 {
                            textures[idx][j] = buf[j] as u8;
                        }
                    }
                }
                // Offsets del sidedef. Una llamada por lado.
                let mut xoff = 0.0_f32;
                let mut yoff = 0.0_f32;
                // SAFETY: punteros a locales válidos.
                let ook = unsafe {
                    supay_scene_wall_offsets(
                        i as std::ffi::c_int,
                        side as std::ffi::c_int,
                        &mut xoff,
                        &mut yoff,
                    )
                };
                if ook != 0 {
                    tex_x_offsets[side as usize] = xoff;
                    tex_y_offsets[side as usize] = yoff;
                }
            }
            walls.push(WallSeg {
                x1,
                y1,
                x2,
                y2,
                front_sector: front,
                back_sector: back,
                flags,
                textures,
                tex_x_offsets,
                tex_y_offsets,
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
    // Nodos BSP (Fase 3.13). Vacío hasta que cargue el mapa.
    // SAFETY: idem.
    let n_nodes = unsafe { supay_scene_num_nodes() }.max(0) as usize;
    let mut nodes_vec = Vec::with_capacity(n_nodes);
    for i in 0..n_nodes {
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut dx = 0.0_f32;
        let mut dy = 0.0_f32;
        let mut child_front = 0_u16;
        let mut child_back = 0_u16;
        // SAFETY: punteros a locales válidos.
        let ok = unsafe {
            supay_scene_node(
                i as std::ffi::c_int,
                &mut x,
                &mut y,
                &mut dx,
                &mut dy,
                &mut child_front,
                &mut child_back,
            )
        };
        if ok != 0 {
            nodes_vec.push(NodeSnap {
                partition_x: x,
                partition_y: y,
                partition_dx: dx,
                partition_dy: dy,
                children: [child_front, child_back],
            });
        }
    }

    // SAFETY: idem.
    let sky_pic = unsafe { supay_scene_sky_pic() };

    // Player overlay counters (Fase 3.14 + 3.16 ext con berserk).
    let mut dmg = 0_i32;
    let mut bon = 0_i32;
    let mut p_inv = 0_i32;
    let mut p_rad = 0_i32;
    let mut p_str = 0_i32;
    // SAFETY: punteros a locales válidos.
    let _ = unsafe {
        supay_scene_player_overlays_ext(
            &mut dmg,
            &mut bon,
            &mut p_inv,
            &mut p_rad,
            &mut p_str,
        )
    };
    let player_overlays = PlayerOverlays {
        damage_count: dmg.max(0) as u16,
        bonus_count: bon.max(0) as u16,
        power_invuln: p_inv.max(0) as u32,
        power_radsuit: p_rad.max(0) as u32,
        power_strength: p_str.max(0) as u32,
    };

    // Psprite del arma (Fase 3.15) + flash overlay (Fase 3.16).
    let weapon = capture_psprite(false);
    let weapon_flash = capture_psprite(true);

    // Fase 3.20: stats vitales del jugador (HUD inferior).
    let player_stats = capture_player_stats();

    SceneSnapshot {
        tick,
        player,
        walls: Arc::from(walls),
        sectors: Arc::from(sects),
        sprites: Arc::from(sprs),
        subsectors: Arc::from(subs),
        segs: Arc::from(segs_vec),
        nodes: Arc::from(nodes_vec),
        sky_pic,
        player_overlays,
        weapon,
        weapon_flash,
        player_stats,
    }
}

/// Captura los stats del jugador (health/armor/ammo/keys) desde el motor.
/// Devuelve `Default` si el jugador no existe (pre-mapa) — el HUD se
/// pintará hueco.
#[cfg(not(doomgeneric_stub))]
fn capture_player_stats() -> PlayerStats {
    let mut health = 0_i32;
    let mut armor_points = 0_i32;
    let mut armor_type = 0_i32;
    let mut ready_weapon = 0_i32;
    let mut ammo = [0_i32; 4];
    let mut maxammo = [0_i32; 4];
    let mut cards = [0_u8; 6];
    // SAFETY: punteros a locales válidos en la frame actual. Los buffers
    // de 4 ints y 6 bytes matchean los `_Static_assert`-s de
    // `scene_export.c`.
    let _ = unsafe {
        supay_scene_player_stats(
            &mut health,
            &mut armor_points,
            &mut armor_type,
            &mut ready_weapon,
            ammo.as_mut_ptr(),
            maxammo.as_mut_ptr(),
            cards.as_mut_ptr(),
        )
    };
    PlayerStats {
        health,
        armor_points,
        armor_type: armor_type.max(0).min(u8::MAX as i32) as u8,
        ready_weapon: ready_weapon.max(0).min(u8::MAX as i32) as u8,
        ammo,
        max_ammo: maxammo,
        cards: [
            cards[0] != 0,
            cards[1] != 0,
            cards[2] != 0,
            cards[3] != 0,
            cards[4] != 0,
            cards[5] != 0,
        ],
    }
}

/// Captura uno de los dos psprites del jugador. `is_flash=false` →
/// `ps_weapon`; `true` → `ps_flash`. Devuelve `Default` (inactivo) si
/// el motor no expone state activo para ese slot.
#[cfg(not(doomgeneric_stub))]
fn capture_psprite(is_flash: bool) -> WeaponSpriteSnap {
    let mut sprite = 0_u16;
    let mut frame = 0_u8;
    let mut sx = 0.0_f32;
    let mut sy = 0.0_f32;
    // SAFETY: punteros a locales.
    let ok = if is_flash {
        unsafe { supay_scene_player_flash(&mut sprite, &mut frame, &mut sx, &mut sy) }
    } else {
        unsafe { supay_scene_player_weapon(&mut sprite, &mut frame, &mut sx, &mut sy) }
    };
    WeaponSpriteSnap {
        active: ok != 0,
        sprite,
        frame,
        sx,
        sy,
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
        // El modo stub no expone pitch — el host puede sobreescribirlo
        // post-capture si quiere validar mouse-look sin vendor.
        view_pitch: 0.0,
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
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
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
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
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
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
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
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
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
        nodes: Arc::from(Vec::<NodeSnap>::new()),
        sky_pic: NO_SKY_PIC,
        // Stub: sin overlays. El red flash sintético se puede testear en
        // modo real moviéndose hacia un enemigo.
        player_overlays: PlayerOverlays::default(),
        // Stub: sin arma — no hay psprite que mostrar sin jugador real.
        weapon: WeaponSpriteSnap::default(),
        weapon_flash: WeaponSpriteSnap::default(),
        // Stub: HUD hueco (default = todos los valores en cero).
        player_stats: PlayerStats::default(),
    }
}
