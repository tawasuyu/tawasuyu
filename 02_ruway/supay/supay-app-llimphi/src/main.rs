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

use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};

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
    /// Flashes temporales (impactos, etc.). Se decrementan cada tick.
    temp_lights: Vec<TempLight>,
}

#[derive(Clone)]
enum Msg {
    Tick,
    Key(KeyEvent),
    Fire,
    Quit,
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
            temp_lights: Vec::with_capacity(8),
        }
    }

    fn on_key(_: &Model, e: &KeyEvent) -> Option<Msg> {
        if matches!(&e.key, Key::Named(NamedKey::Escape)) && e.state == KeyState::Pressed {
            return Some(Msg::Quit);
        }
        // Disparo: Space al apretar (no al soltar).
        if e.state == KeyState::Pressed && matches!(&e.key, Key::Named(NamedKey::Space)) {
            return Some(Msg::Fire);
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
                if m.ammo > 0 {
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
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
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
        .children(vec![scene, hud])
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
// Lógica del tick — movimiento + colisión simple cell-based
// =====================================================================

fn advance(m: &mut Model) {
    if m.input.turn_left {
        m.pa -= TURN_SPEED;
    }
    if m.input.turn_right {
        m.pa += TURN_SPEED;
    }
    // mantener [0, 2π)
    let two_pi = std::f32::consts::TAU;
    if m.pa < 0.0 {
        m.pa += two_pi;
    } else if m.pa >= two_pi {
        m.pa -= two_pi;
    }

    let (sin, cos) = m.pa.sin_cos();
    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    if m.input.forward {
        dx += cos * MOVE_SPEED;
        dy += sin * MOVE_SPEED;
    }
    if m.input.backward {
        dx -= cos * MOVE_SPEED;
        dy -= sin * MOVE_SPEED;
    }
    if m.input.strafe_left {
        dx += sin * STRAFE_SPEED;
        dy -= cos * STRAFE_SPEED;
    }
    if m.input.strafe_right {
        dx -= sin * STRAFE_SPEED;
        dy += cos * STRAFE_SPEED;
    }

    // Movimiento por eje con colisión separada (sliding contra paredes).
    const RADIUS: f32 = 0.18;
    let new_x = m.px + dx;
    let new_y = m.py + dy;
    if !is_blocked(new_x, m.py, RADIUS) {
        m.px = new_x;
    }
    if !is_blocked(m.px, new_y, RADIUS) {
        m.py = new_y;
    }

    // Snapshot del material apuntado al centro de la pantalla
    // (rayo recto) — útil para HUD/debug.
    let snap = cast_ray(m.px, m.py, m.pa);
    m.last_hit_material = snap.material;

    // Avance de bullets + colisión vs pared + colisión vs enemy.
    advance_bullets(m);
    // AI y movimiento de enemies.
    advance_enemies(m);
    // Envejecimiento de decals + temp_lights.
    m.decals.retain(|d| d.ttl > 0);
    for d in m.decals.iter_mut() {
        d.ttl = d.ttl.saturating_sub(1);
    }
    m.temp_lights.retain(|tl| tl.ttl > 0);
    for tl in m.temp_lights.iter_mut() {
        tl.ttl = tl.ttl.saturating_sub(1);
    }
}

fn spawn_flash(m: &mut Model, x: f32, y: f32, color: (f32, f32, f32), strength: f32) {
    m.temp_lights.push(TempLight {
        x,
        y,
        color,
        strength,
        ttl: FLASH_TTL,
        ttl_max: FLASH_TTL,
    });
}

/// AI por enemy:
/// - Si está muerto/dying: solo decrementa countdown si dying.
/// - Si está vivo: chequea LOS al jugador; si la hay y dist <
///   `ENEMY_AGGRO_RANGE`, persigue. Si dist < `ENEMY_MELEE_RANGE` y
///   `attack_cd == 0`, pega al jugador y resetea cooldown.
fn advance_enemies(m: &mut Model) {
    let player_x = m.px;
    let player_y = m.py;
    let mut total_damage: u32 = 0;
    for e in m.enemies.iter_mut() {
        // Cooldown del ataque siempre decrementa.
        e.attack_cd = e.attack_cd.saturating_sub(1);
        match e.state {
            EnemyState::Dead => continue,
            EnemyState::Dying(rem) => {
                if rem <= 1 {
                    e.state = EnemyState::Dead;
                } else {
                    e.state = EnemyState::Dying(rem - 1);
                }
                continue;
            }
            EnemyState::Idle | EnemyState::Walking => {}
        }
        let dx = player_x - e.x;
        let dy = player_y - e.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > ENEMY_AGGRO_RANGE || !has_los(e.x, e.y, player_x, player_y) {
            e.state = EnemyState::Idle;
            continue;
        }
        e.state = EnemyState::Walking;
        // Melee: golpea cuando está pegado.
        if dist < ENEMY_MELEE_RANGE && e.attack_cd == 0 {
            total_damage = total_damage.saturating_add(ENEMY_MELEE_DAMAGE);
            e.attack_cd = ENEMY_MELEE_CD;
            continue;
        }
        // Persecución: vector unitario × speed, colisión cell-based.
        if dist > 0.01 {
            let inv = 1.0 / dist;
            let step_x = dx * inv * ENEMY_SPEED;
            let step_y = dy * inv * ENEMY_SPEED;
            // Eje X primero, eje Y después — sliding contra paredes.
            const ER: f32 = 0.18;
            let nx = e.x + step_x;
            if !is_blocked(nx, e.y, ER) {
                e.x = nx;
            }
            let ny = e.y + step_y;
            if !is_blocked(e.x, ny, ER) {
                e.y = ny;
            }
        }
    }
    if total_damage > 0 {
        m.health = m.health.saturating_sub(total_damage);
    }
}

/// Avanza cada bullet. Tres maneras de morir:
/// 1. Choca pared → decal + flash.
/// 2. Choca enemy alive (dist < `BULLET_HIT_RADIUS`) → enemy.hp -=
///    BULLET_DAMAGE + flash; sin decal.
/// 3. TTL agotado → muerte silenciosa.
fn advance_bullets(m: &mut Model) {
    let mut new_decals: Vec<Decal> = Vec::new();
    let mut new_flashes: Vec<(f32, f32)> = Vec::new();
    let mut bullet_hits_enemy: Vec<usize> = Vec::new(); // idx enemy
    let mut survivors: Vec<Bullet> = Vec::with_capacity(m.bullets.len());

    for mut b in m.bullets.drain(..) {
        if b.ttl == 0 {
            continue;
        }
        b.ttl -= 1;
        let nx = b.x + b.vx;
        let ny = b.y + b.vy;

        // 1. Pared.
        if tile(nx as i32, ny as i32) != 0 {
            new_decals.push(Decal {
                x: b.x,
                y: b.y,
                ttl: DECAL_TTL,
            });
            new_flashes.push((b.x, b.y));
            continue;
        }

        // 2. Enemy alive — chequea contra todos.
        let mut hit_enemy: Option<usize> = None;
        for (i, e) in m.enemies.iter().enumerate() {
            if matches!(e.state, EnemyState::Dead | EnemyState::Dying(_)) {
                continue;
            }
            let edx = e.x - nx;
            let edy = e.y - ny;
            if edx * edx + edy * edy < BULLET_HIT_RADIUS * BULLET_HIT_RADIUS {
                hit_enemy = Some(i);
                break;
            }
        }
        if let Some(i) = hit_enemy {
            bullet_hits_enemy.push(i);
            new_flashes.push((nx, ny));
            continue;
        }

        b.x = nx;
        b.y = ny;
        survivors.push(b);
    }
    m.bullets = survivors;

    for d in new_decals {
        if m.decals.len() >= MAX_DECALS {
            m.decals.remove(0);
        }
        m.decals.push(d);
    }
    for (fx, fy) in new_flashes {
        spawn_flash(m, fx, fy, FLASH_COLOR_IMPACT, FLASH_STRENGTH_IMPACT);
    }
    // Aplicar daño a enemies golpeados (puede ocurrir varias veces
    // contra el mismo enemy si varias balas lo tocan en el mismo tick).
    for i in bullet_hits_enemy {
        if i >= m.enemies.len() {
            continue;
        }
        let e = &mut m.enemies[i];
        if matches!(e.state, EnemyState::Dead | EnemyState::Dying(_)) {
            continue;
        }
        e.hp -= BULLET_DAMAGE;
        if e.hp <= 0 {
            e.state = EnemyState::Dying(ENEMY_DYING_TICKS);
        }
    }
}

fn is_blocked(x: f32, y: f32, r: f32) -> bool {
    // Bounding box AABB del jugador contra celdas.
    let x0 = (x - r).floor() as i32;
    let x1 = (x + r).floor() as i32;
    let y0 = (y - r).floor() as i32;
    let y1 = (y + r).floor() as i32;
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            if tile(cx, cy) != 0 {
                return true;
            }
        }
    }
    false
}

// =====================================================================
// Raycaster (DDA estilo Lode Vandevenne)
// =====================================================================

struct RayHit {
    /// Distancia perpendicular al plano de cámara (no euclidean — evita
    /// fish-eye en la altura del slice).
    perp_dist: f32,
    material: u8,
    /// `true` si la pared golpeada es E/W (vertical grid edge);
    /// `false` si N/S. Se usa para el sombreado tipo Doom.
    side_ew: bool,
    /// Posición horizontal del hit dentro de la pared, en `[0, 1)`.
    /// Las texturas procedurales por slice la usan para variar el
    /// patrón a lo largo de la pared (ladrillos, paneles).
    wall_x: f32,
}

fn cast_ray(px: f32, py: f32, ray_angle: f32) -> RayHit {
    let (sin, cos) = ray_angle.sin_cos();
    let dir_x = cos;
    let dir_y = sin;

    let delta_x = if dir_x.abs() < 1e-6 { 1e6 } else { (1.0_f32 / dir_x).abs() };
    let delta_y = if dir_y.abs() < 1e-6 { 1e6 } else { (1.0_f32 / dir_y).abs() };

    let mut map_x = px.floor() as i32;
    let mut map_y = py.floor() as i32;

    let (step_x, mut side_x) = if dir_x < 0.0 {
        (-1, (px - map_x as f32) * delta_x)
    } else {
        (1, (map_x as f32 + 1.0 - px) * delta_x)
    };
    let (step_y, mut side_y) = if dir_y < 0.0 {
        (-1, (py - map_y as f32) * delta_y)
    } else {
        (1, (map_y as f32 + 1.0 - py) * delta_y)
    };

    let mut side_ew = false;
    let mut hit = 0_u8;
    // Loop con tope alto por seguridad — el mapa está cerrado.
    for _ in 0..256 {
        if side_x < side_y {
            side_x += delta_x;
            map_x += step_x;
            side_ew = true;
        } else {
            side_y += delta_y;
            map_y += step_y;
            side_ew = false;
        }
        let t = tile(map_x, map_y);
        if t != 0 {
            hit = t;
            break;
        }
    }

    // Distancia perpendicular: una de las dos componentes según el lado.
    let perp = if side_ew {
        (map_x as f32 - px + (1 - step_x) as f32 * 0.5) / dir_x
    } else {
        (map_y as f32 - py + (1 - step_y) as f32 * 0.5) / dir_y
    };
    let perp = perp.max(0.0001);

    // wall_x: posición en la pared donde golpeó el rayo, normalizada
    // a [0, 1). Para paredes E/W (lado vertical) viene de Y; para N/S
    // de X. Es la coordenada que usan las texturas procedurales.
    let wall_x_raw = if side_ew {
        py + perp * dir_y
    } else {
        px + perp * dir_x
    };
    let wall_x = wall_x_raw - wall_x_raw.floor();

    RayHit {
        perp_dist: perp,
        material: hit,
        side_ew,
        wall_x,
    }
}

// =====================================================================
// Texturas procedurales — sin bitmaps. Cada material define un
// `texture_mul(wall_x, wall_y, tick)` que devuelve un multiplicador
// en `[0.6, 1.15]` aproximadamente. El renderer divide cada slice en
// SLICE_SEGMENTS bandas verticales y pinta cada una con su shade.
// =====================================================================

/// Cantidad de segmentos verticales por slice. Más = más detalle de
/// textura, más rects. 8 es buen compromiso visual/costo a 960×600
/// con COL_STRIDE = 3 (~320 cols × 8 segs = ~2560 rects/frame).
const SLICE_SEGMENTS: usize = 8;

/// Multiplicador de detalle textural. `wall_x ∈ [0, 1)` posición
/// horizontal en la pared, `wall_y ∈ [0, 1)` posición vertical del
/// segmento (0 = arriba), `tick` para texturas animadas (slime).
fn texture_mul(material: u8, wall_x: f32, wall_y: f32, tick: u64) -> f32 {
    match material {
        1 => techbase_mul(wall_x, wall_y),
        2 => brick_mul(wall_x, wall_y),
        3 => metal_mul(wall_x, wall_y),
        4 => slime_mul(wall_x, wall_y, tick),
        _ => 1.0,
    }
}

/// Techbase beige: junta horizontal sutil cada 0.25 unidades + leve
/// shade gradiente vertical. Plano y limpio.
fn techbase_mul(wall_x: f32, wall_y: f32) -> f32 {
    let _ = wall_x;
    // Junta cada 0.25 con grosor ~0.04.
    let row_pos = (wall_y * 4.0).fract();
    let joint = if row_pos < 0.05 || row_pos > 0.95 { 0.78 } else { 1.0 };
    // Gradiente vertical sutil (más oscuro abajo).
    let grad = 0.92 + 0.10 * (1.0 - wall_y);
    joint * grad
}

/// Ladrillo: filas alternadas con offset 0.5 (running bond típico),
/// juntas horizontales más oscuras + juntas verticales en cada
/// ladrillo. Visualmente "Doom HELL ladrillo".
fn brick_mul(wall_x: f32, wall_y: f32) -> f32 {
    // Filas de 0.25 de alto. Cada fila desplaza wall_x medio ladrillo.
    let row = (wall_y * 4.0).floor() as i32;
    let row_offset = if row % 2 == 0 { 0.0 } else { 0.5 };
    let bx = (wall_x + row_offset).fract();
    let by = (wall_y * 4.0).fract();
    // Junta horizontal (gruesa, oscura).
    let h_joint = if by < 0.10 { 0.55 } else { 1.0 };
    // Junta vertical cada 0.5 (ladrillos de medio metro).
    let v_pos = (bx * 2.0).fract();
    let v_joint = if v_pos < 0.06 || v_pos > 0.94 { 0.62 } else { 1.0 };
    // Variación interna por ladrillo (pseudo-random pero determinístico).
    let brick_id = ((bx * 2.0).floor() as i32 + row * 7) as u32;
    let variation = 0.96 + ((brick_id.wrapping_mul(2_654_435_761) >> 24) & 0xF) as f32 / 200.0;
    h_joint * v_joint * variation
}

/// Metal: paneles verticales (0.25 unidades) con bordes oscuros y
/// pequeños "tornillos" en las esquinas (puntos más oscuros).
fn metal_mul(wall_x: f32, wall_y: f32) -> f32 {
    let panel_x = (wall_x * 4.0).fract();
    // Bordes verticales del panel.
    let edge_v = if panel_x < 0.06 || panel_x > 0.94 { 0.72 } else { 1.0 };
    // Tornillos en esquinas (intersección de bordes).
    let near_top = wall_y < 0.06 || (wall_y - 0.5).abs() < 0.03;
    let near_edge = panel_x < 0.10 || panel_x > 0.90;
    let bolt = if near_top && near_edge { 0.55 } else { 1.0 };
    // Sutil highlight central por panel.
    let center_glow = 1.0 + 0.05 * (1.0 - (panel_x - 0.5).abs() * 2.0);
    edge_v * bolt * center_glow
}

/// Slime: patrón orgánico que ondula con el tick. Las celdas brillan
/// y se atenúan en olas — el efecto "fluido vivo" de Doom.
fn slime_mul(wall_x: f32, wall_y: f32, tick: u64) -> f32 {
    let t = tick as f32 * 0.08;
    let wave1 = (wall_y * 7.0 + t).sin() * 0.10;
    let wave2 = (wall_x * 5.0 - t * 0.7).sin() * 0.06;
    let speckle_phase = (wall_y * 17.0 + wall_x * 13.0 + t * 0.4).sin();
    let speckle = if speckle_phase > 0.85 { 0.15 } else { 0.0 };
    (1.0 + wave1 + wave2 + speckle).clamp(0.75, 1.20)
}

// =====================================================================
// Render — paint_with custom dentro del rect del nodo
// =====================================================================

fn scene_pane(model: &Model) -> View<Msg> {
    // Capturamos snapshot del frame. Todo Send+Sync trivial.
    let px = model.px;
    let py = model.py;
    let pa = model.pa;
    let tick = model.tick;
    let bullets = model.bullets.clone();
    let decals = model.decals.clone();
    let static_sprites = model.static_sprites.clone();
    let enemies = model.enemies.clone();
    let temp_lights = model.temp_lights.clone();

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .paint_with(move |scene, _ts, rect: PaintRect| {
        draw_scene(
            scene,
            rect,
            px,
            py,
            pa,
            tick,
            &bullets,
            &decals,
            &static_sprites,
            &enemies,
            &temp_lights,
        );
    })
}

/// Resolución de raycast (columnas verticales). Sub-muestreo a ~1
/// columna por 3 px de pantalla: el costo de cada rayo + slice baja
/// 3× y el resultado se ve casi igual (paredes son superficies
/// continuas). Bajalo a 1.0 si querés calidad full.
const COL_STRIDE: f32 = 3.0;

/// Luz ambiental mínima — sin esto los rincones sin luz son negro
/// puro. Doom original tenía ambient sectorial; acá un escalar
/// global más el aporte de las luces puntuales.
const AMBIENT: f32 = 0.18;

#[allow(clippy::too_many_arguments)]
fn draw_scene(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
    tick: u64,
    bullets: &[Bullet],
    decals: &[Decal],
    static_sprites: &[Sprite],
    enemies: &[Enemy],
    temp_lights: &[TempLight],
) {
    let w = rect.w as f64;
    let h = rect.h as f64;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    // Banding del cielo/piso — barato y enriquece el fondo. El
    // raycast pinta encima.
    draw_sky_and_floor(scene, rect);

    // Z-buffer por columna: perp_dist de la pared que cubre esa
    // columna. Los sprites se pintan después usando esto para
    // ocultarse detrás de paredes más cercanas.
    let total_cols = (w / COL_STRIDE as f64).max(1.0) as usize + 1;
    let mut z_buf: Vec<f32> = vec![f32::INFINITY; total_cols];

    // --- Pass 1: paredes con textura procedural por slice ---
    let mut x_pix = rect.x as f64;
    let x_end = (rect.x + rect.w) as f64;
    let mut i = 0_usize;
    while x_pix < x_end {
        let col_frac = i as f32 / total_cols as f32;
        let ray_angle = pa - FOV * 0.5 + FOV * col_frac;
        let hit = cast_ray(px, py, ray_angle);
        let cos_offset = (ray_angle - pa).cos().max(0.0001);
        let corrected = hit.perp_dist * cos_offset;

        // Hit world-point para iluminación por luces puntuales.
        let hit_x = px + hit.perp_dist * ray_angle.cos();
        let hit_y = py + hit.perp_dist * ray_angle.sin();
        let lights = lighting_contribution(hit_x, hit_y, tick, bullets, temp_lights);
        let mut light_mul = (
            (AMBIENT + lights.0).min(2.0),
            (AMBIENT + lights.1).min(2.0),
            (AMBIENT + lights.2).min(2.0),
        );
        if hit.side_ew {
            // Bias clásico de Doom para distinguir paredes E/W.
            light_mul.0 *= 0.78;
            light_mul.1 *= 0.78;
            light_mul.2 *= 0.78;
        }

        let base = material_color(hit.material);
        let lit = (base.0 * light_mul.0, base.1 * light_mul.1, base.2 * light_mul.2);

        // Altura del slice en píxeles.
        let line_h = (h / corrected as f64).min(h * 4.0);
        let y_mid = h * 0.5 + rect.y as f64;
        let y_top = y_mid - line_h * 0.5;
        let view_top = rect.y as f64;
        let view_bot = (rect.y + rect.h) as f64;
        let x_right = x_pix + COL_STRIDE as f64;

        // Subdivisión vertical en SLICE_SEGMENTS bandas: cada una con
        // su textura procedural aplicada. wall_y normalizado [0, 1).
        let seg_h_world = 1.0_f32 / SLICE_SEGMENTS as f32;
        for j in 0..SLICE_SEGMENTS {
            let wy_lo = j as f32 * seg_h_world;
            let wy_hi = (j + 1) as f32 * seg_h_world;
            let wy_mid = (wy_lo + wy_hi) * 0.5;
            let detail = texture_mul(hit.material, hit.wall_x, wy_mid, tick);
            let mut seg = (lit.0 * detail, lit.1 * detail, lit.2 * detail);
            seg = shade_by_dist(seg, hit.perp_dist);
            seg = apply_fog(seg, hit.perp_dist);

            let seg_y_top = (y_top + wy_lo as f64 * line_h).max(view_top);
            let seg_y_bot = (y_top + wy_hi as f64 * line_h).min(view_bot);
            if seg_y_bot <= seg_y_top {
                continue; // segmento entero fuera del viewport
            }
            let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                x_pix,
                seg_y_top,
                x_right,
                seg_y_bot,
            );
            scene.fill(
                Fill::NonZero,
                llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
                rgb(seg.0, seg.1, seg.2),
                None,
                &r,
            );
        }

        // Guardamos la perp_dist (no la corregida) para z-test con sprites.
        if i < z_buf.len() {
            z_buf[i] = hit.perp_dist;
        }

        x_pix += COL_STRIDE as f64;
        i += 1;
    }

    // --- Pass 2: sprites billboarded con z-test por columna ---
    // Combinamos en una sola lista todos los sprites del frame
    // (estáticos + enemies según su estado + bullets + decals) para
    // que `draw_sprites` los ordene por distancia y pinte de atrás
    // hacia adelante.
    let mut all_sprites: Vec<Sprite> = static_sprites.to_vec();
    for e in enemies {
        let (kind, scale) = match e.state {
            EnemyState::Idle | EnemyState::Walking => (SpriteKind::Imp, 0.85),
            EnemyState::Dying(_) => (SpriteKind::DyingImp, 0.65),
            EnemyState::Dead => (SpriteKind::Corpse, 0.30),
        };
        all_sprites.push(Sprite { x: e.x, y: e.y, kind, scale });
    }
    for b in bullets {
        all_sprites.push(Sprite {
            x: b.x,
            y: b.y,
            kind: SpriteKind::Bullet,
            scale: 0.15,
        });
    }
    for d in decals {
        all_sprites.push(Sprite {
            x: d.x,
            y: d.y,
            kind: SpriteKind::Decal,
            scale: 0.20,
        });
    }
    draw_sprites(
        scene,
        rect,
        px,
        py,
        pa,
        tick,
        &z_buf,
        total_cols,
        &all_sprites,
    );
    // Sutiles: avoid usar `temp_lights` solo para iluminación (ya
    // está) — los flashes en sí no se renderizan como sprites.
    let _ = temp_lights;

    // --- Overlay: crosshair + minimap ---
    draw_crosshair(scene, rect);
    draw_minimap(scene, rect, px, py, pa);
}

/// Pinta todos los sprites visibles. Para cada uno:
/// 1. Transforma `(sprite - player)` al espacio cámara con la inversa
///    de la matriz `[plane | dir]`. `transformed.y` es la profundidad
///    (>0 = delante).
/// 2. `screen_x_center = (w/2) · (1 + transformed.x / transformed.y)`.
/// 3. Altura proporcional a `1/depth` escalada por `sprite.scale`.
/// 4. Pinta columna por columna en el rango horizontal; oculta la
///    columna si la pared en esa columna tiene `perp_dist <= depth`.
fn draw_sprites(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
    tick: u64,
    z_buf: &[f32],
    total_cols: usize,
    sprites: &[Sprite],
) {
    let h = rect.h as f64;
    let half_fov = FOV * 0.5;
    let plane_len = half_fov.tan();
    // dir = (cos, sin); plane = perpendicular a dir · plane_len.
    let (sin_pa, cos_pa) = pa.sin_cos();
    let dir = (cos_pa, sin_pa);
    let plane = (-sin_pa * plane_len, cos_pa * plane_len);
    let inv_det = 1.0 / (plane.0 * dir.1 - dir.0 * plane.1);

    // Ordenar sprites por distancia descendente — los más lejanos
    // primero, así los cercanos pintan encima cuando se superponen.
    let mut visible: Vec<(usize, f32)> = sprites
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let dx = s.x - px;
            let dy = s.y - py;
            (i, dx * dx + dy * dy)
        })
        .collect();
    visible.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (idx, _) in visible {
        let s = &sprites[idx];
        let dx = s.x - px;
        let dy = s.y - py;
        // Transform al espacio cámara.
        let tx = inv_det * (dir.1 * dx - dir.0 * dy);
        let ty = inv_det * (-plane.1 * dx + plane.0 * dy);
        if ty <= 0.001 {
            continue; // detrás de la cámara
        }
        // Centro horizontal en columnas lógicas (0..total_cols).
        let screen_center_frac = 0.5 * (1.0 + tx / ty); // 0..1
        let center_col = screen_center_frac * total_cols as f32;
        // Tamaño aparente.
        let sprite_h = (h as f32 / ty * s.scale).min(h as f32 * 4.0);
        let sprite_w = sprite_h; // 1:1 aspect — los sprites Doom lo son
        let half_cols = (sprite_w * 0.5) / COL_STRIDE;
        let col_start = (center_col - half_cols).max(0.0) as usize;
        let col_end = ((center_col + half_cols).max(0.0) as usize).min(total_cols);

        let y_mid = h * 0.5 + rect.y as f64;
        // Anchor por kind:
        // - Barril/Pillar/Imp/Torch/Decal: apoyan en el "piso" del slice.
        //   Imp respira (bob vertical sinusoidal); Torch oscila sutil.
        // - Bullet: centrado a la altura del jugador (no toca piso ni
        //   techo, vuela horizontal).
        let bob = match s.kind {
            SpriteKind::Imp => (tick as f32 * 0.18).sin() * 0.05 * sprite_h,
            SpriteKind::Torch => (tick as f32 * 0.42).sin() * 0.015 * sprite_h,
            _ => 0.0,
        };
        let slice_h = (h as f32 / ty) as f64;
        let (y_top, y_bot) = match s.kind {
            SpriteKind::Bullet => {
                let half = sprite_h as f64 * 0.5;
                ((y_mid - half).max(rect.y as f64),
                 (y_mid + half).min((rect.y + rect.h) as f64))
            }
            _ => {
                let y_bot_g = (y_mid + slice_h * 0.5 + bob as f64).min((rect.y + rect.h) as f64);
                let y_top_g = (y_bot_g - sprite_h as f64).max(rect.y as f64);
                (y_top_g, y_bot_g)
            }
        };

        // Color con shading + fog + lighting puntual. Para sprites
        // dinámicos pasamos lista vacía de bullets/temp_lights (un
        // sprite no se ilumina a sí mismo; usa su color base).
        let (base, _appearance_h) = s.appearance();
        let lights = lighting_contribution(s.x, s.y, tick, &[], &[]);
        let light_mul = (
            (AMBIENT + lights.0).min(2.0),
            (AMBIENT + lights.1).min(2.0),
            (AMBIENT + lights.2).min(2.0),
        );
        let mut col = (base.0 * light_mul.0, base.1 * light_mul.1, base.2 * light_mul.2);
        col = shade_by_dist(col, ty);
        col = apply_fog(col, ty);
        let color = rgb(col.0, col.1, col.2);

        for cidx in col_start..col_end {
            if cidx >= z_buf.len() {
                break;
            }
            // Z-test: si la pared está más cerca, sprite tapado en esa col.
            if z_buf[cidx] < ty {
                continue;
            }
            let x_pix = rect.x as f64 + cidx as f64 * COL_STRIDE as f64;
            let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                x_pix,
                y_top,
                x_pix + COL_STRIDE as f64,
                y_bot,
            );
            scene.fill(
                Fill::NonZero,
                llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
                color,
                None,
                &r,
            );
        }
    }
}

/// Crosshair central — dos rectángulos finos cruzados con un punto
/// hueco en medio. No es interactivo, sólo orienta el aim.
fn draw_crosshair(scene: &mut llimphi_ui::llimphi_raster::vello::Scene, rect: PaintRect) {
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    let arm: f64 = 8.0;
    let thick: f64 = 1.5;
    let color = Color::from_rgba8(255, 240, 200, 180);
    // Horizontal.
    let h_rect = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        cx - arm,
        cy - thick * 0.5,
        cx + arm,
        cy + thick * 0.5,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        color,
        None,
        &h_rect,
    );
    // Vertical.
    let v_rect = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        cx - thick * 0.5,
        cy - arm,
        cx + thick * 0.5,
        cy + arm,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        color,
        None,
        &v_rect,
    );
    // Punto central — un pequeño cuadrado oscuro para marcar el aim.
    let dot = llimphi_ui::llimphi_raster::kurbo::Rect::new(cx - 1.0, cy - 1.0, cx + 1.0, cy + 1.0);
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(20, 10, 10, 220),
        None,
        &dot,
    );
}

fn draw_sky_and_floor(scene: &mut llimphi_ui::llimphi_raster::vello::Scene, rect: PaintRect) {
    let bands = 16_usize;
    let h = rect.h as f64;
    let band_h = h / bands as f64 * 0.5; // mitad superior = cielo, mitad inferior = piso
    let mid = rect.y as f64 + h * 0.5;
    for i in 0..bands {
        let y_top = mid - (i + 1) as f64 * band_h;
        let y_bot = mid - i as f64 * band_h;
        let frac = (i as f32 + 0.5) / bands as f32;
        let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
            rect.x as f64,
            y_top,
            (rect.x + rect.w) as f64,
            y_bot,
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            ceiling_color(1.0 - frac),
            None,
            &r,
        );
    }
    for i in 0..bands {
        let y_top = mid + i as f64 * band_h;
        let y_bot = mid + (i + 1) as f64 * band_h;
        let frac = (i as f32 + 0.5) / bands as f32;
        let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
            rect.x as f64,
            y_top,
            (rect.x + rect.w) as f64,
            y_bot,
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            floor_color(frac),
            None,
            &r,
        );
    }
}

fn draw_minimap(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
) {
    let cell: f64 = 9.0;
    let pad = 12.0_f64;
    let mm_w = cell * MAP_W as f64;
    let mm_h = cell * MAP_H as f64;
    let x0 = (rect.x + rect.w) as f64 - mm_w - pad;
    let y0 = rect.y as f64 + pad;

    // Fondo translúcido del minimap.
    let bg = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        x0 - 4.0,
        y0 - 4.0,
        x0 + mm_w + 4.0,
        y0 + mm_h + 4.0,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(0, 0, 0, 170),
        None,
        &bg,
    );

    // Celdas.
    for cy in 0..MAP_H {
        for cx in 0..MAP_W {
            let t = tile(cx as i32, cy as i32);
            if t == 0 {
                continue;
            }
            let (r, g, b) = material_color(t);
            let cell_rect = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                x0 + cx as f64 * cell,
                y0 + cy as f64 * cell,
                x0 + (cx + 1) as f64 * cell,
                y0 + (cy + 1) as f64 * cell,
            );
            scene.fill(
                Fill::NonZero,
                llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
                rgb(r, g, b),
                None,
                &cell_rect,
            );
        }
    }

    // Sprites estáticos como puntos coloreados según su tipo. Los
    // bullets/decals/enemies no van al minimap — son ruidosos o
    // requieren state que el minimap no recibe.
    for s in initial_static_sprites().iter() {
        let (base, _) = s.appearance();
        let dot = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (x0 + s.x as f64 * cell, y0 + s.y as f64 * cell),
            2.0,
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            rgb(base.0, base.1, base.2),
            None,
            &dot,
        );
    }

    // Luces como anillos suaves del color de la luz — visualizan el
    // radio de influencia aproximado en el minimap.
    for l in LIGHTS {
        let halo = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (x0 + l.x as f64 * cell, y0 + l.y as f64 * cell),
            (l.strength as f64).sqrt() * cell * 0.9,
        );
        scene.stroke(
            &Stroke::new(0.8),
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            Color::from_rgba8(
                (l.color.0 * 255.0) as u8,
                (l.color.1 * 255.0) as u8,
                (l.color.2 * 255.0) as u8,
                90,
            ),
            None,
            &halo,
        );
    }

    // Jugador + cono FOV.
    let pxc = x0 + px as f64 * cell;
    let pyc = y0 + py as f64 * cell;
    let fov_len = cell * 3.0;
    let left = pa - FOV * 0.5;
    let right = pa + FOV * 0.5;
    let mut path = BezPath::new();
    path.move_to((pxc, pyc));
    path.line_to((pxc + left.cos() as f64 * fov_len, pyc + left.sin() as f64 * fov_len));
    path.move_to((pxc, pyc));
    path.line_to((pxc + right.cos() as f64 * fov_len, pyc + right.sin() as f64 * fov_len));
    path.move_to((pxc, pyc));
    path.line_to((pxc + pa.cos() as f64 * fov_len * 1.1, pyc + pa.sin() as f64 * fov_len * 1.1));
    scene.stroke(
        &Stroke::new(1.0),
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(255, 200, 80, 220),
        None,
        &path,
    );

    let player_dot = llimphi_ui::llimphi_raster::kurbo::Circle::new((pxc, pyc), 2.5);
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(255, 220, 100, 255),
        None,
        &player_dot,
    );
}

fn main() {
    llimphi_ui::run::<Supay>();
}
