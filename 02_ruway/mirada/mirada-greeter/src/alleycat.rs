//! «alleycat» — un screensaver nocturno inspirado en el intro de *Alley Cat*
//! (1984, Bill Williams): un gato callejero que **prowlea** por la cresta de una
//! barda, bajo la luna y una silueta de ciudad, con su cola que ondea.
//!
//! Dos formas, misma escena:
//!
//! - [`paint`] — **stateless** (firma de [`crate::rain::paint`]): pinta el
//!   telón ([`paint_backdrop`]) + un gato vectorial con un ciclo de marcha
//!   por senos. Es el fallback barato y determinista (lo usa el despachador
//!   [`crate::bg`] y el `--shot` sin estado).
//! - [`AlleyCatBg`] — **stateful**: un rig esqueletal de `llimphi-anim` donde las
//!   patas se resuelven por **IK de dos huesos** (las pezuñas se plantan en su
//!   objetivo del ciclo de paso) y la **cola es una cadena Verlet** que ondea por
//!   secundario real. Se stepea en `RainTick`, como el fondo físico. Da la marcha
//!   «viva» que el seno no logra.
//!
//! *Inspiración*, no copia: se recrea el **gesto** procedural, no se portan los
//! sprites originales.

use llimphi_anim::constraint::solve_two_bone_ik;
use llimphi_anim::physics::Physics;
use llimphi_anim::skel::{BoneId, Pose, Skeleton};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, Stroke, Vec2};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

/// splitmix64 — para posiciones/alturas deterministas (estrellas, edificios).
fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn hf(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

/// Mezcla lineal entre dos colores RGB (sin alfa).
fn lerp_rgb(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    let t = t.clamp(0.0, 1.0);
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn col(r: f32, g: f32, b: f32, a: u8) -> Color {
    Color::from_rgba8(
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
        a,
    )
}

/// Fracción del telón donde está la cresta de la barda (el «piso» del gato).
const WALL_FRAC: f32 = 0.72;

/// Pinta **sólo el telón** nocturno (cielo, estrellas, luna, ciudad, barda) sobre
/// `rect`. `t` en segundos; `bright` el acento del tema. Lo comparten el fallback
/// stateless [`paint`] y el rig [`paint_rig`].
pub fn paint_backdrop(
    scene: &mut vello::Scene,
    _ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    if rect.w < 64.0 || rect.h < 64.0 {
        return;
    }
    let (x0, y0, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let acc = (bright.0 as f32, bright.1 as f32, bright.2 as f32);
    let wall_top = y0 + h * WALL_FRAC;

    // ── Cielo nocturno en degradé (franjas horizontales, barato y puro). ──
    let sky_top = (12.0, 14.0, 34.0);
    let sky_horizon = lerp_rgb((40.0, 32.0, 58.0), (acc.0, acc.1, acc.2), 0.10);
    const BANDS: i32 = 56;
    let band_h = (wall_top - y0) / BANDS as f32;
    for i in 0..BANDS {
        let f = i as f32 / (BANDS - 1) as f32;
        let (r, g, b) = lerp_rgb(sky_top, sky_horizon, f);
        let by = y0 + i as f32 * band_h;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(r, g, b, 255),
            None,
            &Rect::new(x0 as f64, by as f64, (x0 + w) as f64, (by + band_h + 1.0) as f64),
        );
    }

    // ── Estrellas que titilan. ──
    let star_n = ((w * (wall_top - y0)) / 9000.0).clamp(40.0, 220.0) as u64;
    for i in 0..star_n {
        let sx = x0 + hf(i ^ 0xA17) * w;
        let sy = y0 + hf(i ^ 0xB29) * (wall_top - y0) * 0.92;
        let phase = hf(i ^ 0xC3D) * 6.2832;
        let tw = 0.45 + 0.55 * ((t * 1.6 + phase).sin() * 0.5 + 0.5);
        let rad = 0.6 + 1.4 * hf(i ^ 0xD4E);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(225.0, 228.0, 245.0, (tw * 215.0) as u8),
            None,
            &Circle::new(Point::new(sx as f64, sy as f64), rad as f64),
        );
    }

    // ── Luna con halo (acento tenue del tema en el resplandor). ──
    let moon_c = Point::new((x0 + w * 0.80) as f64, (y0 + h * 0.20) as f64);
    let moon_r = (h * 0.07).clamp(22.0, 90.0) as f64;
    for k in (1..=4).rev() {
        let rr = moon_r * (1.0 + k as f64 * 0.55);
        let a = (26 / k) as u8;
        let (gr, gg, gb) = lerp_rgb((250.0, 245.0, 220.0), (acc.0, acc.1, acc.2), 0.35);
        scene.fill(Fill::NonZero, Affine::IDENTITY, col(gr, gg, gb, a), None, &Circle::new(moon_c, rr));
    }
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(252.0, 248.0, 226.0, 255),
        None,
        &Circle::new(moon_c, moon_r),
    );
    for (dx, dy, rr) in [(-0.30, -0.20, 0.22), (0.18, 0.10, 0.16), (-0.05, 0.30, 0.13)] {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(232.0, 228.0, 205.0, 255),
            None,
            &Circle::new(Point::new(moon_c.x + moon_r * dx, moon_c.y + moon_r * dy), moon_r * rr),
        );
    }

    // ── Silueta de ciudad detrás de la barda, con ventanas encendidas. ──
    let bldg_w = (w * 0.055).clamp(34.0, 120.0);
    let n_b = (w / bldg_w).ceil() as i32 + 1;
    let city_col = lerp_rgb(sky_horizon, (8.0, 9.0, 20.0), 0.7);
    for i in 0..n_b {
        let bx = x0 + i as f32 * bldg_w;
        let bh = (h * 0.10) + hf(i as u64 ^ 0x5151) * (h * 0.20);
        let top = wall_top - bh;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(city_col.0, city_col.1, city_col.2, 255),
            None,
            &Rect::new(bx as f64, top as f64, (bx + bldg_w - 2.0) as f64, wall_top as f64),
        );
        let wm = 6.0_f32;
        let cw = 8.0_f32;
        let ch = 10.0_f32;
        let cols = (((bldg_w - 2.0 * wm) / (cw + 4.0)).floor() as i32).max(0);
        let rows = (((bh - 2.0 * wm) / (ch + 5.0)).floor() as i32).max(0);
        for cy in 0..rows {
            for cx in 0..cols {
                let seed = hash(i as u64 ^ (cx as u64) << 8 ^ (cy as u64) << 16 ^ 0x9001);
                if seed % 5 == 0 {
                    let flick = 0.6 + 0.4 * ((t * 0.5 + hf(seed) * 6.2832).sin() * 0.5 + 0.5);
                    let wx = bx + wm + cx as f32 * (cw + 4.0);
                    let wy = top + wm + cy as f32 * (ch + 5.0);
                    let (lr, lg, lb) = lerp_rgb((255.0, 220.0, 140.0), acc, 0.35);
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        col(lr, lg, lb, (flick * 235.0) as u8),
                        None,
                        &Rect::new(wx as f64, wy as f64, (wx + cw) as f64, (wy + ch) as f64),
                    );
                }
            }
        }
    }

    // ── La barda de ladrillo sobre la que camina el gato. ──
    let wall_bot = y0 + h;
    let brick = (44.0, 36.0, 40.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(brick.0, brick.1, brick.2, 255),
        None,
        &Rect::new(x0 as f64, wall_top as f64, (x0 + w) as f64, wall_bot as f64),
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(96.0, 86.0, 92.0, 255),
        None,
        &Rect::new(x0 as f64, wall_top as f64, (x0 + w) as f64, (wall_top + 3.0) as f64),
    );
    let bh = 22.0_f32;
    let bw = 46.0_f32;
    let mortar = col(28.0, 22.0, 26.0, 255);
    let mut row = 0;
    let mut yy = wall_top + bh;
    while yy < wall_bot {
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, mortar, None, &line(x0, yy, x0 + w, yy));
        let off = if row % 2 == 0 { 0.0 } else { bw * 0.5 };
        let mut xx = x0 + off;
        while xx < x0 + w {
            scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, mortar, None, &line(xx, yy - bh, xx, yy));
            xx += bw;
        }
        row += 1;
        yy += bh;
    }
}

/// Fallback **stateless**: telón + un gato por senos que cruza en loop. Firma de
/// [`crate::rain::paint`]; lo usa el despachador y el `--shot` sin estado.
pub fn paint(
    scene: &mut vello::Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    if rect.w < 64.0 || rect.h < 64.0 {
        return;
    }
    paint_backdrop(scene, ts, rect, t, bright);

    let (x0, y0, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let acc = (bright.0 as f32, bright.1 as f32, bright.2 as f32);
    let wall_top = y0 + h * WALL_FRAC;

    let unit = (h * 0.045).clamp(10.0, 40.0);
    let span = w + 8.0 * unit;
    let speed = 2.4 * unit;
    let cycle = span / speed;
    let cat_t = (t % cycle) / cycle;
    let feet_x = x0 - 4.0 * unit + cat_t * span;
    let step = t * 7.0;
    let bob = (step * 2.0).sin() * unit * 0.06;
    let feet_y = wall_top + 2.0 - bob;
    paint_cat(scene, feet_x, feet_y, unit, step, acc);
}

/// Una línea como `BezPath`.
fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(Point::new(x1 as f64, y1 as f64));
    p.line_to(Point::new(x2 as f64, y2 as f64));
    p
}

// ───────────────────────── Fallback: gato por senos ─────────────────────────

fn paint_leg(scene: &mut vello::Scene, hip: Point, u: f64, swing: f64, lift: f64, color: Color) {
    let foot = Point::new(hip.x + swing * u * 0.9, hip.y + u * 1.5 - lift * u * 0.7);
    let knee = Point::new((hip.x + foot.x) * 0.5 + u * 0.18, (hip.y + foot.y) * 0.5);
    let mut p = BezPath::new();
    p.move_to(hip);
    p.line_to(knee);
    p.line_to(foot);
    scene.stroke(&Stroke::new(u * 0.34), Affine::IDENTITY, color, None, &p);
}

fn paint_cat(scene: &mut vello::Scene, fx: f32, fy: f32, u: f32, step: f32, acc: (f32, f32, f32)) {
    let fx = fx as f64;
    let fy = fy as f64;
    let u = u as f64;
    let step = step as f64;

    let fur = col(54.0, 56.0, 66.0, 255);
    let belly = col(78.0, 80.0, 92.0, 255);
    let fur_dark = col(38.0, 40.0, 50.0, 255);

    let bx = fx - 1.6 * u;
    let by = fy - 1.45 * u;

    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(0.0, 0.0, 0.0, 70),
        None,
        &Ellipse::new(Point::new(fx - 1.0 * u, fy + 0.10 * u), (2.6 * u, 0.45 * u), 0.0),
    );

    let s1 = step.sin();
    let s2 = (step + std::f64::consts::PI).sin();
    let lift1 = s1.max(0.0);
    let lift2 = s2.max(0.0);

    paint_leg(scene, Point::new(bx + 1.35 * u, by + 0.55 * u), u, s2, lift2, fur_dark);
    paint_leg(scene, Point::new(bx - 1.45 * u, by + 0.55 * u), u, s1, lift1, fur_dark);

    let tail_sway = (step * 1.3).sin();
    let tail_base = Point::new(bx - 2.0 * u, by - 0.1 * u);
    let tail_mid = Point::new(tail_base.x - 1.3 * u, tail_base.y - (0.9 + 0.5 * tail_sway) * u);
    let tail_tip = Point::new(tail_base.x - 1.7 * u + 0.6 * u * tail_sway, tail_base.y - (2.2 + 0.4 * tail_sway) * u);
    let mut tail = BezPath::new();
    tail.move_to(tail_base);
    tail.quad_to(tail_mid, tail_tip);
    scene.stroke(&Stroke::new(u * 0.42), Affine::IDENTITY, fur, None, &tail);

    scene.fill(Fill::NonZero, Affine::IDENTITY, fur, None, &Ellipse::new(Point::new(bx, by), (2.0 * u, 0.9 * u), 0.0));
    scene.fill(Fill::NonZero, Affine::IDENTITY, fur, None, &Circle::new(Point::new(bx - 1.5 * u, by - 0.05 * u), 0.95 * u));
    scene.fill(Fill::NonZero, Affine::IDENTITY, belly, None, &Ellipse::new(Point::new(bx + 0.1 * u, by + 0.55 * u), (1.5 * u, 0.4 * u), 0.0));

    let hx = bx + 2.1 * u;
    let hy = by - 0.7 * u;
    scene.fill(Fill::NonZero, Affine::IDENTITY, fur, None, &Circle::new(Point::new(hx, hy), 0.78 * u));
    paint_ears(scene, hx, hy, u, fur);
    scene.fill(Fill::NonZero, Affine::IDENTITY, belly, None, &Circle::new(Point::new(hx + 0.55 * u, hy + 0.18 * u), 0.34 * u));
    paint_eye(scene, Point::new(hx + 0.30 * u, hy - 0.08 * u), u, acc);

    paint_leg(scene, Point::new(bx + 1.4 * u, by + 0.6 * u), u, s1, lift1, fur);
    paint_leg(scene, Point::new(bx - 1.4 * u, by + 0.6 * u), u, s2, lift2, fur);
}

fn paint_ears(scene: &mut vello::Scene, hx: f64, hy: f64, u: f64, fur: Color) {
    for ear_dx in [-0.45, 0.5] {
        let ex = hx + ear_dx * u;
        let mut ear = BezPath::new();
        ear.move_to(Point::new(ex - 0.32 * u, hy - 0.55 * u));
        ear.line_to(Point::new(ex + 0.05 * u, hy - 1.35 * u));
        ear.line_to(Point::new(ex + 0.42 * u, hy - 0.5 * u));
        ear.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, fur, None, &ear);
    }
}

fn paint_eye(scene: &mut vello::Scene, eye: Point, u: f64, acc: (f32, f32, f32)) {
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(acc.0, acc.1, acc.2, 70), None, &Circle::new(eye, 0.34 * u));
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col((acc.0 + 180.0).min(255.0), (acc.1 + 180.0).min(255.0), (acc.2 + 180.0).min(255.0), 255),
        None,
        &Circle::new(eye, 0.16 * u),
    );
}

// ─────────────────────── Rig esqueletal con IK (stateful) ───────────────────

/// Espacio virtual 16:9 del rig; se escala al `rect` real al pintar (como
/// [`crate::bg_physics`]).
const VW: f64 = 1600.0;
const VH: f64 = 900.0;
/// Unidad de escala del gato en el espacio virtual.
const U: f64 = VH * 0.045;
/// Cresta de la barda en virtual (mismo `WALL_FRAC` que el telón).
const WALL_V: f64 = VH * WALL_FRAC as f64;
/// Centro del cuerpo (sin el bob) y cadera, en virtual.
const BODY_Y: f64 = WALL_V - 1.9 * U;
const HIP_DY: f64 = 0.55 * U; // cadera bajo el centro del cuerpo
/// Largo de cada segmento de pata (fémur / tibia).
const SEG: f64 = 0.85 * U;
/// Parámetros de la marcha.
const STRIDE: f64 = 1.15 * U;
const LIFT: f64 = 0.6 * U;
const DUTY: f64 = 0.62; // fracción del ciclo en apoyo
const SPEED_V: f64 = 2.6 * U; // px/s virtuales
/// Frecuencia de zancada: una zancada por cada `STRIDE` recorrido (sin patinar).
const FREQ: f64 = SPEED_V / STRIDE;

/// Una pata del rig: dos huesos (fémur=`upper`, tibia=`lower`) resueltos por IK.
struct Leg {
    upper: BoneId,
    lower: BoneId,
    hip_dx: f64,
    phase: f64,
    flip: bool,
    near: bool,
}

/// Posición del objetivo de la pezuña, relativa a la cadera apoyada, para una
/// fase `p` del ciclo en `[0,1)`. Apoyo: se arrastra hacia atrás sobre el piso;
/// vuelo: arco hacia adelante levantando la pata.
fn gait_foot(p: f64) -> (f64, f64) {
    if p < DUTY {
        let ps = p / DUTY;
        (STRIDE * (0.5 - ps), 0.0)
    } else {
        let pw = (p - DUTY) / (1.0 - DUTY);
        (STRIDE * (pw - 0.5), -LIFT * (std::f64::consts::PI * pw).sin())
    }
}

fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// Fondo «Alley Cat» con estado: rig de patas por IK + cola Verlet. Espejo del
/// patrón de [`crate::bg_physics::PhysicsBg`] (se stepea en `RainTick`).
pub struct AlleyCatBg {
    skel: Skeleton,
    root: BoneId,
    legs: Vec<Leg>,
    tail: Physics,
    body_x: f64,
    body_cy: f64,
    gait: f64,
    t: f64,
    accent: (f32, f32, f32),
}

/// Cuántas partículas tiene la cola.
const TAIL_N: usize = 8;
const TAIL_SEG: f64 = 0.42 * U;

impl AlleyCatBg {
    /// Construye el rig. `bright` tiñe el acento (ojo, halo).
    pub fn new(bright: (u8, u8, u8)) -> Self {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(None, Pose::identity());

        // Cuatro patas: pares cercano/lejano, fase en trote diagonal.
        // (front_near + back_far) en fase; (back_near + front_far) en contrafase.
        let defs = [
            (1.30, 0.5, true, false),  // delantera lejana
            (-1.45, 0.0, false, false), // trasera lejana
            (1.40, 0.0, true, true),   // delantera cercana
            (-1.35, 0.5, false, true), // trasera cercana
        ];
        let mut legs = Vec::with_capacity(4);
        for (hip_dx, phase, flip, near) in defs {
            let upper = skel.add_bone(
                Some(root),
                Pose::new(Vec2::new(hip_dx * U, HIP_DY), std::f64::consts::FRAC_PI_2, Vec2::new(1.0, 1.0)),
            );
            let lower = skel.add_bone(Some(upper), Pose::translate(Vec2::new(SEG, 0.0)));
            legs.push(Leg { upper, lower, hip_dx: hip_dx * U, phase, flip, near });
        }
        skel.bind();

        // Cola: cadena Verlet anclada a la grupa, colgando hacia atrás.
        let mut tail = Physics::new();
        let base = Point::new(-2.0 * U, BODY_Y - 0.15 * U);
        let mut prev = tail.particle(base, true);
        for i in 1..TAIL_N {
            let p = tail.particle(Point::new(base.x - i as f64 * TAIL_SEG, base.y), false);
            tail.link_with(prev, p, TAIL_SEG, 1.0);
            prev = p;
        }

        let mut me = Self {
            skel,
            root,
            legs,
            tail,
            body_x: -4.0 * U,
            body_cy: BODY_Y,
            gait: 0.0,
            t: 0.0,
            accent: (bright.0 as f32, bright.1 as f32, bright.2 as f32),
        };
        me.pose(0.0);
        me
    }

    /// Avanza el reloj `dt` y re-resuelve la pose (cuerpo, IK de patas, cola).
    pub fn step(&mut self, dt: f64) {
        self.t += dt;
        self.gait += FREQ * dt;
        self.body_x += SPEED_V * dt;
        let span = VW + 8.0 * U;
        if self.body_x > VW + 4.0 * U {
            self.body_x -= span;
        }
        self.pose(dt);
    }

    /// Posa el esqueleto para el estado actual. `dt` stepea la cola (0 = solo
    /// recolocar el ancla, p. ej. al construir).
    fn pose(&mut self, dt: f64) {
        // Bob vertical: dos rebotes por zancada.
        let bob = 0.05 * U * (self.gait * std::f64::consts::TAU * 2.0).sin();
        self.body_cy = BODY_Y + bob;

        self.skel.set_pose(
            self.root,
            Pose::new(Vec2::new(self.body_x, self.body_cy), 0.0, Vec2::new(1.0, 1.0)),
        );
        self.skel.update();

        for leg in &self.legs {
            let p = frac(self.gait + leg.phase);
            let (rx, ry) = gait_foot(p);
            let target = Point::new(self.body_x + leg.hip_dx + rx, WALL_V + ry);
            solve_two_bone_ik(&mut self.skel, leg.upper, leg.lower, Vec2::new(SEG, 0.0), target, leg.flip);
        }

        // Cola: ancla en la grupa (sigue al cuerpo); viento senoidal + gravedad
        // mansa → ondea por detrás con secundario real.
        let rump = Point::new(self.body_x - 2.0 * U, self.body_cy - 0.15 * U);
        if let Some(p0) = self.tail.particles.first_mut() {
            p0.pos = rump;
            p0.prev = rump;
        }
        if dt > 0.0 {
            let wind = (self.t * 2.2).sin() * 420.0 + (self.t * 3.7).sin() * 150.0;
            self.tail.gravity = Vec2::new(wind, 260.0);
            self.tail.step(dt, 6);
        }
    }

    /// Snapshot de dibujo del frame (articulaciones en virtual + cola + acento),
    /// para moverlo a un closure `'static` y pintarlo con [`paint_rig`].
    pub fn snapshot(&self) -> CatSnapshot {
        let legs = self
            .legs
            .iter()
            .map(|l| {
                let hip = self.skel.world(l.upper) * Point::ZERO;
                let knee = self.skel.world(l.lower) * Point::ZERO;
                let foot = self.skel.world(l.lower) * Point::new(SEG, 0.0);
                (hip, knee, foot, l.near)
            })
            .collect();
        let head_nod = 0.04 * U * (self.gait * std::f64::consts::TAU).sin();
        CatSnapshot {
            legs,
            body: Point::new(self.body_x, self.body_cy),
            head: Point::new(self.body_x + 2.1 * U, self.body_cy - 0.7 * U + head_nod),
            tail: self.tail.positions(),
            accent: self.accent,
        }
    }
}

/// Datos de dibujo de un frame del gato (todo en espacio virtual `VW×VH`).
pub struct CatSnapshot {
    /// Por pata: `(cadera, rodilla, pezuña, cercana)`.
    pub legs: Vec<(Point, Point, Point, bool)>,
    pub body: Point,
    pub head: Point,
    pub tail: Vec<Point>,
    pub accent: (f32, f32, f32),
}

/// Pinta el telón + el gato del rig sobre `rect`, escalando el espacio virtual.
pub fn paint_rig(
    snap: &CatSnapshot,
    scene: &mut vello::Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    paint_backdrop(scene, ts, rect, t, bright);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let xf = Affine::translate((rect.x as f64, rect.y as f64))
        * Affine::scale_non_uniform(rect.w as f64 / VW, rect.h as f64 / VH);

    let fur = col(54.0, 56.0, 66.0, 255);
    let belly = col(78.0, 80.0, 92.0, 255);
    let fur_dark = col(38.0, 40.0, 50.0, 255);
    let b = snap.body;

    // Sombra sobre la cresta de la barda.
    scene.fill(
        Fill::NonZero,
        xf,
        col(0.0, 0.0, 0.0, 70),
        None,
        &Ellipse::new(Point::new(b.x - 0.5 * U, WALL_V + 0.05 * U), (2.7 * U, 0.4 * U), 0.0),
    );

    // Patas del lado lejano (detrás del cuerpo, más oscuras).
    for (hip, knee, foot, near) in &snap.legs {
        if !*near {
            paint_rig_leg(scene, xf, *hip, *knee, *foot, fur_dark);
        }
    }

    // Cola (cadena Verlet) por detrás del cuerpo.
    if snap.tail.len() >= 2 {
        let mut path = BezPath::new();
        path.move_to(snap.tail[0]);
        for p in &snap.tail[1..] {
            path.line_to(*p);
        }
        scene.stroke(&Stroke::new(0.42 * U), xf, fur, None, &path);
        // Punta apenas más clara.
        if let Some(tip) = snap.tail.last() {
            scene.fill(Fill::NonZero, xf, belly, None, &Circle::new(*tip, 0.22 * U));
        }
    }

    // Cuerpo + grupa + vientre.
    scene.fill(Fill::NonZero, xf, fur, None, &Ellipse::new(b, (2.0 * U, 0.9 * U), 0.0));
    scene.fill(Fill::NonZero, xf, fur, None, &Circle::new(Point::new(b.x - 1.5 * U, b.y - 0.05 * U), 0.95 * U));
    scene.fill(Fill::NonZero, xf, belly, None, &Ellipse::new(Point::new(b.x + 0.1 * U, b.y + 0.55 * U), (1.5 * U, 0.4 * U), 0.0));

    // Cabeza, orejas, hocico, ojo.
    let head = snap.head;
    scene.fill(Fill::NonZero, xf, fur, None, &Circle::new(head, 0.78 * U));
    for ear_dx in [-0.45, 0.5] {
        let ex = head.x + ear_dx * U;
        let mut ear = BezPath::new();
        ear.move_to(Point::new(ex - 0.32 * U, head.y - 0.55 * U));
        ear.line_to(Point::new(ex + 0.05 * U, head.y - 1.35 * U));
        ear.line_to(Point::new(ex + 0.42 * U, head.y - 0.5 * U));
        ear.close_path();
        scene.fill(Fill::NonZero, xf, fur, None, &ear);
    }
    scene.fill(Fill::NonZero, xf, belly, None, &Circle::new(Point::new(head.x + 0.55 * U, head.y + 0.18 * U), 0.34 * U));
    let eye = Point::new(head.x + 0.30 * U, head.y - 0.08 * U);
    let acc = snap.accent;
    scene.fill(Fill::NonZero, xf, col(acc.0, acc.1, acc.2, 70), None, &Circle::new(eye, 0.34 * U));
    scene.fill(
        Fill::NonZero,
        xf,
        col((acc.0 + 180.0).min(255.0), (acc.1 + 180.0).min(255.0), (acc.2 + 180.0).min(255.0), 255),
        None,
        &Circle::new(eye, 0.16 * U),
    );

    // Patas del lado cercano (encima del cuerpo, más claras).
    for (hip, knee, foot, near) in &snap.legs {
        if *near {
            paint_rig_leg(scene, xf, *hip, *knee, *foot, fur);
        }
    }
}

/// Pinta una pata del rig: trazo cadera→rodilla→pezuña + pata redonda.
fn paint_rig_leg(scene: &mut vello::Scene, xf: Affine, hip: Point, knee: Point, foot: Point, color: Color) {
    let mut p = BezPath::new();
    p.move_to(hip);
    p.line_to(knee);
    p.line_to(foot);
    scene.stroke(&Stroke::new(0.32 * U), xf, color, None, &p);
    scene.fill(Fill::NonZero, xf, color, None, &Circle::new(foot, 0.2 * U));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rig_marcha_viva_y_finita() {
        let mut bg = AlleyCatBg::new((255, 200, 120));
        let s0 = bg.snapshot();
        assert_eq!(s0.legs.len(), 4, "cuatro patas");
        // Geometría inicial finita y razonable.
        for (hip, knee, foot, _) in &s0.legs {
            for p in [hip, knee, foot] {
                assert!(p.x.is_finite() && p.y.is_finite(), "articulación finita");
            }
            // La pezuña cae cerca de la cresta de la barda (IK la planta).
            assert!((foot.y - WALL_V).abs() < LIFT + 1.0, "pezuña sobre la barda");
        }
        assert_eq!(s0.tail.len(), TAIL_N, "cola con todas sus partículas");

        // Tras ~2 s, el cuerpo avanzó y las pezuñas se movieron (marcha viva),
        // sin explotar a NaN/infinito.
        for _ in 0..60 {
            bg.step(1.0 / 30.0);
        }
        let s1 = bg.snapshot();
        assert!(s1.body.x > s0.body.x + U, "el cuerpo avanza");
        let moved = s0
            .legs
            .iter()
            .zip(&s1.legs)
            .any(|((_, _, f0, _), (_, _, f1, _))| (f0.x - f1.x).abs() + (f0.y - f1.y).abs() > 1.0);
        assert!(moved, "las pezuñas deben moverse (marcha)");
        for (hip, knee, foot, _) in &s1.legs {
            for p in [hip, knee, foot] {
                assert!(p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e6 && p.y.abs() < 1e6, "no explota");
            }
        }
        // La cola es secundario vivo: su punta se mueve respecto al inicio.
        let (t0, t1) = (*s0.tail.last().unwrap(), *s1.tail.last().unwrap());
        assert!((t0.x - t1.x).abs() + (t0.y - t1.y).abs() > 1.0, "la cola ondea");
    }

    #[test]
    fn rig_loop_envuelve() {
        let mut bg = AlleyCatBg::new((255, 200, 120));
        // Avanzar bastante: el gato cruza y reaparece (no se va a +inf).
        for _ in 0..4000 {
            bg.step(1.0 / 30.0);
        }
        let s = bg.snapshot();
        assert!(s.body.x.is_finite() && s.body.x < VW + 8.0 * U, "el cuerpo envuelve al salir de cuadro");
    }
}
