//! Hoja imprimible con **fidelidad gráfica**: rasteriza el MISMO árbol de
//! `View` que se ve en pantalla (rueda + cabecera + aspectos) a un PNG de
//! alta resolución, reutilizando la tubería vello+wgpu de Llimphi, y lo
//! abre en el visor de imágenes del sistema para imprimir.
//!
//! **Por qué render real y no HTML.** El HTML reconstruía la carta con
//! tipografía del navegador — perdía la fidelidad del motor (glyphs
//! vectoriales propios, layout exacto, la rueda). Acá montamos el `View`,
//! lo pintamos a una `vello::Scene` y lo escalamos ×N sobre una textura
//! offscreen: lo impreso es pixel-fiel a lo que pinta la app, a cualquier
//! DPI (los vectores no pixelan al ampliar).
//!
//! El render abre una segunda instancia headless de wgpu (`Hal::new(None)`)
//! para no tocar el device de la ventana — cuesta ~1 s de cold-start de
//! shaders, aceptable para una acción manual de "imprimir".

use std::path::PathBuf;
use std::process::Command;

use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::{taffy, LayoutTree};
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint};

use crate::model::Model;

/// Ancho lógico de la hoja (debe coincidir con `chrome::PRINT_SHEET_W` +
/// padding del contenedor). Damos un poco de aire a los lados.
const SHEET_LOGICAL_W: f32 = 616.0;
/// Factor de escala del render — vectores, así que sube el DPI sin pixelar.
const SCALE: f32 = 2.5;
/// Límite de lado de textura (los GPUs suelen topar en 8192/16384).
const MAX_PX: u32 = 8192;

/// Arma la hoja, la rasteriza a PNG de alta resolución y la abre en el
/// visor del sistema. Devuelve la ruta escrita o un mensaje de error.
pub(crate) fn imprimir_carta(model: &Model) -> Result<PathBuf, String> {
    let view = crate::chrome::print_page_content(model);
    let png = render_view_to_png(view, SHEET_LOGICAL_W, SCALE)?;
    let path = std::env::temp_dir().join("cosmos-hoja.png");
    std::fs::write(&path, &png).map_err(|e| format!("no se pudo escribir {path:?}: {e}"))?;
    abrir(&path)?;
    Ok(path)
}

/// Monta un `View`, lo pinta a una escena vello y la rasteriza a un PNG
/// (RGBA8) ampliada ×`scale` sobre una textura offscreen.
fn render_view_to_png(
    view: llimphi_ui::View<crate::model::Msg>,
    logical_w: f32,
    scale: f32,
) -> Result<Vec<u8>, String> {
    // GPU headless (sin surface) + rasterizador + tipografía.
    let hal = pollster::block_on(Hal::new(None)).map_err(|e| format!("gpu init: {e}"))?;
    let mut renderer = Renderer::new(&hal).map_err(|e| e.to_string())?;
    let mut ts = Typesetter::new();

    // Mount + layout. Alto disponible enorme → el alto real lo fija el
    // contenido (la hoja es `height: auto`).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, view);
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (logical_w, 100_000.0), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .map_err(|e| format!("layout: {e}"))?
    };
    // Tamaño real de la hoja según el layout (ancho fijo, alto por
    // contenido) — el PNG queda justo, sin márgenes muertos.
    let root = computed.get(mounted.root).ok_or("sin layout de raíz")?;
    let logical_w_real = root.w.max(1.0);
    let logical_h = root.h.max(1.0);

    // Pintar a coords lógicas, luego escalar la escena entera ×scale.
    let mut inner = vello::Scene::new();
    paint(&mut inner, &mounted, &computed, &mut ts, None, None);
    let mut scene = vello::Scene::new();
    scene.append(&inner, Some(Affine::scale(scale as f64)));

    let w_px = ((logical_w_real * scale).ceil() as u32).clamp(1, MAX_PX);
    let h_px = ((logical_h * scale).ceil() as u32).clamp(1, MAX_PX);

    // Textura offscreen (mismas usages que el gpu-bench: vello escribe por
    // STORAGE_BINDING, leemos por COPY_SRC).
    let tex = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cosmos-print-target"),
        size: wgpu::Extent3d {
            width: w_px,
            height: h_px,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let tview = tex.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(&hal, &scene, &tview, w_px, h_px, Color::from_rgba8(255, 255, 255, 255))
        .map_err(|e| e.to_string())?;

    leer_textura_png(&hal, &tex, w_px, h_px)
}

/// Copia la textura a un buffer mapeable (stride alineado a 256 B como pide
/// wgpu), desempaqueta las filas y codifica un PNG RGBA8 en memoria.
fn leer_textura_png(hal: &Hal, target: &wgpu::Texture, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf_size = (padded * h as usize) as u64;

    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cosmos-print-readback"),
        size: buf_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cosmos-print-copy"),
        });
    encoder.copy_texture_to_buffer(
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(encoder.finish()));

    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    hal.device.poll(wgpu::Maintain::Wait);
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range();

    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let start = row * padded;
        pixels.extend_from_slice(&data[start..start + unpadded]);
    }
    drop(data);
    buf.unmap();

    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&pixels).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Abre `path` con el visor/imagen por defecto del SO. Linux `xdg-open`,
/// macOS `open`, Windows `cmd /C start`.
fn abrir(path: &PathBuf) -> Result<(), String> {
    let p = path.to_string_lossy().to_string();
    let res = if cfg!(target_os = "macos") {
        Command::new("open").arg(&p).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", "", &p]).spawn()
    } else {
        Command::new("xdg-open").arg(&p).spawn()
    };
    res.map(|_| ())
        .map_err(|e| format!("no se pudo abrir el visor: {e} (la hoja quedó en {p})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::llimphi_layout::taffy::prelude::{length, Size, Style};
    use llimphi_ui::View;

    /// Smoke del pipeline headless: monta un `View` con texto + relleno,
    /// lo rasteriza y verifica que sale un PNG válido del tamaño esperado
    /// y con contenido (no todo blanco). Requiere GPU — se ignora por
    /// defecto para no romper CI sin display; correr con `--ignored`.
    #[test]
    #[ignore = "necesita GPU/headless wgpu"]
    fn rasteriza_view_a_png_valido() {
        let view: View<crate::model::Msg> = View::new(Style {
            size: Size {
                width: length(200.0),
                height: length(80.0),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(255, 255, 255, 255))
        .text_aligned(
            "Cosmos ☉♈ test".to_string(),
            24.0,
            Color::from_rgba8(0, 0, 0, 255),
            llimphi_ui::llimphi_text::Alignment::Start,
        );

        let scale = 2.0;
        let png = render_view_to_png(view, 200.0, scale).expect("render");
        // Firma PNG.
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);

        // Decodificar y comprobar dimensiones + que hay píxeles no-blancos
        // (el texto negro dejó marca).
        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoder.read_info().expect("png info");
        assert_eq!(reader.info().width, (200.0 * scale) as u32);
        let mut buf = vec![0u8; reader.output_buffer_size().expect("buffer size")];
        let info = reader.next_frame(&mut buf).expect("frame");
        let any_dark = buf[..info.buffer_size() as usize]
            .chunks_exact(4)
            .any(|px| px[0] < 200 && px[1] < 200 && px[2] < 200);
        assert!(any_dark, "la imagen salió toda blanca — el texto no pintó");
    }

    /// Rasteriza la HOJA real (carta de ejemplo) a `/tmp/cosmos-hoja-test.png`
    /// para inspeccionar la maqueta a ojo. Requiere GPU; correr con:
    /// `cargo test -p cosmos-app-llimphi hoja_de_ejemplo -- --ignored --nocapture`
    #[test]
    #[ignore = "necesita GPU/headless wgpu; genera PNG para inspección"]
    fn hoja_de_ejemplo_a_png() {
        let chart = crate::engine::sample_chart();
        let (render, _err) = crate::engine::compute(&chart, &[], 1, false, 0);
        let cfg = crate::model::CosmosConfig::default();
        let view = crate::chrome::print_page(&chart, &render, &cfg);
        let png = render_view_to_png(view, SHEET_LOGICAL_W, SCALE).expect("render hoja");
        let path = std::env::temp_dir().join("cosmos-hoja-test.png");
        std::fs::write(&path, &png).expect("escribir png");
        eprintln!("hoja escrita en {path:?} ({} bytes)", png.len());
    }

    /// Rasteriza una carta «Hoy» (instante actual, Mundane) en una ubicación
    /// para confirmar que computa una rueda real (no el mock vacío).
    /// `cargo test -p cosmos-app-llimphi hoy_sheet -- --ignored --nocapture`
    #[test]
    #[ignore = "necesita GPU; genera PNG de inspección"]
    fn hoy_sheet_a_png() {
        let chart = crate::engine::now_chart("Lima (ahora)", -12.0464, -77.0428);
        let (render, _e) = crate::engine::compute(&chart, &[], 1, false, 0);
        let cfg = crate::model::CosmosConfig::default();
        let view = crate::chrome::print_page(&chart, &render, &cfg);
        let png = render_view_to_png(view, SHEET_LOGICAL_W, SCALE).expect("render hoy");
        let path = std::env::temp_dir().join("cosmos-hoy.png");
        std::fs::write(&path, &png).expect("escribir png");
        eprintln!("hoja Hoy en {path:?} ({} bytes)", png.len());
    }

    /// Rasteriza el dial de 90° (vista Uraniana) a un PNG para inspección.
    /// `cargo test -p cosmos-app-llimphi dial_90 -- --ignored --nocapture`
    #[test]
    #[ignore = "necesita GPU; genera PNG de inspección"]
    fn dial_90_a_png() {
        use llimphi_ui::llimphi_layout::taffy::prelude::{length, Size, Style};
        let chart = crate::engine::sample_chart();
        let (render, _e) = crate::engine::compute(&chart, &[], 1, false, 0);
        let size = 540.0_f32;
        let rgba = |r, g, b, a| cosmos_render::Rgba { r, g, b, a };
        let cmds = crate::chrome::uranian_dial_cmds(
            &render,
            size,
            &cosmos_render::Palette::light(),
            rgba(0.10, 0.10, 0.12, 1.0), // tinta
            rgba(0.35, 0.35, 0.40, 1.0), // grilla
            rgba(0.82, 0.12, 0.12, 1.0), // rojo
            rgba(1.0, 0.99, 0.96, 1.0),  // papel
            12.0,                        // rotación de prueba
        );
        let canvas = cosmos_canvas_llimphi::canvas_view::<crate::model::Msg>(
            cmds,
            size,
            Some(Color::from_rgba8(255, 252, 245, 255)),
        );
        let view = View::new(Style {
            size: Size {
                width: length(size),
                height: length(size),
            },
            ..Default::default()
        })
        .children(vec![canvas]);
        let png = render_view_to_png(view, size, 2.0).expect("render dial");
        let path = std::env::temp_dir().join("cosmos-dial90.png");
        std::fs::write(&path, &png).expect("escribir png");
        eprintln!("dial 90 en {path:?} ({} bytes)", png.len());
    }

    /// Lámina con todos los glyphs de signos y planetas para inspección.
    /// `cargo test -p cosmos-app-llimphi glyph_sheet -- --ignored --nocapture`
    #[test]
    #[ignore = "necesita GPU; genera PNG de inspección"]
    fn glyph_sheet_a_png() {
        use cosmos_render::glyphs::{planet_commands, sign_commands};
        use cosmos_render::{DrawCommand, Rgba, TextAnchor};
        use llimphi_ui::llimphi_layout::taffy::prelude::{length, Size, Style};
        let ink = Rgba { r: 0.08, g: 0.08, b: 0.10, a: 1.0 };
        let lab = Rgba { r: 0.45, g: 0.45, b: 0.5, a: 1.0 };
        let signs = [
            "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra",
            "scorpio", "sagittarius", "capricorn", "aquarius", "pisces",
        ];
        let planets = [
            "sun", "moon", "mercury", "venus", "mars", "jupiter", "saturn",
            "uranus", "neptune", "pluto", "chiron", "north_node", "south_node",
            "lilith",
        ];
        let cols = 7;
        let cell = 96.0_f32;
        let glyph = 46.0_f32;
        let mut cmds: Vec<DrawCommand> = Vec::new();
        let mut place = |i: usize, label: &str, gcmds: Vec<DrawCommand>, cmds: &mut Vec<DrawCommand>| {
            let cx = (i % cols) as f32 * cell + cell / 2.0;
            let cy = (i / cols) as f32 * cell + cell / 2.0;
            // recoloca el glyph (vienen centrados en su propio cx/cy)
            cmds.extend(gcmds);
            cmds.push(DrawCommand::Text {
                x: cx,
                y: cy + glyph * 0.6,
                content: label.to_string(),
                color: lab,
                size: 10.0,
                anchor: TextAnchor::Middle,
            });
        };
        let mut idx = 0usize;
        for s in signs {
            let cx = (idx % cols) as f32 * cell + cell / 2.0;
            let cy = (idx / cols) as f32 * cell + cell / 2.0;
            place(idx, s, sign_commands(s, cx, cy, glyph, ink, 2.2), &mut cmds);
            idx += 1;
        }
        // siguiente fila redonda
        idx = ((idx + cols - 1) / cols) * cols;
        for p in planets {
            let cx = (idx % cols) as f32 * cell + cell / 2.0;
            let cy = (idx / cols) as f32 * cell + cell / 2.0;
            place(idx, p, planet_commands(p, cx, cy, glyph, ink, 2.0), &mut cmds);
            idx += 1;
        }
        let rows = (idx + cols - 1) / cols;
        let side = (cols.max(rows)) as f32 * cell;
        let canvas = cosmos_canvas_llimphi::canvas_view::<crate::model::Msg>(
            cmds,
            side,
            Some(Color::from_rgba8(255, 253, 248, 255)),
        );
        let view = View::new(Style {
            size: Size {
                width: length(side),
                height: length(side),
            },
            ..Default::default()
        })
        .children(vec![canvas]);
        let png = render_view_to_png(view, side, 2.0).expect("render glyphs");
        let path = std::env::temp_dir().join("cosmos-glyphs.png");
        std::fs::write(&path, &png).expect("escribir png");
        eprintln!("glyphs en {path:?} ({} bytes)", png.len());
    }
}
