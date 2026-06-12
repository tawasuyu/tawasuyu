//! Ruteo A* sobre la red de líneas del `MapData`: soberano, offline, sin
//! servicio externo. Los vértices se funden por cuantización para unir cruces.

use std::collections::{BinaryHeap, HashMap};

use crate::tipos::{Coord, MapData};

/// Resultado de un ruteo: la polilínea seguida y su longitud en metros.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteResult {
    pub path: Vec<Coord>,
    pub meters: f64,
}

/// Calcula la ruta más corta entre `from` y `to` sobre la red de líneas
/// (`data.lines`), con A\* y heurística haversine. Soberano y offline: es
/// matemática de grafos sobre el dato cargado, sin OSRM ni servicio externo.
/// `None` si no hay red o los extremos quedan desconectados.
///
/// Los vértices se funden por proximidad (cuantización a ~0,1 m), así las
/// líneas que comparten un cruce quedan conectadas en el grafo.
pub fn route(data: &MapData, from: Coord, to: Coord) -> Option<RouteResult> {
    if data.lines.is_empty() {
        return None;
    }
    // Grafo no dirigido: nodos = vértices fundidos; aristas = tramos.
    let mut ids: HashMap<(i64, i64), usize> = HashMap::new();
    let mut coords: Vec<Coord> = Vec::new();
    let mut adj: Vec<Vec<(usize, f64)>> = Vec::new();
    for line in &data.lines {
        for w in line.windows(2) {
            let a = intern_node(w[0], &mut ids, &mut coords, &mut adj);
            let b = intern_node(w[1], &mut ids, &mut coords, &mut adj);
            if a == b {
                continue;
            }
            let d = haversine(w[0], w[1]);
            adj[a].push((b, d));
            adj[b].push((a, d));
        }
    }
    let src = nearest_node(&coords, from)?;
    let dst = nearest_node(&coords, to)?;

    // A* con heurística admisible (línea recta haversine al destino).
    let n = coords.len();
    let mut g = vec![f64::INFINITY; n];
    let mut came = vec![usize::MAX; n];
    g[src] = 0.0;
    let mut heap = BinaryHeap::new();
    heap.push(AStarNode {
        f: haversine(coords[src], coords[dst]),
        node: src,
    });
    while let Some(AStarNode { node, .. }) = heap.pop() {
        if node == dst {
            break;
        }
        for &(nb, w) in &adj[node] {
            let tentative = g[node] + w;
            if tentative < g[nb] {
                g[nb] = tentative;
                came[nb] = node;
                heap.push(AStarNode {
                    f: tentative + haversine(coords[nb], coords[dst]),
                    node: nb,
                });
            }
        }
    }
    if g[dst].is_infinite() {
        return None;
    }
    // Reconstruir el camino de destino a origen y darlo vuelta.
    let mut path = Vec::new();
    let mut cur = dst;
    while cur != usize::MAX {
        path.push(coords[cur]);
        if cur == src {
            break;
        }
        cur = came[cur];
    }
    path.reverse();
    Some(RouteResult {
        path,
        meters: g[dst],
    })
}

/// Distancia geodésica entre dos coordenadas (haversine), en metros.
pub fn haversine(a: Coord, b: Coord) -> f64 {
    const R: f64 = 6_371_000.0;
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlat = (b[1] - a[1]).to_radians();
    let dlon = (b[0] - a[0]).to_radians();
    let h = (dlat * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon * 0.5).sin().powi(2);
    2.0 * R * h.sqrt().clamp(-1.0, 1.0).asin()
}

/// Inserta (o reusa) el nodo del grafo para una coordenada, fundiendo por
/// cuantización a ~1e-6° (~0,1 m) para unir cruces compartidos.
fn intern_node(
    c: Coord,
    ids: &mut HashMap<(i64, i64), usize>,
    coords: &mut Vec<Coord>,
    adj: &mut Vec<Vec<(usize, f64)>>,
) -> usize {
    let k = ((c[0] * 1e6).round() as i64, (c[1] * 1e6).round() as i64);
    if let Some(&i) = ids.get(&k) {
        return i;
    }
    let i = coords.len();
    ids.insert(k, i);
    coords.push(c);
    adj.push(Vec::new());
    i
}

/// Nodo del grafo más cercano a una coordenada (snap del clic a la red).
fn nearest_node(coords: &[Coord], c: Coord) -> Option<usize> {
    coords
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| haversine(**a, c).total_cmp(&haversine(**b, c)))
        .map(|(i, _)| i)
}

/// Entrada de la cola de prioridad de A\*: min-heap por `f` (total order vía
/// `total_cmp`, invertido para que el menor quede en la cima).
struct AStarNode {
    f: f64,
    node: usize,
}
impl PartialEq for AStarNode {
    fn eq(&self, o: &Self) -> bool {
        self.f == o.f
    }
}
impl Eq for AStarNode {}
impl Ord for AStarNode {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.f.total_cmp(&self.f)
    }
}
impl PartialOrd for AStarNode {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
