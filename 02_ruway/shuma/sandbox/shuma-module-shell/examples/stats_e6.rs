//! Verificación headless de **`:stats` (E6)**: la telemetría local del
//! historial renderizada como sección «resumen» + tabla ordenable «por
//! comando» (el mismo widget que `ls -l`). El detector de `sections.rs`
//! reconoce el comando `:stats` y parsea las filas tab-separadas que emite
//! `apply_stats`. Acá sembramos esas líneas a mano (mismo formato que el
//! productor) para confirmar que el render llega a la tabla.
//!
//! `cargo run -p shuma-module-shell --example stats_e6 -- [out.png]`

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

const W: u32 = 1000;
const H: u32 = 460;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "stats_e6.png".to_string());
    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);

    // Bloque que emula la salida de `:stats` (formato exacto de apply_stats:
    // 1 línea de resumen sin tab + header + filas tab-separadas).
    let b = 1u64;
    let mut p = OutputLine::prompt("$ :stats");
    p.block = b;
    state.output.push(p);
    let rows = [
        "412 comandos en historial · 9 binarios distintos · 380 con código de salida · pico 14–15h UTC",
        "comando\tveces\tfallos\t%fallo\tp50ms\tp95ms\túltimo",
        "cargo\t142\t11\t7\t1840\t9200\t3m",
        "git\t98\t2\t2\t60\t180\t1m",
        "ls\t54\t0\t0\t12\t40\tahora",
        "rg\t31\t1\t3\t25\t90\t22m",
        "shuma\t18\t0\t0\t450\t1200\t2h",
        "ssh\t12\t3\t25\t820\t4100\t1d",
        "podman\t9\t1\t11\t300\t2600\t4h",
    ];
    for r in rows {
        let mut l = OutputLine::stdout(r);
        l.block = b;
        state.output.push(l);
    }
    state
        .block_command
        .insert(b, "$ :stats".to_string());

    state.block_seq = b;
    state.current_block = b;
    if let Ok(mut g) = state.out_viewport_h.lock() {
        *g = 400.0;
    }
    state.scroll_px = 0.0;

    let v = shuma_module_shell::view::<()>(&state, &theme, |_m| ());
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
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
        label: Some("stats-e6"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
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
        .render_to_view(&hal, &scene, &view, W, H, Color::from_rgba8(20, 20, 26, 255))
        .expect("render_to_view");
    write_png(&hal, &target, &out);
    eprintln!("stats_e6: {out} ({W}x{H})");
}

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
    hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
