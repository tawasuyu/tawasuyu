//! Modo `--film`: *filma* una escena voxel guionada a un video, la rebanada
//! vertical de la idea "generar escenas controladas, mover personajes y filmar".
//!
//! Junta las cuatro piezas nuevas en un pipeline punta a punta, **sin pantalla**:
//!
//! 1. un **reparto** de [`Actor`]es (muñecos articulados) que caminan un trayecto
//!    sobre el terreno (posa la malla por frame con `Renderer3d::set_geometry`);
//! 2. una **cámara guionada** por [`CameraTrack`] (keyframes interpolados suave);
//! 3. el render headless de cada cuadro a PNG (reusa [`crate::write_png`]);
//! 4. el muxeo de la secuencia a video con [`foreign_av::encode_frames`].
//!
//! Es determinista (todo en función del índice de frame) → reproducible. La
//! *dirección* (quién camina a dónde, dónde corta la cámara) vive acá, en la capa
//! de contenido; el motor sólo provee cámara/escena/actor genéricos.

use std::sync::Once;

use foreign_vox::{VoxModel, Voxel};
use llimphi_3d::glam::Vec3;
use llimphi_3d::{
    Atmosphere, CamKey, Camera3d, CameraTrack, Hud, HudQuad, Renderer3d, Scene3d, VoxelRenderer,
};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_voxel::{Actor, ActorKey, ActorScript, Clip, Sequence, Shot};

use crate::world::{World, FMT};
use crate::{DIM_XZ, SEED};

/// Resolución y cadencia del film (16:9, 30 fps). Por defecto 960×540 @2×; se puede
/// subir a 720p/1080p/4K por flag (ver [`film_dims`]).
const W: u32 = 960;
const H: u32 = 540;
const FPS: u32 = 30;
/// Factor de **supersampling** (SSAA): se renderiza a `SS×` y se baja promediando
/// → antialias de los bordes duros del ray-march. El film/vox lo usan; `--poses`
/// queda a 1× (su HUD se mide en pixels de pantalla).
const SS: u32 = 2;

/// Resolución de salida elegida por flag de línea de comandos. Devuelve `(w, h, ss)`
/// — el ancho/alto **finales** del video y el factor de supersampling. En 4K se baja
/// `ss` a 1 (un buffer 4K×2 = 8K sería ~250 MB y pocas GPUs lo aguantan); en cine
/// "real" el ray-march ya queda nítido por la resolución nativa.
/// - `--4k`            → 3840×2160 @1×
/// - `--1080`/`--1080p`→ 1920×1080 @2×
/// - `--720`/`--720p`  → 1280×720  @2×
/// - (default)         → 960×540   @2×
fn film_dims() -> (u32, u32, u32) {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--4k") {
        (3840, 2160, 1)
    } else if has("--1080") || has("--1080p") {
        (1920, 1080, 2)
    } else if has("--720") || has("--720p") {
        (1280, 720, 2)
    } else {
        (W, H, SS)
    }
}
/// Carpeta de cuadros y salida del video.
const FRAME_DIR: &str = "/tmp/voxel_film";
const OUT: &str = "/tmp/voxel_film.mkv";

/// Paleta del reparto (piel, remera, pantalón) — tres figuras distinguibles.
const CAST: [([f32; 3], [f32; 3], [f32; 3]); 3] = [
    ([0.90, 0.72, 0.58], [0.82, 0.28, 0.26], [0.20, 0.20, 0.28]), // remera roja
    ([0.86, 0.68, 0.54], [0.22, 0.55, 0.78], [0.18, 0.20, 0.24]), // remera azul
    ([0.92, 0.78, 0.62], [0.92, 0.80, 0.30], [0.26, 0.22, 0.20]), // remera amarilla
];

/// Filma el guion ([`screenplay`]) y escribe el video. Reproduce la [`Sequence`]
/// cuadro a cuadro (determinista): cada actor se posa según su `ActorScript`
/// (con cross-fade de clips), la cámara sale de los planos con cortes duros.
pub fn film() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut world = World::build(&hal.device, &hal.queue, DIM_XZ, SEED);
    world.show_monument(false); // película de personajes: sin el cubo flotante

    let (seq, mut cast) = screenplay(&world);
    // Un `Renderer3d` por actor (su malla se re-sube cada frame).
    let mut actor_r: Vec<Renderer3d> = cast.iter().map(|_| Renderer3d::new(&hal.device, FMT)).collect();

    let (w, h, ss) = film_dims();
    let (ssw, ssh) = (w * ss, h * ss);
    if (w, h, ss) != (W, H, SS) {
        eprintln!("film: resolución {w}x{h} @{ss}× (supersampled a {ssw}x{ssh})");
    }
    let inter = make_target(&hal, ssw, ssh); // render a ss× para SSAA
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    prepare_dir();
    let frames = seq.frames(FPS);
    let dt = 1.0 / FPS as f32;
    for f in 0..frames {
        let t = f as f32 / FPS as f32;

        // Cada actor obedece su guion: posición sobre el relieve, rumbo y clip
        // (el cambio de clip dispara el cross-fade interno del `Actor`).
        for (a, script) in cast.iter_mut().zip(&seq.actors) {
            let s = script.sample(t);
            a.pos = world.ground_at(s.gx as u32, s.gz as u32);
            a.facing = s.facing;
            a.set_clip(s.clip);
            a.advance(dt);
        }
        for (a, r) in cast.iter().zip(actor_r.iter_mut()) {
            let (v, i) = a.mesh();
            r.set_geometry(&hal.device, &v, &i);
            r.set_model(a.model());
        }

        world.tick(dt); // la manada de fondo deambula
        world.animate(t * 0.5); // el monumento gira
        let camera = seq.camera(t);

        // IK de mirada: los actores siguen la cámara con la cabeza (dentro del rango
        // creíble) → "conscientes del lente". Re-malla con la cabeza ya orientada.
        for (a, r) in cast.iter_mut().zip(actor_r.iter_mut()) {
            a.look_at(Some(camera.eye));
            let (v, i) = a.mesh();
            r.set_geometry(&hal.device, &v, &i);
        }

        let refs: Vec<&Renderer3d> = actor_r.iter().collect();
        render_frame(&hal, &mut renderer, &mut world, &camera, &refs, &inter_view, (ssw, ssh));
        crate::write_png_downsampled(&hal, &inter, ssw, ssh, ss, &frame_path(f));
        if f % 15 == 0 {
            eprintln!("film: cuadro {f}/{frames}");
        }
    }

    // --- Banda sonora: compone+sintetiza un WAV con takiy para muxear, con acentos
    // sobre los **beats del guion** (cortes de cámara + gestos) → música sincronizada.
    let audio = "/tmp/voxel_film.wav";
    let beats = seq.beat_times();
    let secs = crate::soundtrack::render_to(audio, &beats);
    let beat_list: Vec<String> = beats.iter().map(|t| format!("{t:.2}s")).collect();
    eprintln!(
        "film: banda sonora {audio} ({secs:.1}s, {} acentos sobre la acción: {})",
        beats.len(),
        beat_list.join(", ")
    );

    // --- Muxeo a video (video AV1 + audio Opus). Si no hay ffmpeg, deja los PNG.
    let pattern = format!("{FRAME_DIR}/frame_%04d.png");
    match foreign_av::encode_frames(&pattern, FPS, 30, Some(std::path::Path::new(audio)), OUT) {
        Ok(()) => eprintln!("film: video escrito {OUT} ({frames} cuadros, {w}x{h}@{FPS}, con audio)"),
        Err(e) => eprintln!(
            "film: cuadros en {FRAME_DIR}/ pero ffmpeg falló ({e:?}); \
             podés muxear a mano: ffmpeg -framerate {FPS} -i {pattern} -i {audio} -c:v libsvtav1 -c:a libopus {OUT}"
        ),
    }
}

/// El **guion** de la película (la "dirección", editable): tres actores entran
/// caminando por un llano, se detienen en fila, se giran hacia la cámara y cada
/// uno hace un gesto distinto (saludar / festejar / señalar). La cámara tiene dos
/// planos con un **corte duro**: un establishing que entra durante la caminata, y
/// un plano corto que empuja sobre el trío mientras gesticulan. Devuelve la
/// [`Sequence`] (data) + los [`Actor`]es con su color (estado visual).
fn screenplay(world: &World) -> (Sequence, Vec<Actor>) {
    use std::f32::consts::{FRAC_PI_2, PI};

    let span = 16.0_f32;
    let (gx0, lanes) = find_flat_strip(world, span);
    let gx0 = gx0 as f32;
    let gx_start = gx0 + 3.0;
    let gx_stop = gx0 + span - 3.0;
    let (t_walk, t_turn, dur) = (2.6_f32, 3.0_f32, 5.6_f32);
    let emotes = [Clip::Wave, Clip::Cheer, Clip::Point];

    // Estado visual (color) + guion. Trío junto: carriles Z apretados (±2.5)
    // alrededor del centro de la franja, para que entren los tres sin que el más
    // cercano se coma el lente.
    let center_z = lanes[1] as f32;
    let offsets = [-2.5_f32, 0.0, 2.5];
    let mut actors = Vec::with_capacity(3);
    let mut scripts = Vec::with_capacity(3);
    for ((&off, (skin, shirt, pants)), emote) in offsets.iter().zip(CAST).zip(emotes) {
        let gzf = center_z + off;
        let pos = world.ground_at(gx_start as u32, gzf as u32);
        actors.push(Actor::new(pos, FRAC_PI_2).with_colors(skin, shirt, pants));
        scripts.push(ActorScript::new(vec![
            ActorKey::at(0.0, gx_start, gzf),                          // arranca a la izquierda
            ActorKey::at(t_walk, gx_stop, gzf).facing(FRAC_PI_2),      // camina hasta la marca (mira +X)
            ActorKey::at(t_turn, gx_stop, gzf).play(emote).facing(PI), // gira a cámara y gesticula
            ActorKey::at(dur, gx_stop, gzf).play(emote).facing(PI),
        ]));
    }

    // Dos focos: el plano 1 mira el punto medio de la caminata; el 2, la marca de
    // llegada donde gesticulan. Cámara cerca (los actores grandes, sin que el
    // relieve los oculte), a la altura del pecho.
    let focus_walk = world.ground_at(((gx_start + gx_stop) * 0.5) as u32, lanes[1]) + Vec3::new(0.0, 1.0, 0.0);
    let focus_emote = world.ground_at(gx_stop as u32, lanes[1]) + Vec3::new(0.0, 1.0, 0.0);
    let cut = 2.8_f32;
    let shot1 = CameraTrack::new(vec![
        CamKey::look(0.0, focus_walk + Vec3::new(-3.5, 4.5, -13.0), focus_walk, 50.0),
        CamKey::look(t_walk + 0.2, focus_walk + Vec3::new(-1.0, 3.0, -10.0), focus_walk, 46.0),
    ]);
    let shot2 = CameraTrack::new(vec![
        CamKey::look(0.0, focus_emote + Vec3::new(0.4, 2.8, -11.0), focus_emote, 42.0),
        CamKey::look(dur - cut, focus_emote + Vec3::new(1.6, 2.3, -9.0), focus_emote, 39.0),
    ]);
    let seq = Sequence::new(scripts, vec![Shot::new(0.0, shot1), Shot::new(cut, shot2)], dur);
    (seq, actors)
}

/// Modo `--poses`: vuelca **un** PNG (`/tmp/actor_clips.png`) con una fila de
/// actores, cada uno en un [`Clip`] distinto y **etiquetado** (reusa el texto del
/// HUD), para verificar de un vistazo la librería de animación. Vista frontal:
/// los actores miran a la cámara.
pub fn poses_shot() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut world = World::build(&hal.device, &hal.queue, DIM_XZ, SEED);
    world.show_monument(false); // no tapar la fila con el cubo flotante

    let clips = [Clip::Idle, Clip::Walk, Clip::Run, Clip::Wave, Clip::Point, Clip::Cheer];
    let labels = ["IDLE", "WALK", "RUN", "WAVE", "POINT", "CHEER"];
    let shirts: [[f32; 3]; 6] = [
        [0.82, 0.28, 0.26],
        [0.22, 0.55, 0.78],
        [0.92, 0.80, 0.30],
        [0.30, 0.70, 0.40],
        [0.70, 0.40, 0.78],
        [0.85, 0.50, 0.25],
    ];
    let span = 14.0_f32;
    let (gx0, lanes) = find_flat_strip(&world, span);
    let gz = lanes[1];
    let n = clips.len();

    let mut cast: Vec<Actor> = Vec::with_capacity(n);
    let mut actor_r: Vec<Renderer3d> = Vec::with_capacity(n);
    for k in 0..n {
        let gx = gx0 as f32 + (k as f32 + 0.5) * span / n as f32;
        let pos = world.ground_at(gx as u32, gz);
        let mut a = Actor::new(pos, std::f32::consts::PI) // mira al -Z (a la cámara)
            .with_colors([0.88, 0.70, 0.56], shirts[k], [0.18, 0.20, 0.28]);
        a.set_clip(clips[k]);
        a.advance(1.0); // una pose representativa (no la inicial neutra)
        cast.push(a);
        actor_r.push(Renderer3d::new(&hal.device, FMT));
    }
    for (a, r) in cast.iter().zip(actor_r.iter_mut()) {
        let (v, i) = a.mesh();
        r.set_geometry(&hal.device, &v, &i);
        r.set_model(a.model());
    }

    // Cámara frontal centrada en la fila, cerca y a la altura del pecho.
    let mid = world.ground_at((gx0 as f32 + span * 0.5) as u32, gz) + Vec3::new(0.0, 0.9, 0.0);
    let mut camera = Camera3d::orbit(mid, std::f32::consts::PI, 0.16, span * 0.78);
    camera.fovy_rad = 48_f32.to_radians();

    let inter = make_target(&hal, W, H); // poses queda 1× (HUD en pixels de pantalla)
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());
    world.tick(0.0);
    world.animate(0.6);

    renderer
        .render_to_view(&hal, &vello::Scene::new(), &inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("poses") });
    let refs: Vec<&Renderer3d> = actor_r.iter().collect();
    world.render_with(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), &camera, &refs);

    // Etiquetas: proyectar la cabeza de cada actor a pantalla y poner el nombre.
    let vp = camera.view_proj(W as f32 / H as f32);
    let mut quads: Vec<HudQuad> = Vec::new();
    for (a, label) in cast.iter().zip(labels) {
        let ndc = vp.project_point3(a.pos + Vec3::new(0.0, 2.15, 0.0));
        let sx = (ndc.x * 0.5 + 0.5) * W as f32;
        let sy = (1.0 - (ndc.y * 0.5 + 0.5)) * H as f32;
        let px = 2.0;
        let tw = HudQuad::text_width(label, px);
        let (x, y) = (sx - tw * 0.5, sy);
        quads.push(HudQuad { x: x - 4.0, y: y - 4.0, w: tw + 8.0, h: 7.0 * px + 8.0, color: [0.0, 0.0, 0.0, 0.5] });
        quads.extend(HudQuad::text(label, x, y, px, [0.95, 0.97, 1.0, 0.96]));
    }
    let mut hud = Hud::new(&hal.device, FMT);
    hud.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), &quads);

    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    crate::write_png(&hal, &inter, W, H, "/tmp/actor_clips.png");
    eprintln!("poses: /tmp/actor_clips.png ({n} clips)");
}

/// Modo `--vox`: prueba el **puente MagicaVoxel** punta a punta. Genera un modelo
/// reconocible (un golem), lo **escribe** a `.vox` (`foreign_vox::write`), lo
/// **reimporta** (`llimphi_voxel::load_grid` → bytes → `VoxModel` → `VoxelGrid`)
/// y lo renderiza orbitando a `/tmp/vox_import.png`. Así el PNG demuestra el
/// camino real de carga sobre bytes `.vox` codificados a spec.
pub fn vox_shot() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    let model = golem_model();
    let path = "/tmp/vox_demo.vox";
    std::fs::write(path, foreign_vox::write(&model)).expect("escribir .vox");
    let grid = llimphi_voxel::load_grid(path).expect("importar .vox");
    let dim = grid.dim();

    let mut vr = VoxelRenderer::new(&hal.device, &hal.queue, FMT, &grid);
    vr.sun_dir = [0.45, 0.8, -0.4]; // sol al frente (ilumina la cara hacia cámara)
    vr.atmosphere = Atmosphere {
        sky_zenith: [60, 110, 190],
        sky_horizon: [202, 216, 236],
        fog_density: 0.0, // modelo chico: sin niebla
        god_rays: 0.0,
    };
    let mut scene = Scene3d::new();

    let d = dim.iter().copied().max().unwrap_or(16) as f32;
    // Vista 3/4 frontal (yaw ~205° mira la cara con los ojos, levemente de costado).
    let camera = Camera3d::orbit(Vec3::ZERO, 205_f32.to_radians(), 12_f32.to_radians(), d * 1.9);

    let (w, h, ss) = film_dims();
    let (ssw, ssh) = (w * ss, h * ss);
    let inter = make_target(&hal, ssw, ssh); // ss× para SSAA
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(&hal, &vello::Scene::new(), &inter_view, ssw, ssh, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("vox") });
    scene.render(&hal.device, &hal.queue, &mut enc, &inter_view, (ssw, ssh), &camera, Some(&vr), &[]);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    crate::write_png_downsampled(&hal, &inter, ssw, ssh, ss, "/tmp/vox_import.png");
    eprintln!(
        "vox: {} ({} voxels) → {path} → /tmp/vox_import.png (grid {}x{}x{})",
        "golem",
        model.voxels.len(),
        dim[0],
        dim[1],
        dim[2]
    );
}

/// Un golem voxel de demo (espacio `.vox`, `z` arriba): piernas, torso, brazos,
/// cabeza con ojos y una antena — pensado para leerse como un **asset diseñado**
/// (no terreno) al importarlo.
fn golem_model() -> VoxModel {
    let mut m = VoxModel::new([11, 7, 17]);
    // Paleta: cuerpo teal, cabeza gris, ojos oscuros, piernas azul, antena naranja.
    m.palette[1] = [60, 150, 140, 255];
    m.palette[2] = [205, 208, 214, 255];
    m.palette[3] = [25, 28, 34, 255];
    m.palette[4] = [40, 60, 110, 255];
    m.palette[5] = [232, 130, 40, 255];

    let v = &mut m.voxels;
    fill_box(v, 2..4, 2..5, 0..4, 4); // pierna izq
    fill_box(v, 7..9, 2..5, 0..4, 4); // pierna der
    fill_box(v, 2..9, 1..6, 4..10, 1); // torso
    fill_box(v, 0..2, 2..5, 5..10, 1); // brazo izq
    fill_box(v, 9..11, 2..5, 5..10, 1); // brazo der
    fill_box(v, 3..8, 1..6, 10..15, 2); // cabeza
    v.push(Voxel { x: 4, y: 0, z: 12, i: 3 }); // ojo izq (cara y=0)
    v.push(Voxel { x: 6, y: 0, z: 12, i: 3 }); // ojo der
    fill_box(v, 5..6, 3..4, 15..17, 5); // antena
    m
}

/// Apila los voxels de una caja `[xr]×[yr]×[zr]` con índice de color `i`.
fn fill_box(
    v: &mut Vec<Voxel>,
    xr: std::ops::Range<u8>,
    yr: std::ops::Range<u8>,
    zr: std::ops::Range<u8>,
    i: u8,
) {
    for z in zr.clone() {
        for y in yr.clone() {
            for x in xr.clone() {
                v.push(Voxel { x, y, z, i });
            }
        }
    }
}

/// Render de un cuadro: limpia la intermedia (base negra; el cielo lo pinta la
/// atmósfera del voxel en los misses) y compone terreno + monumento + actores
/// en el pase de [`World::render_with`].
#[allow(clippy::too_many_arguments)]
fn render_frame(
    hal: &Hal,
    renderer: &mut Renderer,
    world: &mut World,
    camera: &llimphi_3d::Camera3d,
    actors: &[&Renderer3d],
    inter_view: &wgpu::TextureView,
    (w, h): (u32, u32),
) {
    renderer
        .render_to_view(hal, &vello::Scene::new(), inter_view, w, h, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("film") });
    world.render_with(&hal.device, &hal.queue, &mut enc, inter_view, (w, h), camera, actors);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
}

/// Crea la textura intermedia `w×h` (mismo descriptor que el modo `--shot`).
fn make_target(hal: &Hal, w: u32, h: u32) -> wgpu::Texture {
    hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("film-inter"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Ruta del cuadro `f` (`frame_0000.png`, …).
fn frame_path(f: u32) -> String {
    format!("{FRAME_DIR}/frame_{f:04}.png")
}

/// Elige el tramo **más plano** del mundo (mínima diferencia de altura a lo largo
/// de tres carriles Z, todos sobre el nivel del mar): así los actores caminan/
/// gesticulan en fila sin que el relieve los hunda u ocluya. Si no hay tierra
/// (mundo todo agua), cae a un default central.
fn find_flat_strip(world: &World, span: f32) -> (u32, [u32; 3]) {
    let dy = (DIM_XZ * 4 / 10).max(48) as f32;
    let land_min = (0.30 - 0.5) * dy + 2.0;
    let walk = span.ceil() as u32;
    let dim = DIM_XZ;
    let (lo, hi) = (12u32, dim.saturating_sub(12));
    let mut best: Option<(u32, [u32; 3])> = None;
    let mut best_spread = f32::MAX;
    for oz in (lo..hi.saturating_sub(10)).step_by(4) {
        let lanes = [oz, oz + 5, oz + 10];
        for ox in (lo..hi.saturating_sub(walk)).step_by(4) {
            let mut lo_y = f32::MAX;
            let mut hi_y = f32::MIN;
            let mut land = true;
            'sample: for &gz in &lanes {
                for s in 0..=6 {
                    let gx = ox + walk * s / 6;
                    let y = world.ground_at(gx, gz).y;
                    if y < land_min {
                        land = false;
                        break 'sample;
                    }
                    lo_y = lo_y.min(y);
                    hi_y = hi_y.max(y);
                }
            }
            if land && (hi_y - lo_y) < best_spread {
                best_spread = hi_y - lo_y;
                best = Some((ox, lanes));
            }
        }
    }
    best.unwrap_or((dim / 3, [dim / 2 - 5, dim / 2, dim / 2 + 5]))
}

/// Asegura `FRAME_DIR` vacío de PNGs viejos (una vez por proceso).
fn prepare_dir() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = std::fs::create_dir_all(FRAME_DIR);
        if let Ok(rd) = std::fs::read_dir(FRAME_DIR) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "png") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    });
}
