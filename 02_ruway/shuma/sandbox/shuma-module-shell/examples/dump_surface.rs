//! Volcado headless del shell con la **superficie de terminal virtualizada**
//! activa (`SHUMA_TERMINAL_SURFACE`). Verificación obligatoria del SDD: simula
//! el **viewport medido** (`out_viewport_h` sembrado) + **scroll al fondo**
//! (`scroll_px = 0`), con un flood de miles de líneas + un bloque colapsado +
//! stderr, para que cualquier bug de scroll/anchor/negro salga a la luz.
//!
//! `cargo run -p shuma-module-shell --example dump_surface -- [out.png]`

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
const H: u32 = 640;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    // Activar la superficie ANTES del primer `view()` (el flag se lee una vez).
    std::env::set_var("SHUMA_TERMINAL_SURFACE", "1");

    let out = std::env::args().nth(1).unwrap_or_else(|| "surface.png".to_string());
    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);

    let mut blk = 0u64;
    let mut cmd = |state: &mut shuma_module_shell::State,
                   blk: &mut u64,
                   prompt: &str,
                   body: &[OutputLine],
                   close: &str| {
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
        b
    };

    cmd(
        &mut state,
        &mut blk,
        "$ ls -la ~/gioser",
        &[
            OutputLine::stdout("total 248"),
            OutputLine::stdout("drwxr-xr-x  12 sergio sergio 4096 00_unanchay"),
            OutputLine::stdout("drwxr-xr-x   8 sergio sergio 4096 02_ruway"),
            OutputLine::stdout("-rw-r--r--   1 sergio sergio 11k CLAUDE.md"),
        ],
        "✔ exit 0",
    );
    cmd(
        &mut state,
        &mut blk,
        "$ cargo build -p llimphi-widget-terminal",
        &[
            OutputLine::stdout("   Compiling llimphi-widget-terminal v0.1.0"),
            OutputLine::stderr("warning: unused variable `x`"),
            OutputLine::stdout("    Finished `dev` profile in 1.89s"),
        ],
        "✔ exit 0",
    );

    // FLOOD: un find con miles de líneas en un solo bloque.
    blk += 1;
    let flood = blk;
    {
        let mut p = OutputLine::prompt("$ find / -name '*.rs'");
        p.block = flood;
        state.output.push(p);
        for i in 0..3000 {
            let mut l = OutputLine::stdout(&format!(
                "/home/sergio/gioser/02_ruway/llimphi/widgets/terminal/src/archivo_{i:05}.rs"
            ));
            l.block = flood;
            state.output.push(l);
        }
        let mut n = OutputLine::notice("✔ exit 0");
        n.block = flood;
        state.output.push(n);
    }

    let git = cmd(
        &mut state,
        &mut blk,
        "$ git status",
        &[
            OutputLine::stdout("On branch main"),
            OutputLine::stdout("Changes not staged for commit:"),
            OutputLine::stdout("  modified:   src/blocks.rs"),
        ],
        "✔ exit 0",
    );
    state.collapsed.insert(git); // colapsado: sólo header

    cmd(
        &mut state,
        &mut blk,
        "$ cat noexiste.txt",
        &[OutputLine::stderr("cat: noexiste.txt: No such file or directory")],
        "✘ exit 1",
    );
    cmd(
        &mut state,
        &mut blk,
        "$ echo listo",
        &[OutputLine::stdout("listo")],
        "✔ exit 0",
    );

    state.block_seq = blk;
    state.current_block = blk;

    // Simular el viewport medido (el painter lo pondría el frame anterior) +
    // anclado al fondo. ~520px es el alto del panel de output con este chrome.
    if let Ok(mut g) = state.out_viewport_h.lock() {
        *g = 520.0;
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
        label: Some("dump-surface"),
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
    eprintln!("dump_surface: {out} ({W}x{H}) — {} líneas en {blk} comandos", state.output.len());
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
