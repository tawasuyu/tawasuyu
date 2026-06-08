//! Demo headless del HAL GPU directo — Fase 6 del SDD
//! `02_ruway/llimphi/SDD.md` §"GPU directo wgpu".
//!
//! A diferencia de `spike_gpu_directo` (que compara vello vs un pipeline
//! mock para tomar la decisión arquitectónica), este ejemplo usa
//! directamente la API pública `GpuPipelines` + `GpuBatch` sobre N
//! puntos (rects 1.2×1.2 px) sintéticos. Su rol es:
//!
//! - Documentar el uso mínimo: 8 líneas de código + uso de Color.
//! - Ejercitar el HAL sin ninguna app (sin winit, sin runtime Elm).
//! - Servir de benchmark de referencia post-implementación: tiempo
//!   total CPU+GPU para 100K / 500K / 1M / 5M rects.
//!
//! Corre con: `cargo run -p llimphi-raster --example gpu_million_points --release`.

use std::io::Write;
use std::time::Instant;

use llimphi_hal::{wgpu, Hal};
use llimphi_raster::peniko::Color;
use llimphi_raster::{GpuBatch, GpuPipelines};

const W: u32 = 1024;
const H: u32 = 1024;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const WARMUP: usize = 5;
const MEASURED: usize = 15;
const SIZES: &[u32] = &[100_000, 500_000, 1_000_000, 5_000_000];

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let pipelines = GpuPipelines::new(&hal.device, FMT);

    let (_tex, view) = make_target(&hal.device);

    println!();
    println!("gpu_million_points — GpuBatch + 3 pipelines · target {W}×{H} Rgba8Unorm");
    println!("warmup {WARMUP}, measured {MEASURED}");
    println!("  {:>10} | {:>14} | {:>14}", "N", "ms / frame", "Mprim/s");
    println!("  {:->10} + {:->14} + {:->14}", "", "", "");

    for &n in SIZES {
        let ms = bench(&hal, &pipelines, &view, n);
        let throughput = (n as f64 / 1_000_000.0) / (ms / 1000.0);
        println!("  {:>10} | {:>14.3} | {:>14.2}", n, ms, throughput);
        let _ = std::io::stdout().flush();
    }
    println!();
    println!("(en llvmpipe estos números son CPU-bound — ver Fase 0 del SDD)");
    println!();
}

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpu_million_points-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn bench(hal: &Hal, pipelines: &GpuPipelines, view: &wgpu::TextureView, n: u32) -> f64 {
    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED);
    for frame in 0..(WARMUP + MEASURED) {
        let t0 = Instant::now();
        let mut batch = GpuBatch::new(pipelines);
        let mut state: u32 = 0x1234_5678;
        for _ in 0..n {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (state % W) as f32;
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let y = (state % H) as f32;
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = ((state >>  0) & 0xFF) as f32 / 255.0;
            let g = ((state >>  8) & 0xFF) as f32 / 255.0;
            let b = ((state >> 16) & 0xFF) as f32 / 255.0;
            batch.add_rect(x, y, 1.2, 1.2, Color::new([r, g, b, 1.0]));
        }
        let mut encoder = hal.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("gpu_million_points-enc"),
            },
        );
        batch.flush(
            &hal.device,
            &hal.queue,
            &mut encoder,
            view,
            (W as f32, H as f32),
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        );
        hal.queue.submit(std::iter::once(encoder.finish()));
        hal.device.poll(wgpu::PollType::wait_indefinitely());
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP {
            samples.push(dt);
        }
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}
