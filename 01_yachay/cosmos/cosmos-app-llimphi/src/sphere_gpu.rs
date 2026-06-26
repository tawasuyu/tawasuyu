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
use llimphi_3d::glam::{Mat4, Vec3, Vec4};
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
const EARTH_R: f32 = 0.23;

/// Contornos continentales esquemáticos (lat, lon en grados). Copiados de
/// `cosmos_render::sphere3d::earth` — referenciales, no un mapa de precisión.
const CONTINENTES: &[&[(f32, f32)]] = &[
    // África
    &[
        (35.0, -6.0), (37.0, 10.0), (33.0, 22.0), (31.0, 32.0), (12.0, 43.0),
        (11.0, 51.0), (-4.0, 40.0), (-26.0, 33.0), (-34.0, 26.0), (-34.0, 19.0),
        (-18.0, 12.0), (0.0, 9.0), (5.0, -4.0), (11.0, -15.0), (21.0, -17.0),
        (28.0, -13.0),
    ],
    // Sudamérica
    &[
        (12.0, -72.0), (11.0, -61.0), (5.0, -52.0), (-5.0, -35.0), (-23.0, -43.0),
        (-34.0, -54.0), (-52.0, -69.0), (-55.0, -67.0), (-42.0, -74.0),
        (-18.0, -70.0), (-5.0, -81.0), (2.0, -79.0), (8.0, -77.0),
    ],
    // Norteamérica
    &[
        (70.0, -160.0), (71.0, -125.0), (68.0, -95.0), (63.0, -78.0),
        (47.0, -53.0), (45.0, -67.0), (30.0, -81.0), (25.0, -81.0),
        (20.0, -97.0), (23.0, -110.0), (34.0, -120.0), (48.0, -125.0),
        (60.0, -148.0),
    ],
    // Eurasia
    &[
        (36.0, -9.0), (43.0, -9.0), (58.0, 5.0), (71.0, 26.0), (73.0, 80.0),
        (73.0, 140.0), (66.0, 180.0), (53.0, 141.0), (40.0, 130.0), (30.0, 122.0),
        (22.0, 110.0), (9.0, 105.0), (8.0, 77.0), (21.0, 72.0), (25.0, 57.0),
        (13.0, 45.0), (30.0, 33.0), (41.0, 28.0), (38.0, 15.0), (40.0, 0.0),
    ],
    // Australia
    &[
        (-11.0, 131.0), (-12.0, 142.0), (-25.0, 153.0), (-38.0, 147.0),
        (-35.0, 138.0), (-32.0, 116.0), (-22.0, 114.0), (-14.0, 127.0),
    ],
    // Antártida (casquete polar aproximado)
    &[
        (-72.0, -180.0), (-70.0, -120.0), (-73.0, -60.0), (-70.0, 0.0),
        (-73.0, 60.0), (-70.0, 120.0), (-72.0, 170.0),
    ],
];

/// Geometría dinámica de un frame: líneas + sprites opacos + glows aditivos +
/// la malla de la mini-Tierra (rehorneada por frame: continentes reales en su
/// posición + día/noche según el Sol de la carta).
pub(crate) struct SphereGeom {
    lines: Vec<LineVertex>,
    cores: Vec<Billboard>,
    glows: Vec<Billboard>,
    earth_verts: Vec<Vertex3d>,
    earth_indices: Vec<u16>,
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
        // Mini-Tierra: la malla se rehornea cada frame en sphere_geometry (los
        // colores dependen de la carta: continentes orientados + día/noche).
        // Acá sólo una inicial; draw() la reemplaza con set_geometry.
        let (pos, eindices) = llimphi_3d::uv_sphere(28, 56);
        let everts: Vec<Vertex3d> =
            pos.iter().map(|p| Vertex3d { pos: *p, color: [0.05, 0.16, 0.34] }).collect();
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
        if !geom.earth_verts.is_empty() {
            self.earth.set_geometry(device, &geom.earth_verts, &geom.earth_indices);
        }
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

/// Ascensión recta del Medio Cielo (RAMC) desde la longitud eclíptica del MC.
/// Réplica de `cosmos_render::sphere3d::ramc_deg`.
fn ramc_deg(mc_deg: f32, eps_rad: f32) -> f32 {
    let lmc = mc_deg.to_radians();
    (lmc.sin() * eps_rad.cos()).atan2(lmc.cos()).to_degrees()
}

/// Dirección (marco GPU) de un punto geográfico (lat, lon). La longitud del
/// observador y el RAMC fijan la fase de rotación de la Tierra: el observador
/// está en AR = RAMC, así cualquier otra longitud `lon` está en
/// AR = RAMC + (lon − lon_obs). Réplica de `earth::geo_to_ecliptic` en marco GPU.
fn geo_to_gpu(lat: f32, lon: f32, lon_obs: f32, ramc: f32, eps: &Mat4) -> Vec3 {
    eq_to_gpu(ramc + lon - lon_obs, lat, eps)
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

/// Nombres de signos (orden zodiacal desde Aries).
const SIGN_NAMES: [&str; 12] = [
    "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra", "scorpio", "sagittarius",
    "capricorn", "aquarius", "pisces",
];

/// Una etiqueta de texto anclada a un punto 3D del cielo (se proyecta a pantalla
/// y se pinta con vello ENCIMA del pase GPU vía `paint_over`).
pub(crate) struct SphereLabel {
    pub world: Vec3,
    pub text: String,
    pub color: [f32; 4],
    pub size: f32,
}

/// Etiquetas de la esfera: signos (mitad de cada sector), ángulos ASC/MC/DSC/IC
/// y los cuerpos natales (abreviatura). Coords de mundo (radio 1 = cielo).
pub(crate) fn sphere_labels(model: &RenderModel, pal: &Palette) -> Vec<SphereLabel> {
    let r = 1.0_f32;
    let ecl = rgb(pal.dial_ring);
    let ang = rgb(pal.angle_highlight);
    let mut out = Vec::new();
    // Signos.
    for (i, name) in SIGN_NAMES.iter().enumerate() {
        let lon = i as f32 * 30.0 + 15.0;
        out.push(SphereLabel {
            world: eclip(lon) * r * 1.18,
            text: cosmos_render::sign_unicode(name).to_string(),
            color: [ecl[0], ecl[1], ecl[2], 0.95],
            size: 13.0,
        });
    }
    // Ángulos.
    for (deg, txt) in [
        (model.ascendant_deg, "Asc"),
        (model.midheaven_deg, "MC"),
        (model.descendant_deg, "Dsc"),
        (model.imum_coeli_deg, "IC"),
    ] {
        out.push(SphereLabel {
            world: eclip(deg) * r * 1.22,
            text: txt.to_string(),
            color: [ang[0], ang[1], ang[2], 1.0],
            size: 12.0,
        });
    }
    // Cuerpos natales (abreviatura del glifo).
    for layer in &model.layers {
        if matches!(layer.kind, LayerKind::Bodies) && layer.module_id == "natal" {
            for g in &layer.glyphs {
                out.push(SphereLabel {
                    world: eclip(g.deg) * r * 1.11,
                    text: cosmos_render::planet_unicode_with_retro(&g.symbol, g.retrograde),
                    color: [1.0, 1.0, 1.0, 1.0],
                    size: 12.0,
                });
            }
        }
    }
    out
}

/// Proyecta un punto de mundo a pixeles de pantalla dentro de `rect`, con la
/// misma cámara que el pase GPU. Devuelve `None` si cae detrás de la esfera
/// (hemisferio lejano) o fuera del frustum. Para el overlay de etiquetas.
pub(crate) fn project_label(
    world: Vec3,
    rect: (f32, f32, f32, f32),
    yaw_deg: f32,
    pitch_deg: f32,
    dist: f32,
) -> Option<(f32, f32)> {
    let (rx, ry, rw, rh) = rect;
    if rw < 1.0 || rh < 1.0 {
        return None;
    }
    let cam = Camera3d::orbit(Vec3::ZERO, yaw_deg.to_radians(), pitch_deg.to_radians(), dist);
    // Cull del hemisferio lejano (el punto mira hacia el lado opuesto al ojo).
    if world.normalize_or_zero().dot(cam.eye.normalize_or_zero()) < -0.05 {
        return None;
    }
    let clip = cam.view_proj(rw / rh) * Vec4::new(world.x, world.y, world.z, 1.0);
    if clip.w <= 0.001 {
        return None;
    }
    let ndc_x = clip.x / clip.w;
    let ndc_y = clip.y / clip.w;
    if !(-1.3..=1.3).contains(&ndc_x) || !(-1.3..=1.3).contains(&ndc_y) {
        return None;
    }
    Some((
        rx + (ndc_x * 0.5 + 0.5) * rw,
        ry + (1.0 - (ndc_y * 0.5 + 0.5)) * rh,
    ))
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

    // --- Vía Láctea: bruma dentro de la bóveda. Banda de glows grandes y muy
    // tenues sobre el ecuador galáctico (más densa hacia el centro galáctico) ---
    for s in cosmos_render::sky_data::galactic_equator(160) {
        let p = eq_to_gpu(s.ra_deg, s.dec_deg, &eps) * r * 0.999;
        let a = 0.05 + 0.10 * s.toward_center;
        let sz = 0.22 + 0.12 * s.toward_center;
        glows.push(sprite(p, sz, [0.62, 0.66, 0.85], a));
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

    // --- Marcas de grados (ticks cruzando la eclíptica) + divisiones de signos.
    // Cada 5° un tick chico; cada 10° mediano; cada 30° (borde de signo) un
    // arco de latitud que parte el zodíaco, más un espolón hacia afuera. ---
    let mut d = 0;
    while d < 360 {
        let lon = d as f32;
        let (lat_span, alpha, col) = if d % 30 == 0 {
            (10.0, 0.7, ecl) // división de signo
        } else if d % 10 == 0 {
            (4.0, 0.5, ecl)
        } else {
            (2.2, 0.32, ecl)
        };
        push_polyline(
            &mut lines,
            &[eclip_latlon(lon, -lat_span) * r, eclip_latlon(lon, lat_span) * r],
            col,
            alpha,
        );
        if d % 30 == 0 {
            // Espolón hacia afuera en el borde de signo.
            push_polyline(&mut lines, &[eclip(lon) * r, eclip(lon) * r * 1.07], ecl, 0.8);
        }
        d += 5;
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

    // --- Mini-Tierra: continentes reales (en posición correcta por el RAMC) +
    // observador + día/noche + atmósfera ---
    let eps_rad = OBLIQUITY_DEG.to_radians();
    let ramc = ramc_deg(model.midheaven_deg, eps_rad);
    let lon_obs = model.geo_longitude_deg;
    let er = EARTH_R;
    // Sol de la carta (si está) → punto subsolar y terminador día/noche.
    let sun_dir: Option<Vec3> = model
        .layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Bodies) && l.module_id == "natal")
        .flat_map(|l| l.glyphs.iter())
        .find(|g| g.symbol == "sun")
        .map(|g| eclip(g.deg));

    // Globo relleno y sombreado con continentes REALES horneados (geografía
    // correcta por el RAMC) + día/noche real. Se sube como malla en draw().
    let (earth_verts, earth_indices) = earth_baked(ramc, lon_obs, sun_dir);

    // Observador, en su lugar real sobre la Tierra — marca grande que destaca:
    // halo + núcleo + un anillo de puntos para que se lea aunque sea chico.
    let obs = geo_to_gpu(model.geo_latitude_deg, lon_obs, lon_obs, ramc, &eps) * er * 1.04;
    glows.push(sprite(obs, 0.12, [1.0, 0.35, 0.15], 1.0)); // aura naranja fuerte
    glows.push(sprite(obs, 0.05, [1.0, 0.9, 0.6], 1.0)); // núcleo cálido brillante
    cores.push(sprite(obs, 0.02, [1.0, 1.0, 1.0], 1.0)); // punto blanco nítido

    // Halo de atmósfera (glow azul, ocluido por el globo → brilla el limbo).
    glows.push(sprite(Vec3::ZERO, er * 2.7, [0.35, 0.62, 1.0], 0.9));

    SphereGeom { lines, cores, glows, earth_verts, earth_indices }
}

// =====================================================================
// Mini-Tierra — globo relleno y sombreado, con continentes REALES horneados
// en el color del vértice (geografía correcta por el RAMC) + día/noche real.
// =====================================================================

/// ¿El punto geográfico (lat, lon en grados) cae dentro del polígono `poly`
/// (lista de (lat, lon))? Ray-casting planar — suficiente para los contornos
/// esquemáticos (no cruzan el antimeridiano salvo casquetes, que igual quedan).
fn point_in_polygon(lat: f32, lon: f32, poly: &[(f32, f32)]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (yi, xi) = poly[i];
        let (yj, xj) = poly[j];
        if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Hornea la malla de la Tierra para esta carta: continentes reales (en su
/// posición correcta por el RAMC y la longitud del observador) + océano + hielo
/// polar, todo sombreado por el día/noche contra el Sol de la carta. Color por
/// vértice (Gouraud) → globo liso y realista, sin depender de la normal del
/// shader. La malla se orienta en coords de MUNDO (el `set_model` sólo escala).
fn earth_baked(ramc: f32, lon_obs: f32, sun_dir: Option<Vec3>) -> (Vec<Vertex3d>, Vec<u16>) {
    let (pos, idx) = llimphi_3d::uv_sphere(36, 72);
    let inv_eps = Mat4::from_rotation_x(-OBLIQUITY_DEG.to_radians());
    let verts = pos
        .into_iter()
        .map(|p| {
            let n = Vec3::from_array(p); // dirección de mundo (unitaria)
            // Mundo → geográfico: deshacer la oblicuidad y el RAMC.
            let eq = inv_eps.transform_point3(n);
            let dec = eq.y.clamp(-1.0, 1.0).asin().to_degrees();
            let ra = eq.z.atan2(eq.x).to_degrees();
            let lat = dec;
            let mut lon = ra - ramc + lon_obs;
            lon = (lon + 180.0).rem_euclid(360.0) - 180.0; // a -180..180
            let land = CONTINENTES.iter().any(|poly| point_in_polygon(lat, lon, poly));
            let ice = lat.abs() > 74.0;
            let albedo = if ice {
                [0.88, 0.92, 0.97]
            } else if land {
                // Verde-tierra con leve variación por latitud (desierto cálido).
                let warm = (1.0 - (lat.abs() / 60.0).min(1.0)) * 0.12;
                [0.22 + warm, 0.46 - warm * 0.4, 0.20]
            } else {
                [0.04, 0.15, 0.33] // océano azul profundo
            };
            // Día/noche real: Lambert contra el Sol de la carta (en mundo).
            let lit = match sun_dir {
                Some(s) => 0.16 + 1.0 * n.dot(s).max(0.0),
                None => 0.7,
            };
            let color = [
                (albedo[0] * lit).min(1.0),
                (albedo[1] * lit + 0.01).min(1.0),
                (albedo[2] * lit + 0.03).min(1.0),
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
