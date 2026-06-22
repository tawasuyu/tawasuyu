//! Demo headless de [`PlanarReflection`].
//!
//! Simula un mundo reflejado pintando la textura de reflexión de un color cálido
//! brillante, y dibuja una superficie de agua horizontal (`y=0`) vista en
//! ángulo rasante. Certifica **por texto**:
//!  - la reflexión es **visible** en el agua (hay píxeles cálidos = el color
//!    reflejado aparece en la superficie);
//!  - el **Fresnel** funciona: la parte lejana/rasante del agua refleja más
//!    (más cálida) que la cercana (más tinte azul).
//!
//! `cargo run -p llimphi-3d --example reflection_demo --release`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, PlanarReflection, ReflectionPlane, SurfaceParams};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 640;
const H: u32 = 400;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut refl = PlanarReflection::new(&hal.device, FMT);

    // Agua: quad horizontal en y=0, receding hacia el fondo.
    refl.set_surface_quad(
        &hal.device,
        [
            [-8.0, 0.0, 2.0],   // cerca-izq
            [8.0, 0.0, 2.0],    // cerca-der
            [8.0, 0.0, -16.0],  // lejos-der
            [-8.0, 0.0, -16.0], // lejos-izq
        ],
    );

    // Cámara baja mirando casi a ras del agua (ángulo rasante → Fresnel alto lejos).
    let camera = Camera3d {
        eye: Vec3::new(0.0, 0.9, 5.0),
        target: Vec3::new(0.0, 0.2, -6.0),
        ..Camera3d::default()
    };
    let aspect = W as f32 / H as f32;
    let plane = ReflectionPlane::horizontal_y(0.0);

    refl.prepare(&hal.device, (W, H));
    refl.upload_surface(
        &hal.queue,
        &camera.view_proj(aspect),
        &plane,
        &camera.eye.to_array(),
        &SurfaceParams::default(),
        (W, H),
        0.0,
    );

    let px = render(&hal, &mut renderer, &mut refl);
    write_png(&px, "reflection_demo.png");

    // --- Certificación ---
    // Superficie = píxel no-fondo (el fondo vello es muy oscuro).
    let mut warm = 0u64; // reflexión cálida visible
    let mut surf = 0u64;
    let (mut y_min, mut y_max) = (H as i32, 0i32);
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let i = ((y * W as i32 + x) * 4) as usize;
            let (r, g, b) = (px[i] as i32, px[i + 1] as i32, px[i + 2] as i32);
            if r.max(g).max(b) > 25 {
                surf += 1;
                y_min = y_min.min(y);
                y_max = y_max.max(y);
                if r > 120 {
                    warm += 1;
                }
            }
        }
    }
    assert!(surf > 0, "no se dibujó superficie");
    let mid = (y_min + y_max) / 2;
    // R medio en la mitad lejana (arriba) vs cercana (abajo) de la superficie.
    let (mut rt, mut nt, mut rb, mut nb) = (0f64, 0u64, 0f64, 0u64);
    for y in y_min..=y_max {
        for x in 0..W as i32 {
            let i = ((y * W as i32 + x) * 4) as usize;
            let (r, g, b) = (px[i] as i32, px[i + 1] as i32, px[i + 2] as i32);
            if r.max(g).max(b) <= 25 {
                continue;
            }
            if y < mid {
                rt += r as f64;
                nt += 1;
            } else {
                rb += r as f64;
                nb += 1;
            }
        }
    }
    let r_far = rt / nt.max(1) as f64;
    let r_near = rb / nb.max(1) as f64;

    eprintln!("reflection_demo: {W}x{H}");
    eprintln!("  superficie: {surf} px (filas {y_min}..{y_max}); reflexión cálida (r>120): {warm} px");
    eprintln!("  R medio  lejos/rasante={r_far:.1}  cerca={r_near:.1}  (Fresnel: lejos debe > cerca)");
    assert!(warm > 500, "la reflexión cálida debería verse en el agua");
    assert!(r_far > r_near + 8.0, "el Fresnel debería reflejar más en la zona rasante");
    eprintln!("  OK — reflexión planar visible + Fresnel rasante en la superficie genérica.");
}

fn render(hal: &Hal, renderer: &mut Renderer, refl: &mut PlanarReflection) -> Vec<u8> {
    let inter = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("inter"),
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
    let inter_view = inter.create_view(&wgpu::TextureViewDescriptor::default());
    let depth = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth.create_view(&Default::default());

    let base = vello::Scene::new();
    renderer
        .render_to_view(hal, &base, &inter_view, W, H, Color::from_rgba8(6, 6, 10, 255))
        .expect("render base");

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("refl") });

    // (1) Pase de reflexión: en vez de rendir un mundo reflejado, lo simulamos
    //     limpiando a un color cálido brillante (el "cielo/mundo" reflejado).
    {
        let _rp = refl.reflection_pass(
            &mut enc,
            wgpu::Color { r: 1.0, g: 0.62, b: 0.30, a: 1.0 },
        );
    }

    // (2) Pase principal: superficie reflectante sobre el fondo vello.
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("refl-main"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &inter_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        refl.draw_surface(&mut pass);
    }

    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    readback(hal, &inter)
}

fn readback(hal: &Hal, target: &wgpu::Texture) -> Vec<u8> {
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
    pixels
}

fn write_png(pixels: &[u8], path: &str) {
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(pixels).unwrap();
}
