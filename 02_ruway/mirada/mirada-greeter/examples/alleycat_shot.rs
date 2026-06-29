//! Volcado headless del **gato de Alley Cat** para evaluar su rediseño: detecta
//! un trecho en que el gato corre (en saltitos) por el piso y vuelca N frames
//! consecutivos —zoom sobre el gato— a una tira PNG, además de imprimir las
//! stats de **brinco** como texto (hop/airborne y rango vertical), para certificar
//! la suspensión sin depender sólo de mirar la imagen.
//!
//! `cargo run -p mirada-greeter --example alleycat_shot -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[path = "../src/alleycat.rs"]
mod alleycat;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

const VW: f64 = 1600.0;
const VH: f64 = 900.0;
const CELL: u32 = 300; // lado de cada frame en la tira
const COLS: usize = 8; // frames consecutivos
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "alleycat.png".to_string());
    let bright = (255u8, 200, 120);

    let mut bg = alleycat::AlleyCatBg::new(bright);
    let dt = 1.0 / 30.0;

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");

    // ── Detectar un trecho de carrera y capturar COLS frames seguidos. ──
    // Avanzamos hasta acumular ≥10 frames seguidos con avance horizontal claro.
    let mut prev_x = bg.snapshot().body.x;
    let mut moving = 0;
    let mut warm = 0;
    while warm < 8000 {
        bg.step(dt);
        warm += 1;
        let x = bg.snapshot().body.x;
        if (x - prev_x).abs() > 1.0 {
            moving += 1;
        } else {
            moving = 0;
        }
        prev_x = x;
        if moving >= 10 {
            break;
        }
    }

    let strip_w = CELL * COLS as u32;
    let strip_h = CELL;
    let mut strip = vec![10u8, 13, 19, 255].repeat((strip_w * strip_h) as usize);

    let mut min_cy = f64::INFINITY;
    let mut max_cy = f64::NEG_INFINITY;
    let mut max_hop = 0f32;
    println!("alleycat_shot: frame  body_cy   hop   airborne");
    for c in 0..COLS {
        let snap = bg.snapshot();
        // Ventana virtual centrada en el gato (incluye cola y cabeza).
        let win = 5.5 * (VH * 0.046); // ~ varios CU de ancho
        let center = (snap.body.x, snap.body.y - 0.6 * (VH * 0.046));
        min_cy = min_cy.min(snap.body.y);
        max_cy = max_cy.max(snap.body.y);
        max_hop = max_hop.max(snap.hop);
        println!(
            "alleycat_shot: {:>4}  {:>7.1}  {:>4.2}   {:>4.2}",
            c, snap.body.y, snap.hop, snap.airborne
        );

        // xf que mapea la ventana virtual a la celda CELL×CELL.
        let scale = CELL as f64 / win;
        let rect = PaintRect {
            x: (CELL as f64 / 2.0 - center.0 * scale) as f32,
            y: (CELL as f64 / 2.0 - center.1 * scale) as f32,
            w: (VW * scale) as f32,
            h: (VH * scale) as f32,
        };

        let mut scene = vello::Scene::new();
        let mut ts = Typesetter::new();
        alleycat::paint_rig(&snap, &mut scene, &mut ts, rect, (warm + c) as f32 * dt as f32, bright);

        let cell = render_cell(&hal, &mut renderer, &scene);
        // Blit de la celda a la tira.
        for row in 0..CELL as usize {
            let dst = ((row as u32 * strip_w + c as u32 * CELL) * 4) as usize;
            let src = (row * CELL as usize * 4) as usize;
            strip[dst..dst + CELL as usize * 4].copy_from_slice(&cell[src..src + CELL as usize * 4]);
        }
        bg.step(dt);
    }

    write_png(&out, &strip, strip_w, strip_h);
    println!(
        "alleycat_shot: escrito {out} ({strip_w}x{strip_h}, {COLS} frames)\n\
         alleycat_shot: rango vertical del cuerpo = {:.1} px virtual (BOUND~{:.1}); hop máx = {:.2}",
        max_cy - min_cy,
        0.95 * (VH * 0.046 * 0.46),
        max_hop
    );
    assert!(max_cy - min_cy > 2.0, "el cuerpo debe botar (saltitos): rango {}", max_cy - min_cy);
}

/// Renderiza la escena a una textura CELL×CELL y devuelve sus píxeles RGBA.
fn render_cell(hal: &Hal, renderer: &mut Renderer, scene: &vello::Scene) -> Vec<u8> {
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cat-cell"),
        size: wgpu::Extent3d { width: CELL, height: CELL, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let bgc = Color::from_rgba8(10, 13, 19, 255);
    renderer.render_to_view(hal, scene, &view, CELL, CELL, bgc).expect("render");
    readback(hal, &target)
}

fn readback(hal: &Hal, target: &wgpu::Texture) -> Vec<u8> {
    let unpadded = (CELL * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * CELL as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
                rows_per_image: Some(CELL),
            },
        },
        wgpu::Extent3d { width: CELL, height: CELL, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut px = Vec::with_capacity((CELL * CELL * 4) as usize);
    for row in 0..CELL as usize {
        let s = row * padded;
        px.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    px
}

fn write_png(path: &str, rgba: &[u8], w: u32, h: u32) {
    let file = File::create(Path::new(path)).expect("crear png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}
