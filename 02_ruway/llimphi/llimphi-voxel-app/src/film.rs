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

use llimphi_3d::glam::Vec3;
use llimphi_3d::{CamKey, CameraTrack, Renderer3d};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_voxel::Actor;

use crate::world::{World, FMT};
use crate::{DIM_XZ, SEED};

/// Resolución y cadencia del film (16:9, 30 fps).
const W: u32 = 960;
const H: u32 = 540;
const FPS: u32 = 30;
/// Duración en segundos (→ `FPS·SECS` cuadros).
const SECS: f32 = 4.0;
/// Carpeta de cuadros y salida del video.
const FRAME_DIR: &str = "/tmp/voxel_film";
const OUT: &str = "/tmp/voxel_film.mkv";

/// Paleta del reparto (piel, remera, pantalón) — tres figuras distinguibles.
const CAST: [([f32; 3], [f32; 3], [f32; 3]); 3] = [
    ([0.90, 0.72, 0.58], [0.82, 0.28, 0.26], [0.20, 0.20, 0.28]), // remera roja
    ([0.86, 0.68, 0.54], [0.22, 0.55, 0.78], [0.18, 0.20, 0.24]), // remera azul
    ([0.92, 0.78, 0.62], [0.92, 0.80, 0.30], [0.26, 0.22, 0.20]), // remera amarilla
];

/// Filma la escena y escribe el video. Imprime el progreso por stderr.
pub fn film() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut world = World::build(&hal.device, &hal.queue, DIM_XZ, SEED);

    // --- Reparto: tres figuras en fila, caminando a lo largo de +X sobre el
    // relieve. Las posiciones de grilla las posa `World::ground_at` por frame.
    let walk_speed = 7.0_f32; // voxels/seg de avance
    // Busca un tramo de **tierra firme** (sobre el nivel del mar) para que no
    // caminen sobre el agua: el casting elige dónde rodar.
    let (gx0, lanes) = find_land_strip(&world, walk_speed * SECS);
    let gx0 = gx0 as f32;
    let mut cast: Vec<Actor> = lanes
        .iter()
        .zip(CAST)
        .map(|(&gz, (skin, shirt, pants))| {
            let pos = world.ground_at(gx0 as u32, gz);
            Actor::new(pos, std::f32::consts::FRAC_PI_2).with_colors(skin, shirt, pants) // mira a +X
        })
        .collect();
    // Un `Renderer3d` por actor (su malla se re-sube cada frame).
    let mut actor_r: Vec<Renderer3d> = cast.iter().map(|_| Renderer3d::new(&hal.device, FMT)).collect();

    // --- Cámara guionada: encuadra el centro del reparto a media altura del
    // cuerpo, con una grúa que baja y entra y luego un travelling lateral.
    let focus = world.ground_at((gx0 as u32) + 8, lanes[1]) + Vec3::new(6.0, 1.1, 0.0);
    let track = CameraTrack::new(vec![
        CamKey::look(0.0, focus + Vec3::new(-26.0, 16.0, -30.0), focus, 50.0),
        CamKey::look(2.0, focus + Vec3::new(-10.0, 5.0, -18.0), focus, 42.0),
        CamKey::look(SECS, focus + Vec3::new(16.0, 4.0, -14.0), focus, 42.0),
    ]);

    // --- Textura intermedia reusada por todos los cuadros.
    let inter = make_target(&hal);
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    prepare_dir();
    let frames = (FPS as f32 * SECS) as u32;
    let dt = 1.0 / FPS as f32;
    for f in 0..frames {
        let t = f as f32 / FPS as f32;
        let gx = gx0 + walk_speed * t;

        // Actores: avanzan en grilla, se posan sobre el suelo, caminan.
        for (a, &gz) in cast.iter_mut().zip(lanes.iter()) {
            a.pos = world.ground_at(gx as u32, gz);
            a.advance(dt, true);
        }
        for (a, r) in cast.iter().zip(actor_r.iter_mut()) {
            let (v, i) = a.mesh();
            r.set_geometry(&hal.device, &v, &i);
            r.set_model(a.model());
        }

        world.tick(dt); // la manada de fondo deambula
        world.animate(t * 0.5); // el monumento gira
        let camera = track.sample(t);

        let refs: Vec<&Renderer3d> = actor_r.iter().collect();
        render_frame(&hal, &mut renderer, &mut world, &camera, &refs, &inter, &inter_view);
        crate::write_png(&hal, &inter, W, H, &frame_path(f));
        if f % 15 == 0 {
            eprintln!("film: cuadro {f}/{frames}");
        }
    }

    // --- Muxeo a video. Si no hay ffmpeg, deja igual los PNG y avisa.
    let pattern = format!("{FRAME_DIR}/frame_%04d.png");
    match foreign_av::encode_frames(&pattern, FPS, 30, None, OUT) {
        Ok(()) => eprintln!("film: video escrito {OUT} ({frames} cuadros, {W}x{H}@{FPS})"),
        Err(e) => eprintln!(
            "film: cuadros en {FRAME_DIR}/ pero ffmpeg falló ({e:?}); \
             podés muxear a mano: ffmpeg -framerate {FPS} -i {pattern} -c:v libsvtav1 {OUT}"
        ),
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
    inter: &wgpu::Texture,
    inter_view: &wgpu::TextureView,
) {
    let _ = inter;
    renderer
        .render_to_view(hal, &vello::Scene::new(), inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("base");
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("film") });
    world.render_with(&hal.device, &hal.queue, &mut enc, inter_view, (W, H), camera, actors);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
}

/// Crea la textura intermedia (mismo descriptor que el modo `--shot`).
fn make_target(hal: &Hal) -> wgpu::Texture {
    hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("film-inter"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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

/// Busca un origen `(gx0, [carriles Z])` tal que el tramo que recorrerán los
/// actores (`gx0 .. gx0+walk` en cada carril) caiga sobre **tierra firme** (por
/// encima del nivel del mar), para que la escena no muestre figuras caminando
/// sobre el agua. Si no encuentra (mundo todo agua), cae a un default central.
fn find_land_strip(world: &World, walk: f32) -> (u32, [u32; 3]) {
    // Nivel del mar en Y de **mundo** (centrado): el terreno arma el agua a
    // `0.30·dy` y el mundo está centrado restando `dy/2` → `(0.30−0.5)·dy`.
    let dy = (DIM_XZ * 4 / 10).max(48) as f32;
    let land_min = (0.30 - 0.5) * dy + 2.0; // margen: bien sobre la orilla
    let walk = walk.ceil() as u32;
    let dim = DIM_XZ;
    let is_land = |gx: u32, gz: u32| world.ground_at(gx, gz).y > land_min;
    let lo = 12u32;
    let hi = dim.saturating_sub(12);
    for oz in (lo..hi.saturating_sub(12)).step_by(6) {
        let lanes = [oz, oz + 6, oz + 12];
        for ox in (lo..hi.saturating_sub(walk)).step_by(6) {
            let mid = ox + walk / 2;
            let end = ox + walk;
            if lanes
                .iter()
                .all(|&gz| is_land(ox, gz) && is_land(mid, gz) && is_land(end, gz))
            {
                return (ox, lanes);
            }
        }
    }
    (dim / 3, [dim / 2 - 6, dim / 2, dim / 2 + 6]) // fallback
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
