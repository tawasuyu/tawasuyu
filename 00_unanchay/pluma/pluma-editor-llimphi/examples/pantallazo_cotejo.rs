//! Pantallazo headless del **cotejo** de pluma — comparar dos archivos al
//! estilo multilienzo: el original a la izquierda, su versión editada a la
//! derecha, y **en el medio el lienzo de diferencias** (un resumen por
//! sección). Cada sección se tiñe por su divergencia: **verde** donde coincide,
//! virando a **rojo** cuanto más fuerte es la diferencia. Las cintas del carril
//! engrosan con la coincidencia (match = banda verde gruesa; reescritura =
//! cinta roja fina). Los tres lienzos son intercambiables como cualquier
//! multilienzo.
//!
//! Calca el harness de `pantallazo_multilienzo`: view → mount → layout → paint
//! → render headless → PNG.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example pantallazo_cotejo \
//!   --release -- /tmp/shots/cotejo.png
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Rect, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, View};
use pluma_align::CartaHebras;
use pluma_cotejo::{columna_diferencias, cotejar, IndiceAtoms as IdxCotejo, ParamsCotejo, ResumidorTextual};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{multilienzo_cotejo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras};
use pluma_editor_llimphi::Palette;
use uuid::Uuid;

const W: u32 = 1320;
const H: u32 = 820;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// El modelo del cotejo: tres cuerpos (izq, diferencias, der), sus cartas y el
/// mapa de divergencias unificado.
struct Semilla {
    cuerpos: Vec<Cuerpo>,
    atoms: Vec<NarrativeAtom>,
    cartas: Vec<CartaHebras>,
    divergencias: HashMap<Uuid, f32>,
    conteo: String,
}

fn sembrar() -> Semilla {
    // -- Documento original (izquierda) -----------------------------------
    let texto_a = [
        "Pluma es un editor de documentos como haz de cuerpos.",
        "Cada cuerpo es un lienzo del mismo material bajo otra mirada.",
        "Los párrafos se alinean uno a uno entre cuerpos.",
        "El motor gráfico se llamaba GPUI en las primeras versiones.",
        "La persistencia vive en una base sled embebida.",
    ];
    // -- Versión editada (derecha): 1 idéntico, 2 reformulados, 1 reescrito,
    //    1 párrafo agregado al final ---------------------------------------
    let texto_b = [
        "Pluma es un editor de documentos como haz de cuerpos.",
        "Cada cuerpo es un lienzo del mismo material visto desde otra intención.",
        "Los párrafos quedan alineados uno a uno entre los cuerpos del haz.",
        "Hoy todo lo gráfico corre sobre Llimphi con wgpu y vello.",
        "La persistencia vive en una base sled embebida.",
        "Un cotejo compara dos versiones sección por sección.",
    ];

    let atoms_a: Vec<NarrativeAtom> = texto_a.iter().map(|t| NarrativeAtom::new(*t, "a")).collect();
    let atoms_b: Vec<NarrativeAtom> = texto_b.iter().map(|t| NarrativeAtom::new(*t, "b")).collect();
    let mut izq = Cuerpo::nuevo("a", "original.md", Intencion::Original, 0);
    for a in &atoms_a {
        izq.agregar(a.id, 0);
    }
    let mut der = Cuerpo::nuevo("b", "editado.md", Intencion::Custom { kind: "versión".into() }, 0);
    for a in &atoms_b {
        der.agregar(a.id, 0);
    }

    // Índice para el cotejo (sólo izq + der).
    let idx_cot: IdxCotejo = atoms_a.iter().chain(atoms_b.iter()).map(|a| (a.id, a)).collect();
    let cot = cotejar(&izq, &der, &idx_cot, &ParamsCotejo::default(), 1);
    let col = columna_diferencias(&cot, &izq, &der, &idx_cot, &ResumidorTextual, 2);

    let c = cot.conteos();
    let conteo = format!(
        "{} idénticas · {} reformuladas · {} reescritas · {} agregadas · {} eliminadas",
        c.identicas, c.similares, c.divergentes, c.agregadas, c.eliminadas
    );

    // Divergencias unificadas: izq/der (del cotejo) + las del lienzo del medio.
    let mut divergencias = cot.divergencias.clone();
    divergencias.extend(col.divergencias.iter().map(|(k, v)| (*k, *v)));

    // Atoms de todos los cuerpos vivos para el índice del render.
    let mut atoms = atoms_a;
    atoms.extend(col.atoms.iter().cloned());
    atoms.extend(atoms_b);

    Semilla {
        cuerpos: vec![izq, col.cuerpo, der],
        atoms,
        cartas: vec![col.carta_izq, col.carta_der],
        divergencias,
        conteo,
    }
}

fn armar_view(s: &Semilla) -> View<()> {
    let cfg = MultilienzoConfig {
        altura_atom: 96.0,
        gap_atom: 14.0,
        ancho_cuerpo: 372.0,
        ancho_carril: 88.0,
        padding_top: 16.0,
        ..MultilienzoConfig::default()
    };
    let paleta = PaletaHebras::default();
    let palette = Palette::default();

    let index: IndiceAtoms = s.atoms.iter().map(|a| (a.id, a)).collect();
    let cuerpos_ref: Vec<&Cuerpo> = s.cuerpos.iter().collect();
    let cartas_ref: Vec<Option<&CartaHebras>> = s.cartas.iter().map(Some).collect();

    let interior = multilienzo_cotejo_view::<()>(
        &cuerpos_ref,
        &index,
        &cartas_ref,
        &s.divergencias,
        &cfg,
        &paleta,
        &palette,
        "",
    );

    let header = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(46.0_f32) },
        padding: Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(11.0_f32),
            bottom: length(11.0_f32),
        },
        gap: Size { width: length(16.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(vec![
        View::<()>::new(Style {
            size: Size { width: length(560.0_f32), height: length(24.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "pluma · cotejo — dos versiones, la diferencia por sección en el medio".to_string(),
            15.0,
            palette.fg_text,
            Alignment::Start,
        ),
        View::<()>::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(0.3_f32), height: length(24.0_f32) },
            ..Default::default()
        })
        .text_aligned(s.conteo.clone(), 12.0, palette.fg_muted, Alignment::End),
    ]);

    let cuerpo_centrado = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        justify_content: Some(taffy::JustifyContent::Center),
        align_items: Some(taffy::AlignItems::Center),
        ..Default::default()
    })
    .children(vec![interior]);

    View::<()>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_app)
    .clip(true)
    .children(vec![header, cuerpo_centrado])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "pantallazo_cotejo.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let semilla = sembrar();
    let root = armar_view(&semilla);
    let palette = Palette::default();

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
        label: Some("pantallazo-cotejo"),
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
    let [r, g, b, _] = palette.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_cotejo: escrito {out} ({W}x{H}) · {}", semilla.conteo);
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
