//! `supay-app-llimphi` — Fase 0.5 del proyecto supay.
//!
//! Raycaster estilo Wolfenstein/Doom-early para validar el pipeline
//! "tick deterministic separado del render" antes de cablear
//! doomgeneric real en Fase 1. Esta iteración suma **sprites
//! billboarded con z-test** y **sector lights** — los dos rasgos
//! que vuelven el modelo legible: ya hay scene extraction implícita
//! (mapa estático + sprite list + light list son tres canales
//! independientes) y el renderer las consume cada una con su
//! pipeline.
//!
//! Mapa 16×16 hardcoded (paredes con 4 materiales), jugador con
//! (x, y, angle), tick a 35 Hz vía `Handle::spawn_periodic`,
//! pintado vía `View::paint_with`. Para cada columna de la pantalla
//! lanzamos un rayo, DDA por la grilla, calculamos altura del slice
//! con perpendicular distance (evita fish-eye), sombreamos por
//! distancia + lado de pared (paredes E/W más oscuras que N/S como
//! Doom original) + contribución sumada de luces puntuales (falloff
//! `1/(1+d²)`) + niebla volumétrica.
//!
//! Sprites: lista de objetos con `(x, y, kind, scale)`. Por sprite
//! transformamos a espacio cámara con el inverso de la matriz
//! `[plane | dir]`, calculamos `screen_x` + altura proporcional a
//! `1/depth`, y pintamos columna por columna respetando un z-buffer
//! por columna que guardamos durante el raycast de paredes.
//!
//! Sector lights: lista de `(x, y, r, g, b, strength)`. Cada luz
//! aporta `strength · color · 1/(1 + d²)` al hit world-point del
//! rayo, antes de fog. La iluminación total se clampea para no
//! sobre-exponer.
//!
//! Anti-fish-eye: la distancia que usamos para el alto de pared es
//! `hit_dist · cos(ray_angle - player_angle)`, no `hit_dist` puro.
//!
//! Controles: W/S adelante/atrás, A/D strafe, ←/→ giro, Esc cierra.

use std::sync::Arc;
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};

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

// =====================================================================
// Mapa hardcoded — 1 = pared, número = material id (1..4)
// =====================================================================

const MAP_W: usize = 16;
const MAP_H: usize = 16;

#[rustfmt::skip]
const MAP: [u8; MAP_W * MAP_H] = [
    1,1,1,1,1,1,2,2,2,2,2,1,1,1,1,1,
    1,0,0,0,0,1,0,0,0,0,2,0,0,0,0,1,
    1,0,0,0,0,1,0,0,0,0,0,0,0,0,0,1,
    1,0,0,3,0,0,0,0,3,0,2,0,0,0,0,1,
    1,0,0,0,0,1,0,0,0,0,2,1,1,0,1,1,
    1,1,1,0,1,1,0,0,0,0,2,0,0,0,0,1,
    1,0,0,0,0,0,0,0,0,0,0,0,3,0,0,1,
    1,0,0,4,0,0,0,4,0,0,0,0,0,0,0,1,
    1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,
    1,1,0,1,1,0,0,0,0,1,1,0,1,1,0,1,
    1,0,0,0,1,0,0,3,0,1,0,0,0,0,0,1,
    1,0,0,0,1,0,0,0,0,1,0,0,0,0,0,1,
    1,0,0,0,1,1,0,0,1,1,0,0,4,0,0,1,
    1,0,3,0,0,0,0,0,0,0,0,0,0,0,0,1,
    1,0,0,0,0,0,2,2,0,0,0,0,0,0,0,1,
    1,1,1,1,1,1,2,2,1,1,1,1,1,1,1,1,
];

fn tile(x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= MAP_W as i32 || y >= MAP_H as i32 {
        return 1;
    }
    MAP[y as usize * MAP_W + x as usize]
}

// =====================================================================
// Sprites — objetos del mundo billboarded
// =====================================================================

/// Tipo de sprite. Cada uno tiene un color base y un perfil de altura
/// (qué fracción del slice ocupa verticalmente). En Fase 1 los reemplaza
/// la tabla de sprites del WAD (animaciones por estado + 8 ángulos).
#[derive(Clone, Copy)]
enum SpriteKind {
    Barrel,
    Pillar,
    Imp,
    Torch,
    /// Proyectil del jugador (bullet trazante amarillo). Spawneado por
    /// `Msg::Fire`, avanza a velocidad fija, muere al chocar pared.
    Bullet,
    /// Decal de impacto en pared. Lo deja un bullet al morir por
    /// colisión. Estático con TTL.
    Decal,
    /// Imp herido en `Dying`: tinte más oscuro, perdiendo color rojo.
    DyingImp,
    /// Cadáver del imp (`Dead`): tinte muy oscuro, apoyado al piso
    /// con scale reducida.
    Corpse,
    /// Pickup de munición — cajita cyan brillante.
    AmmoBox,
    /// Pickup de vida — cruz verde brillante.
    HealthKit,
}

#[derive(Clone, Copy)]
struct Sprite {
    x: f32,
    y: f32,
    kind: SpriteKind,
    /// Multiplicador de tamaño aparente. 1.0 = pared completa de 1
    /// unidad. Los barriles ocupan ~0.5, las antorchas ~0.7, los
    /// imps ~0.85.
    scale: f32,
}

impl Sprite {
    /// Color base + altura fraccional. La altura define qué % del
    /// slice se pinta — un barril pintado a 0.5 ocupa la mitad inferior
    /// del slice (porque está apoyado en el piso).
    fn appearance(&self) -> ((f32, f32, f32), f32) {
        match self.kind {
            SpriteKind::Barrel => ((0.32, 0.78, 0.30), 0.5),
            SpriteKind::Pillar => ((0.55, 0.50, 0.42), 1.0),
            SpriteKind::Imp => ((0.78, 0.20, 0.18), 0.85),
            SpriteKind::Torch => ((0.95, 0.78, 0.30), 0.7),
            SpriteKind::Bullet => ((1.00, 0.90, 0.30), 0.15),
            SpriteKind::Decal => ((0.18, 0.08, 0.06), 0.20),
            // Dying: rojo más opaco, perdiendo brillo.
            SpriteKind::DyingImp => ((0.45, 0.12, 0.10), 0.65),
            // Corpse: mancha rojiza oscura tirada en el piso.
            SpriteKind::Corpse => ((0.22, 0.07, 0.06), 0.30),
            // Cajita cyan — pickup de munición.
            SpriteKind::AmmoBox => ((0.45, 0.85, 0.95), 0.35),
            // Cruz verde brillante — pickup de vida.
            SpriteKind::HealthKit => ((0.30, 0.95, 0.40), 0.35),
        }
    }
}

/// Decorados estáticos (no se mueven, no pelean). Los imps van aparte
/// como [`Enemy`] porque tienen HP y AI.
fn initial_static_sprites() -> Vec<Sprite> {
    vec![
        Sprite { x: 4.5, y: 3.5, kind: SpriteKind::Barrel, scale: 0.5 },
        Sprite { x: 11.5, y: 4.5, kind: SpriteKind::Pillar, scale: 1.0 },
        Sprite { x: 6.5, y: 9.5, kind: SpriteKind::Barrel, scale: 0.5 },
        Sprite { x: 8.5, y: 12.5, kind: SpriteKind::Torch, scale: 0.7 },
        Sprite { x: 3.5, y: 13.5, kind: SpriteKind::Torch, scale: 0.7 },
    ]
}

// =====================================================================
// Enemies — imps con HP, AI de persecución, ataque cuerpo a cuerpo
// =====================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnemyState {
    /// Quieto: no vio al jugador (sin LOS o fuera de rango).
    Idle,
    /// Persiguiendo: hay LOS al jugador.
    Walking,
    /// Recibió daño letal; entra en animación de muerte por N ticks
    /// antes de pasar a `Dead`.
    Dying(u32),
    /// Cadáver pintado en el piso. Sin colisión ni daño.
    Dead,
}

#[derive(Clone, Copy)]
struct Enemy {
    x: f32,
    y: f32,
    hp: i32,
    state: EnemyState,
    /// Cooldown del ataque cuerpo a cuerpo. Cuando es 0 y el imp toca
    /// al jugador, le pega y se resetea.
    attack_cd: u32,
}

const ENEMY_HP: i32 = 100;
const ENEMY_SPEED: f32 = 0.045; // u/tick — más lento que el jugador
const ENEMY_AGGRO_RANGE: f32 = 8.0; // unidades
const ENEMY_MELEE_RANGE: f32 = 0.9;
const ENEMY_MELEE_DAMAGE: u32 = 8;
const ENEMY_MELEE_CD: u32 = 25; // ticks (~0.7 s entre golpes)
const ENEMY_DYING_TICKS: u32 = 14;
const BULLET_DAMAGE: i32 = 25;
const BULLET_HIT_RADIUS: f32 = 0.35;

fn initial_enemies() -> Vec<Enemy> {
    vec![
        Enemy { x: 7.5, y: 5.5, hp: ENEMY_HP, state: EnemyState::Idle, attack_cd: 0 },
        Enemy { x: 12.5, y: 11.5, hp: ENEMY_HP, state: EnemyState::Idle, attack_cd: 0 },
    ]
}

/// Línea de visión libre entre `(ax, ay)` y `(bx, by)` — DDA que
/// chequea si alguna celda intermedia es pared. True = visible.
fn has_los(ax: f32, ay: f32, bx: f32, by: f32) -> bool {
    let dx = bx - ax;
    let dy = by - ay;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 0.01 {
        return true;
    }
    // Muestreo cada 0.1 unidades — suficiente para grid de 1 u.
    let steps = (dist / 0.1).ceil() as i32;
    let inv = 1.0 / steps as f32;
    for i in 1..steps {
        let t = i as f32 * inv;
        let cx = (ax + dx * t).floor() as i32;
        let cy = (ay + dy * t).floor() as i32;
        if tile(cx, cy) != 0 {
            return false;
        }
    }
    true
}

// =====================================================================
// Temp lights — flashes instantáneos con TTL (impactos, disparos)
// =====================================================================

#[derive(Clone, Copy)]
struct TempLight {
    x: f32,
    y: f32,
    color: (f32, f32, f32),
    /// Intensidad pico en el tick de spawn. Cae linealmente con el TTL.
    strength: f32,
    ttl: u32,
    ttl_max: u32,
}

const FLASH_TTL: u32 = 4; // ticks (~115 ms)
const FLASH_STRENGTH_IMPACT: f32 = 3.5;
const FLASH_COLOR_IMPACT: (f32, f32, f32) = (1.0, 0.75, 0.30);

// =====================================================================
// Pickups — items que el jugador recoge pasando por encima
// =====================================================================

#[derive(Clone, Copy)]
enum PickupKind {
    Ammo,
    Health,
}

#[derive(Clone, Copy)]
struct Pickup {
    x: f32,
    y: f32,
    kind: PickupKind,
}

const AMMO_PICKUP_AMOUNT: u32 = 12;
const HEALTH_PICKUP_AMOUNT: u32 = 25;
const HEALTH_MAX: u32 = 100;
const PICKUP_RADIUS: f32 = 0.55;

fn initial_pickups() -> Vec<Pickup> {
    vec![
        Pickup { x: 4.5, y: 7.5, kind: PickupKind::Ammo },
        Pickup { x: 11.5, y: 8.5, kind: PickupKind::Health },
        Pickup { x: 2.5, y: 11.5, kind: PickupKind::Ammo },
        Pickup { x: 13.5, y: 14.5, kind: PickupKind::Health },
        Pickup { x: 6.5, y: 14.5, kind: PickupKind::Ammo },
    ]
}

/// Proyectil del jugador. Vida finita en ticks; se mueve a velocidad
/// constante por (vx, vy); muere al chocar pared y deja un decal.
#[derive(Clone, Copy)]
struct Bullet {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    ttl: u32,
}

const BULLET_SPEED: f32 = 0.45; // unidades/tick
const BULLET_TTL: u32 = 60; // ticks (~1.7 s)
const BULLET_LIGHT_STRENGTH: f32 = 1.4;
const BULLET_LIGHT_COLOR: (f32, f32, f32) = (1.0, 0.85, 0.40);

/// Decal en la pared (marca de impacto). Estático con TTL para que
/// los decals se vayan limpiando solos.
#[derive(Clone, Copy)]
struct Decal {
    x: f32,
    y: f32,
    ttl: u32,
}

const DECAL_TTL: u32 = 240; // ticks (~7 s)
const MAX_DECALS: usize = 32;

// =====================================================================
// Sector lights — luces puntuales con falloff inverso al cuadrado
// =====================================================================

#[derive(Clone, Copy)]
struct Light {
    x: f32,
    y: f32,
    /// Color de la luz en lineal [0, 1].
    color: (f32, f32, f32),
    /// Multiplicador de intensidad. Strenghts típicos: 1.5 antorcha
    /// cálida, 0.8 luz fría tenue, 2.5 portal infernal.
    strength: f32,
}

const LIGHTS: &[Light] = &[
    Light { x: 8.5, y: 12.5, color: (1.00, 0.70, 0.35), strength: 2.2 },
    Light { x: 3.5, y: 13.5, color: (1.00, 0.70, 0.35), strength: 1.8 },
    Light { x: 7.5, y: 5.5, color: (0.85, 0.20, 0.15), strength: 2.5 }, // halo infernal en el imp
    Light { x: 13.5, y: 6.5, color: (0.35, 0.55, 1.00), strength: 1.4 }, // luz fría
];

/// Contribución sumada de todas las luces al hit_world_point.
/// Atenuación `1/(1 + 0.6·d²)`. El resultado se SUMA al color
/// ambient + base del material; el caller clampea.
///
/// `tick` modula el flicker de las luces cálidas (las que tiñen
/// hacia naranja, identificadas por `color.0 > color.2`) — el
/// resultado es que las antorchas parpadean orgánicamente con
/// fases distintas por índice; las luces frías quedan estables.
///
/// `bullets` aportan cada uno una luz puntual amarilla mientras
/// vuelan — el proyectil ilumina dinámicamente las paredes que
/// pasa cerca.
fn lighting_contribution(
    hit_x: f32,
    hit_y: f32,
    tick: u64,
    bullets: &[Bullet],
    temp_lights: &[TempLight],
) -> (f32, f32, f32) {
    let mut acc = (0.0_f32, 0.0_f32, 0.0_f32);
    for (idx, l) in LIGHTS.iter().enumerate() {
        let dx = l.x - hit_x;
        let dy = l.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let flicker = if l.color.0 > l.color.2 {
            // Antorcha cálida: parpadeo orgánico ±12% con fase por idx.
            let phase = idx as f32 * 1.37;
            1.0 + 0.12 * (tick as f32 * 0.31 + phase).sin()
                + 0.05 * (tick as f32 * 0.71 + phase * 1.7).sin()
        } else {
            1.0
        };
        let atten = l.strength * flicker / (1.0 + 0.6 * d2);
        acc.0 += l.color.0 * atten;
        acc.1 += l.color.1 * atten;
        acc.2 += l.color.2 * atten;
    }
    // Bullets: luz cálida amarillenta con falloff fuerte (caen rápido
    // con la distancia para no inundar el pasillo entero).
    for b in bullets {
        let dx = b.x - hit_x;
        let dy = b.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let atten = BULLET_LIGHT_STRENGTH / (1.0 + 1.2 * d2);
        acc.0 += BULLET_LIGHT_COLOR.0 * atten;
        acc.1 += BULLET_LIGHT_COLOR.1 * atten;
        acc.2 += BULLET_LIGHT_COLOR.2 * atten;
    }
    // Temp lights: flashes con strength que cae con el TTL (fade-out
    // lineal). Falloff espacial moderado.
    for tl in temp_lights {
        let dx = tl.x - hit_x;
        let dy = tl.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let life = tl.ttl as f32 / tl.ttl_max.max(1) as f32;
        let atten = tl.strength * life / (1.0 + 1.0 * d2);
        acc.0 += tl.color.0 * atten;
        acc.1 += tl.color.1 * atten;
        acc.2 += tl.color.2 * atten;
    }
    acc
}

// =====================================================================
// Materiales — cada id de pared a un color base. La iluminación final
// es: color · (ambient + lights) · side_bias · 1/(1+kd) · fog.
// =====================================================================

fn material_color(id: u8) -> (f32, f32, f32) {
    match id {
        1 => (0.62, 0.55, 0.46), // techbase beige
        2 => (0.48, 0.18, 0.16), // ladrillo rojo infernal
        3 => (0.28, 0.40, 0.52), // metal azul
        4 => (0.18, 0.55, 0.30), // slime verde
        _ => (0.5, 0.5, 0.5),
    }
}

// Color del piso y techo (gradiente vertical simple).
fn floor_color(y_frac: f32) -> Color {
    // y_frac = 0 al horizonte, 1 al pie de la pantalla.
    let g = 0.08 + 0.16 * y_frac;
    let r = 0.08 + 0.10 * y_frac;
    let b = 0.10 + 0.05 * y_frac;
    rgb(r, g, b)
}

fn ceiling_color(y_frac: f32) -> Color {
    let g = 0.02 + 0.04 * (1.0 - y_frac);
    let r = 0.04 + 0.10 * (1.0 - y_frac);
    let b = 0.04 + 0.05 * (1.0 - y_frac);
    rgb(r, g, b)
}

// Niebla: mezcla el color de la pared con el fog color según distancia
// normalizada. Convierte una escena flat en una escena con profundidad.
const FOG_COLOR: (f32, f32, f32) = (0.05, 0.04, 0.06);
const FOG_END: f32 = 14.0; // unidades de mapa; >= aquí es niebla pura

fn apply_fog(color: (f32, f32, f32), dist: f32) -> (f32, f32, f32) {
    let t = (dist / FOG_END).clamp(0.0, 1.0);
    lerp_rgb(color, FOG_COLOR, t)
}

fn shade_by_dist(color: (f32, f32, f32), dist: f32) -> (f32, f32, f32) {
    // Atenuación 1/(1 + k·d) — más Doom-like que 1/d² (que se va a
    // negro muy rápido en pasillos cortos).
    let k = 0.18;
    let atten = 1.0 / (1.0 + k * dist);
    (color.0 * atten, color.1 * atten, color.2 * atten)
}

fn lerp_rgb(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    let to = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to(r), to(g), to(b), 255)
}

// =====================================================================
// Modelo y bucle
// =====================================================================

const TICK_HZ: u64 = 35; // ticks/seg — la frecuencia canónica de Doom
const TICK_MS: u64 = 1_000 / TICK_HZ;
const MOVE_SPEED: f32 = 0.10; // unidades de mapa por tick
const STRAFE_SPEED: f32 = 0.08;
const TURN_SPEED: f32 = 0.055; // radianes por tick
const FOV: f32 = 1.05; // ~60° de ángulo total

#[derive(Default)]
struct Input {
    forward: bool,
    backward: bool,
    strafe_left: bool,
    strafe_right: bool,
    turn_left: bool,
    turn_right: bool,
}

struct Model {
    px: f32,
    py: f32,
    pa: f32, // ángulo en radianes
    input: Input,
    tick: u64,
    last_hit_material: u8,
    /// Vida del jugador. Bajada por ataque cuerpo a cuerpo de enemigos.
    health: u32,
    /// Munición restante. `Msg::Fire` la decrementa si > 0.
    ammo: u32,
    bullets: Vec<Bullet>,
    decals: Vec<Decal>,
    static_sprites: Vec<Sprite>,
    enemies: Vec<Enemy>,
    pickups: Vec<Pickup>,
    /// Flashes temporales (impactos, etc.). Se decrementan cada tick.
    temp_lights: Vec<TempLight>,
    /// El jugador murió (HP llegó a 0). Bloquea movimiento + disparo;
    /// Space pasa a reiniciar la partida.
    game_over: bool,
    /// Todos los enemigos muertos. Mismo handling que `game_over` —
    /// Space reinicia.
    victory: bool,
    /// Tema activo — sólo viste la barra de menú y los overlays. El
    /// raycaster pinta con su propia paleta hardcoded.
    theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada dentro del dropdown abierto (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal.
    menu_anim: Tween<f32>,
    /// Menú contextual del escenario: ancla `(x, y)` en coords de
    /// ventana. `None` cerrado. No hay objetos seleccionables en el
    /// mundo — el contextual expone las acciones de juego (disparar /
    /// reiniciar), no edición.
    context_menu: Option<(f32, f32)>,
}

/// Estado inicial del jugador + estructuras dinámicas. Lo usan `init`
/// y `reset_game` (al apretar Space tras game_over/victory).
fn reset_game(m: &mut Model) {
    m.px = 2.5;
    m.py = 2.5;
    m.pa = 0.6;
    m.input = Input::default();
    m.health = 100;
    m.ammo = 50;
    m.bullets.clear();
    m.decals.clear();
    m.enemies = initial_enemies();
    m.pickups = initial_pickups();
    m.temp_lights.clear();
    m.game_over = false;
    m.victory = false;
    m.last_hit_material = 0;
}

#[derive(Clone)]
enum Msg {
    Tick,
    Key(KeyEvent),
    Fire,
    Reset,
    Quit,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en la barra o en el contextual — se traduce al
    /// `Msg` real existente.
    MenuCommand(String),
    /// Navegación por teclado dentro del dropdown: +1 baja, -1 sube.
    MenuNav(i32),
    /// Enter sobre la fila resaltada del dropdown.
    MenuActivate,
    /// Tick de la animación del menú (sólo re-render).
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click sobre el escenario → abre el menú contextual de juego
    /// anclado en `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cicla el preset de tema (sólo cosmético para la barra/overlays).
    CycleTheme,
}

struct Supay;

impl App for Supay {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "supay · fase 0 · raycaster"
    }

    fn initial_size() -> (u32, u32) {
        (960, 600)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        Model {
            px: 2.5,
            py: 2.5,
            pa: 0.6,
            input: Input::default(),
            tick: 0,
            last_hit_material: 0,
            health: 100,
            ammo: 50,
            bullets: Vec::with_capacity(16),
            decals: Vec::with_capacity(MAX_DECALS),
            static_sprites: initial_static_sprites(),
            enemies: initial_enemies(),
            pickups: initial_pickups(),
            temp_lights: Vec::with_capacity(8),
            game_over: false,
            victory: false,
            theme: Theme::dark(),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
        }
    }

    fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
        // Con el menú principal abierto las flechas navegan: ←/→ cambian de
        // menú raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta y
        // Esc cierra. Tiene prioridad y consume la tecla.
        if e.state == KeyState::Pressed {
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
        }
        if matches!(&e.key, Key::Named(NamedKey::Escape)) && e.state == KeyState::Pressed {
            // Esc cierra primero cualquier menú abierto; sólo sale del
            // juego si no hay overlay activo.
            if model.menu_open.is_some() || model.context_menu.is_some() {
                return Some(Msg::CloseMenus);
            }
            return Some(Msg::Quit);
        }
        // Space tiene dos modos según el estado: si el jugador está
        // en game_over o victory, dispara reset; en juego normal,
        // dispara una bala.
        if e.state == KeyState::Pressed && matches!(&e.key, Key::Named(NamedKey::Space)) {
            return Some(if model.game_over || model.victory {
                Msg::Reset
            } else {
                Msg::Fire
            });
        }
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Quit => {
                handle.quit();
            }
            Msg::Fire => {
                if !m.game_over && !m.victory && m.ammo > 0 {
                    m.ammo -= 1;
                    let (sin, cos) = m.pa.sin_cos();
                    // Spawn ligeramente delante del jugador para que el
                    // sprite no asome detrás del crosshair.
                    m.bullets.push(Bullet {
                        x: m.px + cos * 0.25,
                        y: m.py + sin * 0.25,
                        vx: cos * BULLET_SPEED,
                        vy: sin * BULLET_SPEED,
                        ttl: BULLET_TTL,
                    });
                }
            }
            Msg::Reset => {
                reset_game(&mut m);
            }
            Msg::Key(e) => {
                let pressed = e.state == KeyState::Pressed;
                let text_lower = e
                    .text
                    .as_ref()
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                let ch = text_lower.chars().next();
                match (&e.key, ch) {
                    (_, Some('w')) => m.input.forward = pressed,
                    (_, Some('s')) => m.input.backward = pressed,
                    (_, Some('a')) => m.input.strafe_left = pressed,
                    (_, Some('d')) => m.input.strafe_right = pressed,
                    (Key::Named(NamedKey::ArrowLeft), _) => m.input.turn_left = pressed,
                    (Key::Named(NamedKey::ArrowRight), _) => m.input.turn_right = pressed,
                    (Key::Named(NamedKey::ArrowUp), _) => m.input.forward = pressed,
                    (Key::Named(NamedKey::ArrowDown), _) => m.input.backward = pressed,
                    _ => {}
                }
            }
            Msg::Tick => {
                m.tick = m.tick.wrapping_add(1);
                advance(&mut m);
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
        let scene = scene_pane(model);
        let hud = hud_panel(model);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(rgb(0.02, 0.02, 0.03))
        .children(vec![menubar, scene, hud])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // Prioridad: menú contextual del escenario.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu_for_scene(model, x, y));
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

/// HUD inferior estilo Doom clásico: tres celdas (vida, munición,
/// material apuntado). Sin lógica de daño todavía — la vida queda en
/// 100 mientras no haya enemigos que ataquen.
fn hud_panel(model: &Model) -> View<Msg> {
    let mat_name = match model.last_hit_material {
        1 => "techbase",
        2 => "ladrillo",
        3 => "metal",
        4 => "slime",
        _ => "—",
    };
    let hud_bg = rgb(0.08, 0.06, 0.06);
    let border = rgb(0.42, 0.10, 0.06);

    let cell = |label: &str, value: &str, value_color: (f32, f32, f32)| -> View<Msg> {
        let label_v = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(12.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(label.to_string(), 10.0, rgb(0.65, 0.55, 0.45), Alignment::Center);
        let value_v = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            value.to_string(),
            20.0,
            rgb(value_color.0, value_color.1, value_color.2),
            Alignment::Center,
        );
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            ..Default::default()
        })
        .children(vec![label_v, value_v])
    };

    // Color de la vida cambia rojo cuando es baja.
    let health_color = if model.health > 50 {
        (0.80, 0.95, 0.55)
    } else if model.health > 25 {
        (0.95, 0.85, 0.30)
    } else {
        (0.95, 0.30, 0.25)
    };
    let ammo_color = if model.ammo > 0 {
        (0.95, 0.85, 0.30)
    } else {
        (0.95, 0.30, 0.25)
    };

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![
        cell("VIDA", &model.health.to_string(), health_color),
        cell("MUNICION", &model.ammo.to_string(), ammo_color),
        cell("OBJETIVO", mat_name, (0.85, 0.80, 0.70)),
    ]);

    // Borde superior rojizo + fondo oscuro, alto 50 px.
    let border_strip = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(border);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(52.0_f32),
        },
        ..Default::default()
    })
    .fill(hud_bg)
    .children(vec![border_strip, row])
}

// =====================================================================
// Menú principal + contextual del escenario
// =====================================================================

/// Viewport para clampear overlays. Supay no trackea el tamaño real de
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

/// El menú principal del raycaster. Archivo / Jugar / Ver / Ayuda — sólo
/// comandos que mapean a `Msg` reales ya existentes. No hay "Editar": la
/// app no tiene campos de texto editables, es un canvas de juego.
///
/// El submenú Jugar refleja en gris el estado real: "Disparar" se
/// deshabilita sin munición o en game over/victory.
fn app_menu(model: &Model) -> AppMenu {
    let can_fire = !model.game_over && !model.victory && model.ammo > 0;
    let fire_item = MenuItem::new("Disparar", "play.fire").shortcut("Space");
    let fire_item = if can_fire { fire_item } else { fire_item.disabled() };

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Reiniciar partida", "file.reset"))
                .item(MenuItem::new("Salir", "file.quit").shortcut("Esc").separated()),
        )
        .menu(Menu::new("Jugar").item(fire_item))
        .menu(Menu::new("Ver").item(MenuItem::new("Cambiar tema", "view.theme")))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id (de la barra o del contextual) al `Msg` real y
/// lo dispatcha. Todos los ids mapean a acciones que ya existían.
fn handle_menu_command(cmd: &str, handle: &Handle<Msg>) {
    let msg = match cmd {
        "file.reset" => Some(Msg::Reset),
        "file.quit" => Some(Msg::Quit),
        "play.fire" => Some(Msg::Fire),
        "view.theme" => Some(Msg::CycleTheme),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => None,
    };
    if let Some(msg) = msg {
        handle.dispatch(msg);
    }
}

/// Menú contextual del escenario. No hay objetos seleccionables en el
/// mundo, así que expone las acciones de juego disponibles según el
/// estado (disparar gris sin munición o en game over/victory). Sin
/// edición — esto es un canvas de raycaster, no texto.
fn context_menu_for_scene(model: &Model, x: f32, y: f32) -> View<Msg> {
    let can_fire = !model.game_over && !model.victory && model.ammo > 0;
    let header = if model.game_over {
        "fin de partida".to_string()
    } else if model.victory {
        "victoria".to_string()
    } else {
        format!("munición {}", model.ammo)
    };

    let fire = ContextMenuItem::action("Disparar").with_shortcut("Space");
    let items = vec![
        if can_fire { fire } else { fire.disabled() },
        ContextMenuItem::separator(),
        ContextMenuItem::action("Reiniciar partida"),
    ];

    // Mapeo de índice de item → command id de `handle_menu_command`.
    let cmds: Vec<&'static str> = vec!["play.fire", "", "file.reset"];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        Msg::MenuCommand(cmds.get(i).copied().unwrap_or("").to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport_of(model),
        header: Some(header),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

// =====================================================================
// Submódulos del bin: simulación (tick) y raycaster (render).
// Los tipos + consts viven aquí (root) — los módulos los ven por la
// regla de visibilidad descendiente vía `use super::*`.
// =====================================================================
mod render;
mod sim;
use render::*;
use sim::*;

fn main() {
    llimphi_ui::run::<Supay>();
}
