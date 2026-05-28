//! Matemática agnóstica de surface — radios canónicos del wheel,
//! conversión polar → pantalla, spread anti-solapamiento, detección
//! de clusters, format de coordenadas.
//!
//! Vive aquí (no en el canvas Llimphi) porque exactamente la misma
//! lógica corre en el cliente web (WASM) y en la app desktop. Cualquier
//! ajuste de geometría aparece en ambos a la vez.

use core::f32::consts::PI;

use crate::OUTER_RING_MODULES;

// =====================================================================
// Radii — geometría radial canónica de la rueda
// =====================================================================

/// Geometría radial canónica del wheel. Aros nombrados según convención
/// de Sergio, de afuera hacia adentro:
///
/// * **Aro A** (`sign_outer`) — externo del zodiaco.
/// * **Zona AB** — sign dial: glyphs de signos zodiacales.
/// * **Aro B** (`sign_inner` = `topo_houses_outer`) — interno del
///   zodiaco / externo del bloque ascensional.
/// * **Zona BC** — casas topocéntricas (cusps b→c) + planetas
///   topocéntricos, ambos con sus coordenadas.
/// * **Aro C** (`topo_houses_inner` = `houses_outer`) — separador
///   ascensional / casas geo.
/// * **Zona CD** — casas geocéntricas (cusps c→d) + sus coordenadas.
/// * **Aro D** (`houses_inner`) — externo de los planetas natales.
///   Junto a D, hacia adentro, se posan los planetas natales y sus
///   coordenadas.
/// * **Aro E** (`aspects`) — el más interno. Desde aquí nacen las
///   líneas de aspecto / relaciones / overlays opcionales.
///
/// Los overlays adicionales (transits, midpoints, progression, solar
/// arc, composite) viven INTERIORES al aro E — solo se pintan
/// cuando el módulo correspondiente está activo, así no compiten
/// con el layout base.
#[derive(Clone, Copy, Debug)]
pub struct Radii {
    pub sign_outer: f32,        // Aro A
    pub sign_inner: f32,        // Aro B
    pub topo_houses_outer: f32, // = Aro B
    pub topocentric: f32,       // Zona BC: planetas topo
    pub topo_houses_inner: f32, // Aro C
    pub houses_outer: f32,      // = Aro C
    pub houses_inner: f32,      // Aro D
    pub bodies: f32,            // Zona D-E: planetas natales (junto a D)
    pub pd_direct: f32,         // GR (cuando activo): exterior al cinturón natal
    pub pd_converse: f32,       // GR (cuando activo): interior al cinturón natal
    pub aspects: f32,           // Aro E (invisible, ancla de líneas)
    // Overlays adicionales — todos interiores a E.
    pub transits: f32,
    pub midpoints: f32,
    pub progression: f32,
    pub solar_arc: f32,
    pub composite: f32,
}

impl Radii {
    pub fn from_outer(r: f32) -> Self {
        Self {
            sign_outer: r,
            sign_inner: r * 0.92,
            topo_houses_outer: r * 0.92,
            topocentric: r * 0.85,
            topo_houses_inner: r * 0.78,
            houses_outer: r * 0.78,
            houses_inner: r * 0.62,
            bodies: r * 0.57,
            pd_direct: r * 0.545,
            pd_converse: r * 0.515,
            aspects: r * 0.49,
            transits: r * 0.43,
            midpoints: r * 0.39,
            progression: r * 0.33,
            solar_arc: r * 0.27,
            composite: r * 0.21,
        }
    }

    /// Radio del ring de cuerpos según el `module_id` del Layer.
    pub fn body_ring(&self, module_id: &str) -> f32 {
        match module_id {
            "progression" => self.progression,
            "solar_arc" => self.solar_arc,
            "composite" => self.composite,
            "midpoints" => self.midpoints,
            "topocentric" => self.topocentric,
            "pd_direct" => self.pd_direct,
            "pd_converse" => self.pd_converse,
            _ => self.bodies,
        }
    }

    /// Resuelve qué radios corresponden a una capa de aspectos según el
    /// `module_id`: natal-natal en `aspects`, cross con cada overlay
    /// desde `bodies` (extremo natal) al ring del módulo. Los módulos
    /// del outer ring (OUTER_RING_MODULES) comparten el slot de
    /// tránsito (son mutuamente excluyentes a nivel de Shell).
    pub fn aspect_endpoints(&self, module_id: &str) -> (f32, f32) {
        if OUTER_RING_MODULES.contains(&module_id) {
            return (self.bodies, self.transits);
        }
        match module_id {
            "progression" => (self.bodies, self.progression),
            "solar_arc" => (self.bodies, self.solar_arc),
            "composite" => (self.bodies, self.composite),
            _ => (self.aspects, self.aspects),
        }
    }
}

// =====================================================================
// polar_to_screen — convención de rotación del wheel
// =====================================================================

/// Convierte una longitud eclíptica a coords cartesianas relativas al
/// centro del wheel. Convención: el Ascendente cae a las 9 (lado
/// izquierdo). `rot_offset_deg` permite rotar la vista (jog-dial).
pub fn polar_to_screen(
    longitude_deg: f32,
    ascendant_deg: f32,
    rot_offset_deg: f32,
    radius: f32,
) -> (f32, f32) {
    let deg = 180.0 - (longitude_deg - ascendant_deg + rot_offset_deg);
    let rad = deg * PI / 180.0;
    (radius * rad.cos(), radius * rad.sin())
}

// =====================================================================
// Spread anti-solapamiento de glyphs
// =====================================================================

/// Reposiciona angularmente un conjunto de longitudes para que pares
/// adyacentes mantengan al menos `min_sep_deg` de separación, **sin
/// que ningún glyph se aleje más de `max_shift_deg` de su posición
/// real**. La acotación es clave para evitar que un cluster denso
/// "empuje" a planetas que estaban lejos.
///
/// Algoritmo: iteramos hasta 80 veces; en cada pasada re-ordenamos
/// los displays para mantener el orden circular, y en cada par
/// adyacente que esté muy cerca acumulamos fuerzas en sentidos
/// opuestos. Aplicamos las fuerzas con `damping = 0.6` y clampeamos
/// cada display al rango `[raw[i] - max_shift, raw[i] + max_shift]`.
/// Si el cluster es tan denso que el clamp impide alcanzar el
/// `min_sep`, el residual queda alto y el caller encoge los discos.
///
/// Devuelve `(displays, residual)` con `residual ∈ [0, 1]` =
/// fracción de presión no resuelta tras el clamp.
pub fn spread_angles(
    angles_deg: &[f32],
    min_sep_deg: f32,
    max_shift_deg: f32,
) -> (Vec<f32>, f32) {
    let n = angles_deg.len();
    if n <= 1 {
        return (angles_deg.to_vec(), 0.0);
    }
    if (n as f32) * min_sep_deg >= 360.0 {
        return (angles_deg.to_vec(), 1.0);
    }
    let raw: Vec<f32> = angles_deg.iter().map(|a| a.rem_euclid(360.0)).collect();
    let mut displays: Vec<f32> = raw.clone();
    let mut last_residual = 0.0_f32;

    let clamp_to_raw = |display: f32, raw: f32, max_shift: f32| -> f32 {
        let mut delta = display - raw;
        if delta > 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
        let clamped = delta.clamp(-max_shift, max_shift);
        (raw + clamped).rem_euclid(360.0)
    };

    let damping: f32 = 0.6;
    for _ in 0..80 {
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            displays[a]
                .partial_cmp(&displays[b])
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let mut forces = vec![0.0_f32; n];
        let mut max_residual: f32 = 0.0;
        for k in 0..n {
            let i = order[k];
            let j = order[(k + 1) % n];
            let diff = (displays[j] - displays[i]).rem_euclid(360.0);
            if diff < min_sep_deg {
                let push = (min_sep_deg - diff) / 2.0;
                forces[i] -= push;
                forces[j] += push;
                let r = (min_sep_deg - diff) / min_sep_deg;
                if r > max_residual {
                    max_residual = r;
                }
            }
        }
        for i in 0..n {
            let stepped = (displays[i] + forces[i] * damping).rem_euclid(360.0);
            displays[i] = clamp_to_raw(stepped, raw[i], max_shift_deg);
        }
        last_residual = max_residual;
        if max_residual < 0.001 {
            break;
        }
    }

    (displays, last_residual)
}

/// Detecta clusters de longitudes angularmente cercanas. Dos
/// elementos están en el mismo cluster si su separación circular es
/// menor a `threshold_deg`. Devuelve los índices originales
/// agrupados; cada Vec interno representa un cluster (incluso si
/// es de tamaño 1). Cluster con wrap-around (último→primero) se
/// fusionan correctamente.
pub fn find_clusters(angles_deg: &[f32], threshold_deg: f32) -> Vec<Vec<usize>> {
    let n = angles_deg.len();
    if n == 0 {
        return Vec::new();
    }
    let mut idxed: Vec<(usize, f32)> = angles_deg
        .iter()
        .copied()
        .map(|a| a.rem_euclid(360.0))
        .enumerate()
        .collect();
    idxed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal));
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = vec![idxed[0].0];
    let mut last = idxed[0].1;
    for (idx, a) in idxed.iter().skip(1).copied() {
        if (a - last) < threshold_deg {
            cur.push(idx);
        } else {
            clusters.push(core::mem::take(&mut cur));
            cur.push(idx);
        }
        last = a;
    }
    clusters.push(cur);
    if clusters.len() >= 2 {
        let first_a = angles_deg[clusters[0][0]].rem_euclid(360.0);
        let last_a = angles_deg[*clusters.last().unwrap().last().unwrap()].rem_euclid(360.0);
        let wrap_diff = 360.0 - last_a + first_a;
        if wrap_diff < threshold_deg {
            let mut tail = clusters.pop().unwrap();
            tail.extend(clusters[0].iter().copied());
            clusters[0] = tail;
        }
    }
    clusters
}

// =====================================================================
// Coord formatter
// =====================================================================

/// Formato compacto con precisión de minutos: `"DD°MM'<Sg>"` con
/// `<Sg>` = código alfabético del signo (`Ar`/`Ta`/`Ge`/…). Ej:
/// 14.93° → `"14°56'Ar"`.
///
/// **Por qué letras y no `♈♉♊…`** en el coord label: los glyphs
/// astrológicos del dial principal los dibujamos como geometría
/// (`cosmos_render::glyphs`) — para los signos dentro del texto del
/// coord eso no es práctico (sería embeber paths SVG en texto), así
/// que usamos el código de 2 letras. El símbolo grande del signo
/// queda en el dial; el coord label le agrega precisión numérica.
pub fn format_coord_compact(deg: f32) -> String {
    let normalized = deg.rem_euclid(360.0);
    let total_minutes = (normalized * 60.0).round() as i64;
    let total_minutes = total_minutes.rem_euclid(360 * 60);
    let sign_idx = (total_minutes / (30 * 60)) as usize % 12;
    let within_sign = total_minutes - (sign_idx as i64) * 30 * 60;
    let deg_int = (within_sign / 60) as i32;
    let minutes = (within_sign % 60) as i32;
    let sign_code = match sign_idx {
        0 => "Ar",
        1 => "Ta",
        2 => "Ge",
        3 => "Cn",
        4 => "Le",
        5 => "Vi",
        6 => "Li",
        7 => "Sc",
        8 => "Sg",
        9 => "Cp",
        10 => "Aq",
        _ => "Pi",
    };
    format!("{}°{:02}'{}", deg_int, minutes, sign_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_min_sep(displays: &[f32], min_sep: f32) {
        let n = displays.len();
        let mut sorted = displays.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let tol = min_sep * 0.02;
        for i in 0..n {
            let nxt = (i + 1) % n;
            let diff = (sorted[nxt] - sorted[i]).rem_euclid(360.0);
            assert!(
                diff + tol >= min_sep,
                "vecinos {} y {} a {}° (mínimo {})",
                sorted[i],
                sorted[nxt],
                diff,
                min_sep
            );
        }
    }

    #[test]
    fn spread_empty_and_single_unchanged() {
        let (r, residual) = spread_angles(&[], 10.0, 30.0);
        assert!(r.is_empty());
        assert_eq!(residual, 0.0);
        let (r, residual) = spread_angles(&[42.0], 10.0, 30.0);
        assert_eq!(r, vec![42.0]);
        assert_eq!(residual, 0.0);
    }

    #[test]
    fn spread_spaced_input_left_alone() {
        let input = vec![0.0, 30.0, 90.0, 200.0];
        let (out, residual) = spread_angles(&input, 10.0, 30.0);
        assert!(residual < 0.001);
        for (a, b) in input.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1e-3, "{} vs {}", a, b);
        }
    }

    #[test]
    fn spread_tight_cluster_gets_spread() {
        let input = vec![100.0, 101.0, 102.0];
        let (out, residual) = spread_angles(&input, 10.0, 30.0);
        assert!(residual < 0.05, "residual {}", residual);
        assert_min_sep(&out, 10.0);
    }

    #[test]
    fn spread_shift_is_bounded() {
        let input = vec![100.0, 101.0];
        let (out, _) = spread_angles(&input, 10.0, 2.0);
        for (raw, disp) in input.iter().zip(out.iter()) {
            let mut delta = (disp - raw).abs();
            if delta > 180.0 {
                delta = 360.0 - delta;
            }
            assert!(delta <= 2.0 + 0.01, "shift {} > 2°", delta);
        }
    }

    #[test]
    fn spread_distant_planet_unaffected_by_dense_cluster() {
        let input = vec![100.0, 100.5, 101.0, 200.0];
        let (out, _) = spread_angles(&input, 10.0, 10.0);
        let mut delta = (out[3] - 200.0).abs();
        if delta > 180.0 {
            delta = 360.0 - delta;
        }
        assert!(delta < 5.0, "planeta lejano se movió {}°", delta);
    }

    #[test]
    fn coord_zero_aries() {
        assert_eq!(format_coord_compact(0.0), "0°00'Ar");
    }

    #[test]
    fn coord_fourteen_fiftysix_aries() {
        assert_eq!(format_coord_compact(14.933_3), "14°56'Ar");
    }

    #[test]
    fn coord_rollover_to_taurus() {
        assert_eq!(format_coord_compact(29.9995), "0°00'Ta");
    }

    #[test]
    fn coord_negative_wraps() {
        assert_eq!(format_coord_compact(-10.0), "20°00'Pi");
    }

    #[test]
    fn polar_to_screen_asc_on_left() {
        // Si la longitud = asc, el punto cae a las 9 (x = -radius, y = 0).
        let (x, y) = polar_to_screen(120.0, 120.0, 0.0, 100.0);
        assert!((x - (-100.0)).abs() < 1e-3, "x={}", x);
        assert!(y.abs() < 1e-3, "y={}", y);
    }
}
