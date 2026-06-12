//! Helpers geométricos de bajo nivel: push de primitivas, etiquetado,
//! constructores de features, distancias en pantalla y tests de inclusión.
//!
//! Son funciones internas — no forman parte de la API pública del crate.

use crate::tipos::{Coord, FeatureProps, Label, MapData, Ring};
use crate::tipos::{MAX_LABELS, MAX_PROPS};

// ─── Empuje de primitivas ────────────────────────────────────────────────────

pub fn push_points(data: &mut MapData, pts: &[Coord], budget: &mut usize, feat: usize) {
    for p in pts {
        if *budget == 0 {
            return;
        }
        data.points.push(*p);
        data.point_feat.push(feat);
        *budget -= 1;
    }
}

pub fn push_line(data: &mut MapData, mut line: Ring, budget: &mut usize, feat: usize) {
    if line.len() < 2 {
        return;
    }
    line.truncate(*budget);
    if line.len() < 2 {
        return;
    }
    *budget -= line.len();
    data.lines.push(line);
    data.line_feat.push(feat);
}

pub fn push_polygon(data: &mut MapData, rings: Vec<Ring>, budget: &mut usize, feat: usize) {
    let mut kept: Vec<Ring> = Vec::new();
    for mut ring in rings {
        if *budget == 0 {
            break;
        }
        ring.truncate(*budget);
        if ring.len() < 3 {
            continue;
        }
        *budget -= ring.len();
        kept.push(ring);
    }
    if !kept.is_empty() {
        data.polygons.push(kept);
        data.polygon_feat.push(feat);
    }
}

// ─── Features y etiquetas ────────────────────────────────────────────────────

/// Crea una feature con un nombre opcional (para formatos sin propiedades
/// ricas: GPX/KML, o geometrías sueltas) y devuelve su índice.
pub fn make_feature(data: &mut MapData, name: Option<&str>) -> usize {
    let mut fp = FeatureProps::default();
    if let Some(n) = name {
        fp.name = Some(n.to_string());
        fp.props.push(("name".to_string(), n.to_string()));
    }
    data.features.push(fp);
    data.features.len() - 1
}

/// Construye [`FeatureProps`] desde el objeto `properties` de una Feature
/// GeoJSON: conserva escalares (número/string/bool) en orden, y los números
/// también en `numbers` para choropleth. Omite null/array/objeto.
pub fn feature_props(props: Option<&serde_json::Value>) -> FeatureProps {
    let mut fp = FeatureProps::default();
    let Some(obj) = props.and_then(|p| p.as_object()) else {
        return fp;
    };
    for (k, v) in obj {
        if fp.props.len() >= MAX_PROPS {
            break;
        }
        match v {
            serde_json::Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    fp.numbers.push((k.clone(), f));
                    fp.props.push((k.clone(), n.to_string()));
                }
            }
            serde_json::Value::String(s) => fp.props.push((k.clone(), s.clone())),
            serde_json::Value::Bool(b) => fp.props.push((k.clone(), b.to_string())),
            _ => {}
        }
    }
    fp
}

/// Ancla una etiqueta a `at` si hay nombre y punto, respetando [`MAX_LABELS`].
pub fn label_at(data: &mut MapData, name: Option<&str>, at: Option<Coord>) {
    if let (Some(text), Some(at)) = (name, at) {
        if data.labels.len() < MAX_LABELS {
            data.labels.push(Label {
                at,
                text: text.to_string(),
            });
        }
    }
}

// ─── Geometría analítica ─────────────────────────────────────────────────────

/// Vértice central de una polilínea (rótulo de líneas).
pub fn midpoint(line: &[Coord]) -> Option<Coord> {
    if line.is_empty() {
        None
    } else {
        Some(line[line.len() / 2])
    }
}

/// Centroide simple (promedio de vértices) de un anillo, ignorando el último
/// si repite el primero (anillos GeoJSON cerrados).
pub fn centroid(ring: &[Coord]) -> Option<Coord> {
    let pts: &[Coord] = match ring.split_last() {
        Some((last, head)) if !head.is_empty() && last == &head[0] => head,
        _ => ring,
    };
    if pts.is_empty() {
        return None;
    }
    let (mut sx, mut sy) = (0.0, 0.0);
    for [lon, lat] in pts {
        sx += lon;
        sy += lat;
    }
    let n = pts.len() as f64;
    Some([sx / n, sy / n])
}

// ─── Helpers de parseo GeoJSON ───────────────────────────────────────────────

/// Lee una coordenada `[lon, lat(, z)]` de un valor JSON. `None` si no es un
/// array de al menos dos números finitos.
pub fn coord(v: Option<&serde_json::Value>) -> Option<Coord> {
    let arr = v?.as_array()?;
    let lon = arr.first()?.as_f64()?;
    let lat = arr.get(1)?.as_f64()?;
    if lon.is_finite() && lat.is_finite() {
        Some([lon, lat])
    } else {
        None
    }
}

/// Lee una lista de coordenadas `[[lon,lat], ...]`.
pub fn coord_list(v: Option<&serde_json::Value>) -> Vec<Coord> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().filter_map(|c| coord(Some(c))).collect()
}

/// Lee una lista de anillos `[[[lon,lat], ...], ...]`.
pub fn coord_rings(v: Option<&serde_json::Value>) -> Vec<Ring> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().map(|r| coord_list(Some(r))).collect()
}

// ─── Geometría en espacio de pantalla ────────────────────────────────────────

/// Distancia de un punto `(px, py)` al segmento `a–b`, en pantalla.
pub fn dist_point_seg(px: f64, py: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (ax, ay) = a;
    let (bx, by) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    if len2 <= 1e-12 {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    let (qx, qy) = (ax + t * dx, ay + t * dy);
    (px - qx).hypot(py - qy)
}

/// Test punto-en-anillo (even-odd / ray casting) en espacio de pantalla.
pub fn point_in_ring_screen(
    px: f64,
    py: f64,
    ring: &[Coord],
    proj: &crate::camara::Projection,
) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = proj.to_screen(ring[i]);
        let (xj, yj) = proj.to_screen(ring[j]);
        if (yi > py) != (yj > py) {
            let x_cross = (xj - xi) * (py - yi) / (yj - yi) + xi;
            if px < x_cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
