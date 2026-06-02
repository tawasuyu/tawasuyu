//! Volcado headless del view del shell a PNG: monta el `View` del módulo,
//! computa el layout y lo pinta a una `vello::Scene`, luego lee la textura.
//! Sirve para VER qué renderiza el shell sin levantar ventana (llvmpipe).
//!
//! `cargo run -p shuma-module-shell --example dump_shell -- [out.png]`

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
const H: u32 = 640;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "shell.png".to_string());

    // Estado del shell con un comando tipeado (para ver el resaltado) + algo
    // de historial (para ver el ghost) si lo hubiera.
    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);
    state.input.set_text("ls -la | grep foo");

    // Card de un pipe directo ya ejecutado con captura por etapa (tee):
    // la etapa 0 (`ls`) queda desplegada mostrando sus líneas intermedias,
    // y la salida final (`grep`) va al cuerpo. Verifica el render del
    // desplegable por etapa sin levantar ventana.
    use shuma_module_shell::OutputLine;
    let block = 1u64;
    let push = |state: &mut shuma_module_shell::State, mut l: OutputLine| {
        l.block = block;
        state.output.push(l);
    };
    push(&mut state, OutputLine::prompt("$ ls -la | grep foo"));
    push(&mut state, OutputLine::stage_stdout(0, "total 12"));
    push(&mut state, OutputLine::stage_stdout(0, "-rw-r--r-- 1 foo.rs"));
    push(&mut state, OutputLine::stage_stdout(0, "-rw-r--r-- 1 bar.rs"));
    push(&mut state, OutputLine::stdout("-rw-r--r-- 1 foo.rs"));
    push(&mut state, OutputLine::notice("✔ exit 0"));
    state.block_seq = block;
    state.current_block = block;
    state.expanded_stages.insert((block, 0));
    // Reprocess armado sobre este bloque → chip resaltado + banner.
    state.reprocess_source = Some(block);
    // Popup de completado abierto sobre el input.
    state.input.set_text("ca");
    state.completion = Some(shuma_line::Completion {
        kind: shuma_line::CompletionKind::Command,
        candidates: vec![
            "cargo".into(),
            "cat".into(),
            "cal".into(),
            "case".into(),
            "captoinfo".into(),
        ],
        replace_start: 0,
        replace_end: 2,
    });
    state.completion_index = 1;

    let v = shuma_module_shell::view::<()>(&state, &theme, |_m| ());

    // view → layout → scene (misma secuencia que el eventloop).
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

    // GPU headless (llvmpipe) → textura → readback → PNG.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("dump-shell"),
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
    eprintln!("dump_shell: escrito {out} ({W}x{H})");
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
