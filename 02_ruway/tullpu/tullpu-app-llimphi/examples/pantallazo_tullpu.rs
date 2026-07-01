//! Pantallazo headless de `tullpu` — editor de imágenes por capas IA-able.
//!
//! Monta la **view real** de la app (menubar + header + panel de capas +
//! lienzo compuesto + panel de operaciones) con un documento sembrado
//! creíble: siete capas sobre un atardecer procedural — cielo en gradiente,
//! sol con blend Pantalla, un halo derivado por `Blur` y un resplandor
//! derivado por la op IA `Restyle` (regenerados de verdad vía
//! `regenerar_stale_con_ia` + `ProveedorMock`), montañas en silueta, un
//! trazo de pincel con máscara degradada (capa seleccionada) y una viñeta
//! en Multiplicar. El composite lo arma el compositor real
//! (`tullpu-render::componer`); el marquee de selección y el histograma
//! salen del mismo painter que usa la app. Nada depende de la hora actual.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `pantallazo_agora` / `pantallazo_mapa`).
//!
//! `cargo run -p tullpu-app-llimphi --example pantallazo_tullpu --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos sus módulos reales por
// `#[path]` para llamar exactamente los mismos paneles que pinta la app.
#[path = "../src/blend.rs"]
mod blend;
#[path = "../src/carga.rs"]
mod carga;
#[path = "../src/compose.rs"]
mod compose;
#[path = "../src/historial.rs"]
mod historial;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/ops.rs"]
mod ops;
#[path = "../src/texto.rs"]
mod texto;
#[path = "../src/view.rs"]
mod view;
#[path = "../src/viewport.rs"]
mod viewport;

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_clipboard::SystemClipboard;
use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, View};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use pixel_verbo_core::{OpPixel, Proveedor};
use pixel_verbo_mock::ProveedorMock;
use tullpu_core::{Capa, Historial, Lienzo, ModoFusion, OpLocal, TransformacionPixel};
use tullpu_ops::transformacion_ia;
use tullpu_render::AlmacenEnMemoria;

use crate::compose::aplicar_y_recomponer;
use crate::model::{Herramienta, Model, Msg, RectImagen, Simetria, HIST_CAP};
use crate::view::{header, panel_capas, panel_lienzo, panel_ops};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Dimensiones del documento sembrado (el lienzo de la imagen, no la ventana).
const LW: u32 = 720;
const LH: u32 = 450;

// =============================================================================
//  Buffers procedurales del documento demo (deterministas, sin reloj)
// =============================================================================

/// Cielo de atardecer: gradiente vertical índigo → naranja hacia el horizonte.
fn buffer_cielo() -> Vec<u8> {
    let mut v = Vec::with_capacity((LW * LH * 4) as usize);
    for y in 0..LH {
        let t = y as f32 / LH as f32;
        // Arriba índigo profundo, al 60% un naranja cálido, abajo se apaga.
        let (r, g, b) = if t < 0.6 {
            let k = t / 0.6;
            (
                (24.0 + k * (236.0 - 24.0)),
                (28.0 + k * (120.0 - 28.0)),
                (92.0 + k * (72.0 - 92.0)),
            )
        } else {
            let k = (t - 0.6) / 0.4;
            (
                (236.0 - k * 160.0),
                (120.0 - k * 80.0),
                (72.0 - k * 30.0),
            )
        };
        for _x in 0..LW {
            v.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        }
    }
    v
}

/// Sol: disco cálido con borde suave sobre fondo transparente.
fn buffer_sol() -> Vec<u8> {
    let (cx, cy, radio, falloff) = (520.0_f32, 130.0_f32, 56.0_f32, 30.0_f32);
    let mut v = Vec::with_capacity((LW * LH * 4) as usize);
    for y in 0..LH {
        for x in 0..LW {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let a = if d <= radio {
                1.0
            } else {
                (1.0 - (d - radio) / falloff).clamp(0.0, 1.0)
            };
            v.extend_from_slice(&[255, 208, 120, (a * 255.0) as u8]);
        }
    }
    v
}

/// Dos cordones de montañas en silueta (el lejano más claro que el cercano).
fn buffer_montanas() -> Vec<u8> {
    let mut v = vec![0u8; (LW * LH * 4) as usize];
    let h = LH as f32;
    for x in 0..LW {
        let fx = x as f32;
        // Crestas por superposición de senos — perfil irregular pero estable.
        let lejana = h * 0.58
            - h * 0.16 * (0.6 * (fx * 0.013).sin() + 0.4 * (fx * 0.031 + 1.7).sin()).abs();
        let cercana = h * 0.76
            - h * 0.12 * (0.7 * (fx * 0.021 + 0.9).sin() + 0.3 * (fx * 0.047).sin()).abs();
        for y in 0..LH {
            let fy = y as f32;
            let i = ((y * LW + x) * 4) as usize;
            if fy >= cercana {
                v[i..i + 4].copy_from_slice(&[22, 20, 44, 255]);
            } else if fy >= lejana {
                v[i..i + 4].copy_from_slice(&[46, 40, 84, 255]);
            }
        }
    }
    v
}

/// Trazo de pincel a mano alzada: estampas circulares a lo largo de una
/// senoide, en magenta — el mismo resultado visual que dejaría la
/// herramienta pincel de la app.
fn buffer_trazo() -> Vec<u8> {
    let mut v = vec![0u8; (LW * LH * 4) as usize];
    let radio = 6.0_f32;
    let mut x = 50.0_f32;
    while x < LW as f32 - 50.0 {
        let cy = 205.0 + 70.0 * (x * 0.012).sin();
        let (x0, x1) = ((x - radio) as i32, (x + radio) as i32 + 1);
        let (y0, y1) = ((cy - radio) as i32, (cy + radio) as i32 + 1);
        for py in y0.max(0)..y1.min(LH as i32) {
            for px in x0.max(0)..x1.min(LW as i32) {
                let d = ((px as f32 - x).powi(2) + (py as f32 - cy).powi(2)).sqrt();
                let a = ((radio - d) / 1.5).clamp(0.0, 1.0);
                let i = ((py as u32 * LW + px as u32) * 4) as usize;
                let a8 = (a * 255.0) as u8;
                if a8 > v[i + 3] {
                    v[i..i + 4].copy_from_slice(&[236, 64, 168, a8]);
                }
            }
        }
        x += 2.0;
    }
    v
}

/// Viñeta para blend Multiplicar: blanca al centro, se oscurece hacia los
/// bordes (multiplicar por blanco = identidad; por gris = oscurecer).
fn buffer_vineta() -> Vec<u8> {
    let (cx, cy) = (LW as f32 / 2.0, LH as f32 / 2.0);
    let dmax = (cx * cx + cy * cy).sqrt();
    let mut v = Vec::with_capacity((LW * LH * 4) as usize);
    for y in 0..LH {
        for x in 0..LW {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / dmax;
            let g = (255.0 - 215.0 * d * d) as u8;
            v.extend_from_slice(&[g, g, g, 255]);
        }
    }
    v
}

/// Máscara degradada para el trazo: revela entero a la izquierda y se
/// desvanece hacia la derecha (255 = visible, 0 = oculto).
fn mascara_degradada() -> Vec<u8> {
    let mut v = Vec::with_capacity((LW * LH) as usize);
    for _y in 0..LH {
        for x in 0..LW {
            let t = ((x as f32 / LW as f32 - 0.45) / 0.55).clamp(0.0, 1.0);
            v.push((255.0 - 215.0 * t) as u8);
        }
    }
    v
}

// =============================================================================
//  Model demo: el estado que tendría la app tras una sesión corta de uso
// =============================================================================

fn modelo_demo() -> Model {
    let mut almacen = AlmacenEnMemoria::nuevo();
    let mut lienzo = Lienzo::nuevo(LW, LH);

    // --- Capas raster (fondo → frente), cada buffer al almacén por hash.
    let cielo = Capa::raster("cielo", almacen.insertar(buffer_cielo()));

    let mut sol = Capa::raster("sol", almacen.insertar(buffer_sol()));
    sol.blend = ModoFusion::Pantalla;
    let sol_id = sol.id;

    // --- Derivadas del sol: blur local + restyle IA (mock determinista).
    //     Nacen stale con cache en ceros; `aplicar_y_recomponer` las
    //     regenera con la misma maquinaria que la app.
    let mut halo = Capa::derivada(
        "halo",
        sol_id,
        TransformacionPixel::Local(OpLocal::Blur { radio: 12.0 }),
        [0u8; 32],
    );
    halo.blend = ModoFusion::Pantalla;
    halo.opacidad = 0.65;

    let mut resplandor = Capa::derivada(
        "ia:restyle",
        sol_id,
        transformacion_ia(
            "pixel-verbo-mock-v0",
            &OpPixel::Restyle { prompt: "atardecer cobrizo".into() },
        ),
        [0u8; 32],
    );
    resplandor.blend = ModoFusion::Pantalla;
    resplandor.opacidad = 0.55;

    let montanas = Capa::raster("montañas", almacen.insertar(buffer_montanas()));

    let mut trazo = Capa::raster("trazo pincel", almacen.insertar(buffer_trazo()));
    trazo.opacidad = 0.9;
    trazo.mascara = Some(almacen.insertar(mascara_degradada()));
    let trazo_id = trazo.id;

    let mut vineta = Capa::raster("viñeta", almacen.insertar(buffer_vineta()));
    vineta.blend = ModoFusion::Multiplicar;
    vineta.opacidad = 0.75;

    for capa in [cielo, sol, halo, resplandor, montanas, trazo, vineta] {
        lienzo.apilar(capa);
    }

    // --- Proveedor IA: mock en proceso, determinista (sin daemon ni red).
    let proveedor = ProveedorMock::nuevo();
    let proveedor_etiqueta = format!("mock {}", proveedor.model_id());

    let hist = Historial::nuevo(lienzo.clone(), HIST_CAP);
    let mut model = Model {
        lienzo,
        almacen,
        seleccionada: Some(trazo_id),
        imagen: None,
        estado: String::new(),
        proveedor: Box::new(proveedor),
        proveedor_etiqueta,
        thumbs: std::collections::HashMap::new(),
        raiz: std::path::PathBuf::from("."),
        imagenes_disponibles: Vec::new(),
        picker: None,
        renombrando: None,
        hist,
        factor_zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
        herramienta: Herramienta::Marco,
        color_picked: Some([255, 208, 120, 255]),
        histograma: None,
        seleccion: Some(RectImagen { x0: 430, y0: 40, x1: 620, y1: 230 }),
        seleccion_mascara: None,
        seleccion_overlay: None,
        seleccion_drag: None,
        mover_drag: None,
        pincel_drag: None,
        radio_pincel: 6,
        dureza_pincel: 0.8,
        shift_held: false,
        alt_held: false,
        clon_ancla: None,
        clon_offset: None,
        ultimo_pincel: None,
        simetria: Simetria::Ninguna,
        gradiente_drag: None,
        lazo_drag: None,
        editando_texto: None,
        portapapeles: None,
        editando_mascara: false,
        valor_mascara: 255,
        thumbs_mascara: std::collections::HashMap::new(),
        curva_arrastrando: None,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        context_menu: None,
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: Tween::idle(1.0),
        clipboard: SystemClipboard::new(),
        toasts: Vec::new(),
        next_toast: 0,
        transform: None,
        pluma_capa: None,
        pluma_ancla: None,
        pluma_rect: None,
        pluma_control: None,
        snap_grid: None,
    };

    // Regenera las derivadas stale (blur + restyle) con el mock, compone el
    // lienzo con el compositor real, computa histograma y thumbnails — la
    // misma secuencia que dispara cada edición en la app.
    aplicar_y_recomponer(&mut model);
    // Estado como lo dejaría el último gesto (un marquee sobre el sol).
    model.estado = "selección 190×190 @ (430,40)".into();
    model
}

/// Barra de menú con los mismos menús raíz que la app (`app_menu` en
/// main.rs), cerrados en el pantallazo — sólo se ven los rótulos.
fn menu_demo() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Abrir imagen…", "file.abrir").shortcut("Ctrl+P"))
                .item(MenuItem::new("Exportar PNG", "file.png").shortcut("Ctrl+S")),
        )
        .menu(
            Menu::new("Editar")
                .item(MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z"))
                .item(MenuItem::new("Duplicar capa", "edit.duplicar").shortcut("Ctrl+D")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Acercar", "view.zoom_in").shortcut("+"))
                .item(MenuItem::new("Restablecer vista", "view.reset").shortcut("0")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Misma composición que el `view()` de `Tullpu` (main.rs): menubar arriba,
/// header con el estado, y el centro en tres columnas — panel de capas,
/// lienzo compuesto y panel de operaciones.
fn view_demo(model: &Model, menu: &AppMenu, theme: &Theme) -> View<Msg> {
    let menubar = menubar_view(&MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    });
    let cabecera = header(
        theme,
        &model.lienzo,
        &model.estado,
        &model.proveedor_etiqueta,
        model.factor_zoom,
        model.herramienta,
        model.color_picked,
    );
    let centro = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        panel_capas(theme, model),
        panel_lienzo(theme, model),
        panel_ops(theme, model),
    ]);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![menubar, cabecera, centro])
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/tullpu.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    let theme = Theme::dark();
    let model = modelo_demo();
    let menu = menu_demo();
    let root = view_demo(&model, &menu, &theme);

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
        label: Some("pantallazo-tullpu"),
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
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_tullpu: escrito {out} ({W}x{H})");
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
