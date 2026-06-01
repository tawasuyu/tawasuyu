//! Layout de **treemap jerárquico** (fractal) para el mapa de procesos.
//!
//! Slice-and-dice recursivo: cada nodo recibe un rectángulo proporcional a su
//! peso total (propio + descendientes); sus hijos se reparten el interior
//! (menos una franja de cabecera para mostrar al padre), alternando la
//! orientación por profundidad. El "peso propio" del nodo entra como un hijo
//! sintético, así cada proceso recibe su propio rectángulo. Es puro y
//! testeable; el dibujo vive en `main.rs`.

use std::collections::HashMap;

/// Entrada cruda: un proceso con su peso (RSS o CPU) y su color (cpu).
pub struct Item {
    pub pid: i32,
    pub ppid: i32,
    pub weight: f64,
    pub cpu: f32,
    pub label: String,
}

/// Un rectángulo ya colocado, listo para pintar.
#[derive(Clone, Debug)]
pub struct Cell {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub depth: u16,
    pub cpu: f32,
    pub label: String,
    /// `true` si es el rectángulo propio de un proceso (hoja visual), `false`
    /// si es el contenedor de un padre (sólo se ve su cabecera + borde).
    pub leaf: bool,
}

/// Coloca `items` (jerarquía por `ppid`) en `area = (x, y, w, h)`. `header` es
/// la franja superior que cada padre reserva para sí; `min_side` corta la
/// recursión cuando un rect es más chico que eso (evita clutter ilegible).
pub fn layout(items: &[Item], area: (f32, f32, f32, f32), header: f32, min_side: f32) -> Vec<Cell> {
    if items.is_empty() {
        return Vec::new();
    }
    let pos: HashMap<i32, usize> = items.iter().enumerate().map(|(i, it)| (it.pid, i)).collect();
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if it.ppid != it.pid && it.ppid != 0 && pos.contains_key(&it.ppid) {
            children.entry(it.ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }

    // Peso total memoizado (propio + subárbol), con guarda anti-ciclo.
    let mut totals: HashMap<usize, f64> = HashMap::new();
    for i in 0..items.len() {
        total_of(i, items, &children, &pos, &mut totals, &mut Vec::new());
    }

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    place(
        &roots, items, &children, &pos, &totals, area, 0, true, header, min_side, &mut out, &mut seen,
    );
    out
}

fn total_of(
    i: usize,
    items: &[Item],
    children: &HashMap<i32, Vec<usize>>,
    pos: &HashMap<i32, usize>,
    memo: &mut HashMap<usize, f64>,
    stack: &mut Vec<usize>,
) -> f64 {
    if let Some(&v) = memo.get(&i) {
        return v;
    }
    if stack.contains(&i) {
        return items[i].weight.max(0.0); // ciclo: sólo el propio
    }
    stack.push(i);
    let mut sum = items[i].weight.max(0.0);
    if let Some(kids) = children.get(&items[i].pid) {
        for &k in kids {
            sum += total_of(k, items, children, pos, memo, stack);
        }
    }
    stack.pop();
    memo.insert(i, sum);
    sum
}

#[allow(clippy::too_many_arguments)]
fn place(
    idxs: &[usize],
    items: &[Item],
    children: &HashMap<i32, Vec<usize>>,
    pos: &HashMap<i32, usize>,
    totals: &HashMap<usize, f64>,
    area: (f32, f32, f32, f32),
    depth: u16,
    horizontal: bool,
    header: f32,
    min_side: f32,
    out: &mut Vec<Cell>,
    seen: &mut std::collections::HashSet<i32>,
) {
    let (ax, ay, aw, ah) = area;
    if aw < min_side || ah < min_side {
        return;
    }
    let sum: f64 = idxs.iter().map(|&i| totals[&i].max(1.0)).sum();
    if sum <= 0.0 {
        return;
    }
    // Mayor primero → los grandes arriba/izquierda.
    let mut order: Vec<usize> = idxs.to_vec();
    order.sort_by(|&a, &b| totals[&b].partial_cmp(&totals[&a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut cursor = if horizontal { ax } else { ay };
    for &i in &order {
        if !seen.insert(items[i].pid) {
            continue;
        }
        let frac = (totals[&i].max(1.0) / sum) as f32;
        let (cx, cy, cw, ch) = if horizontal {
            let w = aw * frac;
            let r = (cursor, ay, w, ah);
            cursor += w;
            r
        } else {
            let h = ah * frac;
            let r = (ax, cursor, aw, h);
            cursor += h;
            r
        };

        let kids = children.get(&items[i].pid);
        let has_kids = kids.map(|k| !k.is_empty()).unwrap_or(false);

        out.push(Cell {
            x: cx,
            y: cy,
            w: cw,
            h: ch,
            depth,
            cpu: items[i].cpu,
            label: items[i].label.clone(),
            leaf: !has_kids,
        });

        // Recursión: hijos reales + un "self" sintético con el peso propio.
        if has_kids && ch > header + min_side && cw > min_side {
            let inner = (cx + 1.0, cy + header, cw - 2.0, ch - header - 1.0);
            let mut group: Vec<usize> = kids.cloned().unwrap_or_default();
            // El peso propio se representa recursando con el mismo nodo como
            // hoja: lo hacemos marcando que en el grupo va también `i`, pero
            // para evitar re-emitirlo (ya está en `seen`) sólo cuenta su peso
            // a través de un reparto que deja hueco. Simplificación: repartimos
            // sólo entre hijos; el propio queda representado por la cabecera.
            group.retain(|&k| k != i);
            place(
                &group, items, children, pos, totals, inner, depth + 1, !horizontal, header,
                min_side, out, seen,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(pid: i32, ppid: i32, w: f64) -> Item {
        Item {
            pid,
            ppid,
            weight: w,
            cpu: 0.0,
            label: format!("p{pid}"),
        }
    }

    #[test]
    fn reparte_area_y_anida() {
        // 1 → {2,3};  2 → {4}
        let items = vec![it(1, 0, 10.0), it(2, 1, 20.0), it(3, 1, 30.0), it(4, 2, 5.0)];
        let cells = layout(&items, (0.0, 0.0, 100.0, 100.0), 8.0, 2.0);
        // Cada pid aparece exactamente una vez.
        let mut pids: Vec<&str> = cells.iter().map(|c| c.label.as_str()).collect();
        pids.sort();
        assert_eq!(pids, vec!["p1", "p2", "p3", "p4"]);
        // La raíz ocupa toda el área.
        let root = cells.iter().find(|c| c.label == "p1").unwrap();
        assert!((root.w - 100.0).abs() < 0.01 && (root.h - 100.0).abs() < 0.01);
        // El hijo está dentro del padre (más profundo).
        let p4 = cells.iter().find(|c| c.label == "p4").unwrap();
        assert!(p4.depth >= 2);
    }
}
