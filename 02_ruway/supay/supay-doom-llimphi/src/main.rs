//! `supay-doom-llimphi` — Fase 1 del proyecto supay.
//!
//! Doom real corriendo dentro de una ventana Llimphi. La simulación
//! C (doomgeneric) avanza un tick por cada `Msg::Tick` a 35 Hz; el
//! framebuffer 320×200 ARGB que el motor llena se pinta como
//! `View::image` con aspect-fit. Las pulsaciones de teclado de
//! Llimphi se traducen a códigos Doom (`KEY_FIRE`, `KEY_USE`, los
//! `KEY_*ARROW`) y se encolan en el motor con `push_key`.
//!
//! ## Controles
//!
//! - **WASD / flechas** — moverse / girar (los manda como
//!   `KEY_UPARROW`, etc.; doomgeneric los maneja con su config
//!   nativa de Doom).
//! - **,/.** — strafe izquierdo / derecho.
//! - **Ctrl o Enter** — disparar (`KEY_FIRE`).
//! - **Space** — usar / abrir puertas (`KEY_USE`).
//! - **Shift** — correr.
//! - **Tab** — mapa.
//! - **Esc** — menú del juego (Doom abre su menú; segundo Esc cierra ventana).
//! - **Y/N** — confirmaciones del menú.
//! - **PageUp/PageDown/Home** — mirar arriba / abajo / resetear horizonte.
//! - **Arrastrar con el mouse** — mouse-look vertical (mueve el horizonte;
//!   Doom no tiene aim vertical real, es y-shear cosmético).
//!
//! ## Requiere
//!
//! 1. Vendoring de doomgeneric. Ver `supay-core/vendor/README.md`.
//! 2. Un WAD legalmente distribuible (DOOM1.WAD shareware) en el
//!    cwd desde donde se corre. Sin WAD, doomgeneric aborta.
//!
//! En modo stub (sin vendor), la app arranca igual y pinta un mensaje
//! explicando qué falta — útil para verificar el plumbing Llimphi.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, JustifyContent, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use llimphi_theme::Theme;
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};

use app_bus::{AppMenu, Menu, MenuItem};

use supay_core::{
    keys, DoomEngine, SceneSnapshot, SnapshotPair, WallSeg, DOOM_HEIGHT, DOOM_PIXELS, DOOM_WIDTH,
};
use supay_render_llimphi::{scene_view, RenderConfig, WadAtlas};
use supay_wad::Wad;

// =====================================================================
// Paleta Supay — riffs sobre la identidad del Doom clásico:
// negro carbón de cabina, crimson de sangre BFG, ámbar de chasis, gris
// hueso del HUD. Pensada para verse igual sobre wayland o framebuffer.
// =====================================================================
const COLOR_BG_ABYSS: Color = Color::from_rgba8(0, 0, 0, 255);
const COLOR_BG_PANEL: Color = Color::from_rgba8(12, 8, 8, 255);
const COLOR_BG_SUNKEN: Color = Color::from_rgba8(6, 4, 4, 255);
const COLOR_CRIMSON: Color = Color::from_rgba8(180, 30, 30, 255);
const COLOR_CRIMSON_DEEP: Color = Color::from_rgba8(90, 14, 14, 255);
const COLOR_AMBER: Color = Color::from_rgba8(232, 168, 76, 255);
const COLOR_BONE: Color = Color::from_rgba8(216, 204, 188, 255);
const COLOR_DUST: Color = Color::from_rgba8(132, 124, 116, 255);
const COLOR_GREEN_CRT: Color = Color::from_rgba8(140, 188, 96, 255);
const COLOR_RULE: Color = Color::from_rgba8(48, 16, 16, 255);

const HEADER_HEIGHT: f32 = 44.0;
const FOOTER_HEIGHT: f32 = 24.0;
const FRAME_THICKNESS: f32 = 4.0;

const TICK_HZ: u64 = 35; // canónico de Doom
const TICK_MS: u64 = 1_000 / TICK_HZ;
const TICK_PERIOD: Duration = Duration::from_millis(TICK_MS);
/// Frame rate del renderer 3D — desacoplado del tick del motor. A 60 Hz
/// la interpolación entre snapshots se vuelve visible (cada tick de
/// 28.5 ms cubre ~1.7 frames de 16.6 ms, así el alpha barre 0→1 en pasos
/// de ~0.58). En Framebuffer mode el redraw también ayuda al smoothing
/// del scaling del FB.
const FRAME_MS: u64 = 1_000 / 60;

/// Modo de visualización. Toggleable con F3 — Fase 1 vs Fase 3.0.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    /// Pinta el framebuffer 320×200 ARGB del motor original
    /// (renderer software clásico de Doom).
    Framebuffer,
    /// Renderer 3D moderno consumiendo snapshots interpolados
    /// (Fase 3.0 — supay-render-llimphi sobre vello).
    Scene3d,
}

struct Model {
    engine: DoomEngine,
    /// Fase 4.0: motor de audio (SFX). `None` si no hay dispositivo de
    /// salida o el WAD no cargó — el juego corre mudo sin romperse. Cada
    /// `Msg::Tick` drena `engine.poll_sounds()` y se los pasa.
    audio: Option<supay_audio::AudioEngine>,
    tick: u64,
    /// Framebuffer del último frame del motor, ya en formato Rgba8
    /// (lo que `View::image` espera).
    framebuffer_rgba: Vec<u8>,
    /// Fase 2: par de snapshots (prev + next) capturados desde
    /// doomgeneric — el renderer 3D (Fase 3) interpola entre los dos
    /// para correr más rápido que 35 Hz.
    snapshots: SnapshotPair,
    /// Instante real del último `Msg::Tick` — el renderer 3D lo usa
    /// para calcular `alpha = elapsed / TICK_PERIOD` al pintar.
    last_tick_at: Instant,
    view_mode: ViewMode,
    /// Fase 3.3: atlas WAD compartido con el renderer. `None` si el WAD
    /// no pudo cargarse (modo stub o doom1.wad ausente en cwd); en ese
    /// caso el renderer cae a las paletas hardcoded de 3.1.
    atlas: Option<Arc<WadAtlas>>,
    /// Conjunto de pic_idx ya registrados en el atlas. Cuando aparece
    /// un sector con un pic_idx nuevo, lo resolvemos vía
    /// `engine.flat_name` y lo añadimos.
    known_pics: std::collections::HashSet<u16>,
    /// Análogo para spritenums (Fase 3.4): cada vez que un mobj nuevo
    /// aparece, registramos su 4-char base name en el atlas.
    known_sprites: std::collections::HashSet<u16>,
    /// Fase 3.17: pitch cosmético del viewer (mouse-look). Doom no lo
    /// conoce — lo aplica sólo el rasterizador como y-shear sobre la
    /// proyección y el sky backdrop. PageUp/PageDown ajustan; Home
    /// resetea a 0. Clampeado a ±PITCH_MAX = π/3.
    view_pitch: f32,
    /// Fase 3.19: crosshair central on/off (F4). Modernización pura —
    /// Doom clásico no lo tiene; en Llimphi lo prendemos por default
    /// porque la sensación de FPS contemporáneo lo da por sentado.
    show_crosshair: bool,
    /// Fase 3.19: intensidad de la viñeta de cabina (F5 cicla 0 → 0.55
    /// → 0.9). Misma justificación que el crosshair: cosmético total,
    /// no toca la simulación, sólo el rasterizador.
    vignette_strength: f32,
    /// Fase 3.20: HUD inferior on/off (F6). Por default prendido — un
    /// FPS sin status bar visible se siente "incompleto". Pero algunos
    /// jugadores prefieren la inmersión sin chrome — el toggle queda.
    show_hud: bool,
    /// Fase 3.21: sombras de mobjs en el piso on/off (F7). Default on.
    sprite_shadows: bool,
    /// Fase 3.22: muzzle world light on/off (F8). Default on.
    muzzle_world_light: bool,
    /// Fase 3.22: instante del último frame `FF_FULLBRIGHT` detectado
    /// en `weapon` o `weapon_flash`. `None` ⇒ nunca disparó / ya decayó.
    /// El alpha del boost se computa cada frame como
    /// `max(0, 1 - elapsed/MUZZLE_DECAY_SECS)`.
    muzzle_glow_at: Option<Instant>,
    /// Fase 3.23: oclusión sectorial del muzzle boost on/off (F9). Default
    /// on — el fogonazo respeta paredes (sólo ilumina el cuarto actual y
    /// los conectados directamente). Off vuelve al 3.22 (ilumina todo lo
    /// que está dentro del radio, ignorando paredes).
    muzzle_occlusion: bool,
    /// Fase 3.26: luces dinámicas desde mobjs FF_FULLBRIGHT on/off (F10).
    /// Default on — proyectiles (imp fireball, plasma, rocket, BFG),
    /// puffs de impacto y frames de explosión irradian un boost cálido
    /// sobre paredes/pisos/techos/sprites cercanos. Off vuelve al 3.25
    /// (sólo el muzzle del jugador ilumina el mundo).
    world_lights_enabled: bool,
    /// Fase 3.28: rim-light del arma desde world lights (F11). Default on
    /// — el sprite del psprite recoge tinte RGB ambiente (torch azul →
    /// pistola azulada, fireball pasando cerca → rim rojizo). Off vuelve
    /// al 3.27 (arma sólo recibe el `light_level` del sector como
    /// shading scalar).
    weapon_rim_light: bool,
    /// Fase 3.46: decals efímeros de impacto. Se siembran cuando un
    /// sprite PUFF (bala contra pared) o BLUD (sangre) aparece nuevo, y
    /// se desvanecen con la edad.
    decals: Vec<HostDecal>,
    /// Clasificación cacheada de spritenums → tipo de decal (sólo PUFF
    /// y BLUD producen marca; el resto no). Resuelto vía `sprite_name`
    /// la primera vez que se ve cada spritenum.
    decal_kind: std::collections::HashMap<u16, DecalKind>,
    /// Spritenums ya clasificados (evita reconsultar `sprite_name`).
    checked_decal_spritenums: std::collections::HashSet<u16>,
    /// Posiciones XY de los sprites de impacto vistos el tick anterior —
    /// dedup posicional para no sembrar un decal por tick mientras el
    /// puff vive (≈4 ticks).
    prev_impacts: Vec<(f32, f32)>,
    /// Tema activo — sólo viste la barra de menú y los overlays. El
    /// juego (framebuffer / renderer 3D) pinta con su propia paleta
    /// hardcoded; el tema no lo toca.
    theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada dentro del dropdown abierto (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal.
    menu_anim: Tween<f32>,
    /// Menú contextual del juego: ancla `(x, y)` en coords de ventana.
    /// `None` cerrado. No hay objetos seleccionables ni texto editable
    /// — el contextual expone las acciones de juego (disparar / usar /
    /// vista), no edición.
    context_menu: Option<(f32, f32)>,
}

/// Fase 3.46: tipo de marca de impacto.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DecalKind {
    /// Scorch oscuro de bala contra superficie (PUFF).
    Scorch,
    /// Splat de sangre (BLUD).
    Blood,
}

/// Fase 3.46: marca persistida en el host con su edad. El `alpha` que
/// recibe el renderer se computa de `ttl / DECAL_TTL`.
struct HostDecal {
    x: f32,
    y: f32,
    z: f32,
    ttl: u32,
    color: (u8, u8, u8),
    radius: f32,
    /// Fase 3.47: tangente del lineseg impactado para apoyar el decal
    /// plano. `(0, 0)` ⇒ billboard.
    tangent: (f32, f32),
    /// Fase 3.48: impacto contra piso/techo ⇒ charco horizontal.
    horizontal: bool,
    /// Fase 3.52: span del lineseg impactado (offsets firmados a lo largo
    /// de la tangente hasta sus extremos) para recortar el decal al borde
    /// de la pared. `None` ⇒ billboard / charco / sin span.
    wall_span: Option<(f32, f32)>,
}

/// Fase 3.47/3.52: tangente unitaria del lineseg más cercano a `(x, y)`
/// dentro de `max_dist`, junto con el **span** del segmento — offsets
/// firmados `(s_min, s_max)` a lo largo de la tangente desde el impacto
/// hasta los dos extremos del lineseg. El renderer usa el span para
/// recortar el decal al borde de la pared (Fase 3.52). Devuelve
/// `((0, 0), None)` si ninguna pared está cerca (sangre en el aire) ⇒ el
/// renderer cae al billboard.
fn nearest_wall_seg(
    walls: &[WallSeg],
    x: f32,
    y: f32,
    max_dist: f32,
) -> ((f32, f32), Option<(f32, f32)>) {
    let mut best_d2 = max_dist * max_dist;
    let mut best = ((0.0_f32, 0.0_f32), None);
    for w in walls {
        let dx = w.x2 - w.x1;
        let dy = w.y2 - w.y1;
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 {
            continue;
        }
        let t = (((x - w.x1) * dx + (y - w.y1) * dy) / len2).clamp(0.0, 1.0);
        let px = w.x1 + t * dx;
        let py = w.y1 + t * dy;
        let d2 = (px - x) * (px - x) + (py - y) * (py - y);
        if d2 < best_d2 {
            best_d2 = d2;
            let inv = len2.sqrt().recip();
            let (tx, ty) = (dx * inv, dy * inv);
            // Offsets a lo largo de la tangente desde el impacto a cada
            // extremo. Como la tangente apunta de v1 a v2, `s_to_v1 ≤
            // s_to_v2` siempre ⇒ par (min, max).
            let s_to_v1 = (w.x1 - x) * tx + (w.y1 - y) * ty;
            let s_to_v2 = (w.x2 - x) * tx + (w.y2 - y) * ty;
            best = ((tx, ty), Some((s_to_v1, s_to_v2)));
        }
    }
    best
}

/// Distancia máxima (unidades) impacto→pared para apoyar el decal plano.
/// Más allá, billboard (sangre/impacto lejos de cualquier pared).
const DECAL_WALL_SNAP_DIST: f32 = 32.0;

/// Fase 3.48: holgura (unidades) para considerar un impacto "contra el
/// piso o el techo" del sector ⇒ decal horizontal en vez de pared.
const DECAL_PLANE_SNAP: f32 = 12.0;

/// Vida de un decal en ticks (35 Hz). ~6 s antes de desvanecerse.
const DECAL_TTL: u32 = 210;
/// Máximo de decals vivos; al llenarse, se descarta el más viejo.
const MAX_DECALS: usize = 64;
/// Radio² (unidades²) para el dedup posicional de impactos entre ticks.
/// ~12 u de radio — puffs de un mismo ráfaga cercanos colapsan a uno.
const DECAL_DEDUP_EPS2: f32 = 144.0;

/// Tiempo de decaimiento del fogonazo del arma — el boost cae de 1.0 a 0
/// en este intervalo. ~160 ms cubre 5-6 ticks Doom (35 Hz), suficiente
/// para que se lea como "destello" sin durar demasiado.
const MUZZLE_DECAY_SECS: f32 = 0.16;

/// Pasos del cicle F5 para la viñeta — off / sutil (default) / fuerte.
const VIGNETTE_STEPS: [f32; 3] = [0.0, 0.55, 0.9];

/// Rango sano del pitch en el host (mismo clamp que el renderer).
const PITCH_MAX: f32 = std::f32::consts::FRAC_PI_3;
/// Paso del pitch por tap de PageUp/PageDown (en radianes). ~6° por tap.
const PITCH_STEP: f32 = 0.105;
/// Sensibilidad del mouse-look por drag (radianes de pitch por pixel de
/// desplazamiento vertical). Un arrastre de ~300 px cubre el rango ±60°.
/// El clamp final vive en `Msg::PitchDelta`. Doom no tiene aim vertical
/// real: esto mueve el horizonte (y-shear cosmético del renderer).
const MOUSE_LOOK_SENS: f32 = 0.0035;

#[derive(Clone)]
enum Msg {
    /// Tick del motor Doom (35 Hz). Avanza la simulación + captura
    /// snapshot.
    Tick,
    /// Redraw a 60 Hz para que el closure de paint_with recompute
    /// `alpha = elapsed / TICK_PERIOD` y la interpolación entre
    /// snapshots sea visible. No toca el model más allá del rebuild.
    Frame,
    Key(KeyEvent),
    ToggleViewMode,
    /// Fase 3.17: cambio de pitch cosmético. `delta` se suma al
    /// `view_pitch` actual y se clampea a ±PITCH_MAX. `delta=0.0` con
    /// `reset=true` fuerza pitch=0.
    PitchDelta {
        delta: f32,
        reset: bool,
    },
    /// Fase 3.19: alterna el crosshair central (F4).
    ToggleCrosshair,
    /// Fase 3.19: cicla la intensidad de la viñeta (F5) por VIGNETTE_STEPS.
    CycleVignette,
    /// Fase 3.20: alterna el HUD inferior (F6).
    ToggleHud,
    /// Fase 3.21: alterna sombras de sprites en el piso (F7).
    ToggleSpriteShadows,
    /// Fase 3.22: alterna el muzzle world light (F8).
    ToggleMuzzleLight,
    /// Fase 3.23: alterna la oclusión sectorial del muzzle (F9).
    ToggleMuzzleOcclusion,
    /// Fase 3.26: alterna las world lights de mobjs FF_FULLBRIGHT (F10).
    ToggleWorldLights,
    /// Fase 3.28: alterna el rim-light del arma desde world lights (F11).
    ToggleWeaponRimLight,
    Quit,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra o en el contextual — se traduce al
    /// `Msg` real existente (toggle de vista, tecla del motor, etc.).
    MenuCommand(String),
    /// Navegación por teclado dentro del dropdown: +1 baja, -1 sube.
    MenuNav(i32),
    /// Enter sobre la fila resaltada del dropdown.
    MenuActivate,
    /// Tick de la animación del menú (sólo re-render).
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click sobre el juego → abre el menú contextual anclado en
    /// `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cicla el preset de tema (sólo cosmético: barra de menú + overlays).
    CycleTheme,
    /// Inyecta un tap (press+release) de una tecla del motor Doom — lo
    /// usan los comandos de menú "Disparar" / "Usar" / "Menú del juego".
    DoomKeyTap(u8),
}

struct Supay;

impl App for Supay {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "supay · fase 1 · doom"
    }

    fn initial_size() -> (u32, u32) {
        // 4× la resolución original de Doom = 1280×800 — el aspect-fit
        // de `View::image` se encarga del resto. Bordes negros si la
        // relación no encaja.
        (1280, 800)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        handle.spawn_periodic(Duration::from_millis(FRAME_MS), || Msg::Frame);
        // Args estilo argv. `-iwad doom1.wad` busca el WAD en cwd.
        // Fase 4.0: quitamos `-nosound`. Ese flag sólo lo honra
        // `i_sound.c`, que está EXCLUIDO del build (arrastra SDL_mixer);
        // nuestro `audio_stubs.c` provee la API. `S_StartSound`
        // (s_sound.c) llega a `I_StartSound` sin guard, así que ahora
        // ese stub graba el evento (lump + vol + sep) en un ring buffer
        // que drenamos cada tick y reproducimos con `supay-audio`. El
        // stub no derefencia ningún lump → sin el segfault que motivó
        // el flag histórico.
        let args = vec![
            "doomgeneric".to_string(),
            "-iwad".to_string(),
            "doom1.wad".to_string(),
        ];
        // Cargamos el WAD desde el mismo path que pasamos al motor.
        // Si falla (no existe, mal formato), seguimos sin atlas — el
        // renderer cae a las paletas hardcoded de 3.1 sin romperse.
        let atlas = match Wad::open("doom1.wad") {
            Ok(wad) => Some(Arc::new(WadAtlas::new(wad, HashMap::new()))),
            Err(e) => {
                eprintln!("supay: WAD no cargado ({e}) — renderer 3D usará paletas fallback");
                None
            }
        };
        // Fase 4.0: motor de audio. Carga un segundo `Wad` (el del atlas
        // lo consumió `WadAtlas::new`) y abre el dispositivo de salida
        // por defecto. Si no hay WAD o no hay device, queda `None` y el
        // juego corre mudo — el sink es best-effort.
        let audio = match Wad::open("doom1.wad") {
            Ok(wad) => match supay_audio::AudioEngine::new(wad) {
                Ok(eng) => Some(eng),
                Err(e) => {
                    eprintln!("supay: audio no disponible ({e}) — juego mudo");
                    None
                }
            },
            Err(_) => None,
        };
        Model {
            engine: DoomEngine::new(args),
            audio,
            tick: 0,
            framebuffer_rgba: vec![0; DOOM_PIXELS * 4],
            snapshots: SnapshotPair::new(),
            last_tick_at: Instant::now(),
            view_mode: ViewMode::Framebuffer,
            atlas,
            known_pics: std::collections::HashSet::new(),
            known_sprites: std::collections::HashSet::new(),
            view_pitch: 0.0,
            show_crosshair: true,
            vignette_strength: VIGNETTE_STEPS[1],
            show_hud: true,
            sprite_shadows: true,
            muzzle_world_light: true,
            muzzle_glow_at: None,
            muzzle_occlusion: true,
            world_lights_enabled: true,
            weapon_rim_light: true,
            decals: Vec::new(),
            decal_kind: std::collections::HashMap::new(),
            checked_decal_spritenums: std::collections::HashSet::new(),
            prev_impacts: Vec::new(),
            theme: Theme::dark(),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
        }
    }

    fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
        if e.state == KeyState::Pressed {
            // Con el menú principal abierto las flechas navegan: ←/→ cambian
            // de menú raíz (con wrap), ↑/↓ mueven la fila activa, Enter
            // ejecuta y Esc cierra. Tiene prioridad y consume la tecla antes
            // de que llegue al motor de Doom.
            if let Some(mi) = model.menu_open {
                let n = app_menu(model).menus.len().max(1);
                match &e.key {
                    Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowLeft) => {
                        return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                    }
                    Key::Named(NamedKey::ArrowRight) => {
                        return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                    }
                    Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                    Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                    Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                    _ => return None,
                }
            }
            // Esc cierra primero el contextual abierto antes de dejar que
            // llegue al motor como KEY_ESCAPE.
            if matches!(&e.key, Key::Named(NamedKey::Escape)) && model.context_menu.is_some() {
                return Some(Msg::CloseMenus);
            }
            if matches!(&e.key, Key::Named(NamedKey::F12)) {
                // F12 cierra la ventana — Esc lo manejamos como KEY_ESCAPE del juego.
                return Some(Msg::Quit);
            }
            if matches!(&e.key, Key::Named(NamedKey::F3)) {
                // F3 alterna framebuffer ↔ renderer 3D (Fase 1 ↔ Fase 3.0).
                return Some(Msg::ToggleViewMode);
            }
            // Fase 3.19: F4 alterna crosshair, F5 cicla la viñeta.
            if matches!(&e.key, Key::Named(NamedKey::F4)) {
                return Some(Msg::ToggleCrosshair);
            }
            if matches!(&e.key, Key::Named(NamedKey::F5)) {
                return Some(Msg::CycleVignette);
            }
            if matches!(&e.key, Key::Named(NamedKey::F6)) {
                return Some(Msg::ToggleHud);
            }
            if matches!(&e.key, Key::Named(NamedKey::F7)) {
                return Some(Msg::ToggleSpriteShadows);
            }
            if matches!(&e.key, Key::Named(NamedKey::F8)) {
                return Some(Msg::ToggleMuzzleLight);
            }
            if matches!(&e.key, Key::Named(NamedKey::F9)) {
                return Some(Msg::ToggleMuzzleOcclusion);
            }
            if matches!(&e.key, Key::Named(NamedKey::F10)) {
                return Some(Msg::ToggleWorldLights);
            }
            if matches!(&e.key, Key::Named(NamedKey::F11)) {
                return Some(Msg::ToggleWeaponRimLight);
            }
            // Fase 3.17: mouse-look cosmético. PageUp = mirar arriba,
            // PageDown = mirar abajo, Home = resetear horizonte. No pasan
            // al motor (Doom no usa estas teclas) y sólo afectan el
            // renderer 3D.
            if matches!(&e.key, Key::Named(NamedKey::PageUp)) {
                return Some(Msg::PitchDelta { delta: PITCH_STEP, reset: false });
            }
            if matches!(&e.key, Key::Named(NamedKey::PageDown)) {
                return Some(Msg::PitchDelta { delta: -PITCH_STEP, reset: false });
            }
            if matches!(&e.key, Key::Named(NamedKey::Home)) {
                return Some(Msg::PitchDelta { delta: 0.0, reset: true });
            }
        }
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Quit => handle.quit(),
            Msg::Tick => {
                m.tick = m.tick.wrapping_add(1);
                m.engine.tick();
                // Fase 4.0: drenar los sfx que el motor encoló este tick.
                // Fase 4.1: cambio de música (arranque de nivel, idmus, etc.).
                let sounds = m.engine.poll_sounds();
                let music = m.engine.poll_music();
                refresh_framebuffer(&mut m);
                // Fase 2: snapshot tras cada tick.
                let mut snap = m.engine.capture_scene(m.tick);
                // Fase 3.17: el motor C deja `view_pitch=0`; lo inyectamos
                // post-capture para que el renderer 3D haga y-shear sobre
                // este snapshot y los próximos interpolados.
                snap.player.view_pitch = m.view_pitch;
                // Audio: reproducción + acústica. Se hace tras capturar el
                // snapshot porque la oclusión (Fase 4.5) necesita la geometría
                // de la escena y la posición del oyente de este mismo tick.
                if let Some(audio) = m.audio.as_mut() {
                    let (lx, ly) = (snap.player.x, snap.player.y);
                    for ev in sounds {
                        // Fase 4.5: oclusión geométrica de los sfx con origen
                        // (los emitidos por el jugador no traen pos → 0, secos).
                        // Fase 4.7: distancia fuente→oyente para la absorción
                        // de aire (sin pos → 0, sin filtrar).
                        let (occ, dist) = ev.pos.map_or((0.0, 0.0), |(sx, sy)| {
                            let d = ((sx - lx).powi(2) + (sy - ly).powi(2)).sqrt();
                            (snap.occlusion(lx, ly, sx, sy), d)
                        });
                        audio.play(&ev.name, ev.vol, ev.sep, occ, dist);
                    }
                    match music {
                        Some(supay_core::MusicCommand::Play { data, looping }) => {
                            audio.play_music(&data, looping)
                        }
                        Some(supay_core::MusicCommand::Stop) => audio.stop_music(),
                        None => {}
                    }
                    // Fase 4.3: acústica por sector — el reverb se ajusta al
                    // tamaño/apertura del cuarto donde está el jugador.
                    audio.set_ambience(ambience_for(&snap));
                }
                // Fase 3.3: registrar en el atlas cualquier pic_idx
                // nuevo que aparezca en sectores. WadAtlas usa interior
                // mutability — el Arc compartido con el renderer ve
                // los nombres nuevos sin necesidad de reconstruirlo.
                if let Some(atlas) = m.atlas.as_ref() {
                    for sec in snap.sectors.iter() {
                        for pic in [sec.floor_pic, sec.ceiling_pic] {
                            if m.known_pics.insert(pic) {
                                if let Some(name) = m.engine.flat_name(pic) {
                                    atlas.set_flat_name(pic, name);
                                }
                            }
                        }
                    }
                    for spr in snap.sprites.iter() {
                        if m.known_sprites.insert(spr.sprite) {
                            if let Some(name) = m.engine.sprite_name(spr.sprite) {
                                atlas.set_sprite_name(spr.sprite, name);
                            }
                        }
                    }
                    // Fase 3.59: el psprite del arma en mano (y su flash) NO
                    // está en `snap.sprites` — su spritenum (SPR_PISG, etc.)
                    // hay que registrarlo aparte o `draw_weapon_sprite` no
                    // resuelve el patch y el arma nunca se dibuja.
                    for ws in [
                        snap.weapon.active.then_some(snap.weapon.sprite),
                        snap.weapon_flash.active.then_some(snap.weapon_flash.sprite),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if m.known_sprites.insert(ws) {
                            if let Some(name) = m.engine.sprite_name(ws) {
                                atlas.set_sprite_name(ws, name);
                            }
                        }
                    }
                }
                // Fase 3.22: detectamos un fogonazo del arma vía bit
                // FF_FULLBRIGHT (0x80) en `weapon.frame` (pistol fire
                // frame "PISGB") o `weapon_flash.frame` (chaingun/plasma/
                // BFG/shotgun, que usan el slot ps_flash). Cuando lo
                // vemos, resetamos `muzzle_glow_at` para que el render
                // calcule el alpha del boost por elapsed.
                let weapon_bright = snap.weapon.active && (snap.weapon.frame & 0x80) != 0;
                let flash_bright =
                    snap.weapon_flash.active && (snap.weapon_flash.frame & 0x80) != 0;
                if weapon_bright || flash_bright {
                    m.muzzle_glow_at = Some(Instant::now());
                }
                // Fase 3.46: decals de impacto. (1) clasificamos cada
                // spritenum nuevo (PUFF→scorch, BLUD→sangre) una sola vez.
                for spr in snap.sprites.iter() {
                    if m.checked_decal_spritenums.insert(spr.sprite) {
                        if let Some(name) = m.engine.sprite_name(spr.sprite) {
                            let up = name.to_ascii_uppercase();
                            let kind = if up == "PUFF" {
                                Some(DecalKind::Scorch)
                            } else if up == "BLUD" {
                                Some(DecalKind::Blood)
                            } else {
                                None
                            };
                            if let Some(k) = kind {
                                m.decal_kind.insert(spr.sprite, k);
                            }
                        }
                    }
                }
                // (2) posiciones de impacto presentes este tick (+ sector
                // para decidir piso/techo vs pared).
                let impacts: Vec<(f32, f32, f32, DecalKind, u32)> = snap
                    .sprites
                    .iter()
                    .filter_map(|spr| {
                        m.decal_kind
                            .get(&spr.sprite)
                            .map(|&k| (spr.x, spr.y, spr.z, k, spr.sector))
                    })
                    .collect();
                // (3) envejecer los decals vivos.
                m.decals.retain_mut(|d| {
                    d.ttl = d.ttl.saturating_sub(1);
                    d.ttl > 0
                });
                // (4) sembrar los impactos nuevos (no vistos el tick
                // anterior, dedup posicional).
                for &(x, y, z, k, sector) in &impacts {
                    let is_new = !m.prev_impacts.iter().any(|&(px, py)| {
                        let dx = px - x;
                        let dy = py - y;
                        dx * dx + dy * dy < DECAL_DEDUP_EPS2
                    });
                    if !is_new {
                        continue;
                    }
                    if m.decals.len() >= MAX_DECALS {
                        m.decals.remove(0);
                    }
                    let (color, radius) = match k {
                        DecalKind::Scorch => ((24, 21, 18), 5.0),
                        DecalKind::Blood => ((104, 12, 12), 5.0),
                    };
                    // Fase 3.48: si el impacto cae cerca del piso o del
                    // techo de su sector, el decal yace horizontal (charco).
                    // Si no, Fase 3.47: lo apoyamos sobre la pared más
                    // cercana (tangente); sin pared, billboard.
                    let horizontal = snap.sectors.get(sector as usize).is_some_and(|s| {
                        z <= s.floor_height + DECAL_PLANE_SNAP
                            || z >= s.ceiling_height - DECAL_PLANE_SNAP
                    });
                    let (tangent, wall_span) = if horizontal {
                        ((0.0, 0.0), None)
                    } else {
                        nearest_wall_seg(&snap.walls, x, y, DECAL_WALL_SNAP_DIST)
                    };
                    m.decals.push(HostDecal {
                        x,
                        y,
                        z,
                        ttl: DECAL_TTL,
                        color,
                        radius,
                        tangent,
                        horizontal,
                        wall_span,
                    });
                }
                m.prev_impacts = impacts.iter().map(|&(x, y, _, _, _)| (x, y)).collect();
                m.snapshots.push(snap);
                m.last_tick_at = Instant::now();
            }
            Msg::Frame => {
                // No-op a nivel de modelo. Existe sólo para que Llimphi
                // dispare un view rebuild + redraw y el closure de
                // paint_with recompute `alpha` desde Instant::now() —
                // así la interpolación entre snapshots se vuelve
                // visible aún cuando no haya entrada del usuario.
            }
            Msg::Key(e) => {
                if let Some(doom_key) = translate_key(&e.key, e.text.as_deref()) {
                    let pressed = e.state == KeyState::Pressed;
                    m.engine.push_key(pressed, doom_key);
                }
            }
            Msg::ToggleViewMode => {
                m.view_mode = match m.view_mode {
                    ViewMode::Framebuffer => ViewMode::Scene3d,
                    ViewMode::Scene3d => ViewMode::Framebuffer,
                };
            }
            Msg::PitchDelta { delta, reset } => {
                m.view_pitch = if reset {
                    0.0
                } else {
                    (m.view_pitch + delta).clamp(-PITCH_MAX, PITCH_MAX)
                };
            }
            Msg::ToggleCrosshair => {
                m.show_crosshair = !m.show_crosshair;
            }
            Msg::CycleVignette => {
                // Buscamos el step actual y avanzamos al siguiente; si
                // el valor no matchea ninguno (raro), arrancamos en off.
                let idx = VIGNETTE_STEPS
                    .iter()
                    .position(|&s| (s - m.vignette_strength).abs() < 1e-3)
                    .unwrap_or(0);
                m.vignette_strength = VIGNETTE_STEPS[(idx + 1) % VIGNETTE_STEPS.len()];
            }
            Msg::ToggleHud => {
                m.show_hud = !m.show_hud;
            }
            Msg::ToggleSpriteShadows => {
                m.sprite_shadows = !m.sprite_shadows;
            }
            Msg::ToggleMuzzleLight => {
                m.muzzle_world_light = !m.muzzle_world_light;
                if !m.muzzle_world_light {
                    // Apagar limpio: que el siguiente render vea alpha=0.
                    m.muzzle_glow_at = None;
                }
            }
            Msg::ToggleMuzzleOcclusion => {
                m.muzzle_occlusion = !m.muzzle_occlusion;
            }
            Msg::ToggleWorldLights => {
                m.world_lights_enabled = !m.world_lights_enabled;
            }
            Msg::ToggleWeaponRimLight => {
                m.weapon_rim_light = !m.weapon_rim_light;
            }
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
                m.menu_active = usize::MAX;
                // Animación de aparición/swap del dropdown.
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        m.menu_open = None;
                        m.context_menu = None;
                        handle_menu_command(&cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.context_menu = None;
                m.menu_active = usize::MAX;
            }
            Msg::ContextMenuOpen(x, y) => {
                m.menu_open = None;
                m.context_menu = Some((x, y));
            }
            Msg::CycleTheme => {
                m.theme = Theme::next_after(m.theme.name);
            }
            Msg::DoomKeyTap(code) => {
                // Tap sintético: press + release en el mismo update para
                // que el motor lo procese como una pulsación completa.
                m.engine.push_key(true, code);
                m.engine.push_key(false, code);
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                m.context_menu = None;
                handle_menu_command(&cmd, handle);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));
        let header = header_bar(model);
        let body = match model.view_mode {
            ViewMode::Framebuffer => {
                if model.engine.real {
                    framed_pane(game_pane(model))
                } else {
                    stub_message_pane()
                }
            }
            ViewMode::Scene3d => framed_pane(scene_view(
                &model.snapshots,
                model.last_tick_at,
                TICK_PERIOD,
                RenderConfig {
                    atlas: model.atlas.clone(),
                    crosshair: model.show_crosshair,
                    vignette: model.vignette_strength,
                    hud: model.show_hud,
                    sprite_shadows: model.sprite_shadows,
                    // Fase 3.22: alpha del muzzle decae linealmente desde
                    // el último fogonazo. Cuando el toggle está apagado
                    // o nunca disparó, alpha=0 ⇒ render no aplica boost.
                    muzzle_glow_alpha: muzzle_alpha_now(model),
                    // Fase 3.23: oclusión sectorial del muzzle. Default on.
                    muzzle_occlusion: model.muzzle_occlusion,
                    // Fase 3.26: luces dinámicas desde mobjs FF_FULLBRIGHT.
                    world_lights_enabled: model.world_lights_enabled,
                    // Fase 3.28: rim-light del arma desde world lights.
                    weapon_rim_light: model.weapon_rim_light,
                    // Fase 3.43: gradiente vertical continuo para el
                    // shading/tinte de paredes texturizadas. `bands` aquí
                    // sólo controla la densidad de muestreo (4 alturas →
                    // 5 stops). Reemplaza las bandas discretas de 3.42 por
                    // una transición suave sin costuras. La default de
                    // librería sigue off (contrato bit-exact); lo
                    // activamos en el host porque es estrictamente mejor.
                    wall_vertical_bands: 4,
                    wall_vertical_gradient: true,
                    // Fase 3.44: gradiente de profundidad para pisos/techos
                    // — la parte cercana al jugador queda más clara (menos
                    // fog + pool de luz del muzzle/proyectil), la lejana
                    // más oscura. Mismo criterio que walls: default de
                    // librería off (bit-exact), on en el host.
                    plane_depth_gradient: true,
                    // Fase 3.46: pasamos los decals vivos con su alpha
                    // ya computado del fade (ttl/DECAL_TTL). El renderer
                    // los dibuja como billboards z-ordenados.
                    decals: model
                        .decals
                        .iter()
                        .map(|d| supay_render_llimphi::Decal {
                            x: d.x,
                            y: d.y,
                            z: d.z,
                            radius: d.radius,
                            color: d.color,
                            alpha: d.ttl as f32 / DECAL_TTL as f32,
                            tangent: d.tangent,
                            horizontal: d.horizontal,
                            wall_span: d.wall_span,
                        })
                        .collect(),
                    ..RenderConfig::default()
                },
            )
            // Mouse-look vertical: arrastrar con el botón izquierdo mueve el
            // horizonte (pitch cosmético). Arrastrar hacia arriba (dy<0)
            // mira hacia arriba. Complementa PageUp/PageDown. Doom no usa
            // el click izquierdo para disparar (fire = Ctrl/Enter), así que
            // hacerlo draggable no roba ningún binding.
            .draggable(|phase, _dx, dy| match phase {
                DragPhase::Move => Some(Msg::PitchDelta {
                    delta: -dy * MOUSE_LOOK_SENS,
                    reset: false,
                }),
                _ => None,
            })),
        };
        let footer = footer_bar(model);
        // Right-click sobre el View RAÍZ (origen 0,0) ⇒ las coords locales
        // que recibe el handler ya son coords de ventana, justo lo que el
        // menú contextual espera como ancla. Por eso lo anclamos acá y no
        // en el `body` (que está desplazado por menubar + header).
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(COLOR_BG_ABYSS)
        .children(vec![menubar, header, body, footer])
        .on_right_click_at(|x, y, _, _| Some(Msg::ContextMenuOpen(x, y)))
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // Prioridad: menú contextual del juego.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_game(model, x, y));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

// =====================================================================
// Menú principal + contextual del juego
// =====================================================================

/// Viewport para clampear overlays. La app no trackea el tamaño real de
/// ventana, así que usamos las constantes de `initial_size()`.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Supay::initial_size();
    (w as f32, h as f32)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &'a Model) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal de Doom. Archivo / Jugar / Ver / Ayuda — sólo
/// comandos que mapean a `Msg` reales ya existentes (taps del motor o
/// toggles del renderer). No hay "Editar": la app no tiene campos de
/// texto editables, es un canvas de juego.
///
/// El submenú Jugar inyecta teclas del motor (disparar / usar / menú de
/// Doom). El submenú Ver refleja en gris/check los toggles cosméticos
/// del renderer 3D.
fn app_menu(model: &Model) -> AppMenu {
    let scene3d = model.view_mode == ViewMode::Scene3d;

    // Helper: item de toggle marcado con [x]/[ ] según el flag.
    let toggle = |label: &str, on: bool, cmd: &str| -> MenuItem {
        let mark = if on { "[x] " } else { "[ ] " };
        MenuItem::new(&format!("{mark}{label}"), cmd)
    };

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Menú del juego", "file.menu").shortcut("Esc"))
                .item(MenuItem::new("Salir", "file.quit").shortcut("F12").separated()),
        )
        .menu(
            Menu::new("Jugar")
                .item(MenuItem::new("Disparar", "play.fire").shortcut("Ctrl"))
                .item(MenuItem::new("Usar / abrir", "play.use").shortcut("Space"))
                .item(MenuItem::new("Mapa", "play.map").shortcut("Tab")),
        )
        .menu(
            Menu::new("Ver")
                .item(
                    MenuItem::new(
                        if scene3d {
                            "Cambiar a framebuffer"
                        } else {
                            "Cambiar a renderer 3D"
                        },
                        "view.toggle_mode",
                    )
                    .shortcut("F3"),
                )
                .item(toggle("Crosshair", model.show_crosshair, "view.crosshair").shortcut("F4"))
                .item(MenuItem::new("Ciclar viñeta", "view.vignette").shortcut("F5"))
                .item(toggle("HUD", model.show_hud, "view.hud").shortcut("F6"))
                .item(
                    toggle("Sombras de sprites", model.sprite_shadows, "view.shadows")
                        .shortcut("F7"),
                )
                .item(toggle("Tema (barra)", true, "view.theme").separated())
                .item(toggle("Mirar arriba", false, "view.pitch_up").shortcut("PgUp"))
                .item(MenuItem::new("Resetear horizonte", "view.pitch_reset").shortcut("Home")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id (de la barra o del contextual) al `Msg` real y
/// lo dispatcha. Todos los ids mapean a acciones que ya existían.
fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    use supay_core::keys;
    let msg = match cmd {
        "file.menu" => Some(Msg::DoomKeyTap(keys::KEY_ESCAPE)),
        "file.quit" => Some(Msg::Quit),
        "play.fire" => Some(Msg::DoomKeyTap(keys::KEY_FIRE)),
        "play.use" => Some(Msg::DoomKeyTap(keys::KEY_USE)),
        "play.map" => Some(Msg::DoomKeyTap(keys::KEY_TAB)),
        "view.toggle_mode" => Some(Msg::ToggleViewMode),
        "view.crosshair" => Some(Msg::ToggleCrosshair),
        "view.vignette" => Some(Msg::CycleVignette),
        "view.hud" => Some(Msg::ToggleHud),
        "view.shadows" => Some(Msg::ToggleSpriteShadows),
        "view.theme" => Some(Msg::CycleTheme),
        "view.pitch_up" => Some(Msg::PitchDelta { delta: PITCH_STEP, reset: false }),
        "view.pitch_reset" => Some(Msg::PitchDelta { delta: 0.0, reset: true }),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

/// Menú contextual del juego. No hay objetos seleccionables ni texto
/// editable, así que expone las acciones de juego/vista más usadas
/// según el estado actual. Sin edición — esto es un canvas de Doom.
fn context_menu_for_game(model: &Model, x: f32, y: f32) -> View<Msg> {
    let header = match model.view_mode {
        ViewMode::Framebuffer => "framebuffer",
        ViewMode::Scene3d => "renderer 3D",
    };

    let items = vec![
        ContextMenuItem::action("Disparar").with_shortcut("Ctrl"),
        ContextMenuItem::action("Usar / abrir").with_shortcut("Space"),
        ContextMenuItem::separator(),
        ContextMenuItem::action(if model.view_mode == ViewMode::Scene3d {
            "Cambiar a framebuffer"
        } else {
            "Cambiar a renderer 3D"
        })
        .with_shortcut("F3"),
        ContextMenuItem::action("Menú del juego").with_shortcut("Esc"),
    ];

    // Mapeo de índice de item → command id de `handle_menu_command`.
    let cmds: Vec<&'static str> = vec!["play.fire", "play.use", "", "view.toggle_mode", "file.menu"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(header.to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

/// Fase 3.22: alpha actual del muzzle world light, computada por
/// `(1 - elapsed/MUZZLE_DECAY_SECS).max(0)`. Devuelve 0 cuando el
/// toggle está apagado o no hubo fogonazo reciente.
fn muzzle_alpha_now(model: &Model) -> f32 {
    if !model.muzzle_world_light {
        return 0.0;
    }
    let Some(t0) = model.muzzle_glow_at else {
        return 0.0;
    };
    let elapsed = t0.elapsed().as_secs_f32();
    (1.0 - elapsed / MUZZLE_DECAY_SECS).clamp(0.0, 1.0)
}

/// Fase 4.3 — deriva la acústica del reverb desde el sector donde está
/// el jugador. Un cuarto bajo suena seco; un hangar largo arrastra cola.
/// Exterior (techo de cielo) lava la reflexión tardía: aire en vez de
/// piedra. Sin mapa cargado → seco (`wet=0`).
fn ambience_for(snap: &SceneSnapshot) -> supay_audio::RoomAmbience {
    let Some(ac) = snap.player_acoustics() else {
        return supay_audio::RoomAmbience::default();
    };
    // Cuartos Doom típicos: 64 (pasillo bajo) .. 512+ (hangar). Normaliza.
    let t = (ac.ceiling_gap / 512.0).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    if ac.outdoor {
        // Aire abierto: poca cola, mucha amortiguación de agudos.
        supay_audio::RoomAmbience {
            wet: lerp(0.04, 0.12),
            room_size: lerp(0.4, 0.7),
            damping: 0.8,
        }
    } else {
        // Recinto de piedra: cola que crece con la altura del cuarto.
        supay_audio::RoomAmbience {
            wet: lerp(0.08, 0.33),
            room_size: lerp(0.3, 0.85),
            damping: 0.45,
        }
    }
}

// =====================================================================
// Header — banda alta con el logo a la izquierda y un HUD a la derecha:
// pill de modo (REAL o STUB), tick monoespaciado, tag de vista, recuento
// de la escena. Look "BFG console".
// =====================================================================
fn header_bar(model: &Model) -> View<Msg> {
    let logo_text = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "SUPAY · DOOM".to_string(),
        22.0,
        COLOR_CRIMSON,
        Alignment::Start,
    );
    let logo_sub = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "PHASE 3.63 · LLIMPHI BUILD".to_string(),
        9.0,
        COLOR_AMBER,
        Alignment::Start,
    );
    let logo = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(18.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![logo_text, logo_sub]);

    let mode_id = if model.engine.real {
        "supay-mode-real"
    } else {
        "supay-mode-stub"
    };
    let mode_bg = if model.engine.real { COLOR_CRIMSON_DEEP } else { COLOR_BG_SUNKEN };
    let mode_fg = if model.engine.real { COLOR_AMBER } else { COLOR_DUST };
    let mode_pill = View::new(Style {
        size: Size {
            width: length(110.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(mode_bg)
    .radius(3.0)
    .text_aligned(
        rimay_localize::t(mode_id),
        11.0,
        mode_fg,
        Alignment::Center,
    );

    let scene = model
        .snapshots
        .next()
        .map(|s| {
            format!(
                "w={} sec={} spr={}",
                s.walls.len(),
                s.sectors.len(),
                s.sprites.len()
            )
        })
        .unwrap_or_else(|| "—".to_string());
    let view_tag = rimay_localize::t(match model.view_mode {
        ViewMode::Framebuffer => "supay-view-fb",
        ViewMode::Scene3d => "supay-view-3d",
    });
    let stats = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(18.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!(
            "TICK {:06}    {}    SCENE [{}]",
            model.tick, view_tag, scene
        ),
        11.0,
        COLOR_BONE,
        Alignment::End,
    );

    let right = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::End),
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![stats, mode_pill]);

    let inner = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_HEIGHT),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(COLOR_BG_PANEL)
    .children(vec![logo, right]);

    // Banda 1 px crimson como underline al header — separador del juego.
    let rule = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(COLOR_RULE);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_HEIGHT + 1.0),
        },
        ..Default::default()
    })
    .children(vec![inner, rule])
}

// =====================================================================
// Footer — banda baja con los controles, color CRT verde sobre negro.
// =====================================================================
fn footer_bar(model: &Model) -> View<Msg> {
    let id = if model.engine.real {
        "supay-controls-hint"
    } else {
        "supay-stub-controls-hint"
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(FOOTER_HEIGHT),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(COLOR_BG_SUNKEN)
    .text_aligned(
        rimay_localize::t(id),
        10.0,
        COLOR_GREEN_CRT,
        Alignment::Start,
    )
}

// =====================================================================
// Marco crimson — envuelve cualquier contenido (FB o renderer 3D) con
// un border de 4 px en rojo profundo. Truco "outer fill = border color,
// inner fill = real content".
// =====================================================================
fn framed_pane(content: View<Msg>) -> View<Msg> {
    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(COLOR_BG_ABYSS)
    .children(vec![content]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(FRAME_THICKNESS),
            right: length(FRAME_THICKNESS),
            top: length(FRAME_THICKNESS),
            bottom: length(FRAME_THICKNESS),
        },
        ..Default::default()
    })
    .fill(COLOR_CRIMSON_DEEP)
    .children(vec![inner])
}

fn game_pane(model: &Model) -> View<Msg> {
    // Reconstruimos el peniko::Image por frame — `Blob::from` clona los
    // bytes para que el Image sea owning. Costo aceptable a 320×200×4
    // = 256 KB/frame.
    let blob = Blob::from(model.framebuffer_rgba.clone());
    let image = Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: DOOM_WIDTH as u32, height: DOOM_HEIGHT as u32 });
    // Patrón flex correcto para "ocupá el resto de la columna":
    // `flex_basis = 0` + `flex_grow = 1` + `height = auto`. Si en su
    // lugar pidiéramos `height = percent(1.0)`, taffy le daría el
    // 100% del padre (= ventana entera) y la imagen se solaparía
    // con el header de 24 px.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .image(image)
}

// =====================================================================
// Stub screen — pantalla "no hay WAD" rediseñada como console-prompt:
// banner crimson arriba, pasos numerados con su comando en color CRT,
// pie con el porqué técnico del motor.
// =====================================================================
fn stub_message_pane() -> View<Msg> {
    let banner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(120.0_f32),
        },
        padding: Rect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(36.0_f32),
            bottom: length(0.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(44.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            "SUPAY · DOOM".to_string(),
            32.0,
            COLOR_CRIMSON,
            Alignment::Start,
        ),
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            rimay_localize::t("supay-stub-title"),
            13.0,
            COLOR_DUST,
            Alignment::Start,
        ),
    ]);

    let mut steps: Vec<View<Msg>> = Vec::new();
    for (n, (title_id, cmd_id)) in [
        ("supay-stub-step-1", "supay-stub-step-1-cmd"),
        ("supay-stub-step-2", "supay-stub-step-2-cmd"),
        ("supay-stub-step-3", "supay-stub-step-3-cmd"),
    ]
    .iter()
    .enumerate()
    {
        steps.push(stub_step((n as u32) + 1, title_id, cmd_id));
    }

    let body = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(8.0_f32),
            bottom: length(20.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .children(steps);

    let foot = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(36.0_f32),
        },
        padding: Rect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(0.0_f32),
            bottom: length(12.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t("supay-stub-footer"),
        11.0,
        COLOR_DUST,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(COLOR_BG_PANEL)
    .children(vec![banner, body, foot])
}

/// Un paso del setup: bullet ámbar con el número + título en hueso +
/// comando shell debajo en verde CRT sobre una caja sunken.
fn stub_step(n: u32, title_id: &'static str, cmd_id: &'static str) -> View<Msg> {
    let bullet = View::new(Style {
        size: Size {
            width: length(28.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{:02}", n),
        13.0,
        COLOR_AMBER,
        Alignment::Center,
    );

    let title = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(22.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t(title_id),
        14.0,
        COLOR_BONE,
        Alignment::Start,
    );

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![bullet, title]);

    let cmd = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(COLOR_BG_SUNKEN)
    .radius(2.0)
    .text_aligned(
        rimay_localize::t(cmd_id),
        11.0,
        COLOR_GREEN_CRT,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![head, cmd])
}

/// Copia el framebuffer ARGB (`0xAARRGGBB` little-endian u32) que el
/// motor llenó al buffer Rgba8 que `peniko::Image` espera. Cuatro
/// canales contiguos por píxel.
fn refresh_framebuffer(m: &mut Model) {
    let src = m.engine.framebuffer();
    let dst = &mut m.framebuffer_rgba;
    debug_assert_eq!(src.len(), DOOM_PIXELS);
    debug_assert_eq!(dst.len(), DOOM_PIXELS * 4);
    for (i, px) in src.iter().enumerate() {
        let r = ((px >> 16) & 0xff) as u8;
        let g = ((px >> 8) & 0xff) as u8;
        let b = (px & 0xff) as u8;
        // doomgeneric llena alpha en 0; lo forzamos a 255 para que el
        // píxel sea opaco — sino el `View::image` lo mezclaría con el
        // fondo.
        let a = 0xff_u8;
        let o = i * 4;
        dst[o] = r;
        dst[o + 1] = g;
        dst[o + 2] = b;
        dst[o + 3] = a;
    }
}

/// Traduce un evento de teclado Llimphi a código Doom. Devolver
/// `None` significa "esta tecla no le importa al motor".
fn translate_key(key: &Key, text: Option<&str>) -> Option<u8> {
    use keys::*;
    // Primero las teclas con nombre estable.
    if let Key::Named(named) = key {
        return Some(match named {
            NamedKey::ArrowUp => KEY_UPARROW,
            NamedKey::ArrowDown => KEY_DOWNARROW,
            NamedKey::ArrowLeft => KEY_LEFTARROW,
            NamedKey::ArrowRight => KEY_RIGHTARROW,
            NamedKey::Escape => KEY_ESCAPE,
            NamedKey::Enter => KEY_ENTER,
            NamedKey::Tab => KEY_TAB,
            NamedKey::Shift => KEY_RSHIFT,
            NamedKey::Space => KEY_USE,
            NamedKey::Control => KEY_FIRE,
            _ => return None,
        });
    }
    // Luego las que vienen como texto.
    let ch = text.and_then(|s| s.chars().next())?;
    Some(match ch.to_ascii_lowercase() {
        'w' => KEY_UPARROW,
        's' => KEY_DOWNARROW,
        'a' => KEY_LEFTARROW,
        'd' => KEY_RIGHTARROW,
        ',' => KEY_STRAFE_L,
        '.' => KEY_STRAFE_R,
        ' ' => KEY_USE,
        'y' => KEY_Y,
        'n' => KEY_N,
        _ => return None,
    })
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Supay>();
}
