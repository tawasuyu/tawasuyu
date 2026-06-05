//! Volcado de scroll: llena el output con muchas líneas y renderiza a una
//! posición de scroll dada para verificar que el área scrollea bien.
//! `cargo run -p shuma-module-shell --example dump_scroll -- <out.png> <scroll_px>`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

const W: u32 = 1000;
const H: u32 = 480;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "scroll.png".to_string());
    let scroll: f32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);

    use shuma_module_shell::OutputLine;
    // 8 comandos, cada uno con su salida → cards apiladas, mucho más alto
    // que el viewport.
    let push = |state: &mut shuma_module_shell::State, block: u64, mut l: OutputLine| {
        l.block = block;
        state.output.push(l);
    };
    for c in 0..8u64 {
        let block = c + 1;
        push(&mut state, block, OutputLine::prompt(format!("$ comando-{c} --largo")));
        for l in 0..6 {
            push(
                &mut state,
                block,
                OutputLine::stdout(format!(
                    "  línea {l} de la salida del comando {c} — texto de relleno"
                )),
            );
        }
        push(&mut state, block, OutputLine::notice("✔ exit 0"));
    }

    // Simulamos el estado estable: el painter ya midió el viewport en un
    // frame anterior. El viewport del output es la ventana (H) menos header
    // (~24+8) e input (~34+8) y banners — aproximamos a H-110.
    let viewport = (H as f32) - 110.0;
    *state.out_viewport_h.lock().unwrap() = viewport;
    state.scroll_px = scroll;

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

    let overflow = state.out_overflow.lock().map(|g| *g).unwrap_or(-1.0);
    eprintln!("dump_scroll: viewport={viewport} scroll_px={scroll} overflow={overflow}");

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-scroll"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
    eprintln!("dump_scroll: escrito {out} ({W}x{H})");
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
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    hal.device.poll(wgpu::Maintain::Wait);
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
