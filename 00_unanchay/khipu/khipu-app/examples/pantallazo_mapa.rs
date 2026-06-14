//! Pantallazo headless del mapa mental de khipu: nodos que respiran por
//! masa, filamentos de afinidad del nodo seleccionado, topónimos de regiones
//! bautizadas y el chip de bautizo de un clúster emergente. Reproduce la
//! composición de `gravity_panel`/`paint_map` (src/map.rs) con datos demo —
//! `khipu-app` es bin-only, así que el painter se calca acá tal cual.
//!
//! Mismo patrón headless que `llimphi-compositor/examples/primitivas_demo.rs`:
//! mount → layout → paint a `vello::Scene` → leer textura → PNG.
//!
//! `cargo run -p khipu-app --example pantallazo_mapa --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{auto, length, percent, Position, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_block, Alignment, TextBlock, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, PaintRect, View};

const W: u32 = 1280;
const H: u32 = 800;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Cámara fija del pantallazo (la del app vive en el Model).
const PAN: (f32, f32) = (0.0, 0.0);
const ZOOM: f32 = 1.1;

/// Un nodo demo del mapa — el mismo dato plano que `MapNode` en src/map.rs.
struct Nodo {
    x: f32,
    y: f32,
    /// Masa "vivida": enciende brillo y tamaño; decae sin atención.
    mass: f32,
    /// `false` = cayó bajo el horizonte (se pinta casi transparente).
    visible: bool,
    color: Color,
    label: &'static str,
    selected: bool,
}

/// Mundo → pantalla local (calco de `world_to_local` en src/map.rs).
fn world_to_local(wx: f32, wy: f32, w: f32, h: f32) -> (f32, f32) {
    (w * 0.5 + (wx + PAN.0) * ZOOM, h * 0.5 + (wy + PAN.1) * ZOOM)
}

fn with_alpha(c: Color, alpha: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, alpha])
}

/// RGB → HSV → rota H → RGB (calco de `rotate_hue` en src/map.rs); los
/// matices de clúster salen del accent del theme rotado por golden-ratio.
fn rotate_hue(c: Color, dh: f32) -> Color {
    let [r, g, b, a] = c.components;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
    let h = if (max - min).abs() < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / (max - min)) % 6.0
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };
    let h2 = ((h / 6.0) + dh).rem_euclid(1.0) * 6.0;
    let c2 = v * s;
    let x = c2 * (1.0 - ((h2 % 2.0) - 1.0).abs());
    let (r2, g2, b2) = match h2 as i32 {
        0 => (c2, x, 0.0),
        1 => (x, c2, 0.0),
        2 => (0.0, c2, x),
        3 => (0.0, x, c2),
        4 => (x, 0.0, c2),
        _ => (c2, 0.0, x),
    };
    let m = v - c2;
    Color::new([r2 + m, g2 + m, b2 + m, a])
}

/// El pintor del mapa — calco fiel de `paint_map` (src/map.rs): topónimos al
/// fondo, filamentos del seleccionado, nodos con halo/brillo por masa.
#[allow(clippy::too_many_arguments)]
fn paint_map(
    scene: &mut vello::Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    nodes: &[Nodo],
    links: &[((f32, f32), (f32, f32), f32)],
    regions: &[(&'static str, f32, f32)],
    theme: Theme,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let to_screen = |wx: f32, wy: f32| -> (f64, f64) {
        let (lx, ly) = world_to_local(wx, wy, rect.w, rect.h);
        ((rect.x + lx) as f64, (rect.y + ly) as f64)
    };

    // Topónimos al fondo: nombre grande y tenue + halo que insinúa territorio.
    for (name, rx, ry) in regions {
        let (cx, cy) = to_screen(*rx, *ry);
        let blob = KurboCircle::new((cx, cy), (96.0 * ZOOM as f64).max(34.0));
        scene.fill(Fill::NonZero, Affine::IDENTITY, with_alpha(theme.accent, 0.05), None, &blob);
        let size = (15.0 * ZOOM).clamp(11.0, 28.0);
        let est_w = name.chars().count() as f64 * size as f64 * 0.52;
        draw_block(
            scene,
            ts,
            &TextBlock::simple(
                name,
                size,
                with_alpha(theme.fg_text, 0.30),
                (cx - est_w * 0.5, cy - size as f64 * 0.6),
            ),
        );
    }

    // Filamentos primero (debajo de los nodos). Más opacos cuanto más afín.
    for (a, b, aff) in links {
        let (ax, ay) = to_screen(a.0, a.1);
        let (bx, by) = to_screen(b.0, b.1);
        let mut path = BezPath::new();
        path.move_to((ax, ay));
        path.line_to((bx, by));
        let alpha = (0.18 + aff * 0.55).clamp(0.0, 0.85);
        scene.stroke(
            &Stroke::new((0.8 + *aff as f64 * 1.6).max(0.6)),
            Affine::IDENTITY,
            with_alpha(theme.accent, alpha),
            None,
            &path,
        );
    }

    // Nodos: tamaño y brillo crecen con la masa viva (el mapa respira).
    for n in nodes {
        let (px, py) = to_screen(n.x, n.y);
        let m = n.mass.clamp(0.0, 2.0);
        let r = (3.0 + m * 4.5) * (0.6 + 0.4 * ZOOM.clamp(0.5, 1.5));
        let glow = if n.visible { (0.35 + m * 0.45).clamp(0.0, 1.0) } else { 0.18 };
        let color = with_alpha(n.color, glow);
        // Halo tenue alrededor de las notas más encendidas.
        if n.visible && m > 0.6 {
            let halo = KurboCircle::new((px, py), (r + 5.0) as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, with_alpha(n.color, 0.10), None, &halo);
        }
        let circle = KurboCircle::new((px, py), r as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &circle);

        if n.selected {
            let ring = KurboCircle::new((px, py), (r + 3.0) as f64);
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, theme.accent, None, &ring);
        }

        if n.visible {
            let lbl_col = with_alpha(theme.fg_text, (glow + 0.25).clamp(0.0, 1.0));
            draw_block(
                scene,
                ts,
                &TextBlock::simple(n.label, 10.0, lbl_col, (px + r as f64 + 4.0, py - 7.0)),
            );
        }
    }
}

/// Chip "✛ {nombre}" anclado a pantalla — calco de `name_region_chip` +
/// `pinned` (src/map.rs), sin el on_click porque acá nadie clickea. El
/// nombre es el topónimo propuesto del clúster (asignación automática del
/// #3): el bautizo arranca con la sugerencia ya cargada.
fn chip_nombrar(sx: f32, sy: f32, name: &str, theme: &Theme) -> View<()> {
    let (w, h) = (160.0_f32, 24.0_f32);
    let chip = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius(12.0)
    .text_aligned(format!("✛ {name}"), 11.0, theme.fg_muted, Alignment::Center);
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(sx - w * 0.5),
            top: length(sy - h * 0.5),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .children(vec![chip])
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/shots/khipu.png".to_string());
    let theme = Theme::dark(); // el theme canónico de khipu (src/main.rs)

    // Matices por clúster, como `cluster_color`: accent rotado golden-ratio.
    let c_huerta = theme.accent;
    let c_lecturas = rotate_hue(theme.accent, 0.16);
    let c_tareas = rotate_hue(theme.accent, 0.33);
    let c_suelto = rotate_hue(theme.accent, 0.50);

    // ~18 notas demo: tres constelaciones (huerta / lecturas / tareas) y unas
    // sueltas en el anillo exterior. Masas dispares = unas arden, otras se
    // enfrían; dos ya cayeron bajo el horizonte (casi transparentes).
    let n = |x: f32, y: f32, mass: f32, visible: bool, color: Color, label: &'static str, selected: bool| Nodo {
        x, y, mass, visible, color, label, selected,
    };
    let nodes = vec![
        // — huerta (región bautizada, arriba-izquierda) —
        n(-340.0, -180.0, 1.7, true, c_huerta, "trasplantar los tomates", true),
        n(-265.0, -230.0, 1.1, true, c_huerta, "compost: girar el lunes", false),
        n(-410.0, -120.0, 0.8, true, c_huerta, "semillas de albahaca", false),
        n(-290.0, -105.0, 0.5, true, c_huerta, "riego por goteo (plano)", false),
        n(-380.0, -255.0, 0.3, true, c_huerta, "podar el limonero", false),
        // — lecturas (región bautizada, derecha) —
        n(300.0, -90.0, 1.4, true, c_lecturas, "Borges: el jardín de se…", false),
        n(380.0, -30.0, 0.9, true, c_lecturas, "\"la memoria es porosa\"", false),
        n(245.0, -10.0, 0.7, true, c_lecturas, "releer cap. 3 de Wiener", false),
        n(355.0, -160.0, 0.45, true, c_lecturas, "cita: mapas ≠ territorio", false),
        // — tareas (clúster denso aún sin nombre, abajo-centro) —
        n(-60.0, 172.0, 1.2, true, c_tareas, "migrar backup al NAS", false),
        n(15.0, 245.0, 0.95, true, c_tareas, "factura de la imprenta", false),
        n(-130.0, 250.0, 0.6, true, c_tareas, "turno del dentista", false),
        n(-35.0, 300.0, 0.4, true, c_tareas, "renovar el dominio", false),
        // — sueltas en el anillo exterior (sin parentela semántica) —
        n(120.0, -280.0, 0.85, true, c_suelto, "idea: glosario quechua", false),
        n(-160.0, -10.0, 0.55, true, c_suelto, "¿sinestesia y tipografía?", false),
        n(430.0, 170.0, 0.35, true, c_suelto, "número de la ferretería", false),
        n(45.0, 45.0, 0.7, true, c_suelto, "llamar a Ema el sábado", false),
        n(-250.0, 85.0, 0.5, true, c_huerta, "croquis de las acequias", false),
        // — bajo el horizonte: la atención las dejó ir —
        n(-480.0, 120.0, 0.05, false, c_suelto, "borrador viejo", false),
        n(180.0, 310.0, 0.08, false, c_lecturas, "link que nunca abrí", false),
    ];

    // Regiones bautizadas: topónimos de continente detrás de los nodos.
    let regions: Vec<(&'static str, f32, f32)> = vec![
        ("huerta", -340.0, -138.0),
        ("lecturas", 315.0, -75.0),
    ];

    // Filamentos del nodo seleccionado ("trasplantar los tomates"): sus
    // parientes más afines se encienden — activación por difusión.
    let sel = (-340.0_f32, -180.0_f32);
    let links: Vec<((f32, f32), (f32, f32), f32)> = vec![
        (sel, (-265.0, -230.0), 0.82),
        (sel, (-290.0, -105.0), 0.66),
        (sel, (-410.0, -120.0), 0.58),
        (sel, (-380.0, -255.0), 0.41),
        (sel, (-160.0, -10.0), 0.24),
    ];

    // El chip de bautizo sobre el centroide del clúster denso sin nombre,
    // ya con el topónimo propuesto del contenido (asignación automática).
    // Coordenadas de pantalla con el rect del lienzo (ventana menos padding 4).
    let (cw, ch) = (W as f32 - 8.0, H as f32 - 8.0);
    let (chip_sx, chip_sy) = world_to_local(-52.0, 247.0, cw, ch);
    let chip = chip_nombrar(chip_sx, chip_sy - 46.0, "Cocina", &theme);

    // Misma composición que `gravity_panel`: panel con padding 4 + canvas
    // `bg_panel_alt` que pinta el mapa, con los overlays como hijos.
    let canvas = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .paint_with(move |scene, ts, rect| {
        paint_map(scene, ts, rect, &nodes, &links, &regions, theme);
    })
    .children(vec![chip]);

    let root = View::<()>::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![canvas]);

    // view → layout → scene (misma secuencia que el eventloop).
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
        label: Some("pantallazo-khipu"),
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
    let [r, g, b, _] = theme.bg_panel.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer.render_to_view(&hal, &scene, &view, W, H, bg).expect("render_to_view");

    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    write_png(&hal, &target, &out);
    eprintln!("pantallazo_mapa: escrito {out} ({W}x{H})");
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
