//! Render headless del panel de chat con el **indicador de voz** en cada estado
//! de escucha, para verificar el botón de micrófono (halo «cava» animado) y el
//! glow del input sin abrir ventana. Es el caso que la Regla 8 permite mirar:
//! un efecto visual nuevo que no se certifica de otra forma.
//!
//! ```sh
//! cargo run -p shuma-module-agente --example mic_estados
//! # → /tmp/mic_<estado>.png (uno por estado)
//! ```

use shuma_agente::Agente;
use shuma_module_agente::{view, EstadoEscucha, Msg, State};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint};

const W: u32 = 960;
const H: u32 = 560;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "/tmp".to_string());
    // Cada estado en una fase distinta del reloj para que el halo se vea a media
    // expansión (no siempre en r=0).
    let estados = [
        ("apagado", EstadoEscucha::Apagado, 0u64),
        ("esperando", EstadoEscucha::Esperando, 400),
        ("oyendo", EstadoEscucha::Oyendo, 300),
        ("despierto", EstadoEscucha::Despierto, 250),
        ("dictando", EstadoEscucha::Dictando, 200),
        ("enrolando", EstadoEscucha::Apagado, 1), // reloj==1 dispara el modo enrolar
    ];
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    for (nombre, escucha, reloj) in estados {
        let out = format!("{dir}/mic_{nombre}.png");
        render_estado(&hal, &mut renderer, escucha, reloj, &out);
        eprintln!("mic_estados: {out} ({escucha:?})");
    }
}

fn render_estado(hal: &Hal, renderer: &mut Renderer, escucha: EstadoEscucha, reloj: u64, out: &str) {
    let theme = Theme::dark();
    let mut state = State::new();
    state.set_agentes(vec![Agente::nuevo("shuma")]);
    // Alto del hilo acotado para que la barra de input (con el micrófono) quede
    // dentro del canvas, no empujada abajo.
    state.fijar_vista_alto((H as f32) - 80.0);
    state.fijar_reloj(reloj);
    state.fijar_escucha(escucha);
    if escucha == EstadoEscucha::Dictando {
        // Mostramos texto dictado en el input.
        state = shuma_module_agente::update(state, Msg::Dictado("abrí cosmos".into()));
    }
    // Estado especial «enrolando»: la palabra Apagado + un enrolamiento a 1/3.
    if matches!(escucha, EstadoEscucha::Apagado) && reloj == 1 {
        state = shuma_module_agente::update(state, Msg::EnrolarWake);
        state = shuma_module_agente::update(state, Msg::EnrolarCapturado);
    }

    let root = view(&state, &theme, |m: Msg| m);

    // view → layout → scene (misma secuencia que el eventloop real).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
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

    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mic-estados"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let texview = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer.render_to_view(hal, &scene, &texview, W, H, bg).expect("render_to_view");

    write_png(hal, &target, out);
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
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();

    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let start = row * padded;
        pixels.extend_from_slice(&data[start..start + unpadded]);
    }
    drop(data);
    buf.unmap();

    let file = std::fs::File::create(path).expect("crear png");
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
}
