//! Pantallazo headless de shuma con el **workspace tipo zellij** activo.
//!
//! Monta la `view()` real de la app (chasis completo: menubar, topbar, canvas
//! con tabs + tiling + flotantes, bottombar) sobre un `Model` sembrado vía la
//! API pública (`new_model` + `update` con `Handle::for_test`): la sesión
//! activa tiene tres paneles tiled, un panel flotante encima y una segunda tab.
//! Así el shot certifica lo que los tests no pueden — que el layout realmente
//! pinta paneles lado a lado + el flotante superpuesto + la barra de tabs.
//!
//! `cargo run -p shuma-shell-llimphi --example pantallazo_shuma --release -- [out.png]`
#![allow(dead_code)]

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, Handle};
use llimphi_widget_panes::Axis;
use shuma_shell_llimphi::{new_model, update, view, Msg};

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() {
    rimay_localize::init();
    let _ = rimay_localize::set_locale("es");

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/shuma.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    // Sembrar el workspace por la API pública: tres paneles tiled + un flotante
    // en la tab 1, y una segunda tab. Volvemos a la tab 1 para que el canvas la
    // muestre con todo el layout.
    let handle = Handle::<Msg>::for_test();
    let mut model = new_model();
    model = update(model, Msg::PaneSplit(Axis::Horizontal), &handle); // 2 paneles
    model = update(model, Msg::PaneSplit(Axis::Vertical), &handle); // 3 paneles
    model = update(model, Msg::FloatNew, &handle); // + flotante (toma foco)
    model = update(model, Msg::TabNew, &handle); // segunda tab
    model = update(model, Msg::TabSwitch(0), &handle); // volver a la 1ª

    let root = view(&model);

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

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-shuma"),
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
    let tview = target.create_view(&wgpu::TextureViewDescriptor::default());
    let bg = Color::from_rgba8(0x12, 0x14, 0x18, 255);
    renderer
        .render_to_view(&hal, &scene, &tview, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_shuma: escrito {out} ({W}x{H})");
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
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map").expect("map ok");
    let data = slice.get_mapped_range();

    let mut rgba = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let start = row * padded;
        rgba.extend_from_slice(&data[start..start + unpadded]);
    }
    drop(data);
    buf.unmap();

    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(&rgba).expect("png data");
}
