//! **Exportar una escena a video**: reproduce la [`SceneSpec`] cuadro a cuadro
//! *headless* (con su propio `Hal`, fuera del contexto GPU de la ventana) — posa
//! los actores y resuelve la cámara del plano vigente igual que el preview en vivo —
//! vuelca PNGs y los muxea a un `.mkv` con `foreign-av` (ffmpeg). Pensado para correr
//! en un worker (`Handle::spawn`): es largo y bloqueante.

use std::path::Path;

use llimphi_3d::glam::Vec3;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_voxel::{world_dim, CharSpec, Project, SceneSpec, WorldRecipe, PREVIEW_DIM_XZ};

use crate::preview::{WorldPreview, FMT};

const W: u32 = 1280;
const H: u32 = 720;
const FPS: u32 = 30;
/// Carpeta de cuadros, banda sonora y archivo de salida.
const FRAME_DIR: &str = "/tmp/voxel_studio_film";
const AUDIO: &str = "/tmp/voxel_studio_film.wav";
const OUT: &str = "/tmp/voxel_studio_film.mkv";
/// Turntable (vitrina de un mundo girando) — para el GIF del README.
const TURN_DIR: &str = "/tmp/voxel_studio_turn";
const TURN_OUT: &str = "/tmp/voxel_studio_turn.mkv";

/// **Turntable de un mundo**: orbita el relieve 360° (sin actores) y muxea a un
/// `.mkv` — la vitrina del motor voxel para el README. Headless.
pub fn turntable(recipe: &WorldRecipe) -> Result<String, String> {
    let hal = pollster::block_on(Hal::new(None)).map_err(|e| format!("gpu: {e:?}"))?;
    let mut renderer = Renderer::new(&hal).map_err(|e| format!("renderer: {e:?}"))?;
    let dim = world_dim(PREVIEW_DIM_XZ);
    let mut preview = WorldPreview::build(&hal.device, &hal.queue, recipe, dim, 1);
    let center = Vec3::new(dim[0] as f32 * 0.5, dim[1] as f32 * 0.32, dim[2] as f32 * 0.5);

    let target = make_target(&hal);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    std::fs::create_dir_all(TURN_DIR).map_err(|e| e.to_string())?;
    if let Ok(rd) = std::fs::read_dir(TURN_DIR) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "png") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    const N: u32 = 120; // 4 s @ 30 fps, una vuelta completa
    let empty = vello::Scene::new();
    let sky = Color::from_rgba8(150, 186, 224, 255);
    for f in 0..N {
        let yaw = (f as f32 / N as f32) * std::f32::consts::TAU + 0.6;
        let camera = llimphi_3d::Camera3d::orbit(center, yaw, 0.42, dim[0] as f32 * 1.5);
        renderer
            .render_to_view(&hal, &empty, &view, W, H, sky)
            .map_err(|e| format!("clear: {e:?}"))?;
        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("turn") });
        preview.render(
            &hal.device, &hal.queue, &mut enc, &view, (W, H), (0.0, 0.0, W as f32, H as f32),
            &camera,
        );
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        write_png(&hal, &target, &format!("{TURN_DIR}/frame_{f:04}.png"));
    }
    let pattern = format!("{TURN_DIR}/frame_%04d.png");
    foreign_av::encode_frames(&pattern, FPS, 30, None, TURN_OUT)
        .map_err(|e| format!("ffmpeg: {e:?}"))?;
    Ok(TURN_OUT.to_string())
}

/// Vuelo sobre un **mundo infinito** — para el GIF del README.
const FLY_DIR: &str = "/tmp/voxel_studio_fly";
const FLY_OUT: &str = "/tmp/voxel_studio_fly.mkv";

/// **Flythrough de un mundo infinito**: como el relieve es función pura de mundo,
/// regenera la ventana en un origen que avanza cada frame (el terreno scrollea sin
/// fin) mientras la cámara vuela hacia adelante, baja, mirando un poco abajo. La
/// niebla densa funde el horizonte y los bordes de la ventana. Headless → `.mkv`.
pub fn flythrough(recipe: &WorldRecipe) -> Result<String, String> {
    let hal = pollster::block_on(Hal::new(None)).map_err(|e| format!("gpu: {e:?}"))?;
    let mut renderer = Renderer::new(&hal).map_err(|e| format!("renderer: {e:?}"))?;
    let dim = [128u32, 56, 128]; // TODO eje ≤128 (tope del VoxelRenderer); la ventana scrollea en Z
    let mut preview = WorldPreview::build(&hal.device, &hal.queue, recipe, dim, 1);
    let target = make_target(&hal);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    std::fs::create_dir_all(FLY_DIR).map_err(|e| e.to_string())?;
    if let Ok(rd) = std::fs::read_dir(FLY_DIR) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "png") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    const N: u32 = 150; // 5 s @ 30 fps
    let speed = 1.5_f32; // voxels de mundo por frame
    let fog = 0.55 / dim[2] as f32; // ligera (como el turntable): el relieve se ve nítido
    let cx = dim[0] / 2;
    let empty = vello::Scene::new();
    let sky = Color::from_rgba8(150, 186, 224, 255);

    for f in 0..N {
        let cam_z = f as f32 * speed;
        // La ventana scrollea (origen avanza en Z) → el mundo fluye hacia el ojo.
        let oz = cam_z as i32;
        preview.set_window(&hal.device, &hal.queue, recipe, [0, oz], fog);

        // Vuelo bajo hacia adelante, ¡EN COORDENADAS CENTRADAS! El shader hace
        // `ro = cam_eye + dim/2` → espera el volumen centrado en el origen.
        // Cámara en el centro X, a 1/4 de la ventana en Z, mirando +Z.
        let cz_grid = dim[2] as f32 * 0.25;
        let ground = preview.ground_at(cx, cz_grid as u32).y;
        let half = Vec3::new(dim[0] as f32, dim[1] as f32, dim[2] as f32) * 0.5;
        let eye = Vec3::new(dim[0] as f32 * 0.5, ground + 14.0, cz_grid) - half;
        let mut camera = llimphi_3d::Camera3d::fly(eye, 0.0, -0.28); // mira +Z, ~16° abajo
        camera.fovy_rad = 64_f32.to_radians();

        renderer
            .render_to_view(&hal, &empty, &view, W, H, sky)
            .map_err(|e| format!("clear: {e:?}"))?;
        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("fly") });
        preview.render(
            &hal.device, &hal.queue, &mut enc, &view, (W, H), (0.0, 0.0, W as f32, H as f32),
            &camera,
        );
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        write_png(&hal, &target, &format!("{FLY_DIR}/frame_{f:04}.png"));
    }
    let pattern = format!("{FLY_DIR}/frame_%04d.png");
    foreign_av::encode_frames(&pattern, FPS, 30, None, FLY_OUT).map_err(|e| format!("ffmpeg: {e:?}"))?;
    Ok(FLY_OUT.to_string())
}

/// Textura intermedia W×H para los renders headless.
fn make_target(hal: &Hal) -> wgpu::Texture {
    hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render-target"),
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

/// Exporta `scene` (de `project`) a un video. Devuelve la ruta del `.mkv` o un
/// mensaje de error. Bloqueante.
pub fn export_scene(project: &Project, scene: &SceneSpec) -> Result<String, String> {
    let recipe = project
        .worlds
        .get(scene.world)
        .map(|w| w.recipe)
        .ok_or("la escena no tiene un mundo de fondo válido")?;

    let hal = pollster::block_on(Hal::new(None)).map_err(|e| format!("gpu: {e:?}"))?;
    let mut renderer = Renderer::new(&hal).map_err(|e| format!("renderer: {e:?}"))?;
    let dim = world_dim(PREVIEW_DIM_XZ);
    let mut preview = WorldPreview::build(&hal.device, &hal.queue, &recipe, dim, 1);

    let scripts = scene.scripts();
    let chars: Vec<CharSpec> = scene
        .actors
        .iter()
        .map(|a| project.character_or_default(a.character))
        .collect();

    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("film-target"),
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
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    prepare_dir()?;
    let frames = (scene.duration * FPS as f32).round().max(1.0) as u32;
    let empty = vello::Scene::new();
    let sky = Color::from_rgba8(150, 186, 224, 255);

    for f in 0..frames {
        let t = f as f32 / FPS as f32;

        // Limpiar el target (fondo cielo) antes de componer el mundo.
        renderer
            .render_to_view(&hal, &empty, &view, W, H, sky)
            .map_err(|e| format!("clear: {e:?}"))?;

        // Posar a los actores sobre el relieve y encuadrar al reparto.
        // El shader voxel espera la cámara/posiciones en COORDS CENTRADAS (la grilla
        // está centrada en el origen: world = grilla − dim/2). `ground_at` devuelve
        // coords de grilla → hay que restar `half`, o el terreno queda corrido fuera
        // de cuadro en los planos cercanos del guion (el reparto "flota" en el cielo).
        let half = Vec3::new(dim[0] as f32, dim[1] as f32, dim[2] as f32) * 0.5;
        let mut poses = Vec::with_capacity(scripts.len());
        let mut centroid = Vec3::ZERO;
        for (script, ch) in scripts.iter().zip(&chars) {
            // Tiempo cuantizado a la tasa propia del actor: el Héroe (12 fps) se
            // mueve y posa en doses; los demás, fluidos. Sello de animación.
            let at = script.quantize(t);
            let s = script.sample(at);
            let pos = preview.ground_at(s.gx.max(0.0) as u32, s.gz.max(0.0) as u32) - half;
            centroid += pos;
            poses.push((pos, s, ch, at));
        }
        let look = if poses.is_empty() {
            Vec3::new(0.0, dim[1] as f32 * (0.32 - 0.5), 0.0)
        } else {
            centroid / poses.len() as f32 + Vec3::new(0.0, 1.0, 0.0)
        };
        let cast_d = 6.0 + poses.len() as f32 * 1.2;
        let camera = scene.camera_at(look, cast_d, t);

        let mut metas = Vec::with_capacity(poses.len());
        for (pos, s, ch, at) in &poses {
            let mut a = ch.to_actor(*pos, s.facing);
            a.set_clip(s.clip);
            a.advance(*at);
            a.look_at(Some(camera.eye));
            let (v, i) = a.mesh();
            metas.push((a.model(), v, i));
        }

        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("film") });
        preview.render_scene(
            &hal.device, &hal.queue, &mut enc, &view, (W, H), (0.0, 0.0, W as f32, H as f32),
            &camera, &metas,
        );
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        write_png(&hal, &target, &format!("{FRAME_DIR}/frame_{f:04}.png"));
    }

    // Banda sonora sincronizada a los beats del guion (cortes + gestos).
    let beats = scene.beat_times();
    crate::soundtrack::render_to(AUDIO, &beats);

    let pattern = format!("{FRAME_DIR}/frame_%04d.png");
    foreign_av::encode_frames(&pattern, FPS, 32, Some(Path::new(AUDIO)), OUT)
        .map_err(|e| format!("ffmpeg falló ({e:?}); los PNG quedaron en {FRAME_DIR}/"))?;
    Ok(OUT.to_string())
}

/// Crea la carpeta de cuadros y borra los PNG viejos (escenas más cortas no deben
/// dejar cuadros sobrantes que ffmpeg muxearía).
fn prepare_dir() -> Result<(), String> {
    std::fs::create_dir_all(FRAME_DIR).map_err(|e| e.to_string())?;
    if let Ok(rd) = std::fs::read_dir(FRAME_DIR) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "png") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();

    use std::io::BufWriter;
    let Ok(file) = std::fs::File::create(path) else { return };
    let mut e = png::Encoder::new(BufWriter::new(file), W, H);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    if let Ok(mut w) = e.write_header() {
        let _ = w.write_image_data(&pixels);
    }
}
