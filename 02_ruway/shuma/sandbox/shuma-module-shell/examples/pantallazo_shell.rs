//! Pantallazo headless de shuma para material público: una sesión sembrada
//! creíble con los tres pilares de la superficie de bloques —
//!
//! 1. `ls -l` reconocido como **tabla ordenable** (headers clickeables, orden
//!    activo por tamaño desc),
//! 2. `ls -R` partido en **sub-bloques colapsables** por directorio,
//! 3. un comando **corriendo en vivo** (streaming, badge ▶) sobre un proceso
//!    real, además de un bloque colapsado entero y el prompt con texto.
//!
//! Mismo pipeline que `dump_surface`: view → mount → layout → vello →
//! readback → PNG. Corre sin display (llvmpipe).
//!
//! `cargo run -p shuma-module-shell --example pantallazo_shell --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use shuma_module_shell::{ActiveRun, BackendHandle, OutputLine};

/// Tamaño del lienzo: `--width`/`--height` por args 2 y 3 (default 1280×800).
/// Permite reproducir layouts con ventana chica (p. ej. media pantalla).
fn shot_size() -> (u32, u32) {
    let w = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1280);
    let h = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(800);
    (w, h)
}

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn main() {
    // La superficie virtualizada es la única vía de output desde la Fase 5
    // (ya no hay flag ni `output_pane` legacy que activar).
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/shuma.png".to_string());
    let (w, h) = shot_size();
    if let Some(dir) = std::path::Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    let theme = llimphi_theme::Theme::default();
    let mut state = shuma_module_shell::State::new(shuma_module::Source::Local);
    // cwd presentable para material público (el header lo muestra).
    state.cwd = std::path::PathBuf::from("/home/sergio/proyectos");
    let now = now_secs();

    // Helper: abre un bloque con prompt + body + notice de cierre opcional.
    let mut blk = 0u64;
    let mut cmd = |state: &mut shuma_module_shell::State,
                   blk: &mut u64,
                   started_ago: u64,
                   prompt: &str,
                   body: &[OutputLine],
                   close: Option<&str>| {
        *blk += 1;
        let b = *blk;
        let mut p = OutputLine::prompt(prompt);
        p.block = b;
        state.output.push(p);
        for l in body {
            let mut l = l.clone();
            l.block = b;
            state.output.push(l);
        }
        if let Some(close) = close {
            let mut n = OutputLine::notice(close);
            n.block = b;
            state.output.push(n);
        }
        state.block_started.insert(b, now.saturating_sub(started_ago));
        state
            .block_command
            .insert(b, prompt.trim_start_matches("$ ").to_string());
        b
    };

    // ── Bloque 1: `ls -l` → tabla ordenable (orden activo: size desc) ──
    let tabla = cmd(
        &mut state,
        &mut blk,
        9 * 60,
        "$ ls -l ~/proyectos",
        &[
            OutputLine::stdout("total 248"),
            OutputLine::stdout("drwxr-xr-x  4 sergio sergio   4096 jun  9 10:12 assets"),
            OutputLine::stdout("-rw-r--r--  1 sergio sergio  18342 jun  9 11:47 informe.md"),
            OutputLine::stdout("-rw-r--r--  1 sergio sergio 104214 jun  8 19:03 datos.csv"),
            OutputLine::stdout("-rwxr-xr-x  1 sergio sergio  61288 jun  9 09:30 servidor"),
            OutputLine::stdout("drwxr-xr-x 12 sergio sergio   4096 jun  7 16:55 src"),
            OutputLine::stdout("-rw-r--r--  1 sergio sergio   2931 jun  9 11:02 config.toml"),
            OutputLine::stdout("-rw-r--r--  1 sergio sergio  44102 jun  6 14:21 fotos.zip"),
            OutputLine::stdout("drwxr-xr-x  2 sergio sergio   4096 jun  9 08:14 respaldos"),
            OutputLine::stdout("-rw-r--r--  1 sergio sergio   8210 jun  5 09:48 notas.txt"),
        ],
        Some("✔ exit 0"),
    );
    // Orden activo por tamaño (col 4) descendente → flecha ▼ en el header.
    state.section_sort.insert((tabla, 0), (4, false));

    // ── Bloque 2: `ls -R` → sub-bloques colapsables por directorio ──
    // `src` y `src/api` quedan expandidos; `src/api/v2` (profundidad ≥2)
    // arranca colapsado por la heurística — se ve el chevron cerrado.
    cmd(
        &mut state,
        &mut blk,
        3 * 60,
        "$ ls -R src",
        &[
            OutputLine::stdout("src:"),
            OutputLine::stdout("main.rs  lib.rs  api  modelos.rs"),
            OutputLine::stdout("util.rs  errores.rs  config.rs"),
            OutputLine::stdout(""),
            OutputLine::stdout("src/api:"),
            OutputLine::stdout("mod.rs  rutas.rs  sesiones.rs  v2"),
            OutputLine::stdout(""),
            OutputLine::stdout("src/api/v2:"),
            OutputLine::stdout("mod.rs  handlers.rs  esquema.rs"),
        ],
        Some("✔ exit 0"),
    );

    // ── Bloque extra: `git status` con cuerpo corto (coloreo semántico) ──
    cmd(
        &mut state,
        &mut blk,
        2 * 60,
        "$ git status",
        &[
            OutputLine::stdout("On branch main"),
            OutputLine::stdout("Changes not staged for commit:"),
            OutputLine::stdout("  modified:   src/api/rutas.rs"),
            OutputLine::stdout("  modified:   config.toml"),
        ],
        Some("✔ exit 0"),
    );

    // ── Bloque 3: comando entero colapsado (sólo header + badge) ──
    let plegado = cmd(
        &mut state,
        &mut blk,
        60,
        "$ git log --oneline -20",
        &[
            OutputLine::stdout("a31f02c feat: tabla ordenable en bloques"),
            OutputLine::stdout("99d7e10 fix: scroll anclado al fondo"),
        ],
        Some("✔ exit 0"),
    );
    state.collapsed.insert(plegado);

    // ── Bloque 4: comando corriendo AHORA (streaming, sin notice de cierre) ──
    let vivo = cmd(
        &mut state,
        &mut blk,
        4,
        "$ cargo build --release",
        &[
            OutputLine::stdout("   Compiling serde v1.0.219"),
            OutputLine::stdout("   Compiling tokio v1.45.0"),
            OutputLine::stdout("   Compiling rayon v1.10.0"),
            OutputLine::stdout("   Compiling image v0.25.6"),
            OutputLine::stdout("   Compiling wgpu v27.0.1"),
            OutputLine::stdout("   Compiling vello v0.7.0"),
            OutputLine::stdout("   Compiling parley v0.6.0"),
            OutputLine::stdout("   Compiling taffy v0.7.7"),
            OutputLine::stdout("   Compiling llimphi-ui v0.1.0"),
            OutputLine::stdout("   Compiling llimphi-widget-terminal v0.1.0"),
            OutputLine::stdout("   Compiling shuma-exec v0.1.0"),
            OutputLine::stdout("   Compiling servidor v0.4.2 (/home/sergio/proyectos)"),
        ],
        None, // sin cierre: el run sigue vivo
    );
    // Bytes ya streameados por el run vivo → badge "▶ 24 KB" en el header.
    state.current_run_bytes = 24_576;

    state.block_seq = blk;
    state.current_block = vivo;

    // Proceso REAL detrás del bloque vivo: `is_running()` debe dar true para
    // que el header pinte el badge ▶. Un sleep largo que matamos al final.
    // cwd REAL para el spawn (el cwd presentable del state puede no existir).
    let spec = shuma_exec::CommandSpec::shell("sleep 60", "/tmp".to_string());
    let handle = shuma_exec::run(&spec);
    let killer = handle.killer();
    state.running = Some(Arc::new(Mutex::new(ActiveRun {
        handle: BackendHandle::Local(handle),
        killer: Some(killer.clone()),
        command: "cargo build --release".to_string(),
        tui: None,
        block: vivo,
    })));

    // Prompt con el próximo comando a medio tipear (resaltado de sintaxis).
    match std::env::var("SHOT_INPUT_LINES").ok().and_then(|v| v.parse::<usize>().ok()) {
        Some(n) if n > 1 => {
            let lines: Vec<String> = (1..=n).map(|i| format!("echo linea {i} de un script pegado")).collect();
            state.input.set_text(lines.join("\n"));
        }
        _ => state.input.set_text("cargo test -p servidor"),
    }

    // Viewport medido (lo pondría el painter el frame anterior) + pinned al
    // fondo. ~680px = alto del panel de output con este chrome a 800px.
    if let Ok(mut g) = state.out_viewport_h.lock() {
        *g = (h as f32 - 94.0).max(50.0);
    }
    state.scroll_px = 0.0;

    let v = shuma_module_shell::view::<()>(&state, &theme, |_m| ());

    // view → layout → scene → textura → PNG (misma secuencia que el eventloop).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, v);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
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
        label: Some("pantallazo-shell"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
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
    renderer
        .render_to_view(&hal, &scene, &view, w, h, Color::from_rgba8(20, 20, 26, 255))
        .expect("render_to_view");
    write_png(&hal, &target, &out, w, h);

    // Bajar el sleep antes de salir (no dejar huérfanos).
    killer.kill();
    eprintln!(
        "pantallazo_shell: {out} ({w}x{h}) — {} líneas en {blk} bloques",
        state.output.len()
    );
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
