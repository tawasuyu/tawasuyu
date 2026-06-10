//! Pantallazo headless de `iniy-explorer-llimphi` — el explorador del
//! corpus de creencias.
//!
//! Monta la **view real** del explorer (la misma `Explorer::view`:
//! menubar + header con conteos + panel de fuentes con reputación +
//! grafo NLI central + panel de aserciones coloreadas por opinión) sobre
//! una DB SQLite sembrada **con la maquinaria real** (`iniy-store`): tres
//! documentos de fuentes distintas (revista médica, blog, wiki), once
//! aserciones con opiniones Jøsang variadas (creencia / descreencia /
//! incertidumbre dominantes + una cita atribuida), ocho relaciones NLI
//! (entailment verde / contradiction rojo) y reputaciones recalculadas
//! por `recalcular_reputaciones` — el blog sale negativo porque sus
//! afirmaciones chocan con la revista y la wiki. Una aserción queda
//! seleccionada para mostrar el anillo ámbar + halos de vecinos.
//!
//! El binario no expone lib, así que incluimos `src/main.rs` dentro de
//! un módulo y el driver vive adentro (acceso a los items privados).
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `pantallazo_tullpu` / `pantallazo_tinkuy`).
//!
//! `cargo run -p iniy-explorer-llimphi --example pantallazo_iniy --release -- [out.png]`
#![allow(dead_code)]

fn main() {
    app::pantallazo();
}

mod app {
    include!("../src/main.rs");

    use std::fs::File;
    use std::io::BufWriter;

    use iniy_core::{ChunkId, DocId, RelacionNli};
    use iniy_ingest::{Chunk, Documento};
    use llimphi_ui::llimphi_hal::{wgpu, Hal};
    use llimphi_ui::llimphi_layout::taffy;
    use llimphi_ui::llimphi_layout::LayoutTree;
    use llimphi_ui::llimphi_raster::{vello, Renderer};
    use llimphi_ui::llimphi_text::Typesetter;
    use llimphi_ui::{measure_text_node, mount, paint};

    const W: u32 = 1600;
    const H: u32 = 1000;
    const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// Atajo: opinión Jøsang válida o pánico (los literales del seed suman 1).
    fn op(b: f32, d: f32, u: f32) -> Opinion {
        Opinion::nueva(b, d, u, 0.5).expect("opinión normalizada")
    }

    /// Siembra el corpus demo en `db_path` con la API real de `iniy-store`:
    /// el mismo camino que recorren `iniy ingest` / `extract` / `nli`.
    /// Devuelve el id de la aserción que el pantallazo deja seleccionada.
    fn sembrar_corpus(db_path: &std::path::Path) -> AsercionId {
        let mut store = Store::abrir(db_path).expect("abrir DB");

        let revista = store
            .obtener_o_crear_fuente("Revista Médica Andina", Some("journal"))
            .expect("fuente");
        let blog = store
            .obtener_o_crear_fuente("Blog Detox Integral", Some("blog"))
            .expect("fuente");
        let wiki = store
            .obtener_o_crear_fuente("Wikipedia", Some("wiki"))
            .expect("fuente");
        let oms = store
            .obtener_o_crear_fuente("IARC / OMS", Some("consenso"))
            .expect("fuente");

        // Un doc = un chunk (suficiente para atribución). Cada aserción
        // lleva la opinión autoral inferida del hedging de su texto.
        let mut docs: Vec<(FuenteId, &str, Vec<(&str, Opinion)>)> = vec![
            (
                revista,
                "Café y salud — revisión 2024",
                vec![
                    ("El consumo moderado de café se asocia a menor riesgo de enfermedad hepática.", op(0.75, 0.05, 0.20)),
                    ("La cafeína eleva transitoriamente la presión arterial.", op(0.85, 0.05, 0.10)),
                    ("No hay evidencia consistente de que el café cause cáncer.", op(0.60, 0.10, 0.30)),
                    ("El café mejora el rendimiento cognitivo a corto plazo.", op(0.70, 0.10, 0.20)),
                ],
            ),
            (
                blog,
                "Los mitos del café que nadie te cuenta",
                vec![
                    ("El café es tóxico para el hígado.", op(0.80, 0.05, 0.15)),
                    ("El café causa cáncer de estómago.", op(0.70, 0.10, 0.20)),
                    ("Dejar el café desintoxica el cuerpo en tres días.", op(0.90, 0.00, 0.10)),
                ],
            ),
            (
                wiki,
                "Cafeína — artículo enciclopédico",
                vec![
                    ("La cafeína es la sustancia psicoactiva más consumida del mundo.", op(0.90, 0.02, 0.08)),
                    ("La IARC retiró al café de la lista de posibles carcinógenos en 2016.", op(0.85, 0.05, 0.10)),
                    ("Dosis superiores a 400 mg de cafeína al día pueden causar insomnio.", op(0.80, 0.05, 0.15)),
                    ("El efecto neto del café sobre la mortalidad sigue en debate.", op(0.25, 0.10, 0.65)),
                ],
            ),
        ];

        let mut ids: Vec<AsercionId> = Vec::new();
        for (fuente_id, titulo, aserciones) in docs.drain(..) {
            let doc_id = DocId::nuevo();
            let chunk_id = ChunkId::nuevo();
            let texto_chunk: String = aserciones
                .iter()
                .map(|(t, _)| *t)
                .collect::<Vec<_>>()
                .join(" ");
            let doc = Documento {
                id: doc_id,
                titulo: titulo.to_string(),
                chunks: vec![Chunk {
                    id: chunk_id,
                    doc_id,
                    orden: 0,
                    texto: texto_chunk,
                }],
            };
            store.persistir_documento(&doc, Some(fuente_id)).expect("doc");
            let lote: Vec<Asercion> = aserciones
                .into_iter()
                .map(|(texto, opinion_autoral)| Asercion {
                    id: AsercionId::nuevo(),
                    doc_id,
                    chunk_id,
                    texto: texto.to_string(),
                    opinion_autoral,
                })
                .collect();
            store.persistir_aserciones(&lote).expect("aserciones");
            ids.extend(lote.iter().map(|a| a.id));
        }

        // La aserción de la wiki sobre la IARC cita a la fuente "IARC / OMS":
        // atribución efectiva distinta del doc (card ámbar en el panel).
        store
            .asignar_fuente_citada(ids[8], Some(oms))
            .expect("cita");

        // Relaciones NLI sembradas (las que produciría `iniy nli`): el blog
        // contradice a la revista y a la wiki; la wiki apuntala a la revista.
        let rel = |e: f32, c: f32| RelacionNli {
            entailment: e,
            contradiction: c,
            neutral: (1.0 - e - c).max(0.0),
        };
        let arista = |p: usize, h: usize, r: RelacionNli| Implicacion {
            premisa: ids[p],
            hipotesis: ids[h],
            relacion: r,
        };
        // Índices: 0-3 revista · 4-6 blog · 7-10 wiki.
        let imps = vec![
            arista(4, 0, rel(0.05, 0.90)), // "tóxico para el hígado" ⊥ "menor riesgo hepático"
            arista(5, 2, rel(0.05, 0.85)), // "causa cáncer" ⊥ "no hay evidencia"
            arista(5, 8, rel(0.05, 0.80)), // "causa cáncer" ⊥ "IARC lo retiró en 2016"
            arista(8, 2, rel(0.90, 0.02)), // "IARC lo retiró" ⊨ "no hay evidencia"
            arista(6, 0, rel(0.05, 0.55)), // "desintoxica en tres días" ⊥ revista
            arista(1, 9, rel(0.65, 0.05)), // "eleva presión" ⊨ "400 mg causan insomnio"
            arista(7, 3, rel(0.55, 0.05)), // "psicoactiva más consumida" ⊨ "mejora cognición"
            arista(10, 0, rel(0.15, 0.30)), // "sigue en debate" ~ revista (débil)
        ];
        store.persistir_implicaciones(&imps).expect("implicaciones");
        store
            .recalcular_reputaciones()
            .expect("reputaciones");

        // Seleccionada: la afirmación del blog con más contradicciones.
        ids[5]
    }

    pub(super) fn pantallazo() {
        let out = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/shots/iniy.png".to_string());
        if let Some(dir) = std::path::Path::new(&out).parent() {
            std::fs::create_dir_all(dir).ok();
        }

        // DB efímera, re-sembrada en cada corrida (PNG reproducible salvo
        // por el layout force-directed, que depende de los Ulid frescos).
        let db_path = std::env::temp_dir().join("iniy-pantallazo.db");
        let _ = std::fs::remove_file(&db_path);
        let seleccionada = sembrar_corpus(&db_path);
        std::env::set_var("INIY_DB", &db_path);

        // El mismo init/update/view que el runtime real, con handle muerto.
        let handle = Handle::for_test();
        let model = Explorer::init(&handle);
        let model = Explorer::update(model, Msg::Seleccionar(seleccionada), &handle);
        let root = Explorer::view(&model);

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
                        None => taffy::prelude::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);

        let hal = pollster::block_on(Hal::new(None)).expect("hal");
        let mut renderer = Renderer::new(&hal).expect("renderer");
        let target = hal.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pantallazo-iniy"),
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
        let theme = Theme::dark();
        let [r, g, b, _] = theme.bg_app.components;
        let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
        renderer
            .render_to_view(&hal, &scene, &view, W, H, bg)
            .expect("render_to_view");

        write_png(&hal, &target, &out);
        eprintln!("pantallazo_iniy: escrito {out} ({W}x{H})");
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
}
