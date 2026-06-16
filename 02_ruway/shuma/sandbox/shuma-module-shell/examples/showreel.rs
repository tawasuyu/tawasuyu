//! **Showreel** de shuma — para el README del repo standalone. NO es eye-candy
//! abstracto: es una vitrina de la **superficie de bloques REAL** del shell, en
//! acción. Cada frame reconstruye el `State` de shuma y lo pinta con la MISMA
//! función pública (`shuma_module_shell::view`) que usa el shell en producción:
//! header con cwd, bloques de comando (prompt + body), la tabla ordenable de
//! `ls -l` (headers clickeables + flecha de orden), los sub-bloques colapsables
//! de `ls -R`, el coloreo semántico de `git status`, un bloque entero plegado y
//! un comando **corriendo en vivo** (badge ▶ + bytes). El **estado** se deriva
//! del tiempo normalizado `t∈[0,1]`: los bloques aparecen con stagger, el
//! comando vivo va streameando líneas, el prompt se va tipeando, el output se
//! desplaza. No se dibuja una terminal falsa: si existe el render, se usa.
//!
//! Beats (timeline):
//!   1. cold-open: prompt sobrio + caret + trazo bezier draw-on (firma).
//!   2. los bloques aparecen con stagger (ls -l tabla, ls -R, git status…).
//!   3. el comando vivo `cargo build` streamea (badge ▶), el output scrollea.
//!   4. el próximo comando se tipea en el prompt.
//!   5. cierre: wordmark «shuma» + subtítulo, frame limpio para screenshot.
//!
//! Render headless y determinista (sin reloj, sin runtime, sin winit): frame
//! `i` de `N` → `t = i/(N-1)` → View → layout (taffy + parley) → vello::Scene →
//! wgpu → PNG. Idéntico al eventloop.
//!
//! ```text
//! cargo run -p shuma-module-shell --example showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_shuma`, `n_frames=300`, `W=1600`, `H=900`.

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint, PaintRect};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Position, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color};
use llimphi_ui::llimphi_raster::vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_ui::View;

use llimphi_theme::{motion, Theme};

use shuma_module_shell::{OutputLine, State};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ───────────────────────── tema / skin ─────────────────────────

#[derive(Clone)]
struct Skin {
    accent: Color,
    bg: Color,
    fg: Color,
    fg_muted: Color,
}

// ───────────────────────── el guion de bloques ─────────────────────────

/// Un bloque del guion: prompt + cuerpo + (opcional) notice de cierre. El
/// `started_ago` alimenta el "hace N min" del header; `collapsed`/`sort` son
/// los estados de la superficie. `live` marca el bloque que sigue corriendo.
struct Block {
    started_ago: u64,
    prompt: &'static str,
    body: &'static [(&'static str, OutKind)],
    close: Option<&'static str>,
    collapsed: bool,
    /// orden activo en su sección 0: `(col, asc)` → flecha en el header.
    sort: Option<(usize, bool)>,
    /// si el cuerpo se revela de a poco (streaming) en su beat.
    live: bool,
}

#[derive(Clone, Copy)]
enum OutKind {
    Out,
    Err,
}

/// El guion completo del reel (mismo contenido creíble que `pantallazo_shell`).
fn script() -> Vec<Block> {
    use OutKind::*;
    vec![
        // ── 1: `ls -l` → tabla ordenable (orden activo: size desc) ──
        Block {
            started_ago: 9 * 60,
            prompt: "$ ls -l ~/proyectos",
            body: &[
                ("total 248", Out),
                ("drwxr-xr-x  4 sergio sergio   4096 jun  9 10:12 assets", Out),
                ("-rw-r--r--  1 sergio sergio  18342 jun  9 11:47 informe.md", Out),
                ("-rw-r--r--  1 sergio sergio 104214 jun  8 19:03 datos.csv", Out),
                ("-rwxr-xr-x  1 sergio sergio  61288 jun  9 09:30 servidor", Out),
                ("drwxr-xr-x 12 sergio sergio   4096 jun  7 16:55 src", Out),
                ("-rw-r--r--  1 sergio sergio   2931 jun  9 11:02 config.toml", Out),
                ("-rw-r--r--  1 sergio sergio  44102 jun  6 14:21 fotos.zip", Out),
                ("drwxr-xr-x  2 sergio sergio   4096 jun  9 08:14 respaldos", Out),
                ("-rw-r--r--  1 sergio sergio   8210 jun  5 09:48 notas.txt", Out),
            ],
            close: Some("✔ exit 0"),
            collapsed: false,
            sort: Some((4, false)),
            live: false,
        },
        // ── 2: `ls -R` → sub-bloques colapsables por directorio ──
        Block {
            started_ago: 3 * 60,
            prompt: "$ ls -R src",
            body: &[
                ("src:", Out),
                ("main.rs  lib.rs  api  modelos.rs", Out),
                ("util.rs  errores.rs  config.rs", Out),
                ("", Out),
                ("src/api:", Out),
                ("mod.rs  rutas.rs  sesiones.rs  v2", Out),
                ("", Out),
                ("src/api/v2:", Out),
                ("mod.rs  handlers.rs  esquema.rs", Out),
            ],
            close: Some("✔ exit 0"),
            collapsed: false,
            sort: None,
            live: false,
        },
        // ── 3: `git status` con coloreo semántico ──
        Block {
            started_ago: 2 * 60,
            prompt: "$ git status",
            body: &[
                ("On branch main", Out),
                ("Changes not staged for commit:", Out),
                ("  modified:   src/api/rutas.rs", Err),
                ("  modified:   config.toml", Err),
            ],
            close: Some("✔ exit 0"),
            collapsed: false,
            sort: None,
            live: false,
        },
        // ── 4: comando entero colapsado (sólo header + badge) ──
        Block {
            started_ago: 60,
            prompt: "$ git log --oneline -20",
            body: &[
                ("a31f02c feat: tabla ordenable en bloques", Out),
                ("99d7e10 fix: scroll anclado al fondo", Out),
            ],
            close: Some("✔ exit 0"),
            collapsed: true,
            sort: None,
            live: false,
        },
        // ── 5: comando corriendo AHORA (streaming, sin cierre) ──
        Block {
            started_ago: 4,
            prompt: "$ cargo build --release",
            body: &[
                ("   Compiling serde v1.0.219", Out),
                ("   Compiling tokio v1.45.0", Out),
                ("   Compiling rayon v1.10.0", Out),
                ("   Compiling image v0.25.6", Out),
                ("   Compiling wgpu v27.0.1", Out),
                ("   Compiling vello v0.7.0", Out),
                ("   Compiling parley v0.6.0", Out),
                ("   Compiling taffy v0.7.7", Out),
                ("   Compiling llimphi-ui v0.1.0", Out),
                ("   Compiling shuma-exec v0.1.0", Out),
                ("   Compiling servidor v0.4.2 (/home/sergio/proyectos)", Out),
            ],
            close: None,
            collapsed: false,
            sort: None,
            live: true,
        },
    ]
}

/// Construye el `State` para el tiempo `t`: los bloques aparecen con stagger
/// (cada uno tras el anterior), el cuerpo del bloque vivo se revela de a poco
/// (streaming), el prompt se va tipeando al final y el scroll sigue al fondo.
fn build_state(t: f32, vp_h: f32) -> State {
    let mut state = State::new(shuma_module::Source::Local);
    state.cwd = std::path::PathBuf::from("/home/sergio/proyectos");
    let now: u64 = 1_700_000_000;

    let blocks = script();
    let n = blocks.len();

    // Ventana de bloques (12%–70%): los bloques entran con stagger. `reveal`
    // ∈[0,n] como float; la parte entera = bloques completos, la fracción =
    // progreso de revelado del bloque en curso.
    let appear = motion::ease_out_cubic(seg(t, 0.12, 0.70));
    let reveal = appear * n as f32;

    let mut blk_id = 0u64;
    for (idx, b) in blocks.iter().enumerate() {
        let block_progress = (reveal - idx as f32).clamp(0.0, 1.0);
        if block_progress <= 0.0 {
            break; // este bloque y los siguientes aún no entraron
        }
        blk_id += 1;
        let id = blk_id;

        // Prompt siempre presente apenas el bloque entra.
        let mut p = OutputLine::prompt(b.prompt);
        p.block = id;
        state.output.push(p);

        // Cuántas líneas del cuerpo mostrar: el bloque vivo (el último) revela
        // su cuerpo gradualmente con block_progress (streaming); los previos
        // entran completos de una.
        let total = b.body.len();
        let shown = if b.live {
            ((block_progress * total as f32).ceil() as usize).min(total)
        } else {
            total
        };
        for (text, kind) in b.body.iter().take(shown) {
            let mut l = match kind {
                OutKind::Out => OutputLine::stdout(*text),
                OutKind::Err => OutputLine::stderr(*text),
            };
            l.block = id;
            state.output.push(l);
        }

        // Notice de cierre sólo cuando el bloque ya entró del todo.
        if block_progress >= 1.0 {
            if let Some(close) = b.close {
                let mut nce = OutputLine::notice(close);
                nce.block = id;
                state.output.push(nce);
            }
        }

        state.block_started.insert(id, now.saturating_sub(b.started_ago));
        state
            .block_command
            .insert(id, b.prompt.trim_start_matches("$ ").to_string());
        if let Some(close) = b.close {
            if block_progress >= 1.0 {
                let _ = close;
                state.block_ended.insert(id, now);
            }
        }
        if b.collapsed && block_progress >= 1.0 {
            state.collapsed.insert(id);
        }
        if let Some(sort) = b.sort {
            state.section_sort.insert((id, 0), sort);
        }
        if b.live {
            state.current_block = id;
            // badge ▶ con bytes que crecen con el streaming.
            state.current_run_bytes = (block_progress * 26_624.0) as u64;
        }
    }
    state.block_seq = blk_id;

    // Prompt: el próximo comando se va tipeando (beat 72%–88%).
    let target = "cargo test -p servidor";
    let typed = seg(t, 0.72, 0.88);
    let chars = (typed * target.chars().count() as f32).round() as usize;
    let shown: String = target.chars().take(chars).collect();
    state.input.set_text(shown);

    // Viewport medido (lo pondría el painter del frame anterior).
    if let Ok(mut g) = state.out_viewport_h.lock() {
        *g = vp_h.max(50.0);
    }
    // Scroll dirigido por `t`: mientras entran los bloques (≤66%) anclamos
    // ARRIBA (scroll_px grande → clampa al tope) para lucir la tabla ordenable
    // de `ls -l`, el feature titular; después (66%–82%) bajamos suave al fondo
    // (0.0 = pinned) para mostrar el comando vivo streameando. `scroll_px` es
    // px desde el fondo: la superficie lo clampa al overflow real.
    let to_bottom = motion::ease_in_out_cubic(seg(t, 0.66, 0.82));
    state.scroll_px = lerp(4000.0, 0.0, to_bottom as f64) as f32;
    // Ancla del scroll (la superficie interpreta `scroll_px` relativo a ella);
    // un valor alto la pone arriba del todo cuando `scroll_px` es grande.
    state.surf_scroll_anchor = 4000.0;

    state
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 40.0));
    p.curve_to(
        (cx - 150.0, cy - 220.0),
        (cx + 150.0, cy + 220.0),
        (cx + 360.0, cy - 40.0),
    );
    p
}

fn trim_path(full: &BezPath, prog: f64) -> (BezPath, Point) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = Point::ZERO;
    for el in full.elements() {
        match el {
            vello::kurbo::PathEl::MoveTo(p) => start = *p,
            vello::kurbo::PathEl::CurveTo(c1, c2, p) => {
                cubic = Some(vello::kurbo::CubicBez::new(start, *c1, *c2, *p));
            }
            _ => {}
        }
    }
    let mut out = BezPath::new();
    let mut head = start;
    if let Some(cb) = cubic {
        out.move_to(cb.p0);
        let steps = 96;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            out.line_to(pt);
            head = pt;
        }
    }
    (out, head)
}

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // ── COLD OPEN (0–12%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.12);
    let line_vis = 1.0 - seg(t, 0.12, 0.19);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.13)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(s.accent, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // Una pista textual durante el cold-open: prompt sobrio centrado, se
    // desvanece cuando entran los bloques.
    let prompt_a = seg(t, 0.03, 0.10) * (1.0 - seg(t, 0.12, 0.18));
    if prompt_a > 0.001 {
        let psz = 30.0_f32;
        let layout = ts.layout(
            "shuma ❯ _", psz, None, Alignment::Start, 1.0, false, None, 500.0, false, false,
        );
        let m = measurement(&layout);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = ch / 2.0 + 70.0;
        let brush = peniko::Brush::Solid(with_alpha(s.fg_muted, prompt_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));
    }

    // ── WORDMARK (84–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.86, 0.96);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 150.0_f32;
        let layout = ts.layout(
            "shuma", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.90, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "a block-based terminal, in Rust", ssz, None, Alignment::Start, 1.0, false, None,
                500.0, false, false,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.accent, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(
                scene,
                &sub,
                &sbrush,
                Affine::translate((sx + dot_r * 2.0 + 14.0, sy)),
            );
        }
    }

    // ── punto teal de firma (esquina inf-der) ───────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.82, 0.88));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── la escena por frame ─────────────────────────

fn build_view(t: f32, cw: f64, ch: f64, theme: &Theme, s: &Skin) -> View<()> {
    // Slide/fade del shell: entra (10–20%), se desvanece antes del wordmark
    // (82–88%).
    let slide = motion::ease_out_cubic(seg(t, 0.10, 0.22));
    let shell_alpha = (slide * (1.0 - seg(t, 0.82, 0.88))).clamp(0.0, 1.0);
    let shell_dy = lerp(14.0, 0.0, slide as f64);

    let mut children: Vec<View<()>> = Vec::new();

    if shell_alpha > 0.001 {
        // Alto de viewport aproximado del panel de output con este chrome.
        let vp_h = ch as f32 - 110.0;
        let state = build_state(t, vp_h);
        let shell = shuma_module_shell::view::<()>(&state, theme, |_m| ());
        let wrap = View::new(Style {
            position: Position::Absolute,
            inset: TaffyRect {
                left: length(0.0),
                top: length(0.0),
                right: length(0.0),
                bottom: length(0.0),
            },
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .alpha(shell_alpha)
        .transform(Affine::translate((0.0, shell_dy)))
        .children(vec![shell]);
        children.push(wrap);
    }

    // Overlay full-screen del vector (cold-open + wordmark).
    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0),
            top: length(0.0),
            right: length(0.0),
            bottom: length(0.0),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with({
        let s = s.clone();
        move |scene, ts, _rect: PaintRect| {
            draw_overlays(scene, ts, t, cw, ch, &s);
        }
    });
    children.push(overlay);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(s.bg)
    .children(children)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .unwrap_or_else(|| "showreel_frames_shuma".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    // Aislar de la config/historial REAL del usuario: `State::new` carga el
    // shumarc y el history desde XDG. Apuntamos XDG a un sandbox vacío para que
    // el reel sea determinista y NO filtre comandos personales (la sugerencia de
    // alias salía del historial real de la máquina).
    let sandbox = std::env::temp_dir().join(format!("shuma-showreel-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&sandbox);
    std::env::set_var("HOME", &sandbox);
    std::env::set_var("XDG_CONFIG_HOME", sandbox.join("config"));
    std::env::set_var("XDG_DATA_HOME", sandbox.join("data"));
    std::env::set_var("XDG_STATE_HOME", sandbox.join("state"));

    let theme = Theme::by_name("Tawa").unwrap_or_default();
    let accent = theme.accent;
    let skin = Skin {
        accent,
        bg: theme.bg_app,
        fg: theme.fg_text,
        fg_muted: theme.fg_muted,
    };
    let [br, bg, bb, _] = skin.bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-shuma"),
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

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(t, cw, ch, &theme, &skin);

        let mut layout = LayoutTree::new();
        let mounted = mount(&mut layout, root);
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

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("showreel-shuma: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-shuma: {n} frames en {out_dir}/ ({w}x{h})");
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
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h as usize {
        let sidx = r * padded;
        pixels.extend_from_slice(&data[sidx..sidx + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
