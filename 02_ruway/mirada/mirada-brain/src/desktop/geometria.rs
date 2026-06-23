//! Funciones de geometría pura: foco espacial y rectángulos auxiliares.

use mirada_layout::Rect;

use crate::action::Direction;

/// El elemento de `candidates` (ventana o salida) más cercano a `from` en
/// la dirección `dir`, excluyendo a `self_id`. Pura — la base del foco
/// espacial entre ventanas y entre monitores.
///
/// Criterio (estilo i3/sway): sólo cuentan los candidatos cuyo centro cae
/// en el semiplano de esa dirección respecto al centro de `from`; entre
/// ellos gana el de menor distancia en el eje principal, penalizando el
/// desvío en el eje perpendicular (`×2`) para preferir el que está
/// «enfrente». Empates: el id menor, para ser determinista.
pub fn nearest_in_direction<T: Copy + Ord>(
    from: Rect,
    candidates: &[(T, Rect)],
    self_id: T,
    dir: Direction,
) -> Option<T> {
    let center = |r: &Rect| (r.x + r.w / 2, r.y + r.h / 2);
    let (fx, fy) = center(&from);
    let mut best: Option<(i64, T)> = None;
    for (id, rect) in candidates {
        if *id == self_id {
            continue;
        }
        let (cx, cy) = center(rect);
        let (dx, dy) = ((cx - fx) as i64, (cy - fy) as i64);
        // ¿Está en el semiplano de la dirección? (`primary` > 0) y, si sí,
        // el coste = primary + 2·|perpendicular|.
        let (primary, perp) = match dir {
            Direction::Left => (-dx, dy),
            Direction::Right => (dx, dy),
            Direction::Up => (-dy, dx),
            Direction::Down => (dy, dx),
        };
        if primary <= 0 {
            continue;
        }
        let cost = primary + 2 * perp.abs();
        let better = match best {
            None => true,
            Some((c, bid)) => cost < c || (cost == c && *id < bid),
        };
        if better {
            best = Some((cost, *id));
        }
    }
    best.map(|(_, id)| id)
}

/// El rectángulo de la terminal dropdown: anclada arriba, a todo el ancho,
/// `pct` % del alto — el gesto «quake» de bajar desde el borde superior.
/// El porcentaje sale de la config ([`crate::config::Config::dropterm_height_pct`]).
pub fn dropdown_rect(screen: Rect, pct: i32) -> Rect {
    Rect::new(screen.x, screen.y, screen.w, (screen.h * pct / 100).max(1))
}

/// El rectángulo flotante por defecto: 60 % de la pantalla, centrado.
pub fn centered_float_rect(screen: Rect) -> Rect {
    let w = screen.w * 3 / 5;
    let h = screen.h * 3 / 5;
    Rect::new(
        screen.x + (screen.w - w) / 2,
        screen.y + (screen.h - h) / 2,
        w,
        h,
    )
}

/// Como [`centered_float_rect`], pero **escalonado**: el `n`-ésimo flotante se
/// desplaza en diagonal para no caer exactamente sobre los anteriores (todos
/// centrados se «pisaban»). El escalón da la vuelta antes de salirse de la
/// pantalla, así una ráfaga de ventanas flotantes queda en cascada visible en
/// vez de un solo montón.
pub fn cascaded_float_rect(screen: Rect, n: usize) -> Rect {
    let base = centered_float_rect(screen);
    const STEP: i32 = 32;
    // Cuántos escalones caben antes de tocar el borde inferior/derecho del
    // área flotante; pasado eso, el contador da la vuelta (módulo).
    let margin = STEP * 6;
    let span_x = (screen.w - base.w - margin).max(STEP);
    let span_y = (screen.h - base.h - margin).max(STEP);
    let ciclo_x = (span_x / STEP).max(1) as usize;
    let ciclo_y = (span_y / STEP).max(1) as usize;
    let off_x = (n % ciclo_x) as i32 * STEP;
    let off_y = (n % ciclo_y) as i32 * STEP;
    Rect::new(base.x + off_x, base.y + off_y, base.w, base.h)
}
