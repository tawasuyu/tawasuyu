//! Demo headless de las **point-lights en el forward-mesh** ([`Renderer3d`]).
//!
//! Pinta un muro plano gris (un quad en el plano XY) con una luz puntual cálida
//! desplazada al `+X`. Certifica **por texto** (sin mirar el PNG):
//!  - sin luces + ambiente 1.0 el render es plano (mitad izq ≈ der) →
//!    compatibilidad hacia atrás;
//!  - con la luz, la mitad derecha (cerca de la luz) es más brillante que la
//!    izquierda y el píxel más iluminado queda **cálido** (r > b) = el tinte de
//!    la luz.
//!
//! `cargo run -p llimphi-3d --example mesh_lights_demo --release`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, PointLight, Renderer3d, Vertex3d};
use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{vello, Renderer};

const W: u32 = 720;
const H: u32 = 480;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // Muro: quad en el plano XY (z=0), gris medio, mirando a +Z (hacia la cámara).
    let g = [0.5, 0.5, 0.5];
    let verts = vec![
        Vertex3d { pos: [-2.4, -1.6, 0.0], color: g },
        Vertex3d { pos: [2.4, -1.6, 0.0], color: g },
        Vertex3d { pos: [2.4, 1.6, 0.0], color: g },
        Vertex3d { pos: [-2.4, 1.6, 0.0], color: g },
    ];
    let indices = vec![0u16, 1, 2, 0, 2, 3];
    let mut r3d = Renderer3d::with_mesh(&hal.device, FMT, &verts, &indices);

    let camera = Camera3d {
        eye: Vec3::new(0.0, 0.0, 5.0),
        target: Vec3::ZERO,
        ..Camera3d::default()
    };

    // (A) Sin luces, ambiente pleno → debe quedar plano (control).
    r3d.lights.clear();
    r3d.ambient = [1.0, 1.0, 1.0];
    let flat = render(&hal, &mut renderer, &mut r3d, &camera);
    write_png(&flat, "mesh_lights_flat.png");

    // (B) Una luz cálida al +X, ambiente bajo.
    r3d.ambient = [0.08, 0.08, 0.10];
    r3d.lights = vec![PointLight {
        pos: [1.7, 0.0, 1.3],
        color: [1.7, 1.05, 0.5], // cálida (naranja), > 1 para brillo intenso
        range: 5.5,
        radius: 0.0, // sin sombras en el forward-mesh
    }];
    let lit = render(&hal, &mut renderer, &mut r3d, &camera);
    write_png(&lit, "mesh_lights_lit.png");

    let (fl, fr, _, _) = halves(&flat);
    let (ll, lr, warm_r, warm_b) = halves(&lit);
    eprintln!("mesh_lights_demo: {W}x{H}");
    eprintln!("  CONTROL (sin luces): lum izq={fl:.1} der={fr:.1} (Δ={:+.1}, debe ~0)", fr - fl);
    eprintln!("  CON LUZ +X:          lum izq={ll:.1} der={lr:.1} (Δ={:+.1}, debe >>0)", lr - ll);
    eprintln!("  píxel más brillante: r={warm_r:.0} b={warm_b:.0} (r>b ⇒ tinte cálido)");
    assert!((fr - fl).abs() < 2.0, "sin luces el muro debería ser plano");
    assert!(lr - ll > 8.0, "la luz +X debería iluminar más la mitad derecha");
    assert!(warm_r > warm_b + 10.0, "la luz cálida debería dar r > b");
    eprintln!("  OK — point-lights iluminan el forward-mesh con dirección y tinte.");
}

/// Devuelve (lum_media_izq, lum_media_der, r_del_pixel_mas_brillante, b_idem).
fn halves(px: &[u8]) -> (f32, f32, f32, f32) {
    let (mut sl, mut nl, mut sr, mut nr) = (0f64, 0u64, 0f64, 0u64);
    let (mut best, mut br, mut bb) = (-1f32, 0f32, 0f32);
    for y in 0..H as usize {
        for x in 0..W as usize {
            let i = (y * W as usize + x) * 4;
            let (r, gg, b) = (px[i] as f32, px[i + 1] as f32, px[i + 2] as f32);
            let l = 0.2126 * r + 0.7152 * gg + 0.0722 * b;
            if x < W as usize / 2 {
                sl += l as f64;
                nl += 1;
            } else {
                sr += l as f64;
                nr += 1;
            }
            if l > best {
                best = l;
                br = r;
                bb = b;
            }
        }
    }
    (
        (sl / nl as f64) as f32,
        (sr / nr as f64) as f32,
        br,
        bb,
    )
}

fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    r3d: &mut Renderer3d,
    camera: &Camera3d,
) -> Vec<u8> {
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

    let base = vello::Scene::new();
    renderer
        .render_to_view(hal, &base, &inter_view, W, H, Color::from_rgba8(8, 8, 12, 255))
        .expect("render base");

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mesh-lights") });
    r3d.render(&hal.device, &hal.queue, &mut enc, &inter_view, (W, H), camera);
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
