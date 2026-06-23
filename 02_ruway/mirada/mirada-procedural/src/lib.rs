//! `mirada-procedural` — fondos de escritorio **generados**: en vez de una
//! imagen, el compositor puede pintar una geometría procedural a partir de una
//! **paleta** de colores. Todo es CPU puro (sin GPU/shaders, sin bytes
//! embebidos) y **determinista** (mismo `(patrón, paleta, tamaño, seed)` → mismo
//! buffer), así se puede certificar headless a PNG y reproducir en cada monitor.
//!
//! Los patrones ([`Pattern`]) son geometrías tipográficas/abstractas pensadas
//! para fondo: rayas diagonales, anillos concéntricos, ondas, malla low-poly,
//! celdas Voronoi y composición Bauhaus. Cada uno usa la paleta como cantera de
//! color; los detalles (cuántas bandas, qué formas) salen de un PRNG sembrado
//! por `seed` — cambiar `seed` da otra variante del mismo patrón.
//!
//! Salida: `Vec<u8>` RGBA (`[R,G,B,A]` por píxel, alfa 255). El consumidor que
//! necesite BGRA (DRM `Argb8888` little-endian) intercambia R↔B al copiar.

#![forbid(unsafe_code)]

/// Los patrones procedurales disponibles. El orden de [`Pattern::ALL`] es el que
/// muestra el panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Bandas diagonales que ciclan la paleta.
    Stripes,
    /// Anillos concéntricos desde el centro.
    Rings,
    /// Interferencia de ondas seno mapeada a un gradiente de la paleta.
    Waves,
    /// Malla low-poly: triángulos planos coloreados por un campo suave.
    LowPoly,
    /// Celdas Voronoi orgánicas, cada una de un color de la paleta.
    Voronoi,
    /// Composición Bauhaus: fondo + formas geométricas dispersas (círculos,
    /// barras, triángulos, semicírculos).
    Bauhaus,
}

impl Pattern {
    /// Todos los patrones, en orden de presentación.
    pub const ALL: [Pattern; 6] = [
        Pattern::Stripes,
        Pattern::Rings,
        Pattern::Waves,
        Pattern::LowPoly,
        Pattern::Voronoi,
        Pattern::Bauhaus,
    ];

    /// Identificador estable kebab-case (para config RON / persistir la elección).
    pub fn slug(self) -> &'static str {
        match self {
            Pattern::Stripes => "stripes",
            Pattern::Rings => "rings",
            Pattern::Waves => "waves",
            Pattern::LowPoly => "low-poly",
            Pattern::Voronoi => "voronoi",
            Pattern::Bauhaus => "bauhaus",
        }
    }

    /// Nombre legible (español) para la UI.
    pub fn label(self) -> &'static str {
        match self {
            Pattern::Stripes => "Rayas",
            Pattern::Rings => "Anillos",
            Pattern::Waves => "Ondas",
            Pattern::LowPoly => "Low-poly",
            Pattern::Voronoi => "Voronoi",
            Pattern::Bauhaus => "Bauhaus",
        }
    }

    /// Parsea desde el slug. `None` si no matchea.
    pub fn from_slug(s: &str) -> Option<Pattern> {
        Pattern::ALL.into_iter().find(|p| p.slug() == s)
    }
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern::Waves
    }
}

/// Paleta por defecto (noche → púrpura → azul, en la línea del gradiente sobrio
/// que ya traía mirada) cuando el usuario no define colores.
pub fn default_palette() -> Vec<[u8; 3]> {
    vec![
        [0x0a, 0x0e, 0x22],
        [0x1b, 0x1a, 0x3e],
        [0x2a, 0x1c, 0x4a],
        [0x3a, 0x2c, 0x6a],
        [0x52, 0x46, 0x9a],
    ]
}

/// PRNG determinista mínimo (SplitMix64). No usamos `rand` para no arrastrar
/// dependencias ni `Math.random` — el fondo debe ser idéntico en cada monitor y
/// en cada arranque.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Flotante en `[0, 1)`.
    fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Entero en `[0, n)` (n > 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Mezcla lineal de dos colores RGB (`t` en `0..1`).
fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t).round() as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t).round() as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t).round() as u8,
    ]
}

/// Muestrea la paleta como un **gradiente continuo** en `t` (`0..1`): reparte
/// los colores equiespaciados e interpola entre los dos más cercanos.
fn sample_gradient(pal: &[[u8; 3]], t: f32) -> [u8; 3] {
    if pal.is_empty() {
        return [0, 0, 0];
    }
    if pal.len() == 1 {
        return pal[0];
    }
    let t = t.clamp(0.0, 1.0);
    let seg = (pal.len() - 1) as f32;
    let pos = t * seg;
    let i = (pos.floor() as usize).min(pal.len() - 2);
    lerp_rgb(pal[i], pal[i + 1], pos - i as f32)
}

/// Genera un fondo procedural `w×h` y devuelve sus bytes **RGBA** (`[R,G,B,A]`,
/// alfa 255). Paleta vacía → [`default_palette`].
pub fn generate_rgba(pattern: Pattern, palette: &[[u8; 3]], w: u32, h: u32, seed: u64) -> Vec<u8> {
    let w = w.max(1) as usize;
    let h = h.max(1) as usize;
    let owned;
    let pal: &[[u8; 3]] = if palette.is_empty() {
        owned = default_palette();
        &owned
    } else {
        palette
    };
    let mut px = vec![0u8; w * h * 4];
    match pattern {
        Pattern::Stripes => stripes(&mut px, w, h, pal, seed),
        Pattern::Rings => rings(&mut px, w, h, pal, seed),
        Pattern::Waves => waves(&mut px, w, h, pal, seed),
        Pattern::LowPoly => low_poly(&mut px, w, h, pal, seed),
        Pattern::Voronoi => voronoi(&mut px, w, h, pal, seed),
        Pattern::Bauhaus => bauhaus(&mut px, w, h, pal, seed),
    }
    px
}

/// Genera el fondo en formato **BGRA** (`[B,G,R,A]`) — el que pide DRM
/// `Argb8888` (little-endian). Conveniencia para el compositor.
pub fn generate_bgra(pattern: Pattern, palette: &[[u8; 3]], w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut px = generate_rgba(pattern, palette, w, h, seed);
    for c in px.chunks_exact_mut(4) {
        c.swap(0, 2); // R↔B
    }
    px
}

#[inline]
fn put(px: &mut [u8], w: usize, x: usize, y: usize, c: [u8; 3]) {
    let i = (y * w + x) * 4;
    px[i] = c[0];
    px[i + 1] = c[1];
    px[i + 2] = c[2];
    px[i + 3] = 255;
}

// ── Patrones ────────────────────────────────────────────────────────────────

/// Bandas diagonales (45°) que ciclan la paleta, con un grosor sembrado.
fn stripes(px: &mut [u8], w: usize, h: usize, pal: &[[u8; 3]], seed: u64) {
    let mut rng = Rng::new(seed);
    let bands = (pal.len() * (2 + rng.below(3))).max(2); // 2N..4N bandas
    let period = (w + h) as f32 / bands as f32;
    for y in 0..h {
        for x in 0..w {
            let d = (x + y) as f32 / period;
            // Fase continua → gradiente suave a través de la paleta (envuelve).
            let t = d.fract();
            let idx = (d.floor() as usize) % pal.len();
            let nxt = (idx + 1) % pal.len();
            put(px, w, x, y, lerp_rgb(pal[idx], pal[nxt], t));
        }
    }
}

/// Anillos concéntricos desde un centro sembrado; el radio mapea a la paleta.
fn rings(px: &mut [u8], w: usize, h: usize, pal: &[[u8; 3]], seed: u64) {
    let mut rng = Rng::new(seed);
    let cx = w as f32 * (0.35 + 0.3 * rng.f32());
    let cy = h as f32 * (0.35 + 0.3 * rng.f32());
    let maxr = ((w * w + h * h) as f32).sqrt();
    let bands = (pal.len() * (3 + rng.below(4))).max(2);
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let r = (dx * dx + dy * dy).sqrt() / maxr * bands as f32;
            let t = r.fract();
            let idx = (r.floor() as usize) % pal.len();
            let nxt = (idx + 1) % pal.len();
            put(px, w, x, y, lerp_rgb(pal[idx], pal[nxt], t));
        }
    }
}

/// Interferencia de un par de ondas seno → escalar `0..1` → gradiente de paleta.
fn waves(px: &mut [u8], w: usize, h: usize, pal: &[[u8; 3]], seed: u64) {
    let mut rng = Rng::new(seed);
    let fx1 = 1.5 + 3.0 * rng.f32();
    let fy1 = 1.0 + 3.0 * rng.f32();
    let fx2 = 2.0 + 4.0 * rng.f32();
    let ph = rng.f32() * std::f32::consts::TAU;
    let wf = w as f32;
    let hf = h as f32;
    for y in 0..h {
        let ny = y as f32 / hf;
        for x in 0..w {
            let nx = x as f32 / wf;
            let v = (nx * fx1 * std::f32::consts::TAU + ny * fy1 * std::f32::consts::TAU).sin()
                + ((nx + ny) * fx2 * std::f32::consts::PI + ph).sin();
            let t = (v * 0.25) + 0.5; // ~[0,1]
            put(px, w, x, y, sample_gradient(pal, t));
        }
    }
}

/// Malla low-poly: grilla de celdas, cada una partida en dos triángulos por la
/// diagonal; el color sale de un campo suave (diagonal) + jitter sembrado por
/// celda, muestreado de la paleta. O(w·h).
fn low_poly(px: &mut [u8], w: usize, h: usize, pal: &[[u8; 3]], seed: u64) {
    let cols = 14usize;
    let rows = 9usize;
    let cw = (w as f32 / cols as f32).max(1.0);
    let ch = (h as f32 / rows as f32).max(1.0);
    // Pre-cálculo del color de cada (celda, triángulo): 0 = arriba-derecha,
    // 1 = abajo-izquierda de la diagonal.
    let mut color_of = vec![[0u8; 3]; cols * rows * 2];
    for cyi in 0..rows {
        for cxi in 0..cols {
            for tri in 0..2 {
                let mut rng = Rng::new(seed ^ ((cxi as u64) << 20) ^ ((cyi as u64) << 8) ^ tri as u64);
                let base = (cxi as f32 / cols as f32 + cyi as f32 / rows as f32) * 0.5;
                let jit = (rng.f32() - 0.5) * 0.18;
                color_of[(cyi * cols + cxi) * 2 + tri] = sample_gradient(pal, base + jit);
            }
        }
    }
    for y in 0..h {
        let cyi = ((y as f32 / ch) as usize).min(rows - 1);
        let fy = y as f32 / ch - cyi as f32;
        for x in 0..w {
            let cxi = ((x as f32 / cw) as usize).min(cols - 1);
            let fx = x as f32 / cw - cxi as f32;
            // Diagonal '/': por encima → triángulo 0, por debajo → 1.
            let tri = if fx + fy < 1.0 { 0 } else { 1 };
            put(px, w, x, y, color_of[(cyi * cols + cxi) * 2 + tri]);
        }
    }
}

/// Celdas Voronoi: N semillas sembradas, cada píxel toma el color de la semilla
/// más cercana (distancia euclídea). Un leve sombreado por distancia da relieve.
fn voronoi(px: &mut [u8], w: usize, h: usize, pal: &[[u8; 3]], seed: u64) {
    let mut rng = Rng::new(seed);
    let n = 22usize;
    let mut seeds: Vec<(f32, f32, [u8; 3])> = Vec::with_capacity(n);
    for _ in 0..n {
        let sx = rng.f32() * w as f32;
        let sy = rng.f32() * h as f32;
        let c = pal[rng.below(pal.len())];
        seeds.push((sx, sy, c));
    }
    let norm = ((w * w + h * h) as f32).sqrt() * 0.18;
    for y in 0..h {
        for x in 0..w {
            let (mut best, mut bestd) = (0usize, f32::MAX);
            for (i, s) in seeds.iter().enumerate() {
                let dx = x as f32 - s.0;
                let dy = y as f32 - s.1;
                let d = dx * dx + dy * dy;
                if d < bestd {
                    bestd = d;
                    best = i;
                }
            }
            // Sombreado: más oscuro cerca del borde de la celda (distancia grande).
            let shade = 1.0 - (bestd.sqrt() / norm).clamp(0.0, 0.35);
            let c = seeds[best].2;
            let c = [
                (c[0] as f32 * shade) as u8,
                (c[1] as f32 * shade) as u8,
                (c[2] as f32 * shade) as u8,
            ];
            put(px, w, x, y, c);
        }
    }
}

/// Composición Bauhaus: fondo del color más oscuro de la paleta + formas
/// geométricas grandes y dispersas (círculo, barra, triángulo, semicírculo) en
/// los demás colores. Las formas se rasterizan sobre su bounding box.
fn bauhaus(px: &mut [u8], w: usize, h: usize, pal: &[[u8; 3]], seed: u64) {
    // Fondo = color más oscuro (menor luma).
    let luma = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    let bg = *pal.iter().min_by(|a, b| luma(**a).total_cmp(&luma(**b))).unwrap();
    for y in 0..h {
        for x in 0..w {
            put(px, w, x, y, bg);
        }
    }
    let mut rng = Rng::new(seed);
    let shapes = 7 + rng.below(6);
    let unit = (w.min(h)) as f32;
    for _ in 0..shapes {
        let kind = rng.below(4);
        let c = pal[rng.below(pal.len())];
        let cx = rng.f32() * w as f32;
        let cy = rng.f32() * h as f32;
        let s = unit * (0.12 + 0.22 * rng.f32());
        match kind {
            0 => fill_circle(px, w, h, cx, cy, s, c, false, 0.0),
            1 => {
                // Barra (rect) con orientación H o V.
                let (bw, bh) = if rng.f32() < 0.5 { (s * 2.4, s * 0.5) } else { (s * 0.5, s * 2.4) };
                fill_rect(px, w, h, cx - bw * 0.5, cy - bh * 0.5, bw, bh, c);
            }
            2 => fill_triangle(px, w, h, cx, cy, s, rng.f32() * std::f32::consts::TAU, c),
            _ => fill_circle(px, w, h, cx, cy, s, c, true, rng.f32() * std::f32::consts::TAU),
        }
    }
}

fn fill_rect(px: &mut [u8], w: usize, h: usize, x0: f32, y0: f32, rw: f32, rh: f32, c: [u8; 3]) {
    let xa = x0.max(0.0) as usize;
    let ya = y0.max(0.0) as usize;
    let xb = ((x0 + rw).min(w as f32)).max(0.0) as usize;
    let yb = ((y0 + rh).min(h as f32)).max(0.0) as usize;
    for y in ya..yb {
        for x in xa..xb {
            put(px, w, x, y, c);
        }
    }
}

/// Círculo (`half=false`) o semicírculo (`half=true`, mitad según `ang`).
fn fill_circle(px: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, r: f32, c: [u8; 3], half: bool, ang: f32) {
    let xa = (cx - r).max(0.0) as usize;
    let ya = (cy - r).max(0.0) as usize;
    let xb = ((cx + r).min(w as f32)).max(0.0) as usize;
    let yb = ((cy + r).min(h as f32)).max(0.0) as usize;
    let (sa, ca) = ang.sin_cos();
    for y in ya..yb {
        for x in xa..xb {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r * r {
                // Semicírculo: el plano definido por `ang` corta el disco.
                if half && (dx * ca + dy * sa) < 0.0 {
                    continue;
                }
                put(px, w, x, y, c);
            }
        }
    }
}

/// Triángulo equilátero centrado en `(cx,cy)`, "radio" `r`, rotado `ang`.
fn fill_triangle(px: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, r: f32, ang: f32, c: [u8; 3]) {
    let mut vx = [0.0f32; 3];
    let mut vy = [0.0f32; 3];
    for k in 0..3 {
        let a = ang + k as f32 * std::f32::consts::TAU / 3.0;
        vx[k] = cx + r * a.cos();
        vy[k] = cy + r * a.sin();
    }
    let xa = vx.iter().cloned().fold(f32::MAX, f32::min).max(0.0) as usize;
    let xb = (vx.iter().cloned().fold(f32::MIN, f32::max).min(w as f32)).max(0.0) as usize;
    let ya = vy.iter().cloned().fold(f32::MAX, f32::min).max(0.0) as usize;
    let yb = (vy.iter().cloned().fold(f32::MIN, f32::max).min(h as f32)).max(0.0) as usize;
    let sign = |ax: f32, ay: f32, bx: f32, by: f32, cxp: f32, cyp: f32| {
        (ax - cxp) * (by - cyp) - (bx - cxp) * (ay - cyp)
    };
    for y in ya..yb {
        for x in xa..xb {
            let (fx, fy) = (x as f32, y as f32);
            let d1 = sign(fx, fy, vx[0], vy[0], vx[1], vy[1]);
            let d2 = sign(fx, fy, vx[1], vy[1], vx[2], vy[2]);
            let d3 = sign(fx, fy, vx[2], vy[2], vx[0], vy[0]);
            let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            if !(neg && pos) {
                put(px, w, x, y, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinista_y_opaco() {
        let pal = default_palette();
        let a = generate_rgba(Pattern::Waves, &pal, 80, 50, 7);
        let b = generate_rgba(Pattern::Waves, &pal, 80, 50, 7);
        assert_eq!(a, b, "mismo input → mismo buffer");
        assert_eq!(a.len(), 80 * 50 * 4);
        assert!(a.chunks_exact(4).all(|c| c[3] == 255), "todo opaco");
    }

    #[test]
    fn cada_patron_usa_la_paleta() {
        // Una paleta de colores MUY distintos: el fondo generado debe contener
        // varios de ellos (no quedar plano) — prueba que la geometría reparte.
        let pal = vec![[230, 40, 40], [40, 200, 60], [50, 70, 230], [240, 210, 40]];
        for p in Pattern::ALL {
            let px = generate_rgba(p, &pal, 200, 120, 3);
            let mut buckets = std::collections::HashSet::new();
            for c in px.chunks_exact(4) {
                buckets.insert((c[0] / 64, c[1] / 64, c[2] / 64));
            }
            assert!(
                buckets.len() >= 3,
                "{:?} salió casi plano ({} cubos de color)",
                p,
                buckets.len()
            );
        }
    }

    #[test]
    fn slug_roundtrip() {
        for p in Pattern::ALL {
            assert_eq!(Pattern::from_slug(p.slug()), Some(p));
        }
        assert_eq!(Pattern::from_slug("nope"), None);
    }
}
