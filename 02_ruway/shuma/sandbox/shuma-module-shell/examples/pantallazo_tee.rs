//! Pantallazo headless de las etapas del tee **accionables** + la salida de IA
//! como bloque propio. Siembra:
//!
//! 1. Un pipe `cat | grep | sort` con captura por etapa: chips rotulados con su
//!    índice `K`, la etapa 1 desplegada con sus líneas + la **fila de acciones**
//!    (filtrar IA / copiar / guardar / explicar) que direcciona `%cN.K`.
//! 2. Un bloque de **respuesta de IA** (`:filtra`) teñido con el acento.
//!
//! `cargo run -p shuma-module-shell --example pantallazo_tee --release -- [out.png] [W] [H]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use shuma_module_shell::OutputLine;

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/shuma_tee.png".to_string());
    let w: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1100);
    let h: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(760);
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);
    state.cwd = std::path::PathBuf::from("/home/sergio/proyectos");
    let now = now_secs();

    // ── Bloque 1: pipe con captura por etapa (tee) ──────────────────────────
    let pipe_blk = 1u64;
    let mut p = OutputLine::prompt("$ cat acceso.log | grep error | sort");
    p.block = pipe_blk;
    state.output.push(p);
    // Etapa 0 (cat): muchas líneas capturadas.
    for t in [
        "10:01 GET /  200",
        "10:02 GET /api error 500",
        "10:03 GET /ok 200",
        "10:04 POST /x error 503",
    ] {
        let mut l = OutputLine::stage_stdout(0, t);
        l.block = pipe_blk;
        state.output.push(l);
    }
    // Etapa 1 (grep): sólo las que matchean.
    for t in ["10:02 GET /api error 500", "10:04 POST /x error 503"] {
        let mut l = OutputLine::stage_stdout(1, t);
        l.block = pipe_blk;
        state.output.push(l);
    }
    // Cuerpo = salida final (sort), stdout normal.
    for t in ["10:02 GET /api error 500", "10:04 POST /x error 503"] {
        let mut l = OutputLine::stdout(t);
        l.block = pipe_blk;
        state.output.push(l);
    }
    let mut n = OutputLine::notice("✔ exit 0");
    n.block = pipe_blk;
    state.output.push(n);
    state.block_started.insert(pipe_blk, now.saturating_sub(120));
    state.block_command.insert(pipe_blk, "cat acceso.log | grep error | sort".to_string());
    // Etapa 1 desplegada → se ven sus líneas + la fila de acciones.
    state.expanded_stages.insert((pipe_blk, 1));

    // ── Bloque 2: respuesta de IA (:filtra) como bloque propio (Ai) ─────────
    let ai_blk = 2u64;
    let mut ph = OutputLine::prompt("🜲 :filtra «contá los errores por ruta» ← %c1.1");
    ph.block = ai_blk;
    state.output.push(ph);
    for t in ["/api  → 1 error (500)", "/x    → 1 error (503)", "total: 2 errores"] {
        let mut l = OutputLine::ai(t);
        l.block = ai_blk;
        state.output.push(l);
    }
    state.block_started.insert(ai_blk, now.saturating_sub(20));
    state
        .block_command
        .insert(ai_blk, "🜲 :filtra «contá los errores por ruta» ← %c1.1".to_string());

    state.block_seq = ai_blk;
    state.current_block = ai_blk;
    state.input.set_text(":predice");

    if let Ok(mut g) = state.out_viewport_h.lock() {
        *g = (h as f32 - 94.0).max(50.0);
    }
    state.scroll_px = 0.0;

    let v = shuma_module_shell::view::<()>(&state, &theme, |_m| ());

    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-tee"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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
    renderer
        .render_to_view(&hal, &scene, &view, w, h, Color::from_rgba8(20, 20, 26, 255))
        .expect("render_to_view");
    write_png(&hal, &target, &out, w, h);
    eprintln!("pantallazo_tee: {out} ({w}x{h}) — {} líneas", state.output.len());
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
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
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
