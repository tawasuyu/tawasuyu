//! Pantallazo headless de `wawa-explorer-llimphi` — el visor host-side del
//! DAG de Wawa.
//!
//! Monta la **view real** de la app (menubar + header con el superbloque +
//! tree del grafo direccionado por contenido a la izquierda + panel de
//! detalle con hex dump e hijos a la derecha) sobre una **imagen `.img`
//! forjada de verdad** por `build-wawa-image.sh` (la misma que arranca en
//! QEMU): se abre con `wawa_explorer_core::Disco`, se expanden las raíces
//! (manifiesto + raíz) y los primeros niveles del DAG hasta llenar el panel,
//! y se selecciona el objeto más jugoso (payload + hijos) para que el
//! detalle muestre hash completo, hex preview y el listado de hijos.
//!
//! La imagen se busca en `WAWA_IMG` (env) o en el `dist/wawa-*/wawa.img`
//! más reciente del workspace.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `pantallazo_tullpu` / `primitivas_demo`).
//!
//! `cargo run -p wawa-explorer-llimphi --example pantallazo_wawa --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos su `main.rs` real por
// `#[path]` para llamar exactamente la misma `view()` que pinta la app.
#[path = "../src/main.rs"]
mod app;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use format::Hash;
use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, App, View};
use wawa_explorer_core::Disco;

use crate::app::{raices_de, Explorer, Model, Msg};

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Filas que entran cómodas en el tree a 22 px — presupuesto de expansión.
const FILAS_MAX: usize = 30;

/// Resuelve la imagen a abrir: `WAWA_IMG` o el `dist/wawa-*/wawa.img` más
/// reciente del workspace (tres niveles arriba de este crate).
fn ruta_imagen() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("WAWA_IMG") {
        return Some(PathBuf::from(p));
    }
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../dist");
    let mut candidatas: Vec<PathBuf> = std::fs::read_dir(&dist)
        .ok()?
        .flatten()
        .map(|e| e.path().join("wawa.img"))
        .filter(|p| p.is_file())
        .collect();
    candidatas.sort();
    candidatas.pop()
}

/// Expande raíces y niveles sucesivos (BFS) sin pasarse del presupuesto de
/// filas visibles, y elige como selección el objeto con más sustancia
/// (payload + hijos) entre los visibles — el detalle muestra hex e hijos.
fn sembrar_arbol(d: &Disco, raices: &[Hash]) -> (HashSet<Hash>, Option<Hash>) {
    let mut expanded: HashSet<Hash> = HashSet::new();
    let mut visibles: Vec<Hash> = raices.to_vec();
    let mut frontera: Vec<Hash> = raices.to_vec();

    'outer: loop {
        let mut siguiente = Vec::new();
        for h in frontera {
            let Some(obj) = d.objeto(&h) else { continue };
            if obj.hijos.is_empty() || expanded.contains(&h) {
                continue;
            }
            if visibles.len() + obj.hijos.len() > FILAS_MAX {
                break 'outer;
            }
            expanded.insert(h);
            visibles.extend(obj.hijos.iter().copied());
            siguiente.extend(obj.hijos.iter().copied());
        }
        if siguiente.is_empty() {
            break;
        }
        frontera = siguiente;
    }

    let selected = visibles
        .iter()
        .filter_map(|h| d.objeto(h).map(|o| (h, o)))
        .max_by_key(|(_, o)| o.datos.len().min(256) + 64 * o.hijos.len().min(16))
        .map(|(h, _)| *h)
        .or_else(|| visibles.first().copied());
    (expanded, selected)
}

fn main() {
    rimay_localize::init();

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/wawa-explorer.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let img = ruta_imagen().expect(
        "no encontré ninguna imagen: seteá WAWA_IMG o forjá una con \
         scripts/build-wawa-image.sh",
    );
    let disco = Disco::abrir(&img).expect("abrir imagen wawa");
    let raices = raices_de(&disco);
    let (expanded, selected) = sembrar_arbol(&disco, &raices);

    // El mismo estado que la app tras abrir la imagen y explorar un rato.
    let model = Model {
        theme: Theme::dark(),
        disco: Some(disco),
        source: PathBuf::from("wawa.img"),
        error: None,
        expanded,
        selected,
        raices,
        iface: Ok("eth0".into()),
        fetched: HashMap::new(),
        fetching: HashSet::new(),
        fetch_errors: HashMap::new(),
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        context_menu: None,
    };

    let root: View<Msg> = Explorer::view(&model);

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
        label: Some("pantallazo-wawa-explorer"),
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
    let [r, g, b, _] = model.theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_wawa: escrito {out} ({W}x{H})");
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
