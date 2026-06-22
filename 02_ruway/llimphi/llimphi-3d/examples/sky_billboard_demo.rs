//! Demo headless de [`SkyBackdrop`] + [`Billboards`].
//!
//! Compone un cielo cilíndrico (panorama con una barra verde brillante) y un
//! billboard blanco en el origen, en un pase con depth. Certifica **por texto**:
//!  - al girar el `yaw`, la barra del cielo se **desplaza horizontalmente**
//!    (muestreo cilíndrico correcto);
//!  - el billboard aparece **centrado** y **de cara a la cámara** (su cuadro
//!    blanco cubre el centro de la imagen en ambos yaws).
//!
//! `cargo run -p llimphi-3d --example sky_billboard_demo --release`

use std::fs::File;
use std::io::BufWriter;

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Billboard, Billboards, Camera3d, SkyBackdrop, SkyParams};
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

    // Cielo: 512×64, fondo azul oscuro + una barra verde brillante en u≈0.
    let (sw, sh) = (512u32, 64u32);
    let mut sky_px = vec![0u8; (sw * sh * 4) as usize];
    for y in 0..sh {
        for x in 0..sw {
            let i = ((y * sw + x) * 4) as usize;
            let bar = x < 6; // barra angosta en el borde izquierdo (u=0)
            let (r, g, b) = if bar { (40, 255, 40) } else { (12, 14, 30) };
            sky_px[i] = r;
            sky_px[i + 1] = g;
            sky_px[i + 2] = b;
            sky_px[i + 3] = 255;
        }
    }
    let mut sky = SkyBackdrop::new(&hal.device, FMT);
    sky.set_texture(&hal.device, &hal.queue, sw, sh, &sky_px);

    // Billboard: atlas 16×16 blanco opaco; un quad en el origen.
    let atlas = vec![255u8; 16 * 16 * 4];
    let mut bb = Billboards::new(&hal.device, FMT);
    bb.set_atlas(&hal.device, &hal.queue, 16, 16, &atlas);
    bb.set_billboards(
        &hal.device,
        &[Billboard {
            center: [0.0, 0.0, 0.0],
            size: [1.4, 1.4],
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
        }],
    );

    let bar0 = {
        let px = render(&hal, &mut renderer, &sky, &bb, 0.0);
        write_png(&px, "sky_billboard_yaw0.png");
        certify_frame(&px, "yaw=0.0")
    };
    let bar1 = {
        let px = render(&hal, &mut renderer, &sky, &bb, 0.5);
        write_png(&px, "sky_billboard_yaw05.png");
        certify_frame(&px, "yaw=0.5")
    };

    eprintln!("sky_billboard_demo: {W}x{H}");
    eprintln!("  barra de cielo: x@yaw0={bar0}  x@yaw0.5={bar1}  (Δ={})", (bar1 - bar0).abs());
    assert!(
        (bar1 - bar0).abs() > 20,
        "la barra del cielo debería desplazarse al girar el yaw"
    );
    eprintln!("  OK — cielo cilíndrico gira con el yaw + billboard centrado de cara a la cámara.");
}

/// Mide: (a) que el billboard blanco cubra el centro; (b) la columna x con más
/// verde (la barra de cielo). Devuelve esa columna x.
fn certify_frame(px: &[u8], tag: &str) -> i32 {
    // (a) Centro blanco (el billboard).
    let ci = ((H / 2 * W + W / 2) * 4) as usize;
    let (r, g, b) = (px[ci], px[ci + 1], px[ci + 2]);
    assert!(
        r > 200 && g > 200 && b > 200,
        "{tag}: el billboard debería pintar blanco en el centro (vi {r},{g},{b})"
    );
    // Esquina (fondo de cielo, no billboard): no debe ser blanca.
    let cor = 0usize;
    assert!(
        !(px[cor] > 200 && px[cor + 1] > 200 && px[cor + 2] > 200),
        "{tag}: la esquina no debería ser el billboard"
    );

    // (b) Columna con más verde puro (verde alto, rojo/azul bajos) = la barra.
    // Muestreamos una banda superior para no contar el billboard.
    let mut best_x = 0i32;
    let mut best = -1.0f32;
    for x in 0..W as usize {
        let mut acc = 0.0f32;
        for y in 0..(H as usize / 4) {
            let i = (y * W as usize + x) * 4;
            let (r, g, b) = (px[i] as f32, px[i + 1] as f32, px[i + 2] as f32);
            acc += (g - r).max(0.0) + (g - b).max(0.0);
        }
        if acc > best {
            best = acc;
            best_x = x as i32;
        }
    }
    best_x
}

fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    sky: &SkyBackdrop,
    bb: &Billboards,
    yaw: f32,
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
        .render_to_view(hal, &base, &inter_view, W, H, Color::from_rgba8(0, 0, 0, 255))
        .expect("render base");

    let aspect = W as f32 / H as f32;
    let camera = Camera3d::orbit(Vec3::ZERO, yaw, 0.0, 4.0);
    sky.upload(
        &hal.queue,
        &SkyParams { yaw, pitch: 0.0, fov_x: std::f32::consts::FRAC_PI_2, aspect, ..SkyParams::default() },
    );
    bb.upload(&hal.queue, aspect, &camera);

    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("sky-bb") });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky-bb-pass"),
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
        sky.draw(&mut pass); // fondo primero
        bb.draw(&mut pass); // billboards encima
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
