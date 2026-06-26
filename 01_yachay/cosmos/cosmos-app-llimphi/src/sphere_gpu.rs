//! Esfera celeste 3D sobre el motor GPU **`llimphi-3d`**.
//!
//! Una escena 3D real, orbitable, con profundidad, compuesta en un pase
//! supersampleado (`PostFx`) confinado al rect del panel. Capas:
//!
//! 1. **Campo estelar de fondo** ([`SkyBackdrop`], mapeo esférico): el polvo de
//!    estrellas tenues que envuelve todo, bloqueado al mundo.
//! 2. **Mini-Tierra** ([`Renderer3d`] + [`uv_sphere`]) al centro: un globo con
//!    continentes y terminador día/noche horneados en el color del vértice
//!    (sombreado Gouraud), rodeado de un **halo de atmósfera** (glow aditivo
//!    azul, ocluido por el globo → sólo brilla el limbo).
//! 3. **Estructura como líneas** ([`Lines3d`]): eclíptica y ecuador celeste
//!    (círculos máximos continuos), rejilla de meridianos/paralelos eclípticos,
//!    espolones del zodíaco, ejes ASC/MC y **figuras de constelaciones**.
//! 4. **Cuerpos y estrellas**: núcleos opacos nítidos ([`Billboards`]) + auras
//!    luminosas aditivas ([`Glows`]) que el **bloom** infla — los astros y las
//!    estrellas brillan.
//!
//! La geometría dinámica (líneas + sprites) se arma en CPU en `view()` y se
//! sube/dibuja en `View::gpu_paint_with`. El globo es estático (se construye una
//! vez). El [`SphereGpu`] vive en el `Model` tras `Arc<Mutex<…>>`, perezoso.

use std::sync::{Arc, Mutex};

use cosmos_render::{LayerKind, Palette, RenderModel, Rgba};
use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{
    Billboard, Billboards, Camera3d, Glows, LineVertex, Lines3d, PostFx, PostFxConfig, Renderer3d,
    SkyBackdrop, SkyMapping, SkyParams, Vertex3d,
};
use llimphi_ui::llimphi_hal::wgpu;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub(crate) const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Oblicuidad de la eclíptica (J2000), para inclinar el ecuador celeste.
const OBLIQUITY_DEG: f32 = 23.439_29;

/// Radio del globo terrestre al centro (la esfera celeste tiene radio 1).
const EARTH_R: f32 = 0.16;

/// Geometría dinámica de un frame: líneas + sprites opacos + glows aditivos.
pub(crate) struct SphereGeom {
    lines: Vec<LineVertex>,
    cores: Vec<Billboard>,
    glows: Vec<Billboard>,
}

/// Estado GPU persistente. Vive en el `Model` tras `Arc<Mutex<…>>`.
pub(crate) struct SphereGpu {
    fx: PostFx,
    sky: SkyBackdrop,
    earth: Renderer3d,
    lines: Lines3d,
    cores: Billboards,
    glows: Glows,
}

/// Ranura compartida que guarda el `SphereGpu` entre frames.
pub(crate) type SphereGpuSlot = Arc<Mutex<Option<SphereGpu>>>;

/// Crea una ranura vacía para el `Model`.
pub(crate) fn slot() -> SphereGpuSlot {
    Arc::new(Mutex::new(None))
}

impl SphereGpu {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // Mini-Tierra: malla estática con continentes + día/noche horneados.
        let (everts, eindices) = earth_mesh();
        let mut earth = Renderer3d::with_mesh(device, FMT, &everts, &eindices);
        earth.lights.clear();
        earth.ambient = [1.0, 1.0, 1.0]; // color del vértice tal cual (ya sombreado)
        earth.set_model(Mat4::from_scale(Vec3::splat(EARTH_R)));

        let lines = Lines3d::new(device, FMT);

        let mut cores = Billboards::new(device, FMT);
        let (aw, ah, atlas) = dot_atlas();
        cores.set_atlas(device, queue, aw, ah, &atlas);

        let mut glows = Glows::new(device, FMT);
        let (gw, gh, gatlas) = glow_atlas();
        glows.set_atlas(device, queue, gw, gh, &gatlas);

        let mut sky = SkyBackdrop::new(device, FMT);
        let (sw, sh, field) = starfield_panorama();
        sky.set_texture(device, queue, sw, sh, &field);

        // Bloom jugado: enciende los cuerpos/estrellas brillantes con un halo,
        // sin lavar el campo estelar tenue (umbral por encima de su brillo).
        let fx = PostFx::with_config(
            device,
            FMT,
            PostFxConfig {
                supersample: 2,
                bloom_strength: 0.75,
                bloom_threshold: 0.58,
                bloom_knee: 0.30,
                bloom_radius: 2.2,
            },
        );
        Self { fx, sky, earth, lines, cores, glows }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        rect: (f32, f32, f32, f32),
        geom: &SphereGeom,
        yaw_deg: f32,
        pitch_deg: f32,
        dist: f32,
    ) {
        let (_rx, _ry, rw, rh) = rect;
        if w == 0 || h == 0 || rw < 1.0 || rh < 1.0 {
            return;
        }
        let aspect = rw / rh;
        let yaw = yaw_deg.to_radians();
        let pitch = pitch_deg.to_radians();
        let cam = Camera3d::orbit(Vec3::ZERO, yaw, pitch, dist);
        let view_proj = cam.view_proj(aspect);

        // Subir geometría del frame.
        self.lines.set_lines(device, &geom.lines);
        self.cores.set_billboards(device, &geom.cores);
        self.glows.set_glows(device, &geom.glows);
        self.earth.upload(queue, aspect, &cam);
        self.lines.upload(queue, view_proj);
        self.cores.upload(queue, aspect, &cam);
        self.glows.upload(queue, aspect, &cam);

        let fov_x = 2.0 * (aspect * (cam.fovy_rad * 0.5).tan()).atan();
        self.sky.upload(
            queue,
            &SkyParams {
                yaw,
                pitch,
                fov_x,
                aspect,
                mapping: SkyMapping::Spherical,
                ..Default::default()
            },
        );

        let ow = rw.round().max(1.0) as u32;
        let oh = rh.round().max(1.0) as u32;
        self.fx.prepare(device, queue, (ow, oh));
        let clear = wgpu::Color { r: 3.0 / 255.0, g: 4.0 / 255.0, b: 11.0 / 255.0, a: 1.0 };
        {
            let mut pass = self.fx.scene_pass(encoder, clear);
            self.sky.draw(&mut pass); // fondo (no escribe depth)
            self.earth.draw(&mut pass); // globo opaco (escribe depth)
            self.cores.draw(&mut pass); // núcleos opacos (escriben depth)
            self.lines.draw(&mut pass); // alambre (testea, no escribe)
            self.glows.draw(&mut pass); // auras aditivas (testea, no escribe)
        }
        self.fx.resolve_in(encoder, target, rect, (w, h));
    }
}

/// Punto sobre `slot`: lo crea si hace falta y dibuja.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    slot: &SphereGpuSlot,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    viewport: (u32, u32),
    rect: (f32, f32, f32, f32),
    geom: &SphereGeom,
    yaw_deg: f32,
    pitch_deg: f32,
    dist: f32,
) {
    let mut guard = match slot.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let gpu = guard.get_or_insert_with(|| SphereGpu::new(device, queue));
    gpu.draw(device, queue, encoder, target, viewport, rect, geom, yaw_deg, pitch_deg, dist);
}

// =====================================================================
// Geometría (CPU)
// =====================================================================

fn rgb(c: Rgba) -> [f32; 3] {
    [c.r, c.g, c.b]
}

/// Punto unidad sobre la eclíptica a longitud `deg` (plano XZ; polo norte
/// eclíptico = +Y). Mismo origen angular que la rueda 2D.
fn eclip(deg: f32) -> Vec3 {
    let l = deg.to_radians();
    Vec3::new(l.cos(), 0.0, l.sin())
}

/// Punto unidad a longitud + latitud eclípticas (grados).
fn eclip_latlon(lon_deg: f32, lat_deg: f32) -> Vec3 {
    let (sl, cl) = lon_deg.to_radians().sin_cos();
    let (sb, cb) = lat_deg.to_radians().sin_cos();
    Vec3::new(cb * cl, sb, cb * sl)
}

/// Dirección ecuatorial (AR, Dec en grados) llevada al marco eclíptico de la
/// GPU (polo +Y), con la **misma** inclinación que el aro del ecuador celeste.
fn eq_to_gpu(ra_deg: f32, dec_deg: f32, eps: &Mat4) -> Vec3 {
    let (sr, cr) = ra_deg.to_radians().sin_cos();
    let (sd, cd) = dec_deg.to_radians().sin_cos();
    // Marco ecuatorial: ecuador en XZ, polo +Y; AR de +X hacia +Z.
    eps.transform_point3(Vec3::new(cd * cr, sd, cd * sr))
}

/// Un sprite redondo de tamaño `size` con `tint` y alpha `a`.
fn sprite(center: Vec3, size: f32, c: [f32; 3], a: f32) -> Billboard {
    Billboard {
        center: center.to_array(),
        size: [size, size],
        uv_min: [0.0, 0.0],
        uv_max: [1.0, 1.0],
        tint: [c[0], c[1], c[2], a],
    }
}

/// Empuja una polilínea (puntos en mundo) como segmentos a `out`.
fn push_polyline(out: &mut Vec<LineVertex>, pts: &[Vec3], color: [f32; 3], alpha: f32) {
    let col = [color[0], color[1], color[2], alpha];
    for w in pts.windows(2) {
        out.push(LineVertex { pos: w[0].to_array(), color: col });
        out.push(LineVertex { pos: w[1].to_array(), color: col });
    }
}

/// Empuja un círculo (cerrado) muestreando `f(theta_deg)` por `n` pasos.
fn push_circle(
    out: &mut Vec<LineVertex>,
    n: usize,
    color: [f32; 3],
    alpha: f32,
    f: impl Fn(f32) -> Vec3,
) {
    let pts: Vec<Vec3> = (0..=n).map(|k| f(k as f32 / n as f32 * 360.0)).collect();
    push_polyline(out, &pts, color, alpha);
}

/// Construye la geometría dinámica de la esfera celeste desde el modelo.
pub(crate) fn sphere_geometry(model: &RenderModel, pal: &Palette) -> SphereGeom {
    let mut lines: Vec<LineVertex> = Vec::new();
    let mut cores: Vec<Billboard> = Vec::new();
    let mut glows: Vec<Billboard> = Vec::new();
    let r = 1.0_f32;
    let eps = Mat4::from_rotation_x(OBLIQUITY_DEG.to_radians());

    let ecl = rgb(pal.dial_ring);
    let equ = rgb(pal.uranus);
    let grid = [0.42, 0.50, 0.72]; // azul tenue para la rejilla
    let conl = [0.62, 0.72, 0.95]; // azul claro para las líneas de constelación
    let starc = if pal.is_dark { [1.0, 1.0, 1.0] } else { [0.22, 0.28, 0.42] };

    // --- Rejilla eclíptica (meridianos + paralelos): da volumen, sutil ---
    for k in 0..12 {
        let lon = k as f32 * 30.0;
        let pts: Vec<Vec3> = (0..=48)
            .map(|i| eclip_latlon(lon, -90.0 + i as f32 / 48.0 * 180.0) * r)
            .collect();
        push_polyline(&mut lines, &pts, grid, 0.16);
    }
    for &lat in &[-60.0_f32, -30.0, 30.0, 60.0] {
        push_circle(&mut lines, 96, grid, 0.16, |t| eclip_latlon(t, lat) * r);
    }

    // --- Constelaciones (figuras de líneas + estrellas en los vértices) ---
    for fig in cosmos_render::constellations_data::FIGURAS {
        for path in fig.paths {
            let pts: Vec<Vec3> = path.iter().map(|&(ra, dec)| eq_to_gpu(ra, dec, &eps) * r).collect();
            // Línea doble (dos radios próximos) para que se lea como trazo.
            push_polyline(&mut lines, &pts, conl, 0.62);
            let pts_out: Vec<Vec3> = pts.iter().map(|p| *p * 1.002).collect();
            push_polyline(&mut lines, &pts_out, conl, 0.40);
            for p in &pts {
                glows.push(sprite(*p, 0.022, starc, 0.6)); // estrella con aura
                cores.push(sprite(*p, 0.008, [1.0, 1.0, 1.0], 1.0)); // punto nítido
            }
        }
    }

    // --- Estrellas brillantes con nombre (catálogo real, por magnitud) ---
    for s in cosmos_render::sky_data::BRIGHT_STARS {
        let p = eq_to_gpu(s.ra_deg, s.dec_deg, &eps) * r;
        let sz = (0.055 - (s.mag + 1.5) * 0.006).clamp(0.016, 0.055);
        let a = (1.25 - s.mag * 0.13).clamp(0.45, 1.0);
        // Tinte levemente azulado para las calientes, cálido para las tibias.
        let tint = if s.mag < 0.5 { [0.85, 0.92, 1.0] } else { [1.0, 0.96, 0.88] };
        glows.push(sprite(p, sz * 2.6, tint, a)); // aura
        cores.push(sprite(p, sz * 0.8, [1.0, 1.0, 1.0], 1.0)); // núcleo nítido
    }

    // --- Eclíptica (aro prominente) + ecuador celeste (inclinado, más tenue),
    // dibujados como banda de 3 líneas próximas para que tengan grosor ---
    for rr in [0.996_f32, 1.0, 1.004] {
        push_circle(&mut lines, 220, ecl, 0.98, |t| eclip(t) * r * rr);
    }
    for rr in [0.997_f32, 1.0, 1.003] {
        push_circle(&mut lines, 200, equ, 0.8, |t| eps.transform_point3(eclip(t)) * r * rr);
    }

    // --- Espolones del zodíaco (cada 30°) ---
    for s in 0..12 {
        let d = s as f32 * 30.0;
        push_polyline(&mut lines, &[eclip(d) * r, eclip(d) * r * 1.07], ecl, 0.85);
    }

    // --- Polos eclípticos ---
    cores.push(sprite(Vec3::Y * r, 0.03, ecl, 1.0));
    cores.push(sprite(-Vec3::Y * r, 0.03, rgb(pal.fg_muted), 0.9));

    // --- Ángulos ASC/MC/DSC/IC (espolones resaltados + aura) ---
    let ang = rgb(pal.angle_highlight);
    for deg in [
        model.ascendant_deg,
        model.midheaven_deg,
        model.descendant_deg,
        model.imum_coeli_deg,
    ] {
        push_polyline(&mut lines, &[eclip(deg) * r * 0.9, eclip(deg) * r * 1.12], ang, 0.9);
        glows.push(sprite(eclip(deg) * r * 1.10, 0.06, ang, 0.7));
    }

    // --- Cuerpos natales: núcleo coloreado nítido + aura luminosa ---
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "natal" {
            for g in &layer.glyphs {
                let p = eclip(g.deg) * r * 1.03;
                let c = rgb(pal.planet(&g.symbol));
                glows.push(sprite(p, 0.16, c, 0.75)); // aura → bloom
                cores.push(sprite(p, 0.07, c, 1.0)); // disco sólido
            }
        }
    }
    // --- Cuerpos topocéntricos: discos chicos atenuados, levemente adentro ---
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "topocentric" {
            for g in &layer.glyphs {
                let p = eclip(g.deg) * r * 0.96;
                let c = rgb(pal.planet(&g.symbol));
                glows.push(sprite(p, 0.08, c, 0.5));
                cores.push(sprite(p, 0.045, c, 0.9));
            }
        }
    }
    // --- Estrellas fijas notables del modelo (sobre la eclíptica) ---
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::FixedStars) {
            for g in &layer.glyphs {
                let p = eclip(g.deg) * r;
                glows.push(sprite(p, 0.05, [1.0, 0.97, 0.9], 0.8));
                cores.push(sprite(p, 0.02, [1.0, 1.0, 1.0], 1.0));
            }
        }
    }

    // --- Halo de atmósfera de la mini-Tierra (glow azul, ocluido por el globo
    // → sólo brilla el limbo) ---
    glows.push(sprite(Vec3::ZERO, EARTH_R * 2.8, [0.35, 0.62, 1.0], 0.9));

    SphereGeom { lines, cores, glows }
}

// =====================================================================
// Mini-Tierra — malla con continentes + terminador horneados
// =====================================================================

/// Valor de "tierra" pseudo-continental en una dirección de la esfera. Suma de
/// senos en longitud/latitud → manchas orgánicas. Determinista.
fn land_value(lon: f32, lat: f32) -> f32 {
    (3.0 * lon).sin() * (2.0 * lat).cos()
        + 0.55 * (7.0 * lon + 1.3).sin() * (5.0 * lat + 0.7).cos()
        + 0.40 * (5.0 * lon + 2.1).cos() * (3.0 * lat).sin()
        + 0.30 * (11.0 * lon).sin() * (7.0 * lat).cos()
}

/// Genera la malla de la Tierra: una esfera UV con color por vértice = albedo
/// (océano/continente/hielo) × sombreado Lambert contra un sol fijo.
fn earth_mesh() -> (Vec<Vertex3d>, Vec<u16>) {
    let (pos, idx) = llimphi_3d::uv_sphere(28, 56);
    let sun = Vec3::new(0.55, 0.35, 0.75).normalize();
    let verts = pos
        .into_iter()
        .map(|p| {
            let n = Vec3::from_array(p); // unitaria = normal
            let lat = n.y.clamp(-1.0, 1.0).asin();
            let lon = n.z.atan2(n.x);
            let v = land_value(lon, lat);
            let ice = lat.abs() > 1.30 || (lat.abs() > 1.12 && v > -0.2);
            let albedo = if ice {
                [0.90, 0.94, 0.98]
            } else if v > 0.35 {
                // Continente: verde con vetas marrones según el valor.
                let t = ((v - 0.35) * 0.8).clamp(0.0, 1.0);
                [0.20 + 0.35 * t, 0.42 - 0.10 * t, 0.16 + 0.04 * t]
            } else {
                // Océano: azul profundo a turquesa hacia la costa.
                let t = ((0.35 - v) * 0.5).clamp(0.0, 1.0);
                [0.03 + 0.02 * t, 0.10 + 0.18 * (1.0 - t), 0.30 + 0.25 * (1.0 - t)]
            };
            let lambert = n.dot(sun).max(0.0);
            // Noche: un azul muy tenue (no negro puro) para leer el volumen.
            let day = 0.12 + 1.05 * lambert;
            let color = [
                (albedo[0] * day).min(1.0),
                (albedo[1] * day + 0.015).min(1.0),
                (albedo[2] * day + 0.03).min(1.0),
            ];
            Vertex3d { pos: p, color }
        })
        .collect();
    (verts, idx)
}

// =====================================================================
// Texturas generadas en CPU
// =====================================================================

/// Atlas de "punto" radial con núcleo lleno (para núcleos opacos nítidos).
fn dot_atlas() -> (u32, u32, Vec<u8>) {
    const N: u32 = 48;
    let mut data = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) * 0.5;
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 - c) / c;
            let dy = (y as f32 - c) / c;
            let r = (dx * dx + dy * dy).sqrt();
            let a = if r >= 1.0 {
                0.0
            } else if r < 0.5 {
                1.0
            } else {
                let t = (1.0 - r) / 0.5;
                (t * t).clamp(0.0, 1.0)
            };
            let idx = ((y * N + x) * 4) as usize;
            data[idx] = 255;
            data[idx + 1] = 255;
            data[idx + 2] = 255;
            data[idx + 3] = (a * 255.0) as u8;
        }
    }
    (N, N, data)
}

/// Atlas de glow radial suave (gaussiano, sin núcleo duro) para auras aditivas.
fn glow_atlas() -> (u32, u32, Vec<u8>) {
    const N: u32 = 64;
    let mut data = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) * 0.5;
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 - c) / c;
            let dy = (y as f32 - c) / c;
            let r2 = dx * dx + dy * dy;
            // Gaussiano recortado al círculo: brillo al centro, cae suave.
            let a = if r2 >= 1.0 { 0.0 } else { (-r2 * 4.0).exp() };
            let idx = ((y * N + x) * 4) as usize;
            data[idx] = 255;
            data[idx + 1] = 255;
            data[idx + 2] = 255;
            data[idx + 3] = (a * 255.0) as u8;
        }
    }
    (N, N, data)
}

/// Panorama de campo estelar (RGBA): azul-noche con estrellas **tenues** y
/// densas. Cada estrella es un blob suave (redondo aunque el muestreo esférico
/// la estire cerca del polo). Determinista (LCG sembrada).
fn starfield_panorama() -> (u32, u32, Vec<u8>) {
    const W: u32 = 2048;
    const H: u32 = 512;
    let mut data = vec![0u8; (W * H * 4) as usize];
    for px in data.chunks_exact_mut(4) {
        px[0] = 4;
        px[1] = 5;
        px[2] = 13;
        px[3] = 255;
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let plot = |data: &mut [u8], x: i64, y: i64, r: u8, g: u8, bl: u8, k: f32| {
        if y < 0 || y >= H as i64 {
            return;
        }
        let xx = x.rem_euclid(W as i64) as u32;
        let idx = ((y as u32 * W + xx) * 4) as usize;
        data[idx] = data[idx].max((r as f32 * k) as u8);
        data[idx + 1] = data[idx + 1].max((g as f32 * k) as u8);
        data[idx + 2] = data[idx + 2].max((bl as f32 * k) as u8);
    };
    for _ in 0..6500 {
        let x = (next() % W) as i64;
        let y = (next() % H) as i64;
        // Brillo bajo-medio-alto: 48..=180 (la bóveda se lee; las pocas más
        // brillantes rozan el bloom y titilan, sin lavar el fondo).
        let b = 48 + (next() % 132) as u8;
        let pick = next() % 10;
        let (r, g, bl) = if pick < 7 {
            (b, b, b)
        } else if pick < 9 {
            (b.saturating_sub(20), b.saturating_sub(8), b)
        } else {
            (b, b.saturating_sub(10), b.saturating_sub(28))
        };
        plot(&mut data, x, y, r, g, bl, 1.0);
        plot(&mut data, x - 1, y, r, g, bl, 0.45);
        plot(&mut data, x + 1, y, r, g, bl, 0.45);
        plot(&mut data, x, y - 1, r, g, bl, 0.45);
        plot(&mut data, x, y + 1, r, g, bl, 0.45);
    }
    (W, H, data)
}
