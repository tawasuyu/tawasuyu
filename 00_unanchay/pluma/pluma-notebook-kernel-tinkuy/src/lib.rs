//! `pluma-notebook-kernel-tinkuy` — kernel de notebook que simula partículas
//! Lennard-Jones con `tinkuy-core` y devuelve el visor rasterizado como PNG
//! más observables como texto.
//!
//! Cierra E5 del roadmap tinkuy: una celda notebook ahora puede correr la
//! misma física que el demo Llimphi y mostrar su estado final como imagen
//! dentro del DAG reactivo de `pluma-notebook-exec::run_from`.
//!
//! ## Lenguaje reconocido
//!
//! `tinkuy-lj`. El source es un bloque de líneas `key = value`. Comentarios
//! con `#` o `//` se ignoran; líneas vacías también. Todas las keys son
//! opcionales — defaults equivalentes al demo `tinkuy-llimphi`:
//!
//! ```text
//! steps      = 200         # pasos de Velocity-Verlet
//! side       = 4           # lattice cúbico SIDE³ partículas (cap a 12)
//! dt         = 0.005       # paso de integración (reducido)
//! temp_init  = 0.5         # temperatura inicial (reducida)
//! sigma      = 1.0         # σ del potencial LJ
//! epsilon    = 1.0         # ε del potencial LJ
//! cutoff     = 2.5         # cutoff del potencial (×σ)
//! seed       = 13369344    # semilla SplitMix64 del PRNG inicial
//! width      = 480         # ancho del PNG resultante (px, cap 1024)
//! height     = 360         # alto del PNG resultante (px, cap 1024)
//! ```
//!
//! Lenguajes distintos a `tinkuy-lj` devuelven `KernelError::Runtime` con
//! mensaje claro — alineado con el contrato de los otros kernels (LLM, WASM).
//!
//! ## Salida
//!
//! - `KernelOutput::stdout`: bloque de observables en texto (step, t, KE, T,
//!   |p|, CID).
//! - `KernelOutput::value`: el CID completo en hex (canon Akasha/AoE).
//! - `KernelOutput::payload`: `OutputPayload::Image { mime: "image/png", … }`
//!   con un render axonométrico del estado final (caja + partículas en
//!   gradiente cold→hot por |v|).

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_notebook_core::cell::{CellOutput, OutputPayload};
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};

use tinkuy_core::{
    kinetic_energy, reflect_walls, temperature, total_momentum, velocity_verlet_step, Grid3D,
    IntegratorParams, Outbox, Snapshot, World,
};
use tinkuy_forces::{clear_accelerations, lennard_jones, LjParams};

/// Kernel notebook tinkuy. Estado vacío — todo lo necesario para una
/// ejecución viaja en el `source` de la celda.
#[derive(Debug, Clone, Default)]
pub struct TinkuyKernel;

impl TinkuyKernel {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Kernel for TinkuyKernel {
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        if language != "tinkuy-lj" {
            return Err(KernelError::Runtime(format!(
                "TinkuyKernel no maneja el lenguaje '{language}' (se esperaba 'tinkuy-lj')"
            )));
        }
        let params = Params::parse(source).map_err(KernelError::Runtime)?;
        let outcome = run_sim(&params).map_err(KernelError::Runtime)?;
        Ok(CellOutput {
            stdout: outcome.stdout.clone(),
            value: Some(outcome.cid_hex.clone()),
            payload: OutputPayload::Image {
                width: params.width,
                height: params.height,
                mime: "image/png".into(),
                bytes: outcome.png,
            },
        })
    }
}

// ─── Parámetros de la celda ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct Params {
    steps: usize,
    side: usize,
    dt: f32,
    temp_init: f32,
    sigma: f32,
    epsilon: f32,
    cutoff: f32,
    seed: u64,
    width: u32,
    height: u32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            steps: 200,
            side: 4,
            dt: 0.005,
            temp_init: 0.5,
            sigma: 1.0,
            epsilon: 1.0,
            cutoff: 2.5,
            seed: 0x00C0_FFEE,
            width: 480,
            height: 360,
        }
    }
}

impl Params {
    fn parse(source: &str) -> Result<Self, String> {
        let mut p = Params::default();
        for (n_linea, raw) in source.lines().enumerate() {
            let linea = strip_comment(raw).trim();
            if linea.is_empty() {
                continue;
            }
            let (k, v) = linea
                .split_once('=')
                .ok_or_else(|| format!("línea {}: se esperaba 'clave = valor'", n_linea + 1))?;
            let k = k.trim();
            let v = v.trim();
            match k {
                "steps" => p.steps = parse_usize(k, v)?,
                "side" => p.side = parse_usize(k, v)?,
                "dt" => p.dt = parse_f32(k, v)?,
                "temp_init" => p.temp_init = parse_f32(k, v)?,
                "sigma" => p.sigma = parse_f32(k, v)?,
                "epsilon" => p.epsilon = parse_f32(k, v)?,
                "cutoff" => p.cutoff = parse_f32(k, v)?,
                "seed" => p.seed = parse_u64(k, v)?,
                "width" => p.width = parse_u32(k, v)?,
                "height" => p.height = parse_u32(k, v)?,
                other => return Err(format!("clave desconocida '{}'", other)),
            }
        }
        // Caps defensivos — el kernel corre adentro de una request síncrona
        // de un notebook; no es lugar para simulaciones masivas. side=12 son
        // 1728 partículas, ya por encima de lo razonable para un PNG estático.
        if p.side == 0 || p.side > 12 {
            return Err(format!("'side' fuera de rango (1..=12): {}", p.side));
        }
        if p.steps > 10_000 {
            return Err(format!("'steps' > 10000 — usá el demo Llimphi: {}", p.steps));
        }
        if p.width == 0 || p.width > 1024 || p.height == 0 || p.height > 1024 {
            return Err(format!(
                "'width'/'height' fuera de rango (1..=1024): {}x{}",
                p.width, p.height
            ));
        }
        if !(p.dt > 0.0 && p.dt < 1.0) {
            return Err(format!("'dt' fuera de rango (0, 1): {}", p.dt));
        }
        if !(p.cutoff > 0.0) {
            return Err(format!("'cutoff' debe ser > 0: {}", p.cutoff));
        }
        Ok(p)
    }
}

fn strip_comment(s: &str) -> &str {
    if let Some(idx) = s.find('#') {
        return &s[..idx];
    }
    if let Some(idx) = s.find("//") {
        return &s[..idx];
    }
    s
}

fn parse_usize(k: &str, v: &str) -> Result<usize, String> {
    v.parse::<usize>().map_err(|_| format!("'{}': se esperaba entero ≥ 0, vino '{}'", k, v))
}
fn parse_u32(k: &str, v: &str) -> Result<u32, String> {
    v.parse::<u32>().map_err(|_| format!("'{}': se esperaba u32, vino '{}'", k, v))
}
fn parse_u64(k: &str, v: &str) -> Result<u64, String> {
    if let Some(hex) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16)
            .map_err(|_| format!("'{}': hex inválido '{}'", k, v));
    }
    v.parse::<u64>().map_err(|_| format!("'{}': se esperaba u64, vino '{}'", k, v))
}
fn parse_f32(k: &str, v: &str) -> Result<f32, String> {
    v.parse::<f32>().map_err(|_| format!("'{}': se esperaba float, vino '{}'", k, v))
}

// ─── Simulación + observables ────────────────────────────────────────────────

struct Outcome {
    stdout: String,
    png: Vec<u8>,
    cid_hex: String,
}

fn run_sim(p: &Params) -> Result<Outcome, String> {
    let n = p.side * p.side * p.side;
    let spacing = 1.5 * p.sigma;
    let l = p.side as f32 * spacing + p.cutoff;
    let bmin = [0.0_f32; 3];
    let bmax = [l, l, l];

    let mut world = World::with_capacity(n);
    let mut rng = SplitMix64::new(p.seed);
    let vscale = p.temp_init.sqrt();
    let half = spacing * 0.5;
    for kk in 0..p.side {
        for jj in 0..p.side {
            for ii in 0..p.side {
                let x = ii as f32 * spacing + half + p.cutoff * 0.5;
                let y = jj as f32 * spacing + half + p.cutoff * 0.5;
                let z = kk as f32 * spacing + half + p.cutoff * 0.5;
                world.spawn(
                    [x, y, z],
                    [
                        rng.next_centered() * vscale,
                        rng.next_centered() * vscale,
                        rng.next_centered() * vscale,
                    ],
                    1.0,
                    0.0,
                );
            }
        }
    }
    // Σp = 0 al arranque — igual que el demo.
    let [px, py, pz] = total_momentum(&world);
    let m_total = n as f64;
    let dvx = (px / m_total) as f32;
    let dvy = (py / m_total) as f32;
    let dvz = (pz / m_total) as f32;
    for i in 0..n {
        world.vxs.0[i] -= dvx;
        world.vys.0[i] -= dvy;
        world.vzs.0[i] -= dvz;
    }

    let dims_x = ((l / p.cutoff).ceil() as u32).max(3);
    let mut grid = Grid3D::new(bmin, p.cutoff, [dims_x; 3], n);
    grid.rebuild(&world);

    let params = IntegratorParams { dt: p.dt, bounds_min: bmin, bounds_max: bmax };
    let mut outboxes = vec![Outbox::default()];
    let lj = LjParams { epsilon: p.epsilon, sigma: p.sigma, cutoff: p.cutoff };

    for _ in 0..p.steps {
        velocity_verlet_step(&mut world, &mut grid, &params, &mut outboxes, |w, g| {
            clear_accelerations(w);
            lennard_jones(w, g, &lj);
        });
        reflect_walls(&mut world, bmin, bmax);
    }

    // Observables finales + snapshot canónico.
    let ke = kinetic_energy(&world);
    let temp = temperature(&world, 1.0);
    let [px, py, pz] = total_momentum(&world);
    let p_mag = (px * px + py * py + pz * pz).sqrt();
    let snap = Snapshot::capture(&world);
    let cid_hex = hex32(&snap.cid);

    let stdout = format!(
        "step  = {}\n\
         t     = {:.4}\n\
         N     = {}\n\
         KE    = {:.6}\n\
         T     = {:.4}\n\
         |p|   = {:.3e}\n\
         CID   = {}\n",
        p.steps,
        p.steps as f64 * p.dt as f64,
        n,
        ke,
        temp,
        p_mag,
        cid_hex,
    );

    let png = render_png(&world, bmin, bmax, p.width, p.height)?;
    Ok(Outcome { stdout, png, cid_hex })
}

// ─── PNG headless ────────────────────────────────────────────────────────────

/// Proyección axonométrica fija — mismo coeficiente que el visor Llimphi.
/// Inline para que el kernel no dependa de la UI.
#[inline]
fn project(x: f32, y: f32, z: f32) -> (f32, f32) {
    (x + z * 0.6, y + z * 0.4)
}

fn render_png(
    world: &World,
    bmin: [f32; 3],
    bmax: [f32; 3],
    w: u32,
    h: u32,
) -> Result<Vec<u8>, String> {
    let mut raster = Raster::new(w, h, [22, 26, 36, 255]);
    let pad = 18.0_f32;
    let avail_w = (w as f32 - 2.0 * pad).max(1.0);
    let avail_h = (h as f32 - 2.0 * pad).max(1.0);

    // Bbox proyectada de la caja sim (8 corners).
    let mut umin = f32::INFINITY;
    let mut umax = f32::NEG_INFINITY;
    let mut vmin = f32::INFINITY;
    let mut vmax = f32::NEG_INFINITY;
    for &cx in &[bmin[0], bmax[0]] {
        for &cy in &[bmin[1], bmax[1]] {
            for &cz in &[bmin[2], bmax[2]] {
                let (u, v) = project(cx, cy, cz);
                umin = umin.min(u);
                umax = umax.max(u);
                vmin = vmin.min(v);
                vmax = vmax.max(v);
            }
        }
    }
    let span_u = (umax - umin).max(1e-6);
    let span_v = (vmax - vmin).max(1e-6);
    let scale = (avail_w / span_u).min(avail_h / span_v);
    let proj_w = span_u * scale;
    let proj_h = span_v * scale;
    let off_x = pad + (avail_w - proj_w) * 0.5;
    let off_y = pad + (avail_h - proj_h) * 0.5;
    let map_uv = |u: f32, v: f32| -> (i32, i32) {
        let cx = off_x + (u - umin) * scale;
        let cy = off_y + (vmax - v) * scale;
        (cx as i32, cy as i32)
    };

    // Wireframe — 8 corners, 12 aristas.
    let corners = [
        (bmin[0], bmin[1], bmin[2]),
        (bmax[0], bmin[1], bmin[2]),
        (bmax[0], bmax[1], bmin[2]),
        (bmin[0], bmax[1], bmin[2]),
        (bmin[0], bmin[1], bmax[2]),
        (bmax[0], bmin[1], bmax[2]),
        (bmax[0], bmax[1], bmax[2]),
        (bmin[0], bmax[1], bmax[2]),
    ];
    let mut cps = [(0_i32, 0_i32); 8];
    for (i, &(x, y, z)) in corners.iter().enumerate() {
        let (u, v) = project(x, y, z);
        cps[i] = map_uv(u, v);
    }
    let edges = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];
    let edge_color = [46, 54, 70, 255]; // theme.border (dark)
    for &(a, b) in &edges {
        raster.line(cps[a].0, cps[a].1, cps[b].0, cps[b].1, edge_color);
    }

    // Partículas — back-to-front por z+0.3·x, color cold→hot por |v|.
    let n = world.len();
    if n > 0 {
        let xs = &world.xs.0[..n];
        let ys = &world.ys.0[..n];
        let zs = &world.zs.0[..n];
        let vxs = &world.vxs.0[..n];
        let vys = &world.vys.0[..n];
        let vzs = &world.vzs.0[..n];

        let mut vmax_sq = 0.0_f32;
        for i in 0..n {
            let v2 = vxs[i] * vxs[i] + vys[i] * vys[i] + vzs[i] * vzs[i];
            if v2 > vmax_sq {
                vmax_sq = v2;
            }
        }
        let v_max = vmax_sq.sqrt().max(1e-6);

        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_by(|&a, &b| {
            let da = zs[a as usize] + xs[a as usize] * 0.3;
            let db = zs[b as usize] + xs[b as usize] * 0.3;
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });

        let cold = [80, 160, 240, 255];
        let hot = [240, 110, 60, 255];
        // Radio fijo: escala con la diagonal de la caja para no quedar
        // microscópico en imágenes grandes ni gigante en chicas.
        let radius = ((scale * 0.06).clamp(2.0, 6.0)) as i32;
        for &idx in &order {
            let i = idx as usize;
            let (u, v) = project(xs[i], ys[i], zs[i]);
            let (cx, cy) = map_uv(u, v);
            let spd = (vxs[i] * vxs[i] + vys[i] * vys[i] + vzs[i] * vzs[i]).sqrt();
            let t = (spd / v_max).clamp(0.0, 1.0);
            let col = lerp_rgba(cold, hot, t);
            raster.disc(cx, cy, radius, col);
        }
    }

    raster.into_png()
}

struct Raster {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Raster {
    fn new(w: u32, h: u32, bg: [u8; 4]) -> Self {
        let mut px = Vec::with_capacity((w as usize) * (h as usize) * 4);
        for _ in 0..(w as usize * h as usize) {
            px.extend_from_slice(&bg);
        }
        Self { w, h, px }
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let off = (y as usize * self.w as usize + x as usize) * 4;
        self.px[off..off + 4].copy_from_slice(&rgba);
    }

    /// Bresenham clásico — todo entero, sin allocs. Clip implícito en `put`.
    fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, rgba: [u8; 4]) {
        let mut x = x0;
        let mut y = y0;
        let dx = (x1 - x0).abs();
        let sx: i32 = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy: i32 = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put(x, y, rgba);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Disco relleno por escaneo de filas. Trivial; radii pequeños (<10 px).
    fn disc(&mut self, cx: i32, cy: i32, r: i32, rgba: [u8; 4]) {
        if r <= 0 {
            self.put(cx, cy, rgba);
            return;
        }
        let r2 = r * r;
        for dy in -r..=r {
            let dx_max_sq = r2 - dy * dy;
            if dx_max_sq < 0 {
                continue;
            }
            let dx_max = (dx_max_sq as f32).sqrt() as i32;
            for dx in -dx_max..=dx_max {
                self.put(cx + dx, cy + dy, rgba);
            }
        }
    }

    fn into_png(self) -> Result<Vec<u8>, String> {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, self.w, self.h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().map_err(|e| format!("png header: {e}"))?;
            writer
                .write_image_data(&self.px)
                .map_err(|e| format!("png data: {e}"))?;
        }
        Ok(buf)
    }
}

#[inline]
fn lerp_rgba(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let lerp = |x: u8, y: u8| ((x as f32) * (1.0 - t) + (y as f32) * t).round() as u8;
    [
        lerp(a[0], b[0]),
        lerp(a[1], b[1]),
        lerp(a[2], b[2]),
        lerp(a[3], b[3]),
    ]
}

fn hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ─── PRNG inline ─────────────────────────────────────────────────────────────

struct SplitMix64(u64);
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn next_centered(&mut self) -> f32 {
        let bits = self.next_u64();
        (bits as i64 as f64 / i64::MAX as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lenguaje_desconocido_devuelve_runtime_error() {
        let k = TinkuyKernel::new();
        let err = k.execute("steps=10", "python").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("no maneja"));
    }

    #[tokio::test]
    async fn celda_vacia_produce_png_con_defaults() {
        let k = TinkuyKernel::new();
        let out = k.execute("", "tinkuy-lj").await.unwrap();
        match out.payload {
            OutputPayload::Image { width, height, ref mime, ref bytes } => {
                assert_eq!(width, 480);
                assert_eq!(height, 360);
                assert_eq!(mime, "image/png");
                // Firma PNG: 89 50 4E 47 0D 0A 1A 0A.
                assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            }
            other => panic!("esperaba Image, vino {:?}", other),
        }
        assert!(out.stdout.contains("CID"));
        // El value de la celda es el CID hex (64 chars).
        assert_eq!(out.value.as_deref().unwrap().len(), 64);
    }

    #[tokio::test]
    async fn parsea_keys_con_comentarios_y_hex_seed() {
        let src = "\
            # config mínima\n\
            steps = 8 // pocos pasos para test rápido\n\
            side = 3\n\
            seed = 0xC0FFEE\n\
            width = 200\n\
            height = 150\n\
        ";
        let k = TinkuyKernel::new();
        let out = k.execute(src, "tinkuy-lj").await.unwrap();
        match out.payload {
            OutputPayload::Image { width, height, .. } => {
                assert_eq!(width, 200);
                assert_eq!(height, 150);
            }
            _ => panic!("esperaba Image"),
        }
    }

    #[tokio::test]
    async fn determinismo_misma_seed_mismo_cid() {
        let src = "steps = 32\nside = 3\nseed = 12345\nwidth=120\nheight=90\n";
        let k = TinkuyKernel::new();
        let a = k.execute(src, "tinkuy-lj").await.unwrap();
        let b = k.execute(src, "tinkuy-lj").await.unwrap();
        assert_eq!(a.value, b.value, "misma seed → mismo CID final");
    }

    #[tokio::test]
    async fn distintas_seeds_distinto_cid() {
        let k = TinkuyKernel::new();
        let a = k
            .execute("steps=32\nside=3\nseed=1\nwidth=120\nheight=90\n", "tinkuy-lj")
            .await
            .unwrap();
        let b = k
            .execute("steps=32\nside=3\nseed=2\nwidth=120\nheight=90\n", "tinkuy-lj")
            .await
            .unwrap();
        assert_ne!(a.value, b.value);
    }

    #[tokio::test]
    async fn clave_desconocida_falla_explicito() {
        let k = TinkuyKernel::new();
        let err = k
            .execute("foo = 42", "tinkuy-lj")
            .await
            .unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("desconocida"), "msg = {}", msg);
    }

    #[tokio::test]
    async fn side_excesivo_falla() {
        let k = TinkuyKernel::new();
        let err = k.execute("side = 50", "tinkuy-lj").await.unwrap_err();
        let KernelError::Runtime(msg) = err;
        assert!(msg.contains("side"));
    }

    #[tokio::test]
    async fn integracion_run_from_con_celda_tinkuy() {
        use pluma_notebook_core::{CellKind, CellState, Notebook};
        use pluma_notebook_exec::run_from;

        let mut nb = Notebook::new();
        let a = nb.push(
            CellKind::Code { language: "tinkuy-lj".into() },
            "steps = 16\nside = 3\nwidth=120\nheight=90\n",
        );
        nb.set_state(a, CellState::Fresh);
        let k = TinkuyKernel::new();
        let report = run_from(&mut nb, &k, a).await.unwrap();
        assert_eq!(report.executed, vec![a]);
        // El CellOutput quedó persistido con un PNG.
        let cell = nb.cell(a).unwrap();
        let out = cell.last_output.as_ref().expect("debería tener salida");
        assert!(matches!(&out.payload, OutputPayload::Image { mime, .. } if mime == "image/png"));
    }
}
