//! Pantallazo headless del `:compara` (cotejo de pluma) en shuma: siembra dos
//! bloques con salidas parecidas, corre `:compara %c1 %c2` de verdad (vía
//! `Msg::RunLine`) y renderiza el bloque de cotejo side-by-side.
//!
//! `cargo run -p shuma-module-shell --example pantallazo_compara --release -- [out.png] [W] [H]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use shuma_module_shell::{Msg, OutputLine};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/shuma_compara.png".to_string());
    let w: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1100);
    let h: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(420);
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);
    state.cwd = std::path::PathBuf::from("/home/sergio/proyectos");

    // Dos corridas del mismo comando, con diferencias: una línea editada, una
    // agregada y una eliminada (ancla idéntica desplazada → huecos reales).
    let seed = |state: &mut shuma_module_shell::State, b: u64, prompt: &str, body: &[&str]| {
        let mut p = OutputLine::prompt(prompt);
        p.block = b;
        state.output.push(p);
        for t in body {
            let mut l = OutputLine::stdout(*t);
            l.block = b;
            state.output.push(l);
        }
        let mut n = OutputLine::notice("✔ exit 0");
        n.block = b;
        state.output.push(n);
        state.block_command.insert(b, prompt.trim_start_matches("$ ").to_string());
    };
    seed(
        &mut state,
        1,
        "$ ./deploy.sh staging",
        &["build: ok en 12.3s", "subiendo imagen v4.2", "migraciones: 3 aplicadas", "health: OK"],
    );
    seed(
        &mut state,
        2,
        "$ ./deploy.sh prod",
        &["build: ok en 11.8s", "warm cache", "subiendo imagen v4.2", "health: OK"],
    );
    state.block_seq = 2;
    state.current_block = 2;

    // SHOT_ANCHOR=1: mostrar el estado de "un clic" (bloque 1 marcado → su chip
    // dice «⇄ elegido», el del bloque 2 «⇄ vs %c1»), sin disparar el cotejo.
    // Por defecto: corré el cotejo de verdad y mostrá su bloque.
    if std::env::var("SHOT_ANCHOR").is_ok() {
        state = shuma_module_shell::update(state, Msg::CompareWith(1));
    } else {
        state = shuma_module_shell::update(state, Msg::RunLine(":compara %c1 %c2".to_string()));
    }
    state.input.set_text("");

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
        label: Some("pantallazo-compara"),
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
    eprintln!("pantallazo_compara: {out} ({w}x{h})");
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
