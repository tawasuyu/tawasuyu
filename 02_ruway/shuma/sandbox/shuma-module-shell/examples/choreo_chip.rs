//! Verificación headless del **chip de coreografía (A1)**: cuando una
//! secuencia repetida (`git pull → cargo build → cargo test`) supera el
//! umbral, el shell ofrece guardarla como grupo ejecutable con un chip
//! discreto sobre el input. Render del `view()` completo a PNG.
//!
//! `cargo run -p shuma-module-shell --example choreo_chip -- [out.png]`

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
const H: u32 = 420;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "choreo_chip.png".to_string());
    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);

    // Sembrar la coreografía repetida 3× (separada por otros comandos, como en
    // el uso real) directamente en `patterns` — lo que haría `refresh_patterns`
    // tras cerrar cada comando.
    let rec = |l: &str| shuma_infer::CommandRecord::parse(l, "/repo", true);
    let records = vec![
        rec("git pull"),
        rec("cargo build"),
        rec("cargo test"),
        rec("ls"),
        rec("git pull"),
        rec("cargo build"),
        rec("cargo test"),
        rec("cd /tmp"),
        rec("git pull"),
        rec("cargo build"),
        rec("cargo test"),
    ];
    state.patterns =
        shuma_infer::detect_patterns(&records, &shuma_infer::InferConfig::default());

    // Un par de bloques de fondo para que la sesión no se vea vacía.
    let mut blk = 0u64;
    let mut cmd = |state: &mut shuma_module_shell::State, blk: &mut u64, prompt: &str, body: &[OutputLine], close: &str| {
        *blk += 1;
        let b = *blk;
        let mut p = OutputLine::prompt(prompt);
        p.block = b;
        state.output.push(p);
        for l in body {
            let mut l = l.clone();
            l.block = b;
            state.output.push(l);
        }
        let mut n = OutputLine::notice(close);
        n.block = b;
        state.output.push(n);
    };
    cmd(&mut state, &mut blk, "$ git pull", &[OutputLine::stdout("Already up to date.")], "✔ exit 0");
    cmd(&mut state, &mut blk, "$ cargo build", &[OutputLine::stdout("    Finished in 2.1s")], "✔ exit 0");

    state.block_seq = blk;
    state.current_block = blk;
    if let Ok(mut g) = state.out_viewport_h.lock() {
        *g = 300.0;
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
        label: Some("choreo-chip"),
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
    eprintln!("choreo_chip: {out} ({W}x{H})");
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
