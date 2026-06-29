//! Pantallazo headless de `paloma` con el **switcher de cuentas** en caliente.
//!
//! Monta la `view()` REAL del frontend sobre un `MockBackend` sembrado y le
//! engancha un proveedor de cuentas con dos cuentas, para que el panel izquierdo
//! muestre el switcher (CUENTAS · una fila por cuenta, la activa con ●) sobre la
//! lista de buzones (BUZONES). Rasteriza a una textura wgpu sin abrir ventana y
//! vuelca PNG (cae a software si no hay GPU).
//!
//! `cargo run -p paloma-llimphi --example pantallazo_paloma --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint};

use paloma_core::{Address, MailBackend};
use paloma_llimphi::{AccountProvider, ConnectedAccount, Model};

const W: u32 = 1180;
const H: u32 = 720;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Proveedor de cuentas de demostración: la activa («demo», la de `Model::new`)
/// + una segunda. `connect` no se ejercita en el render estático.
struct DemoAccounts;
impl AccountProvider for DemoAccounts {
    fn accounts(&self) -> Vec<(String, String)> {
        vec![
            ("demo".into(), "Sergio <sergio@jlsoltech.com>".into()),
            ("trabajo".into(), "Trabajo <s@jls.com>".into()),
        ]
    }
    fn connect(&self, _id: &str) -> Result<ConnectedAccount, String> {
        Err("pantallazo estático: sin red".into())
    }
}

fn demo_backend() -> Box<dyn MailBackend> {
    Box::new(paloma_llimphi::demo::backend())
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/shots/paloma.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }
    rimay_localize::init();
    let _ = rimay_localize::set_locale("es-PE");

    let theme = Theme::dark();
    let me = Address::named("Sergio", "sergio@jlsoltech.com");
    let mut model = Model::new(demo_backend(), me, theme.clone());
    // Engancha el switcher: con ≥2 cuentas, el panel izquierdo lo muestra.
    model.attach_accounts(std::sync::Arc::new(DemoAccounts));

    let root = paloma_llimphi::view(&model);

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

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-paloma"),
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
    let bg = Color::from_rgba8(22, 26, 34, 255);
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_paloma: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
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
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
