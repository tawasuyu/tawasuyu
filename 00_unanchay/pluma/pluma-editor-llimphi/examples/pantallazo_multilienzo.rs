//! Pantallazo headless del **multilienzo** de pluma — "un texto que vive
//! en varias lenguas a la vez". Siembra el mismo modelo que
//! `multilienzo_demo` (madre en español + traducción al inglés derivada
//! por tabla + resumen), llama la `multilienzo_view` real, la monta y la
//! pinta a una textura wgpu sin ventana, y vuelca el resultado a PNG.
//!
//! Calca el patrón de `llimphi-compositor/examples/primitivas_demo.rs`:
//! view → mount → layout → paint → render_to_view → readback → PNG.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example pantallazo_multilienzo \
//!   --release -- /tmp/shots/pluma.png
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, View};
use pluma_align::{alinear_explicito, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette;
use pluma_transform::{Ejecutor, TipoTransformacion, Transformacion};
use pluma_transform_tabla::EjecutorTraducirTabla;
use uuid::Uuid;

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Mismo modelo que arman los demos interactivos, pero sin LLM ni daemon:
/// madre `es` + hija `en` derivada (hebras Derivado verdes, fuerza 1.0)
/// + resumen `es` alineado a mano (hebras Embeddings azules con fuerzas
/// variadas). Una hebra se marca stale para mostrar el trazo punteado.
struct Semilla {
    cuerpos: Vec<Cuerpo>,
    atoms: Vec<NarrativeAtom>,
    cartas: Vec<CartaHebras>,
}

fn sembrar() -> Semilla {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime tokio");

    // -- 1. Cuerpo madre `es` (el original) -------------------------------
    let textos_es = [
        "El cóndor cruzó el cielo del valle al amanecer.",
        "Las llamas pastaban entre los pastizales del altiplano.",
        "Una mujer joven tejía un telar bajo el alero.",
        "El río Apurímac descendía rugiente por las rocas.",
        "Al caer la tarde, las nubes cubrieron el sol.",
    ];
    let atoms_es: Vec<NarrativeAtom> =
        textos_es.iter().map(|t| NarrativeAtom::new(*t, "es")).collect();
    let mut es = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 100);
    for a in &atoms_es {
        es.agregar(a.id, 101);
    }

    // -- 2. Cuerpo `en` derivado por tabla (traducción 1↔1) ---------------
    let traducciones = [
        "The condor crossed the valley sky at dawn.",
        "Llamas grazed among the highland grasslands.",
        "A young woman wove a loom beneath the eaves.",
        "The Apurímac river roared down through the rocks.",
        "As evening fell, the clouds covered the sun.",
    ];
    let mut tabla: HashMap<Uuid, String> = HashMap::new();
    for (atom, tr) in atoms_es.iter().zip(traducciones.iter()) {
        tabla.insert(atom.id, (*tr).to_string());
    }
    let ejecutor = EjecutorTraducirTabla::new(tabla, "en");
    let t_en = Transformacion::nueva(
        es.id,
        Uuid::new_v4(),
        TipoTransformacion::Traducir { lengua_destino: "en".into() },
        "pantallazo",
        200,
    );
    let prod = rt
        .block_on(ejecutor.aplicar(&t_en, &es, 200))
        .expect("traducción por tabla");
    let mut en = prod.hija;
    // El ejecutor deriva el nombre de la madre ("español (original) (en)");
    // para el pantallazo lo dejamos legible.
    en.metadatos.nombre_legible = "english".to_string();
    let atoms_en = prod.atoms_nuevos;
    let mut carta_es_en = prod.carta;

    // Una hebra stale: la madre se editó después de la regeneración —
    // la UI la pinta punteada y desaturada.
    if let Some(h) = carta_es_en.hebras.get_mut(2) {
        h.fresco = false;
    }

    // -- 3. Cuerpo `resumen` (2 párrafos manuales) ------------------------
    let textos_res = [
        "Amanecer andino: cóndor, llamas y una tejedora.",
        "Al anochecer, el Apurímac ruge y las nubes tapan el sol.",
    ];
    let atoms_res: Vec<NarrativeAtom> =
        textos_res.iter().map(|t| NarrativeAtom::new(*t, "es")).collect();
    let mut resumen = Cuerpo::nuevo(
        "es",
        "resumen",
        Intencion::Resumen { palabras_objetivo: Some(20) },
        200,
    );
    for a in &atoms_res {
        resumen.agregar(a.id, 201);
    }

    // -- 4. Hebras en↔resumen explícitas (origen Embeddings) --------------
    // Tres párrafos del inglés convergen en el primer párrafo del resumen;
    // los otros dos en el segundo. Fuerzas variadas → saturación variada.
    let pares: Vec<(Uuid, Uuid, f32)> = vec![
        (atoms_en[0].id, atoms_res[0].id, 0.92),
        (atoms_en[1].id, atoms_res[0].id, 0.78),
        (atoms_en[2].id, atoms_res[0].id, 0.61),
        (atoms_en[3].id, atoms_res[1].id, 0.88),
        (atoms_en[4].id, atoms_res[1].id, 0.83),
    ];
    let carta_en_res = alinear_explicito(
        &en,
        &resumen,
        &pares,
        OrigenAlineamiento::Embeddings { modelo: "e5-small".into(), timestamp: 200 },
    );

    let mut atoms = atoms_es;
    atoms.extend(atoms_en);
    atoms.extend(atoms_res);
    Semilla {
        cuerpos: vec![es, en, resumen],
        atoms,
        cartas: vec![carta_es_en, carta_en_res],
    }
}

/// La misma composición que `multilienzo_demo::view`, más una cabecera
/// fina con el título de la app — el multilienzo real va debajo.
fn armar_view(s: &Semilla) -> View<()> {
    // Geometría ajustada al lienzo 1280×800: tres columnas más anchas
    // y átomos más altos que el default para que el texto respire.
    let cfg = MultilienzoConfig {
        altura_atom: 92.0,
        gap_atom: 14.0,
        ancho_cuerpo: 332.0,
        ancho_carril: 88.0,
        padding_top: 18.0,
        ..MultilienzoConfig::default()
    };
    let paleta = PaletaHebras::default();
    let palette = Palette::default();

    let index: IndiceAtoms = s.atoms.iter().map(|a| (a.id, a)).collect();
    let cuerpos_ref: Vec<&Cuerpo> = s.cuerpos.iter().collect();
    let cartas_ref: Vec<Option<&CartaHebras>> = s.cartas.iter().map(Some).collect();

    let interior = multilienzo_view::<()>(
        &cuerpos_ref,
        &index,
        &cartas_ref,
        &cfg,
        &paleta,
        &palette,
    );

    // Cabecera: título a la izquierda, estado a la derecha — eco de la
    // toolbar de los demos interactivos.
    let header = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
        padding: Rect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
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
            "pluma · multilienzo — un texto que vive en varias lenguas a la vez"
                .to_string(),
            15.0,
            palette.fg_text,
            Alignment::Start,
        ),
        View::<()>::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(0.3_f32), height: length(24.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "3 cuerpos · 2 cartas · hebras: derivada / embeddings / stale".to_string(),
            12.0,
            palette.fg_muted,
            Alignment::End,
        ),
    ]);

    // Centrar el multilienzo en el ancho sobrante (3·332 + 2·88 = 1172 px).
    let cuerpo_centrado = View::<()>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        justify_content: Some(taffy::JustifyContent::Center),
        // Centrado vertical también: reparte el espacio sobrante arriba y
        // abajo en lugar de dejar una banda muerta al pie.
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
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pantallazo_multilienzo.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let semilla = sembrar();
    let root = armar_view(&semilla);
    let palette = Palette::default();

    // view → layout → scene (misma secuencia que el eventloop de llimphi-ui).
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

    // Render headless: textura wgpu sin surface, readback a PNG.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-multilienzo"),
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
    eprintln!("pantallazo_multilienzo: escrito {out} ({W}x{H})");
}

/// Lee la textura de vuelta (con padding de fila wgpu) y la escribe a PNG.
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
