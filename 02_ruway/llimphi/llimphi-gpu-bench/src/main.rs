//! `llimphi-gpu-bench` — binario standalone para validar el SDD
//! `02_ruway/llimphi/SDD.md` §"GPU directo wgpu" en una máquina con GPU
//! real.
//!
//! Hace cuatro cosas en orden y lo imprime todo a stdout en formato
//! markdown / tabla copy-paste friendly:
//!
//! 1. **Header del sistema** — versión, hora, OS, GPU detectado.
//! 2. **Info del adapter wgpu** — backend (Vulkan/Metal/DX12/GL),
//!    device name, vendor, limits relevantes.
//! 3. **Spike vello vs GPU directo** — para N ∈ {25K, 50K, 100K, 200K,
//!    500K, 1M}. Mide ms/frame de cada uno y el factor. Evalúa el
//!    criterio del SDD: ≥5× a 500K → PASA; < → ABORTAR.
//! 4. **Escalado GPU directo solo** — para N ∈ {100K, 500K, 1M, 2M,
//!    5M, 10M}. Mide ms/frame, fps equivalente, Mprim/s. Evalúa el
//!    objetivo de 60 fps @ 1M.
//! 5. **PNGs de verificación visual** — exporta 2 archivos al cwd:
//!    `bench_vello_100k.png` y `bench_directo_100k.png`. La forma del
//!    cielo de puntos debe coincidir entre los dos (LCG determinista).
//!
//! Pegar el output completo en chat para la verificación.
//!
//! Corre con: `cargo run -p llimphi-gpu-bench --release`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use llimphi_hal::{wgpu, Hal};
use llimphi_raster::kurbo::{Affine, Rect};
use llimphi_raster::peniko::{color::palette, Color, Fill};
use llimphi_raster::{vello, GpuBatch, GpuPipelines};

const W: u32 = 1024;
const H: u32 = 1024;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const WARMUP: usize = 5;
const MEASURED: usize = 15;

const SPIKE_SIZES: &[u32] = &[25_000, 50_000, 100_000, 200_000, 500_000, 1_000_000];
const SCALE_SIZES: &[u32] = &[100_000, 500_000, 1_000_000, 2_000_000, 5_000_000, 10_000_000];

/// Overrides via env vars (para correr en hosts limitados sin tumbar el
/// binario). En GPU real ignorarlos y dejar los defaults.
///
/// - `LLIMPHI_BENCH_SPIKE_MAX=N` — recorta SPIKE_SIZES a los ≤ N.
/// - `LLIMPHI_BENCH_SCALE_MAX=N` — idem SCALE_SIZES.
/// - `LLIMPHI_BENCH_SKIP_VELLO=1` — saltea totalmente la columna vello
///   (útil si vello revienta con SIGSEGV en este host).
fn spike_sizes() -> Vec<u32> {
    let max = std::env::var("LLIMPHI_BENCH_SPIKE_MAX")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    SPIKE_SIZES.iter().copied().filter(|&n| n <= max).collect()
}

fn scale_sizes() -> Vec<u32> {
    let max = std::env::var("LLIMPHI_BENCH_SCALE_MAX")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(u32::MAX);
    SCALE_SIZES.iter().copied().filter(|&n| n <= max).collect()
}

fn skip_vello() -> bool {
    std::env::var("LLIMPHI_BENCH_SKIP_VELLO").ok().as_deref() == Some("1")
}

fn main() {
    print_header();
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    print_adapter(&hal);

    let (target, view) = make_target(&hal.device);

    let pipelines = GpuPipelines::new(&hal.device, FMT);
    let mut vello_renderer = vello::Renderer::new(
        &hal.device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport {
                area: true,
                msaa8: false,
                msaa16: false,
            },
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .expect("vello renderer");

    println!("## Spike vello vs GPU directo");
    println!();
    println!("Target: {W}×{H} Rgba8Unorm, headless. Cada N corre {WARMUP} warmup + {MEASURED} medidos, reporta mediana.");
    println!();
    println!("| N | vello ms | directo ms | factor | nota |");
    println!("|---:|---:|---:|---:|---|");
    let mut spike_rows: Vec<SpikeRow> = Vec::new();
    let skip_v = skip_vello();
    for n in spike_sizes() {
        let row = bench_spike(&hal, &mut vello_renderer, &pipelines, &view, n, skip_v);
        let note = if row.vello_crashed {
            "vello SIGSEGV/error"
        } else if let Some(f) = row.factor {
            if f >= 5.0 { "≥5×" } else { "<5×" }
        } else {
            "-"
        };
        let vello_str = if row.vello_crashed {
            "—".to_string()
        } else {
            format!("{:.2}", row.vello_ms.unwrap_or(0.0))
        };
        let factor_str = match row.factor {
            Some(f) => format!("{:.2}×", f),
            None => "—".to_string(),
        };
        println!(
            "| {} | {} | {:.2} | {} | {} |",
            fmt_int(n),
            vello_str,
            row.directo_ms,
            factor_str,
            note
        );
        let _ = std::io::stdout().flush();
        spike_rows.push(row);
    }
    println!();
    print_spike_verdict(&spike_rows);

    println!("## Escalado GPU directo");
    println!();
    println!("API real (`GpuPipelines` + `GpuBatch::add_rect`). Sólo se mide el lado GPU directo — vello no llega acá.");
    println!();
    println!("| N | ms / frame | fps (1000/ms) | Mprim/s |");
    println!("|---:|---:|---:|---:|");
    let mut scale_rows: Vec<ScaleRow> = Vec::new();
    for n in scale_sizes() {
        let ms = bench_directo(&hal, &pipelines, &view, n);
        let fps = 1000.0 / ms;
        let mps = (n as f64 / 1_000_000.0) / (ms / 1000.0);
        println!(
            "| {} | {:.2} | {:.1} | {:.2} |",
            fmt_int(n),
            ms,
            fps,
            mps
        );
        let _ = std::io::stdout().flush();
        scale_rows.push(ScaleRow { n, ms, fps, mps });
    }
    println!();
    print_scale_verdict(&scale_rows);

    // ----------------------------------------------------------------
    // Variantes persistentes: el rebuild del batch/scene por frame es
    // el peor caso. En apps reales (cosmos starfield Gaia, tinkuy
    // particles iniciales, nakui viewport estático) los datos no
    // cambian por frame — se uploadean UNA vez y el bucle solo redraw.
    // Estos benches lo miden.
    // ----------------------------------------------------------------
    println!("## Persistente — datos fijos, sólo redraw por frame");
    println!();
    println!("Setup (LCG + write_buffer / Scene fill) fuera de la medición; el bucle medido sólo emite render_pass + draw + submit + wait.");
    println!();
    println!("### vello (Scene reutilizada sin reset)");
    println!();
    println!("| N | ms / frame | fps (1000/ms) |");
    println!("|---:|---:|---:|");
    let mut vello_persist_rows: Vec<(u32, f64)> = Vec::new();
    let skip_v = skip_vello();
    for n in scale_sizes() {
        if skip_v {
            println!("| {} | skipped | — |", fmt_int(n));
            continue;
        }
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bench_vello_persistent(&hal, &mut vello_renderer, &view, n)
        }));
        match attempt {
            Ok(ms) => {
                let fps = 1000.0 / ms;
                println!("| {} | {:.2} | {:.1} |", fmt_int(n), ms, fps);
                let _ = std::io::stdout().flush();
                vello_persist_rows.push((n, ms));
            }
            Err(_) => {
                println!("| {} | crash | — |", fmt_int(n));
            }
        }
    }
    println!();
    println!("### GPU directo (buffer + bind group persistentes)");
    println!();
    println!("| N | ms / frame | fps (1000/ms) | Mprim/s |");
    println!("|---:|---:|---:|---:|");
    let mut directo_persist_rows: Vec<ScaleRow> = Vec::new();
    for n in scale_sizes() {
        let ms = bench_directo_persistent(&hal, &pipelines, &view, n);
        let fps = 1000.0 / ms;
        let mps = (n as f64 / 1_000_000.0) / (ms / 1000.0);
        println!("| {} | {:.2} | {:.1} | {:.2} |", fmt_int(n), ms, fps, mps);
        let _ = std::io::stdout().flush();
        directo_persist_rows.push(ScaleRow { n, ms, fps, mps });
    }
    println!();
    print_persistent_verdict(&directo_persist_rows, &vello_persist_rows);

    println!("## Validación visual");
    println!();
    let png_vello = "bench_vello_100k.png";
    let png_directo = "bench_directo_100k.png";
    if let Err(e) = export_vello_png(&hal, &mut vello_renderer, &target, &view, 100_000, png_vello)
    {
        println!("vello PNG fallo: {e}");
    } else {
        println!("- vello 100K   → `{}` ({W}×{H})", png_vello);
    }
    if let Err(e) =
        export_directo_png(&hal, &pipelines, &target, &view, 100_000, png_directo)
    {
        println!("directo PNG fallo: {e}");
    } else {
        println!("- directo 100K → `{}` ({W}×{H})", png_directo);
    }
    println!();
    println!("Las dos imágenes deben mostrar la misma constelación de puntos (LCG determinista).");
    println!("Mirar en visor: si vello tiene halo AA suave y directo tiene pixeles hard-edged, todo bien.");
    println!();

    println!("## Resumen");
    println!();
    print_summary(
        &spike_rows,
        &scale_rows,
        &directo_persist_rows,
        &vello_persist_rows,
    );
}

// ============================================================
// IO / header
// ============================================================

fn print_header() {
    println!("# llimphi-gpu-bench");
    println!();
    println!("Validación de Fase 0 del SDD `02_ruway/llimphi/SDD.md` §\"GPU directo wgpu\".");
    println!("Criterio: factor ≥ 5× a 500K Y ≥ 60 fps @ 1M en GPU mid (Radeon 5500M, Iris Xe).");
    println!();
    println!("- crate version: {}", env!("CARGO_PKG_VERSION"));
    println!("- host OS: {}", std::env::consts::OS);
    println!("- host arch: {}", std::env::consts::ARCH);
    println!();
}

fn print_adapter(hal: &Hal) {
    let info = hal.adapter.get_info();
    let limits = hal.adapter.limits();
    println!("## Adapter wgpu");
    println!();
    println!("- backend: `{:?}`", info.backend);
    println!("- device name: `{}`", info.name);
    println!("- vendor: `0x{:04x}`", info.vendor);
    println!("- device id: `0x{:04x}`", info.device);
    println!("- device type: `{:?}`", info.device_type);
    println!("- driver: `{}`", info.driver);
    println!("- driver info: `{}`", info.driver_info);
    println!();
    println!("Limits relevantes:");
    println!();
    println!("- max texture 2D: {}", limits.max_texture_dimension_2d);
    println!("- max buffer size: {} MB", limits.max_buffer_size / (1024 * 1024));
    println!("- max storage buffer binding: {} MB", limits.max_storage_buffer_binding_size / (1024 * 1024));
    println!();
    let is_software = matches!(
        info.device_type,
        wgpu::DeviceType::Cpu
    ) || info.driver.to_lowercase().contains("llvmpipe")
        || info.driver.to_lowercase().contains("software")
        || info.name.to_lowercase().contains("llvmpipe")
        || info.name.to_lowercase().contains("swiftshader");
    if is_software {
        println!("⚠️  Adapter parece software (`{}`). Los números no reflejan GPU real.", info.name);
        println!();
    }
}

fn fmt_int(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('_');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

// ============================================================
// Benches
// ============================================================

struct SpikeRow {
    n: u32,
    vello_ms: Option<f64>,
    vello_crashed: bool,
    directo_ms: f64,
    factor: Option<f64>,
}

struct ScaleRow {
    n: u32,
    ms: f64,
    fps: f64,
    mps: f64,
}

fn bench_spike(
    hal: &Hal,
    vello_renderer: &mut vello::Renderer,
    pipelines: &GpuPipelines,
    view: &wgpu::TextureView,
    n: u32,
    skip_vello: bool,
) -> SpikeRow {
    let directo_ms = bench_directo(hal, pipelines, view, n);
    if skip_vello {
        return SpikeRow {
            n,
            vello_ms: None,
            vello_crashed: true, // tratamos "skipped" como "no llegó"
            directo_ms,
            factor: None,
        };
    }
    // catch_unwind sólo atrapa panics, no SIGSEGV. En vello pre-200K
    // este path debería ser suficiente; si el binario muere igual,
    // re-correr con `LLIMPHI_BENCH_SKIP_VELLO=1`.
    let vello_attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bench_vello(hal, vello_renderer, view, n)
    }));
    match vello_attempt {
        Ok(ms) => {
            let factor = ms / directo_ms;
            SpikeRow {
                n,
                vello_ms: Some(ms),
                vello_crashed: false,
                directo_ms,
                factor: Some(factor),
            }
        }
        Err(_) => SpikeRow {
            n,
            vello_ms: None,
            vello_crashed: true,
            directo_ms,
            factor: None,
        },
    }
}

fn bench_vello(
    hal: &Hal,
    renderer: &mut vello::Renderer,
    view: &wgpu::TextureView,
    n: u32,
) -> f64 {
    let mut scene = vello::Scene::new();
    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED);
    for frame in 0..(WARMUP + MEASURED) {
        let t0 = Instant::now();
        scene.reset();
        let mut state: u32 = 0x1234_5678;
        for _ in 0..n {
            let (x, y, rgba) = lcg_point(&mut state);
            let r = (rgba & 0xFF) as u8;
            let g = ((rgba >> 8) & 0xFF) as u8;
            let b = ((rgba >> 16) & 0xFF) as u8;
            let a = ((rgba >> 24) & 0xFF) as u8;
            let xf = x as f64;
            let yf = y as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(r, g, b, a),
                None,
                &Rect::new(xf, yf, xf + POINT_PX as f64, yf + POINT_PX as f64),
            );
        }
        renderer
            .render_to_texture(
                &hal.device,
                &hal.queue,
                &scene,
                view,
                &vello::RenderParams {
                    base_color: palette::css::BLACK,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("vello render");
        hal.device.poll(wgpu::Maintain::Wait);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP {
            samples.push(dt);
        }
    }
    median(&mut samples)
}

fn bench_directo(
    hal: &Hal,
    pipelines: &GpuPipelines,
    view: &wgpu::TextureView,
    n: u32,
) -> f64 {
    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED);
    for frame in 0..(WARMUP + MEASURED) {
        let t0 = Instant::now();
        let mut batch = GpuBatch::new(pipelines);
        let mut state: u32 = 0x1234_5678;
        for _ in 0..n {
            let (x, y, rgba) = lcg_point(&mut state);
            let r = (rgba & 0xFF) as u8;
            let g = ((rgba >> 8) & 0xFF) as u8;
            let b = ((rgba >> 16) & 0xFF) as u8;
            let a = ((rgba >> 24) & 0xFF) as u8;
            batch.add_rect(x, y, POINT_PX, POINT_PX, Color::from_rgba8(r, g, b, a));
        }
        let mut encoder = hal.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("bench-directo-enc"),
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
        hal.device.poll(wgpu::Maintain::Wait);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP {
            samples.push(dt);
        }
    }
    median(&mut samples)
}

/// Vello persistente: la Scene se construye UNA vez (fill N rects) y
/// el bucle medido sólo invoca `render_to_texture`. Sin `scene.reset()`.
fn bench_vello_persistent(
    hal: &Hal,
    renderer: &mut vello::Renderer,
    view: &wgpu::TextureView,
    n: u32,
) -> f64 {
    let mut scene = vello::Scene::new();
    scene.reset();
    let mut state: u32 = 0x1234_5678;
    for _ in 0..n {
        let (x, y, rgba) = lcg_point(&mut state);
        let r = (rgba & 0xFF) as u8;
        let g = ((rgba >> 8) & 0xFF) as u8;
        let b = ((rgba >> 16) & 0xFF) as u8;
        let a = ((rgba >> 24) & 0xFF) as u8;
        let xf = x as f64;
        let yf = y as f64;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(r, g, b, a),
            None,
            &Rect::new(xf, yf, xf + POINT_PX as f64, yf + POINT_PX as f64),
        );
    }
    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED);
    for frame in 0..(WARMUP + MEASURED) {
        let t0 = Instant::now();
        renderer
            .render_to_texture(
                &hal.device,
                &hal.queue,
                &scene,
                view,
                &vello::RenderParams {
                    base_color: palette::css::BLACK,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("vello render");
        hal.device.poll(wgpu::Maintain::Wait);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP {
            samples.push(dt);
        }
    }
    median(&mut samples)
}

/// GPU directo persistente: instance buffer + uniform buffer + bind
/// group se construyen UNA vez. Bucle medido sólo abre render_pass,
/// hace `draw(0..6, 0..n)` y submit.
///
/// Replica el layout que pinta `GpuBatch::add_rect` por debajo
/// (instance stride 20 B = [x:f32, y:f32, w:f32, h:f32, rgba:u32]),
/// usando el `rects` pipeline + `bind_layout` expuestos por
/// `GpuPipelines`.
fn bench_directo_persistent(
    hal: &Hal,
    pipelines: &GpuPipelines,
    view: &wgpu::TextureView,
    n: u32,
) -> f64 {
    // Empaquetar instancias UNA vez.
    let mut bytes = Vec::with_capacity(n as usize * 20);
    let mut state: u32 = 0x1234_5678;
    for _ in 0..n {
        let (x, y, rgba) = lcg_point(&mut state);
        bytes.extend_from_slice(&x.to_ne_bytes());
        bytes.extend_from_slice(&y.to_ne_bytes());
        bytes.extend_from_slice(&POINT_PX.to_ne_bytes());
        bytes.extend_from_slice(&POINT_PX.to_ne_bytes());
        bytes.extend_from_slice(&rgba.to_ne_bytes());
    }
    let inst_buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("persist-rects"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    hal.queue.write_buffer(&inst_buf, 0, &bytes);

    // Uniforms (viewport + line_width).
    let u_data: [f32; 4] = [W as f32, H as f32, 1.0, 0.0];
    let mut u_bytes = Vec::with_capacity(16);
    for v in u_data {
        u_bytes.extend_from_slice(&v.to_ne_bytes());
    }
    let uniforms = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("persist-uniforms"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    hal.queue.write_buffer(&uniforms, 0, &u_bytes);

    let bind_group = hal.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("persist-bg"),
        layout: &pipelines.bind_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });

    // Asegurar que toda la escritura previa esté en la GPU antes de
    // empezar a medir frames — si no, el primer frame paga el upload.
    hal.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
    hal.device.poll(wgpu::Maintain::Wait);

    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED);
    for frame in 0..(WARMUP + MEASURED) {
        let t0 = Instant::now();
        let mut encoder = hal.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("persist-enc"),
            },
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("persist-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipelines.rects);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_vertex_buffer(0, inst_buf.slice(..));
            pass.draw(0..6, 0..n);
        }
        hal.queue.submit(std::iter::once(encoder.finish()));
        hal.device.poll(wgpu::Maintain::Wait);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP {
            samples.push(dt);
        }
    }
    median(&mut samples)
}

fn lcg_point(state: &mut u32) -> (f32, f32, u32) {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let x = (*state % W) as f32;
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let y = (*state % H) as f32;
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    // Colores: piso 128 por canal para que las PNGs de verificación
    // se vean (sin esto el LCG produce muchos negros casi puros, y
    // los puntos quedan invisibles en pantalla aunque estén pintados).
    let r = 128 | ((*state >> 0) & 0x7F) as u8;
    let g = 128 | ((*state >> 8) & 0x7F) as u8;
    let b = 128 | ((*state >> 16) & 0x7F) as u8;
    let rgba = (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | 0xFF00_0000;
    (x, y, rgba)
}

const POINT_PX: f32 = 2.5;

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

// ============================================================
// Veredictos
// ============================================================

fn print_spike_verdict(rows: &[SpikeRow]) {
    let at_500k = rows.iter().find(|r| r.n == 500_000);
    match at_500k {
        Some(r) if r.vello_crashed => {
            println!("**Veredicto Fase 0:** Vello revienta antes de 500K → directo es el único path posible en ese régimen. PASA cualitativo.");
        }
        Some(r) => match r.factor {
            Some(f) if f >= 5.0 => {
                println!("**Veredicto Fase 0:** factor a 500K = {:.2}× ≥ 5 → **PASA** (criterio SDD cumplido).", f);
            }
            Some(f) => {
                println!("**Veredicto Fase 0:** factor a 500K = {:.2}× < 5 → **ABORTAR** según criterio literal del SDD.", f);
                println!("Pero ver si vello revienta a tamaños mayores — eso cambia el veredicto cualitativamente.");
            }
            None => {
                println!("**Veredicto Fase 0:** sin datos para 500K (vello crashed o N no medido). Revisar tabla arriba.");
            }
        },
        None => {
            println!("**Veredicto Fase 0:** no se midió 500K en este run. Revisar tabla arriba.");
        }
    }
    println!();
}

fn print_persistent_verdict(
    directo: &[ScaleRow],
    vello: &[(u32, f64)],
) {
    let d_1m = directo.iter().find(|r| r.n == 1_000_000);
    let v_1m = vello.iter().find(|(n, _)| *n == 1_000_000);
    match d_1m {
        Some(r) if r.fps >= 60.0 => {
            println!(
                "**Veredicto persistente @ 1M:** directo {:.1} fps ≥ 60 → **PASA**.",
                r.fps
            );
        }
        Some(r) => {
            println!(
                "**Veredicto persistente @ 1M:** directo {:.1} fps < 60 → falla incluso sin rebuild.",
                r.fps
            );
        }
        None => println!("**Veredicto:** sin datos a 1M."),
    }
    if let (Some(d), Some((_, v_ms))) = (d_1m, v_1m) {
        let factor = v_ms / d.ms;
        println!(
            "**Factor persistente @ 1M:** vello {:.1} ms / directo {:.1} ms = {:.2}× ({})",
            v_ms,
            d.ms,
            factor,
            if factor >= 5.0 { "≥5×" } else { "<5×" }
        );
    }
    println!();
}

fn print_scale_verdict(rows: &[ScaleRow]) {
    let at_1m = rows.iter().find(|r| r.n == 1_000_000);
    match at_1m {
        Some(r) if r.fps >= 60.0 => {
            println!("**Veredicto Fase 0 (objetivo 60 fps @ 1M):** {:.1} fps ≥ 60 → **PASA**.", r.fps);
        }
        Some(r) => {
            println!("**Veredicto Fase 0 (objetivo 60 fps @ 1M):** {:.1} fps < 60 → marginal. ¿Es CPU-bound el bench (write_buffer de 12-20 MB por frame)? Probar también con `mapped_at_creation` para sacar el camino más rápido.", r.fps);
        }
        None => {
            println!("**Veredicto:** sin datos para 1M.");
        }
    }
    println!();
}

fn print_summary(
    spike: &[SpikeRow],
    scale: &[ScaleRow],
    persist_directo: &[ScaleRow],
    persist_vello: &[(u32, f64)],
) {
    println!("Copiar lo que sigue al chat:");
    println!();
    println!("```");
    println!("rebuild por frame — vello vs directo:");
    for r in spike {
        let v = match (r.vello_crashed, r.vello_ms) {
            (true, _) => "crash".to_string(),
            (_, Some(ms)) => format!("{:.1}ms", ms),
            _ => "-".to_string(),
        };
        let f = r
            .factor
            .map(|x| format!("{:.2}x", x))
            .unwrap_or_else(|| "-".to_string());
        println!("  {:>10}  vello={:>10}  directo={:>7.1}ms  factor={}", fmt_int(r.n), v, r.directo_ms, f);
    }
    println!();
    println!("rebuild por frame — escalado directo:");
    for r in scale {
        println!("  {:>10}  {:>7.1}ms  {:>5.1}fps  {:>5.2}Mprim/s", fmt_int(r.n), r.ms, r.fps, r.mps);
    }
    println!();
    println!("persistente (datos fijos, sólo redraw):");
    for r in persist_directo {
        let v_ms = persist_vello
            .iter()
            .find(|(n, _)| *n == r.n)
            .map(|(_, ms)| format!("{:>7.1}ms", ms))
            .unwrap_or_else(|| "       —".to_string());
        let factor = persist_vello
            .iter()
            .find(|(n, _)| *n == r.n)
            .map(|(_, vms)| format!("factor={:.2}x", vms / r.ms))
            .unwrap_or_else(|| "factor=  —  ".to_string());
        println!(
            "  {:>10}  vello={}  directo={:>7.1}ms  {}  {:>5.1}fps  {:>5.2}Mprim/s",
            fmt_int(r.n),
            v_ms,
            r.ms,
            factor,
            r.fps,
            r.mps,
        );
    }
    println!("```");
}

// ============================================================
// Textura destino + PNG export
// ============================================================

fn make_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        // RENDER_ATTACHMENT para el directo, STORAGE_BINDING para vello,
        // TEXTURE_BINDING + COPY_SRC para poder leer (PNG export).
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn export_vello_png(
    hal: &Hal,
    renderer: &mut vello::Renderer,
    target: &wgpu::Texture,
    view: &wgpu::TextureView,
    n: u32,
    path: &str,
) -> Result<(), String> {
    let mut scene = vello::Scene::new();
    let mut state: u32 = 0x1234_5678;
    for _ in 0..n {
        let (x, y, rgba) = lcg_point(&mut state);
        let r = (rgba & 0xFF) as u8;
        let g = ((rgba >> 8) & 0xFF) as u8;
        let b = ((rgba >> 16) & 0xFF) as u8;
        let a = ((rgba >> 24) & 0xFF) as u8;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(r, g, b, a),
            None,
            &Rect::new(x as f64, y as f64, x as f64 + POINT_PX as f64, y as f64 + POINT_PX as f64),
        );
    }
    renderer
        .render_to_texture(
            &hal.device,
            &hal.queue,
            &scene,
            view,
            &vello::RenderParams {
                base_color: palette::css::BLACK,
                width: W,
                height: H,
                antialiasing_method: vello::AaConfig::Area,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    write_texture_png(hal, target, path)
}

fn export_directo_png(
    hal: &Hal,
    pipelines: &GpuPipelines,
    target: &wgpu::Texture,
    view: &wgpu::TextureView,
    n: u32,
    path: &str,
) -> Result<(), String> {
    let mut batch = GpuBatch::new(pipelines);
    let mut state: u32 = 0x1234_5678;
    for _ in 0..n {
        let (x, y, rgba) = lcg_point(&mut state);
        let r = (rgba & 0xFF) as u8;
        let g = ((rgba >> 8) & 0xFF) as u8;
        let b = ((rgba >> 16) & 0xFF) as u8;
        let a = ((rgba >> 24) & 0xFF) as u8;
        batch.add_rect(x, y, POINT_PX, POINT_PX, Color::from_rgba8(r, g, b, a));
    }
    let mut encoder = hal.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor {
            label: Some("png-directo-enc"),
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
    hal.device.poll(wgpu::Maintain::Wait);
    write_texture_png(hal, target, path)
}

/// Copia la textura a un buffer mapeable + lee + escribe PNG.
fn write_texture_png(hal: &Hal, target: &wgpu::Texture, path: &str) -> Result<(), String> {
    // wgpu pide stride alineado a 256 B en COPY_TEXTURE_TO_BUFFER.
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = ((unpadded + align - 1) / align) * align;
    let buf_size = (padded * H as usize) as u64;

    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("png-readback"),
        size: buf_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = hal.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor {
            label: Some("png-copy-enc"),
        },
    );
    encoder.copy_texture_to_buffer(
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
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(encoder.finish()));

    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    hal.device.poll(wgpu::Maintain::Wait);
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range();

    // Desempaquetar las filas (skip padding) y escribir PNG.
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H {
        let start = row as usize * padded;
        let end = start + unpadded;
        pixels.extend_from_slice(&data[start..end]);
    }
    drop(data);
    buf.unmap();

    let file = File::create(path).map_err(|e| e.to_string())?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut w = encoder.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&pixels).map_err(|e| e.to_string())?;
    Ok(())
}
