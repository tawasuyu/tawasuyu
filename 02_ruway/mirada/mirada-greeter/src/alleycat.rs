//! «alleycat» — un screensaver nocturno inspirado en el intro de *Alley Cat*
//! (1984, Bill Williams): un gato callejero que **prowlea** por la cresta de una
//! barda, bajo la luna y una silueta de ciudad, con su cola que ondea.
//!
//! Render **puro y determinista**: la escena (cielo en degradé por franjas, luna
//! con halo, estrellas que titilan, ciudad con ventanas encendidas, barda de
//! ladrillo) se deriva sólo de `t`; el gato es un muñeco vectorial con ciclo de
//! marcha (trote diagonal) animado por la fase de paso. Sin estado entre frames.
//! Comparte firma con [`crate::rain::paint`]; `ts` no se usa.
//!
//! *Inspiración*, no copia: se recrea el **gesto** procedural, no se portan los
//! sprites originales. El laburo es la animación; la cañería (el despachador
//! [`crate::bg`]) ya estaba.

use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, Stroke};
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

pub fn paint(
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
    // El acento del tema (paleta del menú «Fondo») tiñe ventanas, ojos y halo.
    let acc = (bright.0 as f32, bright.1 as f32, bright.2 as f32);

    // La cresta de la barda: donde el gato pisa. ~72 % de la altura.
    let wall_top = y0 + h * 0.72;

    // ── Cielo nocturno en degradé (franjas horizontales, barato y puro). ──
    let sky_top = (12.0, 14.0, 34.0); // azul de medianoche
    let sky_horizon = lerp_rgb((40.0, 32.0, 58.0), (acc.0, acc.1, acc.2), 0.10); // malva tibio
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
            &Rect::new(
                x0 as f64,
                by as f64,
                (x0 + w) as f64,
                (by + band_h + 1.0) as f64,
            ),
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
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(gr, gg, gb, a),
            None,
            &Circle::new(moon_c, rr),
        );
    }
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(252.0, 248.0, 226.0, 255),
        None,
        &Circle::new(moon_c, moon_r),
    );
    // Un par de cráteres (mordida sutil con el azul del cielo de fondo).
    for (dx, dy, rr) in [(-0.30, -0.20, 0.22), (0.18, 0.10, 0.16), (-0.05, 0.30, 0.13)] {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(232.0, 228.0, 205.0, 255),
            None,
            &Circle::new(
                Point::new(moon_c.x + moon_r * dx, moon_c.y + moon_r * dy),
                moon_r * rr,
            ),
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
            &Rect::new(
                bx as f64,
                top as f64,
                (bx + bldg_w - 2.0) as f64,
                wall_top as f64,
            ),
        );
        // Rejilla de ventanas; algunas prendidas (acento que titila lento).
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
                        &Rect::new(
                            wx as f64,
                            wy as f64,
                            (wx + cw) as f64,
                            (wy + ch) as f64,
                        ),
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
    // Filo iluminado por la luna en la cresta.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(96.0, 86.0, 92.0, 255),
        None,
        &Rect::new(
            x0 as f64,
            wall_top as f64,
            (x0 + w) as f64,
            (wall_top + 3.0) as f64,
        ),
    );
    // Líneas de mortero (hiladas + juntas alternadas).
    let bh = 22.0_f32;
    let bw = 46.0_f32;
    let mortar = col(28.0, 22.0, 26.0, 255);
    let mut row = 0;
    let mut yy = wall_top + bh;
    while yy < wall_bot {
        scene.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            mortar,
            None,
            &line(x0, yy, x0 + w, yy),
        );
        let off = if row % 2 == 0 { 0.0 } else { bw * 0.5 };
        let mut xx = x0 + off;
        while xx < x0 + w {
            scene.stroke(
                &Stroke::new(1.5),
                Affine::IDENTITY,
                mortar,
                None,
                &line(xx, yy - bh, xx, yy),
            );
            xx += bw;
        }
        row += 1;
        yy += bh;
    }

    // ── El gato. Camina de izquierda a derecha y reaparece (loop). ──
    let unit = (h * 0.045).clamp(10.0, 40.0);
    let span = w + 8.0 * unit; // entra y sale fuera de cuadro
    let speed = 2.4 * unit; // px/s aprox.
    let cycle = span / speed;
    let cat_t = (t % cycle) / cycle; // 0..1
    let feet_x = x0 - 4.0 * unit + cat_t * span;
    // La fase de paso avanza con el tiempo; un leve bob vertical del cuerpo.
    let step = t * 7.0;
    let bob = (step * 2.0).sin() * unit * 0.06;
    let feet_y = wall_top + 2.0 - bob;
    paint_cat(scene, feet_x, feet_y, unit, step, acc);
}

/// Una línea como `BezPath` (kurbo `Line` no implementa `Shape` para `stroke`
/// directo acá; un path de dos puntos es lo simple y seguro).
fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(Point::new(x1 as f64, y1 as f64));
    p.line_to(Point::new(x2 as f64, y2 as f64));
    p
}

/// Dibuja una pata de dos segmentos (cadera→rodilla→pata) con la rodilla
/// empujada hacia afuera. `swing` en `[-1,1]` adelanta/atrasa la pata; `lift`
/// en `[0,1]` la levanta en la fase de vuelo.
fn paint_leg(
    scene: &mut vello::Scene,
    hip: Point,
    u: f64,
    swing: f64,
    lift: f64,
    color: Color,
) {
    let foot = Point::new(
        hip.x + swing * u * 0.9,
        hip.y + u * 1.5 - lift * u * 0.7,
    );
    let knee = Point::new((hip.x + foot.x) * 0.5 + u * 0.18, (hip.y + foot.y) * 0.5);
    let mut p = BezPath::new();
    p.move_to(hip);
    p.line_to(knee);
    p.line_to(foot);
    scene.stroke(
        &Stroke::new((u * 0.34) as f64),
        Affine::IDENTITY,
        color,
        None,
        &p,
    );
}

/// Pinta el gato con la pata delantera derecha apoyada en `(fx, fy)`. `u` es la
/// unidad de escala; `step` la fase de marcha; `acc` el acento del tema (ojos).
fn paint_cat(scene: &mut vello::Scene, fx: f32, fy: f32, u: f32, step: f32, acc: (f32, f32, f32)) {
    let fx = fx as f64;
    let fy = fy as f64;
    let u = u as f64;
    let step = step as f64;

    // Charcoal de gato callejero; vientre apenas más claro.
    let fur = col(54.0, 56.0, 66.0, 255);
    let belly = col(78.0, 80.0, 92.0, 255);
    let fur_dark = col(38.0, 40.0, 50.0, 255); // patas del lado lejano

    // Centro del cuerpo, un poco detrás del punto de apoyo delantero.
    let bx = fx - 1.6 * u;
    let by = fy - 1.45 * u;

    // Sombra elíptica en la cresta.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(0.0, 0.0, 0.0, 70),
        None,
        &Ellipse::new(Point::new(fx - 1.0 * u, fy + 0.10 * u), (2.6 * u, 0.45 * u), 0.0),
    );

    // Trote diagonal: pares (delantera-cerca, trasera-lejos) y viceversa.
    let s1 = step.sin();
    let s2 = (step + std::f64::consts::PI).sin();
    let lift1 = s1.max(0.0);
    let lift2 = s2.max(0.0);

    // Patas del lado lejano (más oscuras, se dibujan primero = detrás).
    paint_leg(scene, Point::new(bx + 1.35 * u, by + 0.55 * u), u, s2 as f64, lift2 as f64, fur_dark); // delantera lejana
    paint_leg(scene, Point::new(bx - 1.45 * u, by + 0.55 * u), u, s1 as f64, lift1 as f64, fur_dark); // trasera lejana

    // Cola: bezier que sale de la grupa y ondea con la fase.
    let tail_sway = (step * 1.3).sin();
    let tail_base = Point::new(bx - 2.0 * u, by - 0.1 * u);
    let tail_mid = Point::new(
        tail_base.x - 1.3 * u,
        tail_base.y - (0.9 + 0.5 * tail_sway) * u,
    );
    let tail_tip = Point::new(
        tail_base.x - 1.7 * u + 0.6 * u * tail_sway,
        tail_base.y - (2.2 + 0.4 * tail_sway) * u,
    );
    let mut tail = BezPath::new();
    tail.move_to(tail_base);
    tail.quad_to(tail_mid, tail_tip);
    scene.stroke(
        &Stroke::new((u * 0.42) as f64),
        Affine::IDENTITY,
        fur,
        None,
        &tail,
    );

    // Cuerpo (elipse) + grupa.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        fur,
        None,
        &Ellipse::new(Point::new(bx, by), (2.0 * u, 0.9 * u), 0.0),
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        fur,
        None,
        &Circle::new(Point::new(bx - 1.5 * u, by - 0.05 * u), 0.95 * u),
    );
    // Vientre, una pincelada más clara abajo.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        belly,
        None,
        &Ellipse::new(Point::new(bx + 0.1 * u, by + 0.55 * u), (1.5 * u, 0.4 * u), 0.0),
    );

    // Cabeza al frente, arriba.
    let hx = bx + 2.1 * u;
    let hy = by - 0.7 * u;
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        fur,
        None,
        &Circle::new(Point::new(hx, hy), 0.78 * u),
    );
    // Orejas (dos triángulos).
    for ear_dx in [-0.45, 0.5] {
        let ex = hx + ear_dx * u;
        let mut ear = BezPath::new();
        ear.move_to(Point::new(ex - 0.32 * u, hy - 0.55 * u));
        ear.line_to(Point::new(ex + 0.05 * u, hy - 1.35 * u));
        ear.line_to(Point::new(ex + 0.42 * u, hy - 0.5 * u));
        ear.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, fur, None, &ear);
    }
    // Hocico (más claro) y ojo brillante (acento del tema, con halo).
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        belly,
        None,
        &Circle::new(Point::new(hx + 0.55 * u, hy + 0.18 * u), 0.34 * u),
    );
    let eye = Point::new(hx + 0.30 * u, hy - 0.08 * u);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(acc.0, acc.1, acc.2, 70),
        None,
        &Circle::new(eye, 0.34 * u),
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col((acc.0 + 180.0).min(255.0), (acc.1 + 180.0).min(255.0), (acc.2 + 180.0).min(255.0), 255),
        None,
        &Circle::new(eye, 0.16 * u),
    );

    // Patas del lado cercano (más claras, encima del cuerpo).
    paint_leg(scene, Point::new(bx + 1.4 * u, by + 0.6 * u), u, s1 as f64, lift1 as f64, fur); // delantera cercana
    paint_leg(scene, Point::new(bx - 1.4 * u, by + 0.6 * u), u, s2 as f64, lift2 as f64, fur); // trasera cercana
}
