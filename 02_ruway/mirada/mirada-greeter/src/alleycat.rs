//! «alleycat» — recrea la **pantalla del callejón** de *Alley Cat* (Bill
//! Williams, Synapse 1983 / IBM 1984): la fachada de un edificio de
//! departamentos con una **grilla de ventanas** (alguna se abre y tira basura al
//! gato), **tendederos** que la cruzan con **ratones** corriendo (los bonus del
//! juego), una **cerca de madera** con graffiti (donde el original pinta score y
//! vidas), **tachos de basura** apoyados que el gato salta para trepar, y un
//! **perro** que cruza el borde inferior cada tanto (la amenaza del callejón). El
//! gato **prowlea** por la cresta de la cerca.
//!
//! No es el cielo/luna/skyline genérico que había antes: es la escena real del
//! juego, con la **paleta del tema del greeter** (no CGA crudo).
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
//! El telón (ratones, ropa, perro) se anima por `t` (determinista), así que el
//! fallback y el rig comparten la misma escena viva.

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

/// Fracción del telón donde está la **cresta de la cerca** (el «piso» del gato).
/// Arriba: fachada con ventanas y tendederos. Abajo: los tablones de la cerca.
const WALL_FRAC: f32 = 0.72;

/// Pinta **sólo el telón** del callejón sobre `rect`: fachada + grilla de
/// ventanas + tendederos con ropa y ratones + cerca de madera con graffiti +
/// tachos + perro que cruza. `t` en segundos; `bright` el acento del tema.
/// `WALL_FRAC` marca la cresta de la cerca (el «piso» del gato). Lo comparten el
/// fallback stateless [`paint`] y el rig [`paint_rig`].
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

    // ── Fachada del edificio: pared cálida y plana sobre la cresta de la cerca. ──
    let facade = lerp_rgb((60.0, 46.0, 52.0), acc, 0.06);
    let facade_d = lerp_rgb((44.0, 33.0, 40.0), acc, 0.04);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(facade.0, facade.1, facade.2, 255),
        None,
        &Rect::new(x0 as f64, y0 as f64, (x0 + w) as f64, wall_top as f64),
    );
    // Hiladas de ladrillo apenas marcadas (textura sutil de la pared).
    let course = (h * 0.03).clamp(12.0, 30.0);
    let mut cyy = y0 + course;
    while cyy < wall_top {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col(facade_d.0, facade_d.1, facade_d.2, 90),
            None,
            &Rect::new(x0 as f64, cyy as f64, (x0 + w) as f64, (cyy + 1.5) as f64),
        );
        cyy += course;
    }

    // ── Grilla de ventanas: encendidas (cálidas), apagadas, y alguna «abierta»
    //    (hueco oscuro) que cada tanto tira un objeto al gato — guiño al juego. ──
    let cellw = (w * 0.085).clamp(46.0, 130.0);
    let cellh = cellw * 1.2;
    let ncols = ((w / cellw).floor() as i32).max(1);
    let nrows = (((wall_top - y0) / cellh).floor() as i32).max(1);
    let gx = x0 + (w - ncols as f32 * cellw) * 0.5;
    let win_w = cellw * 0.55;
    let win_h = cellh * 0.62;
    for r in 0..nrows {
        for c in 0..ncols {
            let wx = gx + c as f32 * cellw + (cellw - win_w) * 0.5;
            let wy = y0 + r as f32 * cellh + (cellh - win_h) * 0.5;
            let seed = hash(c as u64 ^ (r as u64) << 16 ^ 0x515150);
            // Marco oscuro (algo mayor que el vidrio).
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col(24.0, 20.0, 26.0, 255),
                None,
                &Rect::new((wx - 3.0) as f64, (wy - 3.0) as f64, (wx + win_w + 3.0) as f64, (wy + win_h + 3.0) as f64),
            );
            // ¿Abierta? Oscila lento; sólo algunas ventanas se abren.
            let open = seed % 5 == 0 && (t * 0.22 + hf(seed ^ 0x9) * 6.2832).sin() > 0.86;
            let lit = !open && seed % 3 != 0;
            let glass = if open {
                col(8.0, 8.0, 12.0, 255)
            } else if lit {
                let fl = 0.7 + 0.3 * ((t * 0.5 + hf(seed) * 6.2832).sin() * 0.5 + 0.5);
                let (lr, lg, lb) = lerp_rgb((255.0, 214.0, 140.0), acc, 0.30);
                col(lr, lg, lb, (fl * 255.0) as u8)
            } else {
                col(34.0, 30.0, 40.0, 255)
            };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                glass,
                None,
                &Rect::new(wx as f64, wy as f64, (wx + win_w) as f64, (wy + win_h) as f64),
            );
            // Parteluces (cruceta del marco).
            let mull = col(24.0, 20.0, 26.0, 235);
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, mull, None, &line(wx + win_w * 0.5, wy, wx + win_w * 0.5, wy + win_h));
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, mull, None, &line(wx, wy + win_h * 0.5, wx + win_w, wy + win_h * 0.5));
            // Basura que cae de una ventana abierta.
            if open {
                let fallp = (t * 0.6 + hf(seed) * 9.0).fract();
                let ox = wx + win_w * 0.5;
                let oy = wy + win_h + fallp * (wall_top - (wy + win_h));
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    col(182.0, 172.0, 152.0, 230),
                    None,
                    &Rect::new((ox - 4.0) as f64, (oy - 4.0) as f64, (ox + 4.0) as f64, (oy + 4.0) as f64),
                );
            }
        }
    }

    // ── Tendederos: dos líneas con catenaria, ropa que oscila, y un ratón
    //    corriendo por cada una (los bonus del juego). ──
    for li in 0..2u64 {
        let ly = y0 + (wall_top - y0) * (0.34 + 0.30 * li as f32);
        let sag = (wall_top - y0) * 0.025;
        let mut rope = BezPath::new();
        rope.move_to(Point::new(x0 as f64, ly as f64));
        rope.quad_to(
            Point::new((x0 + w * 0.5) as f64, (ly + sag) as f64),
            Point::new((x0 + w) as f64, ly as f64),
        );
        scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, col(184.0, 180.0, 172.0, 220), None, &rope);
        // Catenaria aproximada: dip(fx) = sag · (1 − (2fx−1)²).
        let dip = |fx: f32| sag * (1.0 - (2.0 * fx - 1.0) * (2.0 * fx - 1.0));
        let n_cloth = ((w / 120.0) as i32).max(3);
        for k in 0..n_cloth {
            let fx = (k as f32 + 0.5) / n_cloth as f32;
            let lx = x0 + fx * w;
            let lyk = ly + dip(fx);
            let cw = 16.0 + hf(li ^ k as u64 ^ 0x77) * 16.0;
            let chh = 24.0 + hf(li ^ k as u64 ^ 0x88) * 28.0;
            let hue = [(176.0, 96.0, 96.0), (96.0, 124.0, 170.0), (206.0, 186.0, 96.0), (120.0, 162.0, 124.0)]
                [(hash(li ^ k as u64 ^ 0x3) % 4) as usize];
            let (cr, cg, cb) = lerp_rgb(hue, acc, 0.12);
            let sway = (t * 1.4 + fx * 8.0).sin() * 2.2;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col(cr, cg, cb, 235),
                None,
                &Rect::new((lx - cw * 0.5 + sway) as f64, lyk as f64, (lx + cw * 0.5 + sway) as f64, (lyk + chh) as f64),
            );
        }
        // Ratón: corre de punta a punta (sentido alterna por línea).
        let speed = 0.16 + 0.05 * li as f32;
        let raw = (t * speed + li as f32 * 0.5).fract();
        let mp = if li % 2 == 0 { raw } else { 1.0 - raw };
        let mx = x0 + mp * w;
        let my = ly + dip(mp) - 4.5;
        paint_mouse(scene, mx, my, if li % 2 == 0 { 1.0 } else { -1.0 }, t);
    }

    // ── Cerca de madera. Su cresta (`wall_top`) es el «piso» del gato. ──
    let fence_bot = y0 + h;
    let wood = lerp_rgb((104.0, 70.0, 42.0), acc, 0.04);
    let wood_d = (80.0, 52.0, 30.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(wood.0, wood.1, wood.2, 255),
        None,
        &Rect::new(x0 as f64, wall_top as f64, (x0 + w) as f64, fence_bot as f64),
    );
    // Riel superior más claro: la cresta donde se planta el gato.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(142.0, 100.0, 62.0, 255),
        None,
        &Rect::new(x0 as f64, wall_top as f64, (x0 + w) as f64, (wall_top + 4.0) as f64),
    );
    // Tablones verticales (veta alterna + junta oscura).
    let plankw = (w * 0.026).clamp(16.0, 44.0);
    let mut px = x0;
    let mut pi = 0u64;
    while px < x0 + w {
        if pi % 2 == 1 {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col(wood_d.0, wood_d.1, wood_d.2, 255),
                None,
                &Rect::new(px as f64, (wall_top + 4.0) as f64, (px + plankw) as f64, fence_bot as f64),
            );
        }
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, col(40.0, 26.0, 16.0, 200), None, &line(px, wall_top + 4.0, px, fence_bot));
        px += plankw;
        pi += 1;
    }
    // Graffiti acento sobre los tablones (en el original van score y vidas).
    for gi in 0..4u64 {
        let bx = x0 + (0.15 + 0.7 * hf(gi ^ 0x6171)) * w;
        let by = wall_top + (0.2 + 0.55 * hf(gi ^ 0x7282)) * (fence_bot - wall_top);
        let s = (fence_bot - wall_top) * 0.18;
        let (gr, gg, gb) = lerp_rgb((acc.0, acc.1, acc.2), (240.0, 240.0, 240.0), 0.2);
        let mut sq = BezPath::new();
        sq.move_to(Point::new(bx as f64, by as f64));
        sq.curve_to(
            Point::new((bx + s) as f64, (by - s) as f64),
            Point::new((bx + s * 1.6) as f64, (by + s) as f64),
            Point::new((bx + s * 2.4) as f64, (by - s * 0.4) as f64),
        );
        scene.stroke(&Stroke::new(2.4), Affine::IDENTITY, col(gr, gg, gb, 150), None, &sq);
    }

    // ── Tachos de basura apoyados en la cerca (el gato los salta para trepar). ──
    paint_trashcan(scene, x0 + w * 0.07, fence_bot, h * 0.16, acc);
    paint_trashcan(scene, x0 + w * 0.135, fence_bot, h * 0.13, acc);

    // ── Perro que cruza el borde inferior cada tanto (la amenaza del callejón). ──
    let period = 17.0_f32;
    let dp = (t % period) / period;
    if dp < 0.26 {
        let run = dp / 0.26;
        let dir = if (t / period) as i64 % 2 == 0 { 1.0 } else { -1.0 };
        let dx = if dir > 0.0 {
            x0 - w * 0.12 + run * (w * 1.24)
        } else {
            x0 + w * 1.12 - run * (w * 1.24)
        };
        paint_dog(scene, dx, fence_bot - h * 0.02, h * 0.12, dir, t);
    }
}

/// Un ratón corriendo por un tendedero. `dir` ±1 marca el sentido de avance.
fn paint_mouse(scene: &mut vello::Scene, x: f32, y: f32, dir: f32, t: f32) {
    let (x, y, d) = (x as f64, y as f64, dir as f64);
    let body = col(150.0, 148.0, 156.0, 255);
    let foot = col(120.0, 118.0, 126.0, 255);
    // Patitas (un parpadeo de carrera).
    let wig = ((t * 16.0).sin() * 1.6) as f64;
    scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, foot, None, &line((x - 3.0) as f32, (y + 4.0) as f32, (x - 3.0 + wig) as f32, (y + 7.0) as f32));
    scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, foot, None, &line((x + 3.0) as f32, (y + 4.0) as f32, (x + 3.0 - wig) as f32, (y + 7.0) as f32));
    // Cuerpo.
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Ellipse::new(Point::new(x, y), (7.0, 4.2), 0.0));
    // Cabeza + oreja + ojo, mirando hacia `dir`.
    let hx = x + d * 7.0;
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Circle::new(Point::new(hx, y - 0.5), 3.4));
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(170.0, 150.0, 158.0, 255), None, &Circle::new(Point::new(hx + d, y - 3.6), 2.0));
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(20.0, 18.0, 22.0, 255), None, &Circle::new(Point::new(hx + d * 1.6, y - 1.0), 0.8));
    // Cola hacia atrás.
    let mut tail = BezPath::new();
    tail.move_to(Point::new(x - d * 6.5, y));
    tail.quad_to(Point::new(x - d * 13.0, y - 2.0), Point::new(x - d * 15.0, y + 3.0));
    scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, col(140.0, 138.0, 146.0, 255), None, &tail);
}

/// Un tacho de basura metálico: base en `bottom`, alto `ht`, centrado en `cx`.
fn paint_trashcan(scene: &mut vello::Scene, cx: f32, bottom: f32, ht: f32, acc: (f32, f32, f32)) {
    let bw = ht * 0.72;
    let top = bottom - ht;
    let (l, r) = (cx - bw * 0.5, cx + bw * 0.5);
    let metal = lerp_rgb((104.0, 106.0, 114.0), acc, 0.04);
    let metal_d = (70.0, 72.0, 80.0);
    // Cuerpo.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(metal.0, metal.1, metal.2, 255),
        None,
        &Rect::new(l as f64, (top + ht * 0.12) as f64, r as f64, bottom as f64),
    );
    // Acanaladuras verticales.
    for j in 1..4 {
        let rx = l + bw * (j as f32 / 4.0);
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, col(metal_d.0, metal_d.1, metal_d.2, 200), None, &line(rx, top + ht * 0.16, rx, bottom - 2.0));
    }
    // Tapa (algo más ancha) + perilla.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(metal_d.0, metal_d.1, metal_d.2, 255),
        None,
        &Ellipse::new(Point::new(cx as f64, (top + ht * 0.10) as f64), ((bw * 0.58) as f64, (ht * 0.10) as f64), 0.0),
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(metal.0, metal.1, metal.2, 255),
        None,
        &Ellipse::new(Point::new(cx as f64, (top + ht * 0.02) as f64), ((bw * 0.12) as f64, (ht * 0.05) as f64), 0.0),
    );
}

/// Silueta de un perro corriendo por el piso. `dir` ±1 = sentido, `foot` la línea
/// del piso, `sz` la alzada.
fn paint_dog(scene: &mut vello::Scene, x: f32, foot: f32, sz: f32, dir: f32, t: f32) {
    let (x, foot, s, d) = (x as f64, foot as f64, sz as f64, dir as f64);
    let body = col(36.0, 34.0, 42.0, 255);
    let cy = foot - s * 0.55;
    // Patas en carrera (fases opuestas).
    let g = (t as f64) * 14.0;
    for (px, ph) in [(-0.7, 0.0), (-0.5, std::f64::consts::PI), (0.7, std::f64::consts::PI), (0.5, 0.0)] {
        let sw = (g + ph).sin() * s * 0.18;
        let hipx = x + d * px * s;
        scene.stroke(&Stroke::new(s * 0.12), Affine::IDENTITY, body, None, &line(hipx as f32, (cy + s * 0.2) as f32, (hipx + sw) as f32, foot as f32));
    }
    // Cuerpo + grupa.
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Ellipse::new(Point::new(x, cy), (s * 0.95, s * 0.5), 0.0));
    // Cuello + cabeza hacia `dir`.
    let hx = x + d * s * 1.05;
    let hy = cy - s * 0.25;
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Circle::new(Point::new(hx, hy), s * 0.34));
    // Hocico.
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Ellipse::new(Point::new(hx + d * s * 0.28, hy + s * 0.05), (s * 0.22, s * 0.15), 0.0));
    // Oreja caída.
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(26.0, 24.0, 32.0, 255), None, &Ellipse::new(Point::new(hx - d * s * 0.12, hy - s * 0.1), (s * 0.12, s * 0.2), 0.0));
    // Cola alzada.
    let mut tail = BezPath::new();
    tail.move_to(Point::new(x - d * s * 0.9, cy - s * 0.1));
    tail.quad_to(Point::new(x - d * s * 1.3, cy - s * 0.6), Point::new(x - d * s * 1.1, cy - s * 0.9));
    scene.stroke(&Stroke::new(s * 0.13), Affine::IDENTITY, body, None, &tail);
    // Ojo.
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(220.0, 210.0, 180.0, 230), None, &Circle::new(Point::new(hx + d * s * 0.12, hy - s * 0.05), s * 0.045));
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
