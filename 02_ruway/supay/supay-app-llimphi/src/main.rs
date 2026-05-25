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

use llimphi_theme::Theme;
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
        }
    }
}

const SPRITES: &[Sprite] = &[
    Sprite { x: 4.5, y: 3.5, kind: SpriteKind::Barrel, scale: 0.5 },
    Sprite { x: 7.5, y: 5.5, kind: SpriteKind::Imp, scale: 0.85 },
    Sprite { x: 11.5, y: 4.5, kind: SpriteKind::Pillar, scale: 1.0 },
    Sprite { x: 6.5, y: 9.5, kind: SpriteKind::Barrel, scale: 0.5 },
    Sprite { x: 12.5, y: 11.5, kind: SpriteKind::Imp, scale: 0.85 },
    Sprite { x: 8.5, y: 12.5, kind: SpriteKind::Torch, scale: 0.7 },
    Sprite { x: 3.5, y: 13.5, kind: SpriteKind::Torch, scale: 0.7 },
];

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
fn lighting_contribution(hit_x: f32, hit_y: f32) -> (f32, f32, f32) {
    let mut acc = (0.0_f32, 0.0_f32, 0.0_f32);
    for l in LIGHTS {
        let dx = l.x - hit_x;
        let dy = l.y - hit_y;
        let d2 = dx * dx + dy * dy;
        let atten = l.strength / (1.0 + 0.6 * d2);
        acc.0 += l.color.0 * atten;
        acc.1 += l.color.1 * atten;
        acc.2 += l.color.2 * atten;
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
}

#[derive(Clone)]
enum Msg {
    Tick,
    Key(KeyEvent),
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
        }
    }

    fn on_key(_: &Model, e: &KeyEvent) -> Option<Msg> {
        if matches!(&e.key, Key::Named(NamedKey::Escape)) && e.state == KeyState::Pressed {
            return Some(Msg::Quit);
        }
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Quit => {
                handle.quit();
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
        let theme = Theme::dark();
        let header = header_bar(model, &theme);
        let scene = scene_pane(model);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(rgb(0.02, 0.02, 0.03))
        .children(vec![header, scene])
    }
}

fn header_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let mat_name = match model.last_hit_material {
        1 => "techbase",
        2 => "ladrillo",
        3 => "metal",
        4 => "slime",
        _ => "—",
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
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
    .fill(theme.bg_panel)
    .text_aligned(
        format!(
            "supay · ({:.2}, {:.2}) · θ {:.2} · tick {} · centro: {}",
            model.px, model.py, model.pa, model.tick, mat_name
        ),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    )
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

    RayHit {
        perp_dist: perp,
        material: hit,
        side_ew,
    }
}

// =====================================================================
// Render — paint_with custom dentro del rect del nodo
// =====================================================================

fn scene_pane(model: &Model) -> View<Msg> {
    // Capturamos el snapshot del jugador para la closure (paint_with
    // necesita Send+Sync; (f32, f32, f32) lo es trivialmente).
    let px = model.px;
    let py = model.py;
    let pa = model.pa;

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
        draw_scene(scene, rect, px, py, pa);
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

fn draw_scene(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
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

    // --- Pass 1: paredes ---
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
        let lights = lighting_contribution(hit_x, hit_y);
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
        let mut col = (base.0 * light_mul.0, base.1 * light_mul.1, base.2 * light_mul.2);
        col = shade_by_dist(col, hit.perp_dist);
        col = apply_fog(col, hit.perp_dist);

        // Altura del slice en píxeles.
        let line_h = (h / corrected as f64).min(h * 4.0);
        let y_mid = h * 0.5 + rect.y as f64;
        let y_top = y_mid - line_h * 0.5;
        let y_bot = y_mid + line_h * 0.5;

        let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
            x_pix,
            y_top.max(rect.y as f64),
            x_pix + COL_STRIDE as f64,
            y_bot.min((rect.y + rect.h) as f64),
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            rgb(col.0, col.1, col.2),
            None,
            &r,
        );

        // Guardamos la perp_dist (no la corregida) para z-test con sprites.
        if i < z_buf.len() {
            z_buf[i] = hit.perp_dist;
        }

        x_pix += COL_STRIDE as f64;
        i += 1;
    }

    // --- Pass 2: sprites billboarded con z-test por columna ---
    draw_sprites(scene, rect, px, py, pa, &z_buf, total_cols);

    // --- Overlay: minimap arriba a la derecha ---
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
    z_buf: &[f32],
    total_cols: usize,
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
    let mut visible: Vec<(usize, f32)> = SPRITES
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
        let s = &SPRITES[idx];
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
        // El sprite apoya en el "piso" del slice: y_bot fijo al piso de
        // un slice de altura full = h/ty, y_top sube según scale.
        let slice_h = (h as f32 / ty) as f64;
        let y_bot = (y_mid + slice_h * 0.5).min((rect.y + rect.h) as f64);
        let y_top = (y_bot - sprite_h as f64).max(rect.y as f64);

        // Color con shading + fog + lighting puntual.
        let (base, _appearance_h) = s.appearance();
        let lights = lighting_contribution(s.x, s.y);
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

    // Sprites como puntos coloreados según su tipo.
    for s in SPRITES {
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
