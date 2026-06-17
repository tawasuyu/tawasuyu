//! Pantallazo headless de `puriy` — el navegador soberano renderizando
//! una página de verdad con su propio motor.
//!
//! Arma el `Model` de demo (`demo_model`: dos spaces con pestañas y el
//! sidebar de dientes en vertical), parsea un fixture HTML+CSS rico
//! (gradientes, flexbox, grid de cards, tipografía) con `puriy-engine`
//! **offline** (`Engine::load_html`, sin red), y le entrega el `BoxTree`
//! a la pestaña activa por el MISMO `Msg::Loaded` que dispara una
//! navegación real — el chrome puebla título, status y estado de la
//! página igual que en vivo. Después llama al `view` real del navegador.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `examples/dump_container.rs`).
//!
//! `cargo run -p puriy-llimphi --example pantallazo_puriy --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint, paint_over};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{App, Handle, View};

use puriy_engine::{Engine, Viewport};
use puriy_llimphi::{demo_model, Msg, Puriy};

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Página demo — HTML+CSS que ejercita lo que el motor ya hace bien:
/// gradiente en el hero, nav flexbox, grid de cards con bordes y
/// radios, pills, código monoespaciado y footer. Sin JS ni red.
const FIXTURE: &str = r##"<!doctype html>
<html>
  <head>
    <title>tawasuyu · suite soberana</title>
    <style>
      body { margin: 0; background: #11131a; color: #e6e9f0; font-size: 15px;
             font-family: sans-serif; min-height: 760px; }
      .hero { background: linear-gradient(135deg, #1b2340, #432a55); padding: 32px 44px; }
      .hero h1 { margin: 0 0 10px 0; font-size: 34px; color: #ffffff; }
      .hero p { margin: 0; font-size: 17px; color: #b8c0dd; }
      .nav { display: flex; gap: 22px; padding: 13px 44px; background: #161a26; }
      .nav a { color: #88c0d0; text-decoration: none; font-size: 13px; }
      .nav a.activo { color: #ebcb8b; text-decoration: underline; }
      .cards { display: flex; flex-wrap: wrap; gap: 14px; padding: 24px 44px; }
      .card { box-sizing: border-box; width: 31%; background: #1a1f2e;
              border: 1px solid #2a3148; border-radius: 10px; padding: 14px 16px; }
      .card h3 { margin: 8px 0 6px 0; font-size: 16px; color: #d8dee9; }
      .card p { margin: 0; font-size: 12px; color: #9aa3bd; }
      .pill { background: #2e3650; color: #b9c4e8; font-size: 11px; padding: 2px 8px; border-radius: 8px; }
      .quote { margin: 2px 44px 22px 44px; padding: 13px 18px; background: #161b29;
               border-left: 3px solid #88c0d0; color: #aab3d0; font-size: 13px; }
      .footer { display: flex; gap: 18px; padding: 14px 44px; background: #0d0f15;
                color: #5d6582; font-size: 12px; }
    </style>
  </head>
  <body>
    <div class="nav">
      <a class="activo" href="#suite">suite</a>
      <a href="#motor">motor</a>
      <a href="#wawa">wawa</a>
      <a href="#fuentes">fuentes</a>
    </div>
    <div class="hero">
      <h1>tawasuyu</h1>
      <p>Una suite vertical soberana: kernel, identidad, motor gr&aacute;fico y este navegador — sin Chromium ni WebKit.</p>
    </div>
    <div class="cards">
      <div class="card"><span class="pill">PERCIBIR</span><h3>puriy</h3>
        <p>Motor DOM/CSS propio en Rust. Esta p&aacute;gina la parse&oacute; y layoute&oacute; puriy-engine, offline.</p></div>
      <div class="card"><span class="pill">HACER</span><h3>llimphi</h3>
        <p>Motor gr&aacute;fico Elm-loop sobre wgpu + vello. Pinta el chrome y este viewport.</p></div>
      <div class="card"><span class="pill">RA&Iacute;Z</span><h3>wawa</h3>
        <p>SO bare-metal SASOS: apps WASM aisladas, red propia, almac&eacute;n direccionado por contenido.</p></div>
      <div class="card"><span class="pill">PERCIBIR</span><h3>pluma</h3>
        <p>Documentos vivos: el texto es un DAG de &aacute;tomos; el LLM transforma, no escribe.</p></div>
      <div class="card"><span class="pill">CONOCER</span><h3>cosmos</h3>
        <p>Astrometr&iacute;a pura: ephemeris, sundial, mareas y tr&aacute;nsitos sin supersticiones de runtime.</p></div>
      <div class="card"><span class="pill">HACER</span><h3>mirada</h3>
        <p>Compositor Wayland + window manager: zonas, workspaces y multi-monitor.</p></div>
    </div>
    <div class="quote">El JS corre en QuickJS-NG compilado a WASM dentro de un sandbox wasmi — el lenguaje entero, la jaula nuestra.</div>
    <div class="footer">
      <span>tawasuyu.net</span>
      <span>cuatro cuadrantes: PERCIBIR · CONOCER · HACER · RA&Iacute;Z</span>
      <span>hecho con puriy</span>
    </div>
  </body>
</html>"##;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/puriy.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let mut model = demo_model();

    // Parse + estilo + box tree con el motor real, offline. El viewport
    // que ve la página es el de la ventana (mismo contrato que el chrome).
    let engine = Engine::new().with_viewport(Viewport {
        width: W as f32,
        height: H as f32,
        dpr: 1.0,
    });
    let url = "https://tawasuyu.net";
    let doc = engine.load_html(url, FIXTURE);

    // Entregamos el documento a la pestaña activa por el mismo `Msg::Loaded`
    // que dispara una carga real — título, status ("OK · N boxes"), inputs
    // y <details> se pueblan con el código de producción.
    let handle: Handle<Msg> = Handle::for_test();
    let (tab, gen) = (model.tabs[0].id, model.tabs[0].gen);
    model = Puriy::update(
        model,
        Msg::Loaded {
            tab,
            gen,
            final_url: url.to_string(),
            title: doc.title.clone(),
            box_tree: doc.box_tree,
            source: FIXTURE.to_string(),
            meta_refresh: None,
            scripts: Vec::new(),
        },
        &handle,
    );

    // El `view` real del navegador (mismo que pinta el eventloop).
    let v: View<Msg> = Puriy::view(&model);

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
    // Pasada vello "over" (caret v2, Fase 7.1249): nodos con `paint_over`
    // (el caret del text_input_view) pintan DESPUÉS del texto base, así el
    // caret queda encima del glifo. En el eventloop esto es una pasada final
    // tras el pase GPU; aquí, render de una sola pasada, basta con anexarla a
    // la misma `scene` después de `paint` (no la resetea, sólo agrega).
    paint_over(&mut scene, &mounted, &computed, &mut ts);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-puriy"),
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
    eprintln!("pantallazo_puriy: escrito {out} ({W}x{H})");
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
