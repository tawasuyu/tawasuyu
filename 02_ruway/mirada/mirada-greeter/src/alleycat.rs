//! «alleycat» — recrea la **escena del callejón** de *Alley Cat* (Bill Williams,
//! Synapse 1983 / IBM 1984), pero no como un loop decorativo sino como una
//! **escena procedural con guion**: el gato vive una rutina probabilística sobre
//! un callejón de tres niveles.
//!
//! Geometría (en el espacio virtual `VW×VH`, escalado al `rect` real al pintar):
//!
//! - **Fachada** arriba (`y0 .. WALL_V`): ladrillo, **grilla de ventanas** que se
//!   abren y **tiran zapatos**, y **tendederos** con ropa y ratones.
//! - **Cerca/muro** (`WALL_V .. GROUND_V`): tablones de madera. Su **cresta**
//!   (`WALL_V`) es la pasarela alta del gato. Lo que cae «por detrás del muro» se
//!   pinta **antes** que la cerca, así los tablones lo ocultan.
//! - **Piso del callejón** (`GROUND_V .. VH`): adoquines. Acá pasea el gato, se
//!   apoyan los **barriles** (2 a 4, chicos y grandes) y cruza el **perro**.
//!
//! El gato **«alley»** corre la rutina con una máquina de estados de transiciones
//! probabilísticas ([`Mode`]) a **ritmo dinámico** (un `tempo` que sube y baja):
//!
//! - Corre el piso **en trechos** (no en zigzag): elige un destino, llega, y ahí
//!   **decide** — se detiene a mirar, cambia de dirección y corre otro trecho,
//!   **salta a un barril**, o **salta al muro**.
//! - Sobre los **barriles** se posa. De los barriles, de vez en cuando, **se asoma
//!   otro gato** levantando la tapa y lo **tumba al piso**.
//! - Sobre el **muro** corre, se baja, o **salta al tendedero** — pero **falla** y
//!   **cae por detrás del muro**; un rato después **reaparece**.
//! - Cada tanto sale un **perro**: si alley está **en el piso** lo atrapa y se
//!   funden en una **bola de humo** que se va; luego alley reaparece.
//! - Las **ventanas tiran zapatos**: si alley está **sobre el muro** lo golpean y
//!   cae por detrás.
//!
//! El movimiento «vivo» se mantiene del rig anterior: las patas se resuelven por
//! **IK de dos huesos** (se plantan en la cresta del paso) y la **cola es una
//! cadena Verlet**. La rutina sólo conduce el cuerpo; el rig la encarna.
//!
//! Tres puntos de entrada, una sola escena:
//!
//! - [`AlleyCatBg`] — **stateful**: la simulación completa (gato + props + eventos).
//!   Se stepea en `RainTick` y entrega un [`CatSnapshot`] (`Send + 'static`) al
//!   closure de pintado.
//! - [`paint_rig`] — pinta un `CatSnapshot` en capas con la oclusión correcta.
//! - [`paint`] — **stateless** (firma de [`crate::rain::paint`]): telón quieto + un
//!   gato parado. Es el fallback barato del despachador [`crate::bg`].

use llimphi_anim::constraint::solve_two_bone_ik;
use llimphi_anim::physics::Physics;
use llimphi_anim::skel::{BoneId, Pose, Skeleton};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Ellipse, Point, Rect, Stroke, Vec2};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

use std::f64::consts::{PI, TAU};

// ─────────────────────────── utilidades de color ────────────────────────────

/// splitmix64 — para posiciones/alturas deterministas del telón (ventanas, ropa).
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
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

fn col(r: f32, g: f32, b: f32, a: u8) -> Color {
    Color::from_rgba8(
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
        a,
    )
}

/// Una línea como `BezPath`.
fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(Point::new(x1 as f64, y1 as f64));
    p.line_to(Point::new(x2 as f64, y2 as f64));
    p
}

// ─────────────────────────── geometría del callejón ─────────────────────────

/// Espacio virtual 16:9 de la simulación; se escala al `rect` real al pintar.
const VW: f64 = 1600.0;
const VH: f64 = 900.0;
/// Unidad de escala del gato en el espacio virtual.
const U: f64 = VH * 0.045;

/// Fracción del alto donde está la **cresta del muro** (la pasarela alta del gato).
const WALL_FRAC: f32 = 0.60;
/// Fracción del alto donde está el **piso del callejón** (gato/barriles/perro).
const GROUND_FRAC: f32 = 0.86;
/// Cresta del muro en virtual.
const WALL_V: f64 = VH * WALL_FRAC as f64;
/// Piso del callejón en virtual.
const GROUND_V: f64 = VH * GROUND_FRAC as f64;
/// Altura a la que el gato **alcanza y falla** el tendedero (dentro de la fachada).
const CLOTHES_V: f64 = VH * 0.40;

/// Cuánto está el centro del cuerpo por encima de las pezuñas.
const BODY_ABOVE: f64 = 1.9 * U;
const HIP_DY: f64 = 0.55 * U;
/// Largo de cada segmento de pata (fémur / tibia).
const SEG: f64 = 0.85 * U;
/// Parámetros de la marcha.
const STRIDE: f64 = 1.15 * U;
const LIFT: f64 = 0.6 * U;
const DUTY: f64 = 0.62; // fracción del ciclo en apoyo

fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// Altura de un barril según tamaño.
fn barrel_h(big: bool) -> f64 {
    if big {
        2.4 * U
    } else {
        1.5 * U
    }
}

// ═══════════════════════════════ telón (capas) ══════════════════════════════

/// Pinta la **fachada** (todo lo que vive por encima de la cresta del muro):
/// pared de ladrillo, grilla de ventanas con parpadeo, y tendederos con ropa y
/// ratones. Los **zapatos** que tira no van acá: son props con estado.
fn paint_facade(scene: &mut vello::Scene, rect: PaintRect, t: f32, acc: (f32, f32, f32)) {
    let (x0, y0, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let wall_top = y0 + h * WALL_FRAC;
    let ground = y0 + h * GROUND_FRAC;

    // Pared cálida que llena hasta el piso (la cerca la tapa por delante).
    let facade = lerp_rgb((60.0, 46.0, 52.0), acc, 0.06);
    let facade_d = lerp_rgb((44.0, 33.0, 40.0), acc, 0.04);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(facade.0, facade.1, facade.2, 255),
        None,
        &Rect::new(x0 as f64, y0 as f64, (x0 + w) as f64, ground as f64),
    );
    // Hiladas de ladrillo apenas marcadas.
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

    // ── Grilla de ventanas: encendidas (cálidas), apagadas, alguna «abierta»
    //    (hueco oscuro). Las que tiran zapatos son props con estado, no esto. ──
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
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                col(24.0, 20.0, 26.0, 255),
                None,
                &Rect::new((wx - 3.0) as f64, (wy - 3.0) as f64, (wx + win_w + 3.0) as f64, (wy + win_h + 3.0) as f64),
            );
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
            let mull = col(24.0, 20.0, 26.0, 235);
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, mull, None, &line(wx + win_w * 0.5, wy, wx + win_w * 0.5, wy + win_h));
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, mull, None, &line(wx, wy + win_h * 0.5, wx + win_w, wy + win_h * 0.5));
        }
    }

    // ── Tendederos: dos líneas con catenaria, ropa que oscila, un ratón por línea. ──
    for li in 0..2u64 {
        let ly = y0 + (wall_top - y0) * (0.34 + 0.30 * li as f32);
        let sag = (wall_top - y0) * 0.025;
        let mut rope = BezPath::new();
        rope.move_to(Point::new(x0 as f64, ly as f64));
        rope.quad_to(Point::new((x0 + w * 0.5) as f64, (ly + sag) as f64), Point::new((x0 + w) as f64, ly as f64));
        scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, col(184.0, 180.0, 172.0, 220), None, &rope);
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
        let speed = 0.16 + 0.05 * li as f32;
        let raw = (t * speed + li as f32 * 0.5).fract();
        let mp = if li % 2 == 0 { raw } else { 1.0 - raw };
        let mx = x0 + mp * w;
        let my = ly + dip(mp) - 4.5;
        paint_mouse(scene, mx, my, if li % 2 == 0 { 1.0 } else { -1.0 }, t);
    }
}

/// Pinta la **cerca/muro**: tablones desde la cresta hasta el piso, riel superior
/// (la pasarela del gato) y graffiti. Lo que «cae por detrás» se pinta antes.
fn paint_fence(scene: &mut vello::Scene, rect: PaintRect, acc: (f32, f32, f32)) {
    let (x0, y0, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let wall_top = y0 + h * WALL_FRAC;
    let ground = y0 + h * GROUND_FRAC;
    let wood = lerp_rgb((104.0, 70.0, 42.0), acc, 0.04);
    let wood_d = (80.0, 52.0, 30.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(wood.0, wood.1, wood.2, 255),
        None,
        &Rect::new(x0 as f64, wall_top as f64, (x0 + w) as f64, ground as f64),
    );
    // Riel superior más claro: la cresta donde se planta el gato.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(142.0, 100.0, 62.0, 255),
        None,
        &Rect::new(x0 as f64, wall_top as f64, (x0 + w) as f64, (wall_top + 4.0) as f64),
    );
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
                &Rect::new(px as f64, (wall_top + 4.0) as f64, (px + plankw) as f64, ground as f64),
            );
        }
        scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, col(40.0, 26.0, 16.0, 200), None, &line(px, wall_top + 4.0, px, ground));
        px += plankw;
        pi += 1;
    }
    for gi in 0..4u64 {
        let bx = x0 + (0.15 + 0.7 * hf(gi ^ 0x6171)) * w;
        let by = wall_top + (0.2 + 0.55 * hf(gi ^ 0x7282)) * (ground - wall_top);
        let s = (ground - wall_top) * 0.18;
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
}

/// Pinta el **piso del callejón** (adoquines) desde la base de la cerca hacia abajo.
fn paint_ground(scene: &mut vello::Scene, rect: PaintRect, acc: (f32, f32, f32)) {
    let (x0, y0, w, h) = (rect.x, rect.y, rect.w, rect.h);
    let ground = y0 + h * GROUND_FRAC;
    let bottom = y0 + h;
    let stone = lerp_rgb((48.0, 44.0, 50.0), acc, 0.03);
    let stone_d = lerp_rgb((34.0, 30.0, 36.0), acc, 0.02);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(stone.0, stone.1, stone.2, 255),
        None,
        &Rect::new(x0 as f64, ground as f64, (x0 + w) as f64, bottom as f64),
    );
    // Sombra al pie de la cerca.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        col(0.0, 0.0, 0.0, 60),
        None,
        &Rect::new(x0 as f64, ground as f64, (x0 + w) as f64, (ground + h * 0.012) as f64),
    );
    // Juntas de adoquín, en perspectiva apenas insinuada.
    let rows = 3;
    for r in 0..rows {
        let fy = (r as f32 + 1.0) / (rows as f32 + 1.0);
        let yy = ground + fy * (bottom - ground);
        scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, col(stone_d.0, stone_d.1, stone_d.2, 150), None, &line(x0, yy, x0 + w, yy));
        let cols = 8 + r * 2;
        let off = (r % 2) as f32 * 0.5;
        for cidx in 0..cols {
            let cx = x0 + ((cidx as f32 + off) / cols as f32) * w;
            scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, col(stone_d.0, stone_d.1, stone_d.2, 110), None, &line(cx, yy, cx, (yy + (bottom - ground) / (rows as f32 + 1.0)).min(bottom)));
        }
    }
}

/// Un ratón corriendo por un tendedero. `dir` ±1 marca el sentido.
fn paint_mouse(scene: &mut vello::Scene, x: f32, y: f32, dir: f32, t: f32) {
    let (x, y, d) = (x as f64, y as f64, dir as f64);
    let body = col(150.0, 148.0, 156.0, 255);
    let foot = col(120.0, 118.0, 126.0, 255);
    let wig = ((t * 16.0).sin() * 1.6) as f64;
    scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, foot, None, &line((x - 3.0) as f32, (y + 4.0) as f32, (x - 3.0 + wig) as f32, (y + 7.0) as f32));
    scene.stroke(&Stroke::new(1.4), Affine::IDENTITY, foot, None, &line((x + 3.0) as f32, (y + 4.0) as f32, (x + 3.0 - wig) as f32, (y + 7.0) as f32));
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Ellipse::new(Point::new(x, y), (7.0, 4.2), 0.0));
    let hx = x + d * 7.0;
    scene.fill(Fill::NonZero, Affine::IDENTITY, body, None, &Circle::new(Point::new(hx, y - 0.5), 3.4));
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(170.0, 150.0, 158.0, 255), None, &Circle::new(Point::new(hx + d, y - 3.6), 2.0));
    scene.fill(Fill::NonZero, Affine::IDENTITY, col(20.0, 18.0, 22.0, 255), None, &Circle::new(Point::new(hx + d * 1.6, y - 1.0), 0.8));
    let mut tail = BezPath::new();
    tail.move_to(Point::new(x - d * 6.5, y));
    tail.quad_to(Point::new(x - d * 13.0, y - 2.0), Point::new(x - d * 15.0, y + 3.0));
    scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, col(140.0, 138.0, 146.0, 255), None, &tail);
}

// ═══════════════════════════════ props (virtual) ════════════════════════════
// Dibujantes de los elementos con estado. Trabajan en coordenadas virtuales y
// reciben el `xf` que mapea virtual → pantalla (incluye la escala del rect).

/// Un barril de madera apoyado en el piso. `lid` 0..1 = tapa levantada; `peek`
/// 0..1 = gato asomándose. Tamaño chico/grande por `big`.
fn draw_barrel(scene: &mut vello::Scene, xf: Affine, b: &BarrelSnap, acc: (f32, f32, f32)) {
    let h = barrel_h(b.big);
    let bw = h * 0.78;
    let cx = b.x;
    let bottom = GROUND_V;
    let top = bottom - h;
    let (l, r) = (cx - bw * 0.5, cx + bw * 0.5);
    let wood = lerp_rgb((120.0, 80.0, 46.0), acc, 0.04);
    let wood_d = lerp_rgb((86.0, 56.0, 32.0), acc, 0.03);
    let hoop = lerp_rgb((150.0, 152.0, 158.0), acc, 0.04);

    // Cuerpo abombado (duelas).
    scene.fill(
        Fill::NonZero,
        xf,
        col(wood.0, wood.1, wood.2, 255),
        None,
        &Ellipse::new(Point::new(cx, (top + bottom) * 0.5), (bw * 0.5, h * 0.5), 0.0),
    );
    scene.fill(
        Fill::NonZero,
        xf,
        col(wood.0, wood.1, wood.2, 255),
        None,
        &Rect::new(l + bw * 0.06, top + h * 0.12, r - bw * 0.06, bottom),
    );
    // Duelas verticales.
    for j in 1..5 {
        let rx = l + bw * (j as f64 / 5.0);
        scene.stroke(&Stroke::new(1.6), xf, col(wood_d.0, wood_d.1, wood_d.2, 200), None, &line(rx as f32, (top + h * 0.14) as f32, rx as f32, (bottom - 2.0) as f32));
    }
    // Aros metálicos.
    for fy in [0.22, 0.78] {
        let yy = top + h * fy;
        scene.stroke(&Stroke::new(3.0), xf, col(hoop.0, hoop.1, hoop.2, 230), None, &line(l as f32, yy as f32, r as f32, yy as f32));
    }

    // Hueco oscuro de la boca (visible cuando la tapa sube).
    if b.lid > 0.02 || b.peek > 0.02 {
        scene.fill(
            Fill::NonZero,
            xf,
            col(10.0, 8.0, 12.0, 255),
            None,
            &Ellipse::new(Point::new(cx, top + h * 0.10), (bw * 0.42, h * 0.09), 0.0),
        );
    }

    // Gato que se asoma: dos orejas + cabeza + ojos brillando, subiendo con `peek`.
    if b.peek > 0.02 {
        let pe = b.peek.clamp(0.0, 1.0) as f64;
        let hy = top + h * 0.10 - pe * h * 0.42;
        let hr = bw * 0.30;
        let fur = col(40.0, 42.0, 52.0, 255);
        // Orejas.
        for ear_dx in [-0.55, 0.55] {
            let ex = cx + ear_dx * hr;
            let mut ear = BezPath::new();
            ear.move_to(Point::new(ex - 0.3 * hr, hy - hr * 0.5));
            ear.line_to(Point::new(ex, hy - hr * 1.4));
            ear.line_to(Point::new(ex + 0.4 * hr, hy - hr * 0.45));
            ear.close_path();
            scene.fill(Fill::NonZero, xf, fur, None, &ear);
        }
        scene.fill(Fill::NonZero, xf, fur, None, &Circle::new(Point::new(cx, hy), hr));
        let (er, eg, eb) = (acc.0, acc.1, acc.2);
        scene.fill(Fill::NonZero, xf, col(er, eg, eb, 230), None, &Circle::new(Point::new(cx - hr * 0.34, hy - hr * 0.05), hr * 0.16));
        scene.fill(Fill::NonZero, xf, col(er, eg, eb, 230), None, &Circle::new(Point::new(cx + hr * 0.34, hy - hr * 0.05), hr * 0.16));
    }

    // Tapa: un disco que se levanta con `lid`, ladeado.
    let lift = b.lid.clamp(0.0, 1.0) as f64 * h * 0.5;
    let tilt = b.lid.clamp(0.0, 1.0) as f64 * 0.5;
    let ly = top + h * 0.06 - lift;
    scene.fill(
        Fill::NonZero,
        xf,
        col(wood_d.0, wood_d.1, wood_d.2, 255),
        None,
        &Ellipse::new(Point::new(cx + tilt * bw * 0.3, ly), (bw * 0.46, h * 0.08), -tilt * 0.6),
    );
    scene.fill(
        Fill::NonZero,
        xf,
        col(wood.0, wood.1, wood.2, 255),
        None,
        &Ellipse::new(Point::new(cx + tilt * bw * 0.3, ly - 2.0), (bw * 0.12, h * 0.04), 0.0),
    );
}

/// Silueta de un perro corriendo por el piso. Coordenadas virtuales vía `xf`.
fn draw_dog(scene: &mut vello::Scene, xf: Affine, d: &DogSnap, _acc: (f32, f32, f32)) {
    let (x, foot, s, dir) = (d.x, GROUND_V - 0.04 * VH, d.sz, d.dir as f64);
    let body = col(36.0, 34.0, 42.0, 255);
    let cy = foot - s * 0.55;
    let g = d.phase;
    for (px, ph) in [(-0.7, 0.0), (-0.5, PI), (0.7, PI), (0.5, 0.0)] {
        let sw = (g + ph).sin() * s * 0.18;
        let hipx = x + dir * px * s;
        scene.stroke(&Stroke::new(s * 0.12), xf, body, None, &line(hipx as f32, (cy + s * 0.2) as f32, (hipx + sw) as f32, foot as f32));
    }
    scene.fill(Fill::NonZero, xf, body, None, &Ellipse::new(Point::new(x, cy), (s * 0.95, s * 0.5), 0.0));
    let hx = x + dir * s * 1.05;
    let hy = cy - s * 0.25;
    scene.fill(Fill::NonZero, xf, body, None, &Circle::new(Point::new(hx, hy), s * 0.34));
    scene.fill(Fill::NonZero, xf, body, None, &Ellipse::new(Point::new(hx + dir * s * 0.28, hy + s * 0.05), (s * 0.22, s * 0.15), 0.0));
    scene.fill(Fill::NonZero, xf, col(26.0, 24.0, 32.0, 255), None, &Ellipse::new(Point::new(hx - dir * s * 0.12, hy - s * 0.1), (s * 0.12, s * 0.2), 0.0));
    let mut tail = BezPath::new();
    tail.move_to(Point::new(x - dir * s * 0.9, cy - s * 0.1));
    tail.quad_to(Point::new(x - dir * s * 1.3, cy - s * 0.6), Point::new(x - dir * s * 1.1, cy - s * 0.9));
    scene.stroke(&Stroke::new(s * 0.13), xf, body, None, &tail);
    scene.fill(Fill::NonZero, xf, col(220.0, 210.0, 180.0, 230), None, &Circle::new(Point::new(hx + dir * s * 0.12, hy - s * 0.05), s * 0.045));
}

/// Un zapato cayendo (bota tosca con taco), girando por `spin`.
fn draw_shoe(scene: &mut vello::Scene, xf: Affine, sh: &ShoeSnap) {
    let local = Affine::translate((sh.p.x, sh.p.y)) * Affine::rotate(sh.spin as f64);
    let m = xf * local;
    let leather = col(70.0, 52.0, 40.0, 255);
    let sole = col(40.0, 30.0, 24.0, 255);
    let s = 0.5 * U;
    // Caña + empeine.
    scene.fill(Fill::NonZero, m, leather, None, &Rect::new(-s * 0.8, -s * 0.7, s * 0.4, s * 0.2));
    scene.fill(Fill::NonZero, m, leather, None, &Ellipse::new(Point::new(s * 0.2, s * 0.0), (s * 0.9, s * 0.5), 0.0));
    // Suela + taco.
    scene.fill(Fill::NonZero, m, sole, None, &Rect::new(-s * 0.9, s * 0.3, s * 1.1, s * 0.55));
    scene.fill(Fill::NonZero, m, sole, None, &Rect::new(-s * 0.9, s * 0.3, -s * 0.5, s * 0.9));
}

/// Una bola de humo: discos translúcidos que se expanden y suben mientras se
/// desvanecen. `age` 0..1.
fn draw_smoke(scene: &mut vello::Scene, xf: Affine, at: Point, age: f32) {
    let a = age.clamp(0.0, 1.0);
    let grow = 1.0 + a as f64 * 2.2;
    let rise = a as f64 * 1.6 * U;
    let alpha = ((1.0 - a) * 170.0) as u8;
    for (dx, dy, rr) in [(0.0, 0.0, 1.1), (-0.7, -0.3, 0.8), (0.7, -0.2, 0.85), (0.0, -0.9, 0.95), (-0.4, -1.4, 0.7), (0.5, -1.5, 0.65)] {
        let c = Point::new(at.x + dx * U * grow * 0.6, at.y - rise + dy * U * grow * 0.6);
        let g = 150.0 + 60.0 * rr as f32;
        scene.fill(Fill::NonZero, xf, col(g, g, g + 6.0, alpha), None, &Circle::new(c, rr * U * grow * 0.8));
    }
}

// ═══════════════════════════════ el gato (rig) ══════════════════════════════

/// Pinta el gato del snapshot, espejado según `facing`. Todo en virtual vía `xf`.
fn draw_cat(scene: &mut vello::Scene, xf: Affine, snap: &CatSnapshot) {
    let b = snap.body;
    // Espejo horizontal alrededor del cuerpo cuando mira a la izquierda. Es sólo
    // visual: el rig se resuelve siempre mirando a la derecha.
    let cat_xf = if snap.facing < 0.0 {
        xf * Affine::translate((b.x, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate((-b.x, 0.0))
    } else {
        xf
    };

    let fur = col(54.0, 56.0, 66.0, 255);
    let belly = col(78.0, 80.0, 92.0, 255);
    let fur_dark = col(38.0, 40.0, 50.0, 255);

    // Sombra (sólo apoyado en el piso/cresta, no en el aire).
    if snap.airborne < 0.4 && !snap.cat_behind_wall {
        scene.fill(
            Fill::NonZero,
            xf,
            col(0.0, 0.0, 0.0, 70),
            None,
            &Ellipse::new(Point::new(b.x, snap.support_y + 0.05 * U), (2.4 * U * (1.0 - snap.airborne as f64), 0.36 * U), 0.0),
        );
    }

    // Patas lejanas (detrás del cuerpo).
    for (hip, knee, foot, near) in &snap.legs {
        if !*near {
            draw_leg(scene, cat_xf, *hip, *knee, *foot, fur_dark);
        }
    }

    // Cola (cadena Verlet) por detrás.
    if snap.tail.len() >= 2 {
        let mut path = BezPath::new();
        path.move_to(snap.tail[0]);
        for p in &snap.tail[1..] {
            path.line_to(*p);
        }
        scene.stroke(&Stroke::new(0.42 * U), cat_xf, fur, None, &path);
        if let Some(tip) = snap.tail.last() {
            scene.fill(Fill::NonZero, cat_xf, belly, None, &Circle::new(*tip, 0.22 * U));
        }
    }

    // Cuerpo + grupa + vientre.
    scene.fill(Fill::NonZero, cat_xf, fur, None, &Ellipse::new(b, (2.0 * U, 0.9 * U), 0.0));
    scene.fill(Fill::NonZero, cat_xf, fur, None, &Circle::new(Point::new(b.x - 1.5 * U, b.y - 0.05 * U), 0.95 * U));
    scene.fill(Fill::NonZero, cat_xf, belly, None, &Ellipse::new(Point::new(b.x + 0.1 * U, b.y + 0.55 * U), (1.5 * U, 0.4 * U), 0.0));

    // Cabeza, orejas, hocico, ojo.
    let head = snap.head;
    scene.fill(Fill::NonZero, cat_xf, fur, None, &Circle::new(head, 0.78 * U));
    for ear_dx in [-0.45, 0.5] {
        let ex = head.x + ear_dx * U;
        let mut ear = BezPath::new();
        ear.move_to(Point::new(ex - 0.32 * U, head.y - 0.55 * U));
        ear.line_to(Point::new(ex + 0.05 * U, head.y - 1.35 * U));
        ear.line_to(Point::new(ex + 0.42 * U, head.y - 0.5 * U));
        ear.close_path();
        scene.fill(Fill::NonZero, cat_xf, fur, None, &ear);
    }
    scene.fill(Fill::NonZero, cat_xf, belly, None, &Circle::new(Point::new(head.x + 0.55 * U, head.y + 0.18 * U), 0.34 * U));
    let eye = Point::new(head.x + 0.30 * U, head.y - 0.08 * U);
    let acc = snap.accent;
    scene.fill(Fill::NonZero, cat_xf, col(acc.0, acc.1, acc.2, 70), None, &Circle::new(eye, 0.34 * U));
    scene.fill(
        Fill::NonZero,
        cat_xf,
        col((acc.0 + 180.0).min(255.0), (acc.1 + 180.0).min(255.0), (acc.2 + 180.0).min(255.0), 255),
        None,
        &Circle::new(eye, 0.16 * U),
    );

    // Patas cercanas (encima del cuerpo).
    for (hip, knee, foot, near) in &snap.legs {
        if *near {
            draw_leg(scene, cat_xf, *hip, *knee, *foot, fur);
        }
    }
}

/// Pinta una pata del rig: trazo cadera→rodilla→pezuña + pata redonda.
fn draw_leg(scene: &mut vello::Scene, xf: Affine, hip: Point, knee: Point, foot: Point, color: Color) {
    let mut p = BezPath::new();
    p.move_to(hip);
    p.line_to(knee);
    p.line_to(foot);
    scene.stroke(&Stroke::new(0.32 * U), xf, color, None, &p);
    scene.fill(Fill::NonZero, xf, color, None, &Circle::new(foot, 0.2 * U));
}

// ═══════════════════════════ máquina de estados ═════════════════════════════

/// En qué nivel del callejón está el gato (define su vulnerabilidad).
#[derive(Clone, Copy, PartialEq)]
enum Level {
    Floor,
    Wall,
    Barrel(usize),
}

/// Adónde aterriza un salto.
#[derive(Clone, Copy)]
enum Dest {
    Floor,
    Wall,
    Barrel(usize),
}

/// El estado actual de la rutina del gato. Todos los campos son `Copy`.
#[derive(Clone, Copy)]
enum Mode {
    /// Corre un trecho hasta `to` a `speed`.
    Walk { to: f64, speed: f64 },
    /// Quieto, mirando, `left` segundos.
    Pause { left: f64 },
    /// Posado sobre un barril, `left` segundos.
    Perch { left: f64 },
    /// Salto balístico (cuerpo por parábola).
    Jump { x0: f64, y0: f64, x1: f64, y1: f64, hump: f64, t: f64, dur: f64, dest: Dest, flail: bool },
    /// Tumbado de un barril al piso (arco corto).
    Knocked { x0: f64, y0: f64, x1: f64, t: f64, dur: f64 },
    /// Salta al tendedero y falla (sube, manotea).
    LeapMiss { t: f64, dur: f64 },
    /// Cae por detrás del muro (oculto por la cerca).
    Behind { y0: f64, t: f64, dur: f64 },
    /// Fuera de escena, `left` segundos hasta reaparecer.
    Gone { left: f64 },
    /// Atrapado por el perro: bola de humo, `left` segundos.
    Caught { left: f64 },
}

/// Un barril del callejón.
struct Barrel {
    x: f64,
    big: bool,
    lid: f64,     // 0 cerrado .. 1 tapa levantada (suavizado)
    peek: f64,    // 0 .. 1 gato asomado (suavizado)
    peeking: bool,
    peek_left: f64,
}

/// El perro que cruza el piso.
struct Dog {
    x: f64,
    dir: f64,
    sz: f64,
    phase: f64,
}

/// Un zapato en vuelo.
struct Shoe {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    spin: f64,
    py: f64, // y previo (para detectar el cruce de la cresta)
}

/// Fondo «Alley Cat» con estado: rutina probabilística completa.
pub struct AlleyCatBg {
    // Rig.
    skel: Skeleton,
    root: BoneId,
    legs: Vec<Leg>,
    tail: Physics,
    // Estado del gato.
    body_x: f64,
    body_cy: f64,
    support_y: f64,
    gait: f64,
    facing: f64,
    airborne: f64,
    flail: f64,
    level: Level,
    mode: Mode,
    // Props.
    barrels: Vec<Barrel>,
    dog: Option<Dog>,
    shoes: Vec<Shoe>,
    smoke: Option<(f64, f64, f64, f64)>, // x, y, t, dur
    // Temporizadores de eventos.
    dog_timer: f64,
    shoe_timer: f64,
    peek_timer: f64,
    // Globales.
    t: f64,
    rng: u64,
    accent: (f32, f32, f32),
}

/// Una pata del rig: dos huesos resueltos por IK.
struct Leg {
    upper: BoneId,
    lower: BoneId,
    hip_dx: f64,
    phase: f64,
    flip: bool,
    near: bool,
}

const TAIL_N: usize = 8;
const TAIL_SEG: f64 = 0.42 * U;

/// Posición del objetivo de la pezuña relativa a la cadera, fase `p` en `[0,1)`.
fn gait_foot(p: f64) -> (f64, f64) {
    if p < DUTY {
        let ps = p / DUTY;
        (STRIDE * (0.5 - ps), 0.0)
    } else {
        let pw = (p - DUTY) / (1.0 - DUTY);
        (STRIDE * (pw - 0.5), -LIFT * (PI * pw).sin())
    }
}

impl AlleyCatBg {
    /// Construye la escena. `bright` tiñe el acento (ojos, halo) y siembra el RNG.
    pub fn new(bright: (u8, u8, u8)) -> Self {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(None, Pose::identity());
        let defs = [
            (1.30, 0.5, true, false),
            (-1.45, 0.0, false, false),
            (1.40, 0.0, true, true),
            (-1.35, 0.5, false, true),
        ];
        let mut legs = Vec::with_capacity(4);
        for (hip_dx, phase, flip, near) in defs {
            let upper = skel.add_bone(
                Some(root),
                Pose::new(Vec2::new(hip_dx * U, HIP_DY), PI / 2.0, Vec2::new(1.0, 1.0)),
            );
            let lower = skel.add_bone(Some(upper), Pose::translate(Vec2::new(SEG, 0.0)));
            legs.push(Leg { upper, lower, hip_dx: hip_dx * U, phase, flip, near });
        }
        skel.bind();

        let mut tail = Physics::new();
        let base = Point::new(-2.0 * U, GROUND_V - BODY_ABOVE - 0.15 * U);
        let mut prev = tail.particle(base, true);
        for i in 1..TAIL_N {
            let p = tail.particle(Point::new(base.x - i as f64 * TAIL_SEG, base.y), false);
            tail.link_with(prev, p, TAIL_SEG, 1.0);
            prev = p;
        }

        // RNG sembrado por el tema: cada paleta varía la coreografía, pero un mismo
        // tema es reproducible (sin relojes/azar de sistema).
        let seed = 0xA11E_CA77
            ^ (bright.0 as u64) << 16
            ^ (bright.1 as u64) << 8
            ^ bright.2 as u64;

        let mut me = Self {
            skel,
            root,
            legs,
            tail,
            body_x: VW * 0.3,
            body_cy: GROUND_V - BODY_ABOVE,
            support_y: GROUND_V,
            gait: 0.0,
            facing: 1.0,
            airborne: 0.0,
            flail: 0.0,
            level: Level::Floor,
            mode: Mode::Pause { left: 0.6 },
            barrels: Vec::new(),
            dog: None,
            shoes: Vec::new(),
            smoke: None,
            dog_timer: 6.0,
            shoe_timer: 3.0,
            peek_timer: 3.5,
            t: 0.0,
            rng: seed,
            accent: (bright.0 as f32, bright.1 as f32, bright.2 as f32),
        };
        me.spawn_barrels();
        me.dog_timer = me.rand_range(8.0, 15.0);
        me.shoe_timer = me.rand_range(2.5, 6.0);
        me.peek_timer = me.rand_range(2.0, 4.5);
        me.pose(0.0);
        me
    }

    // ── RNG determinista (splitmix64 sobre un contador interno) ──
    fn rng_next(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn rand(&mut self) -> f64 {
        (self.rng_next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn rand_range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.rand()
    }
    fn chance(&mut self, p: f64) -> bool {
        self.rand() < p
    }

    fn spawn_barrels(&mut self) {
        let n = 2 + (self.rng_next() % 3) as usize; // 2..4
        self.barrels.clear();
        for i in 0..n {
            let f = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.5 };
            let x = VW * (0.20 + 0.60 * f) + self.rand_range(-0.04, 0.04) * VW;
            let big = self.chance(0.5);
            self.barrels.push(Barrel { x, big, lid: 0.0, peek: 0.0, peeking: false, peek_left: 0.0 });
        }
    }

    /// `tempo` dinámico en ~[0.45, 1.7]: lulls y ráfagas que modulan velocidad y
    /// frecuencia de eventos. El «ritmo dinámico» de la escena.
    fn tempo(&self) -> f64 {
        (1.0 + 0.45 * (self.t * 0.07).sin() + 0.22 * (self.t * 0.23 + 1.3).sin()).clamp(0.45, 1.7)
    }

    fn on_floor(&self) -> bool {
        matches!(self.level, Level::Floor)
    }
    fn on_wall(&self) -> bool {
        matches!(self.level, Level::Wall)
    }

    /// Avanza el reloj `dt`: eventos, props, rutina del gato y pose del rig.
    pub fn step(&mut self, dt: f64) {
        self.t = (self.t + dt) % 1_000_000.0;
        let tempo = self.tempo();
        self.update_events(dt, tempo);
        self.update_props(dt, tempo);
        self.update_cat(dt, tempo);
        self.pose(dt);
    }

    // ── Eventos del callejón ──
    fn update_events(&mut self, dt: f64, tempo: f64) {
        // Perro.
        self.dog_timer -= dt * tempo;
        if self.dog_timer <= 0.0 {
            if self.dog.is_none() {
                let dir = if self.chance(0.5) { 1.0 } else { -1.0 };
                let x = if dir > 0.0 { -2.5 * U } else { VW + 2.5 * U };
                let sz = self.rand_range(2.0, 2.5) * U;
                self.dog = Some(Dog { x, dir, sz, phase: 0.0 });
            }
            self.dog_timer = self.rand_range(8.0, 16.0);
        }

        // Zapato desde una ventana.
        self.shoe_timer -= dt * tempo;
        if self.shoe_timer <= 0.0 {
            // Si el gato está sobre el muro, a veces apuntan a él.
            let tx = if self.on_wall() && self.chance(0.6) {
                self.body_x + self.rand_range(-0.6, 0.6) * U
            } else {
                self.rand_range(2.0 * U, VW - 2.0 * U)
            };
            let y = self.rand_range(0.16, 0.46) * WALL_V;
            let vx = self.rand_range(-0.5, 0.5) * U;
            let vy = self.rand_range(2.0, 3.2) * U;
            let spin = self.rand_range(0.0, 6.0);
            self.shoes.push(Shoe { x: tx, y, vx, vy, spin, py: y });
            self.shoe_timer = self.rand_range(2.4, 5.5);
        }

        // Gato asomándose de un barril.
        self.peek_timer -= dt * tempo;
        if self.peek_timer <= 0.0 {
            if !self.barrels.is_empty() {
                let i = (self.rand() * self.barrels.len() as f64) as usize % self.barrels.len();
                if !self.barrels[i].peeking {
                    self.barrels[i].peeking = true;
                    self.barrels[i].peek_left = self.rand_range(1.2, 2.4);
                    // Si alley está justo sobre ese barril, lo tumba al piso.
                    if self.level == Level::Barrel(i) {
                        self.knock_off(i);
                    }
                }
            }
            self.peek_timer = self.rand_range(2.0, 4.5);
        }
    }

    // ── Props: integra perro, zapatos, tapas y humo; resuelve colisiones ──
    fn update_props(&mut self, dt: f64, tempo: f64) {
        // Perro.
        if let Some(d) = &mut self.dog {
            let sp = 3.6 * U * tempo;
            d.x += d.dir * sp * dt;
            d.phase += 14.0 * dt;
        }
        if let Some(d) = &self.dog {
            let off = d.x < -3.5 * U || d.x > VW + 3.5 * U;
            // ¿Atrapa? Sólo si alley está en el piso y no en el aire.
            let catch = self.on_floor()
                && self.airborne < 0.3
                && !matches!(self.mode, Mode::Caught { .. } | Mode::Gone { .. })
                && (d.x - self.body_x).abs() < 1.5 * U;
            if catch {
                let (bx, by) = (self.body_x, self.body_cy);
                self.dog = None;
                self.caught(bx, by);
            } else if off {
                self.dog = None;
            }
        }

        // Zapatos.
        let mut hit_wall = false;
        let mut i = 0;
        while i < self.shoes.len() {
            let mut remove = false;
            {
                let s = &mut self.shoes[i];
                s.py = s.y;
                s.vy += 7.0 * U * dt; // gravedad
                s.x += s.vx * dt;
                s.y += s.vy * dt;
                s.spin += 5.0 * dt;
            }
            let s = &self.shoes[i];
            // ¿Golpea al gato en el muro al cruzar la cresta?
            let crossed = s.py < WALL_V && s.y >= WALL_V - 0.2 * U;
            if crossed && self.on_wall() && (s.x - self.body_x).abs() < 1.3 * U {
                hit_wall = true;
                remove = true;
            } else if s.y > GROUND_V + 0.2 * U || s.x < -U || s.x > VW + U {
                remove = true;
            }
            if remove {
                self.shoes.remove(i);
            } else {
                i += 1;
            }
        }
        if hit_wall && !matches!(self.mode, Mode::Behind { .. } | Mode::Gone { .. }) {
            self.knock_behind();
        }

        // Tapas y asomadas de los barriles (suavizado exponencial).
        for b in &mut self.barrels {
            if b.peeking {
                b.peek_left -= dt;
                if b.peek_left <= 0.0 {
                    b.peeking = false;
                }
            }
            let tgt = if b.peeking { 1.0 } else { 0.0 };
            let k = (dt * 8.0).min(1.0);
            b.lid += (tgt - b.lid) * k;
            b.peek += (tgt - b.peek) * k;
        }

        // Humo.
        if let Some(sm) = &mut self.smoke {
            sm.2 += dt;
            if sm.2 >= sm.3 {
                self.smoke = None;
            }
        }
    }

    // ── Rutina del gato ──
    fn update_cat(&mut self, dt: f64, tempo: f64) {
        match self.mode {
            Mode::Walk { to, speed } => {
                self.airborne = 0.0;
                self.flail = 0.0;
                let dir = (to - self.body_x).signum();
                if dir != 0.0 {
                    self.facing = dir;
                }
                let adv = speed * dt;
                self.body_x += dir * adv;
                self.gait += (speed / STRIDE) * dt;
                self.body_cy = self.support_y - BODY_ABOVE + self.bob();
                if (self.body_x - to).abs() <= adv.max(2.0) {
                    self.body_x = to;
                    self.decide_grounded(tempo);
                }
            }
            Mode::Pause { left } => {
                self.airborne = 0.0;
                self.flail = 0.0;
                self.body_cy = self.support_y - BODY_ABOVE;
                let l = left - dt;
                if l <= 0.0 {
                    self.decide_grounded(tempo);
                } else {
                    self.mode = Mode::Pause { left: l };
                }
            }
            Mode::Perch { left } => {
                self.airborne = 0.0;
                self.flail = 0.0;
                self.body_cy = self.support_y - BODY_ABOVE;
                let l = left - dt;
                if l <= 0.0 {
                    self.decide_barrel(tempo);
                } else {
                    self.mode = Mode::Perch { left: l };
                }
            }
            Mode::Jump { x0, y0, x1, y1, hump, t, dur, dest, flail } => {
                let nt = t + dt;
                let p = (nt / dur).clamp(0.0, 1.0);
                self.body_x = x0 + (x1 - x0) * p;
                let base = y0 + (y1 - y0) * p;
                self.body_cy = base - hump * 4.0 * p * (1.0 - p);
                self.airborne = (PI * p).sin();
                self.flail = if flail { (p * 1.4).min(1.0) } else { 0.25 };
                if (x1 - x0).abs() > 1.0 {
                    self.facing = (x1 - x0).signum();
                }
                if nt >= dur {
                    self.land(dest, x1, y1);
                } else {
                    self.mode = Mode::Jump { x0, y0, x1, y1, hump, t: nt, dur, dest, flail };
                }
            }
            Mode::Knocked { x0, y0, x1, t, dur } => {
                let nt = t + dt;
                let p = (nt / dur).clamp(0.0, 1.0);
                self.body_x = x0 + (x1 - x0) * p;
                let y1 = GROUND_V - BODY_ABOVE;
                // Sale despedido hacia arriba y cae: pequeño rebote.
                self.body_cy = y0 + (y1 - y0) * p - 0.7 * U * (PI * p).sin();
                self.airborne = (PI * p).sin() * 0.8;
                self.flail = 0.7 * (1.0 - p);
                self.facing = (x1 - x0).signum().max(-1.0).min(1.0);
                if nt >= dur {
                    self.level = Level::Floor;
                    self.support_y = GROUND_V;
                    self.body_x = x1;
                    self.body_cy = y1;
                    self.mode = Mode::Pause { left: self.rand_range(0.5, 1.2) };
                } else {
                    self.mode = Mode::Knocked { x0, y0, x1, t: nt, dur };
                }
            }
            Mode::LeapMiss { t, dur } => {
                let nt = t + dt;
                let p = (nt / dur).clamp(0.0, 1.0);
                let start_y = WALL_V - BODY_ABOVE;
                // Sube con ease-out hacia el tendedero (lo roza y no se agarra).
                self.body_cy = start_y + (CLOTHES_V - start_y) * (1.0 - (1.0 - p) * (1.0 - p));
                self.body_x += self.facing * 0.6 * U * dt;
                self.airborne = 1.0;
                self.flail = (p * 1.5).min(1.0);
                if nt >= dur {
                    // Falla: empieza a caer por detrás del muro.
                    self.mode = Mode::Behind { y0: self.body_cy, t: 0.0, dur: 0.75 };
                } else {
                    self.mode = Mode::LeapMiss { t: nt, dur };
                }
            }
            Mode::Behind { y0, t, dur } => {
                let nt = t + dt;
                let p = (nt / dur).clamp(0.0, 1.0);
                // Cae con gravedad (ease-in) hasta perderse tras la cerca.
                let y1 = GROUND_V + 1.0 * U;
                self.body_cy = y0 + (y1 - y0) * p * p;
                self.airborne = 1.0;
                self.flail = (1.0 - p) * 0.8;
                if nt >= dur {
                    self.mode = Mode::Gone { left: self.rand_range(1.2, 2.6) };
                } else {
                    self.mode = Mode::Behind { y0, t: nt, dur };
                }
            }
            Mode::Gone { left } => {
                let l = left - dt;
                if l <= 0.0 {
                    self.reappear();
                } else {
                    self.mode = Mode::Gone { left: l };
                }
            }
            Mode::Caught { left } => {
                let l = left - dt;
                if l <= 0.0 {
                    self.mode = Mode::Gone { left: self.rand_range(0.8, 1.8) };
                } else {
                    self.mode = Mode::Caught { left: l };
                }
            }
        }
    }

    /// Dos rebotes verticales por zancada (el «bob» de la marcha).
    fn bob(&self) -> f64 {
        0.05 * U * (self.gait * TAU * 2.0).sin()
    }

    /// Decisión al terminar un trecho/pausa estando **apoyado** (piso o muro).
    fn decide_grounded(&mut self, tempo: f64) {
        match self.level {
            Level::Floor => {
                // ¿Hay un barril a tiro? Saltar a él con cierta probabilidad.
                let near = self.nearest_barrel(3.6 * U);
                if let Some(i) = near {
                    if self.chance(0.32) {
                        self.jump_to_barrel(i);
                        return;
                    }
                }
                let r = self.rand();
                if r < 0.16 {
                    self.jump_to_wall();
                } else if r < 0.52 {
                    self.mode = Mode::Pause { left: self.rand_range(0.6, 2.2) };
                } else {
                    self.start_walk_floor(tempo);
                }
            }
            Level::Wall => {
                let r = self.rand();
                if r < 0.20 {
                    self.start_leap_miss();
                } else if r < 0.46 {
                    self.jump_down_to_floor();
                } else if r < 0.70 {
                    self.mode = Mode::Pause { left: self.rand_range(0.5, 1.8) };
                } else {
                    self.start_walk_wall(tempo);
                }
            }
            Level::Barrel(_) => self.decide_barrel(tempo),
        }
    }

    /// Decisión al terminar de posarse en un barril.
    fn decide_barrel(&mut self, _tempo: f64) {
        let cur = if let Level::Barrel(i) = self.level { i } else { 0 };
        // A veces salta a un barril vecino, a veces al muro; casi siempre al piso.
        let r = self.rand();
        if r < 0.18 {
            if let Some(j) = self.other_barrel(cur) {
                self.jump_to_barrel(j);
                return;
            }
        }
        if r < 0.32 {
            self.jump_to_wall();
        } else {
            self.jump_down_to_floor();
        }
    }

    fn nearest_barrel(&self, reach: f64) -> Option<usize> {
        let mut best = None;
        let mut bd = reach;
        for (i, b) in self.barrels.iter().enumerate() {
            let d = (b.x - self.body_x).abs();
            if d < bd {
                bd = d;
                best = Some(i);
            }
        }
        best
    }

    fn other_barrel(&mut self, cur: usize) -> Option<usize> {
        if self.barrels.len() < 2 {
            return None;
        }
        let mut j = (self.rand() * self.barrels.len() as f64) as usize % self.barrels.len();
        if j == cur {
            j = (j + 1) % self.barrels.len();
        }
        Some(j)
    }

    fn start_walk_floor(&mut self, tempo: f64) {
        let to = self.rand_range(2.5 * U, VW - 2.5 * U);
        let speed = self.rand_range(2.0, 3.2) * U * tempo;
        self.mode = Mode::Walk { to, speed };
    }

    fn start_walk_wall(&mut self, tempo: f64) {
        let to = self.rand_range(2.0 * U, VW - 2.0 * U);
        let speed = self.rand_range(2.2, 3.6) * U * tempo;
        self.mode = Mode::Walk { to, speed };
    }

    fn jump_to_barrel(&mut self, i: usize) {
        let x1 = self.barrels[i].x;
        let support = GROUND_V - barrel_h(self.barrels[i].big);
        self.start_jump(x1, support, Dest::Barrel(i), 1.2 * U, false);
    }

    fn jump_to_wall(&mut self) {
        let x1 = (self.body_x + self.facing * self.rand_range(1.0, 3.0) * U).clamp(2.0 * U, VW - 2.0 * U);
        self.start_jump(x1, WALL_V, Dest::Wall, 1.4 * U, false);
    }

    fn jump_down_to_floor(&mut self) {
        let x1 = (self.body_x + self.facing * self.rand_range(1.5, 3.0) * U).clamp(2.5 * U, VW - 2.5 * U);
        self.start_jump(x1, GROUND_V, Dest::Floor, 0.6 * U, false);
    }

    fn start_jump(&mut self, x1: f64, dest_support: f64, dest: Dest, extra: f64, flail: bool) {
        let y0 = self.body_cy;
        let y1 = dest_support - BODY_ABOVE;
        let hump = (y0 - y1).abs() * 0.5 + extra;
        let dist = (x1 - self.body_x).abs();
        let dur = (0.42 + dist / (VW) * 0.6).clamp(0.42, 0.85);
        self.mode = Mode::Jump { x0: self.body_x, y0, x1, y1, hump, t: 0.0, dur, dest, flail };
    }

    fn land(&mut self, dest: Dest, x1: f64, y1: f64) {
        self.body_x = x1;
        self.body_cy = y1;
        self.airborne = 0.0;
        self.flail = 0.0;
        match dest {
            Dest::Floor => {
                self.level = Level::Floor;
                self.support_y = GROUND_V;
                self.mode = Mode::Pause { left: self.rand_range(0.25, 0.9) };
            }
            Dest::Wall => {
                self.level = Level::Wall;
                self.support_y = WALL_V;
                self.mode = Mode::Pause { left: self.rand_range(0.3, 1.0) };
            }
            Dest::Barrel(i) => {
                self.level = Level::Barrel(i);
                self.support_y = GROUND_V - barrel_h(self.barrels[i].big);
                self.mode = Mode::Perch { left: self.rand_range(1.0, 3.0) };
            }
        }
    }

    fn start_leap_miss(&mut self) {
        self.mode = Mode::LeapMiss { t: 0.0, dur: 0.55 };
    }

    /// Lo tumba de un barril al piso (lo dispara el evento de asomada).
    fn knock_off(&mut self, i: usize) {
        let from = self.barrels[i].x;
        let dir = if self.chance(0.5) { 1.0 } else { -1.0 };
        let x1 = (from + dir * self.rand_range(1.5, 2.8) * U).clamp(2.5 * U, VW - 2.5 * U);
        self.level = Level::Floor;
        self.mode = Mode::Knocked { x0: self.body_x, y0: self.body_cy, x1, t: 0.0, dur: 0.55 };
    }

    /// Golpeado en el muro → cae por detrás.
    fn knock_behind(&mut self) {
        self.mode = Mode::Behind { y0: self.body_cy, t: 0.0, dur: 0.75 };
    }

    /// Atrapado por el perro → bola de humo y desaparece.
    fn caught(&mut self, bx: f64, by: f64) {
        self.smoke = Some((bx, by, 0.0, 0.95));
        self.mode = Mode::Caught { left: 0.7 };
    }

    /// Reaparece en el piso, en un punto al azar, listo para repetir la escena.
    fn reappear(&mut self) {
        self.level = Level::Floor;
        self.support_y = GROUND_V;
        self.body_x = self.rand_range(3.0 * U, VW - 3.0 * U);
        self.body_cy = GROUND_V - BODY_ABOVE;
        self.facing = if self.chance(0.5) { 1.0 } else { -1.0 };
        self.airborne = 0.0;
        self.flail = 0.0;
        self.mode = Mode::Pause { left: self.rand_range(0.4, 1.0) };
    }

    /// Re-resuelve la pose del rig (cuerpo, IK de patas, cola) para el estado.
    fn pose(&mut self, dt: f64) {
        self.skel.set_pose(
            self.root,
            Pose::new(Vec2::new(self.body_x, self.body_cy), 0.0, Vec2::new(1.0, 1.0)),
        );
        self.skel.update();

        let a = self.airborne.clamp(0.0, 1.0);
        for leg in &self.legs {
            let p = frac(self.gait + leg.phase);
            let (rx, ry) = gait_foot(p);
            let grounded = Point::new(self.body_x + leg.hip_dx + rx, self.support_y + ry);
            // En el aire las patas se recogen bajo el vientre; las delanteras se
            // estiran hacia arriba con `flail` (manotean el tendedero / forcejean).
            let front = leg.hip_dx > 0.0;
            let tuck_lift = if front { self.flail * 1.6 * U } else { self.flail * 0.4 * U };
            let tuck = Point::new(self.body_x + leg.hip_dx * 0.55, self.body_cy + 1.3 * U - tuck_lift);
            let target = Point::new(grounded.x * (1.0 - a) + tuck.x * a, grounded.y * (1.0 - a) + tuck.y * a);
            solve_two_bone_ik(&mut self.skel, leg.upper, leg.lower, Vec2::new(SEG, 0.0), target, leg.flip);
        }

        // Cola: ancla en la grupa; viento senoidal + gravedad → ondea por detrás.
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

    /// Snapshot de dibujo del frame: escena completa (gato + props), `Send + 'static`.
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
        let head_nod = 0.04 * U * (self.gait * TAU).sin();
        let cat_hidden = matches!(self.mode, Mode::Gone { .. } | Mode::Caught { .. });
        let cat_behind_wall = matches!(self.mode, Mode::Behind { .. });
        CatSnapshot {
            legs,
            body: Point::new(self.body_x, self.body_cy),
            head: Point::new(self.body_x + 2.1 * U, self.body_cy - 0.7 * U + head_nod),
            tail: self.tail.positions(),
            accent: self.accent,
            facing: self.facing as f32,
            support_y: self.support_y,
            airborne: self.airborne as f32,
            cat_hidden,
            cat_behind_wall,
            barrels: self
                .barrels
                .iter()
                .map(|b| BarrelSnap { x: b.x, big: b.big, lid: b.lid as f32, peek: b.peek as f32 })
                .collect(),
            dog: self.dog.as_ref().map(|d| DogSnap { x: d.x, dir: d.dir as f32, sz: d.sz, phase: d.phase }),
            shoes: self.shoes.iter().map(|s| ShoeSnap { p: Point::new(s.x, s.y), spin: s.spin as f32 }).collect(),
            smoke: self.smoke.map(|sm| (Point::new(sm.0, sm.1), (sm.2 / sm.3) as f32)),
        }
    }
}

// ─────────────────────────────── snapshot ───────────────────────────────────

/// Un barril en el snapshot.
pub struct BarrelSnap {
    pub x: f64,
    pub big: bool,
    pub lid: f32,
    pub peek: f32,
}

/// El perro en el snapshot.
pub struct DogSnap {
    pub x: f64,
    pub dir: f32,
    pub sz: f64,
    pub phase: f64,
}

/// Un zapato en vuelo en el snapshot.
pub struct ShoeSnap {
    pub p: Point,
    pub spin: f32,
}

/// Datos de dibujo de un frame completo de la escena (todo en virtual `VW×VH`).
pub struct CatSnapshot {
    /// Por pata: `(cadera, rodilla, pezuña, cercana)`.
    pub legs: Vec<(Point, Point, Point, bool)>,
    pub body: Point,
    pub head: Point,
    pub tail: Vec<Point>,
    pub accent: (f32, f32, f32),
    /// ±1: sentido en que mira el gato (espejo visual).
    pub facing: f32,
    /// Línea de apoyo actual (para la sombra).
    pub support_y: f64,
    /// 0 apoyado .. 1 en el aire.
    pub airborne: f32,
    /// Está fuera de escena (no se dibuja el gato).
    pub cat_hidden: bool,
    /// Cae por detrás del muro (se dibuja antes que la cerca).
    pub cat_behind_wall: bool,
    pub barrels: Vec<BarrelSnap>,
    pub dog: Option<DogSnap>,
    pub shoes: Vec<ShoeSnap>,
    /// `(posición, edad 0..1)` de la bola de humo.
    pub smoke: Option<(Point, f32)>,
}

// ─────────────────────────────── pintado ────────────────────────────────────

/// Pinta la escena del snapshot sobre `rect`, en capas, con la oclusión correcta:
/// fachada → (gato si cae por detrás) → cerca → piso → barriles → gato → perro →
/// zapatos → humo. `t` anima el telón (parpadeos, ropa, ratones).
pub fn paint_rig(
    snap: &CatSnapshot,
    scene: &mut vello::Scene,
    _ts: &mut Typesetter,
    rect: PaintRect,
    t: f32,
    bright: (u8, u8, u8),
) {
    if rect.w < 64.0 || rect.h < 64.0 {
        return;
    }
    let acc = (bright.0 as f32, bright.1 as f32, bright.2 as f32);
    let xf = Affine::translate((rect.x as f64, rect.y as f64))
        * Affine::scale_non_uniform(rect.w as f64 / VW, rect.h as f64 / VH);

    paint_facade(scene, rect, t, acc);
    // El gato que cae «por detrás del muro» va antes que la cerca, que lo tapa.
    if snap.cat_behind_wall && !snap.cat_hidden {
        draw_cat(scene, xf, snap);
    }
    paint_fence(scene, rect, acc);
    paint_ground(scene, rect, acc);
    for b in &snap.barrels {
        draw_barrel(scene, xf, b, acc);
    }
    if !snap.cat_behind_wall && !snap.cat_hidden {
        draw_cat(scene, xf, snap);
    }
    if let Some(d) = &snap.dog {
        draw_dog(scene, xf, d, acc);
    }
    for s in &snap.shoes {
        draw_shoe(scene, xf, s);
    }
    if let Some((at, age)) = &snap.smoke {
        draw_smoke(scene, xf, *at, *age);
    }
}

/// Fallback **stateless**: telón quieto + barriles deterministas + un gato parado
/// en el piso. Firma de [`crate::rain::paint`]; lo usa el despachador [`crate::bg`].
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
    let acc = (bright.0 as f32, bright.1 as f32, bright.2 as f32);
    let xf = Affine::translate((rect.x as f64, rect.y as f64))
        * Affine::scale_non_uniform(rect.w as f64 / VW, rect.h as f64 / VH);

    paint_facade(scene, rect, t, acc);
    paint_fence(scene, rect, acc);
    paint_ground(scene, rect, acc);
    // Dos barriles deterministas.
    for (i, &(fx, big)) in [(0.30, true), (0.62, false)].iter().enumerate() {
        let b = BarrelSnap { x: VW * fx, big, lid: 0.0, peek: 0.0 };
        let _ = i;
        draw_barrel(scene, xf, &b, acc);
    }
    // Gato parado, con un ciclo de marcha suave en el lugar.
    let step = t * 4.0;
    paint_cat_idle(scene, xf, VW * 0.45, GROUND_V, step as f64, acc);
}

/// Gato simple por senos para el fallback (sin rig): cuerpo, cabeza, cola, patas.
fn paint_cat_idle(scene: &mut vello::Scene, xf: Affine, fx: f64, support: f64, step: f64, acc: (f32, f32, f32)) {
    let fur = col(54.0, 56.0, 66.0, 255);
    let belly = col(78.0, 80.0, 92.0, 255);
    let fur_dark = col(38.0, 40.0, 50.0, 255);
    let bx = fx;
    let by = support - BODY_ABOVE;

    scene.fill(Fill::NonZero, xf, col(0.0, 0.0, 0.0, 70), None, &Ellipse::new(Point::new(bx, support + 0.05 * U), (2.4 * U, 0.36 * U), 0.0));

    let s1 = step.sin();
    let s2 = (step + PI).sin();
    for (dx, sw) in [(1.35, s2), (-1.45, s1)] {
        let hip = Point::new(bx + dx * U, by + 0.55 * U);
        let foot = Point::new(hip.x + sw * 0.3 * U, support);
        let knee = Point::new((hip.x + foot.x) * 0.5 + 0.18 * U, (hip.y + foot.y) * 0.5);
        let mut p = BezPath::new();
        p.move_to(hip);
        p.line_to(knee);
        p.line_to(foot);
        scene.stroke(&Stroke::new(0.32 * U), xf, fur_dark, None, &p);
    }

    let tail_sway = (step * 1.3).sin();
    let mut tail = BezPath::new();
    tail.move_to(Point::new(bx - 2.0 * U, by - 0.1 * U));
    tail.quad_to(
        Point::new(bx - 3.3 * U, by - (0.9 + 0.5 * tail_sway) * U),
        Point::new(bx - 3.7 * U + 0.6 * U * tail_sway, by - (2.2 + 0.4 * tail_sway) * U),
    );
    scene.stroke(&Stroke::new(0.42 * U), xf, fur, None, &tail);

    scene.fill(Fill::NonZero, xf, fur, None, &Ellipse::new(Point::new(bx, by), (2.0 * U, 0.9 * U), 0.0));
    scene.fill(Fill::NonZero, xf, fur, None, &Circle::new(Point::new(bx - 1.5 * U, by - 0.05 * U), 0.95 * U));
    scene.fill(Fill::NonZero, xf, belly, None, &Ellipse::new(Point::new(bx + 0.1 * U, by + 0.55 * U), (1.5 * U, 0.4 * U), 0.0));

    let hx = bx + 2.1 * U;
    let hy = by - 0.7 * U;
    scene.fill(Fill::NonZero, xf, fur, None, &Circle::new(Point::new(hx, hy), 0.78 * U));
    for ear_dx in [-0.45, 0.5] {
        let ex = hx + ear_dx * U;
        let mut ear = BezPath::new();
        ear.move_to(Point::new(ex - 0.32 * U, hy - 0.55 * U));
        ear.line_to(Point::new(ex + 0.05 * U, hy - 1.35 * U));
        ear.line_to(Point::new(ex + 0.42 * U, hy - 0.5 * U));
        ear.close_path();
        scene.fill(Fill::NonZero, xf, fur, None, &ear);
    }
    scene.fill(Fill::NonZero, xf, belly, None, &Circle::new(Point::new(hx + 0.55 * U, hy + 0.18 * U), 0.34 * U));
    let eye = Point::new(hx + 0.30 * U, hy - 0.08 * U);
    scene.fill(Fill::NonZero, xf, col(acc.0, acc.1, acc.2, 70), None, &Circle::new(eye, 0.34 * U));
    scene.fill(
        Fill::NonZero,
        xf,
        col((acc.0 + 180.0).min(255.0), (acc.1 + 180.0).min(255.0), (acc.2 + 180.0).min(255.0), 255),
        None,
        &Circle::new(eye, 0.16 * U),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rig_marcha_viva_y_finita() {
        let mut bg = AlleyCatBg::new((255, 200, 120));
        let s0 = bg.snapshot();
        assert_eq!(s0.legs.len(), 4, "cuatro patas");
        assert!(s0.barrels.len() >= 2 && s0.barrels.len() <= 4, "2..4 barriles");
        assert_eq!(s0.tail.len(), TAIL_N, "cola con todas sus partículas");
        for (hip, knee, foot, _) in &s0.legs {
            for p in [hip, knee, foot] {
                assert!(p.x.is_finite() && p.y.is_finite(), "articulación finita");
            }
        }

        // Tras un rato la escena evoluciona sin explotar a NaN/infinito.
        for _ in 0..600 {
            bg.step(1.0 / 30.0);
        }
        let s1 = bg.snapshot();
        for (hip, knee, foot, _) in &s1.legs {
            for p in [hip, knee, foot] {
                assert!(p.x.is_finite() && p.y.is_finite() && p.x.abs() < 1e6 && p.y.abs() < 1e6, "no explota");
            }
        }
        assert!(s1.body.x.is_finite() && s1.body.x.abs() < 1e6, "cuerpo finito");
        for b in &s1.barrels {
            assert!(b.x.is_finite() && b.lid.is_finite() && b.peek.is_finite());
        }
    }

    #[test]
    fn rutina_explora_estados() {
        // Corriendo bastante, el gato debe pasar por estados aéreos (saltos),
        // generar eventos (perro/zapatos/asomadas) y permanecer acotado.
        let mut bg = AlleyCatBg::new((120, 200, 255));
        let mut vio_aire = false;
        let mut vio_oculto = false;
        let mut vio_perro = false;
        let mut vio_zapato = false;
        for _ in 0..6000 {
            bg.step(1.0 / 30.0);
            let s = bg.snapshot();
            if s.airborne > 0.5 {
                vio_aire = true;
            }
            if s.cat_hidden || s.cat_behind_wall {
                vio_oculto = true;
            }
            if s.dog.is_some() {
                vio_perro = true;
            }
            if !s.shoes.is_empty() {
                vio_zapato = true;
            }
            assert!(s.body.x >= -10.0 * U && s.body.x <= VW + 10.0 * U, "x acotado");
        }
        assert!(vio_aire, "el gato salta");
        assert!(vio_oculto, "el gato desaparece y reaparece");
        assert!(vio_perro, "sale el perro");
        assert!(vio_zapato, "las ventanas tiran zapatos");
    }
}
