//! **Showreel de mirada** — concept-reel HONESTO para el README del repo
//! standalone (audiencia r/unixporn / r/rust).
//!
//! ⚠️ ESTO NO ES UNA CAPTURA DEL COMPOSITOR REAL. mirada es un compositor
//! Wayland + WM que necesita una sesión Wayland viva (DRM/KMS o un compositor
//! anidado) para renderizar: no se puede capturar headless. Lo que ves acá es
//! una **DEPICCIÓN ESTILIZADA del WM** dibujada con Llimphi en headless,
//! usando las **paletas y parámetros REALES de cada vista** de mirada
//! (`mirada-brain/src/vistas.rs` + `llimphi-theme`). Es un concept-reel: el
//! layout, los colores del chrome, el grosor de marco, el alto de barra de
//! título y el gap son los de fábrica de cada vista; el "escritorio" en sí
//! (ventanas + contenido) es una maqueta, no las apps reales.
//!
//! MENSAJE CENTRAL: **un solo WM, todos los escritorios.** El mismo layout de
//! ventanas (master-stack: una maestra + pila) se transforma atravesando las
//! 7 vistas de fábrica — mirada(Dark) → Windows XP → macOS → KDE Breeze →
//! Solaris CDE → Hyprland → dwm — interpolando paleta y chrome entre cada una.
//! Se lee como una sola piel cambiando, no como capturas distintas pegadas.
//!
//! Render **headless y determinista** (sin reloj, sin runtime): frame `i` de
//! `N` → `t = i/(N-1)` → View → layout (taffy + parley) → vello::Scene → wgpu
//! → PNG. Patrón idéntico a `llimphi-compositor/examples/showreel.rs`.
//!
//! ```text
//! cargo run -p mirada-app-llimphi --example showreel --release -- \
//!     [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_mirada`, `n_frames=360`, `W=1600`, `H=900`.

use std::fs::{create_dir_all, File};
use std::io::BufWriter;

use llimphi_theme::{motion, Theme};
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color, Gradient};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{measurement, Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, PaintRect, View};
use vello::kurbo::{Affine, Point, Rect as KRect, RoundedRect, Stroke};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let [ar, ag, ab, aa] = a.components;
    let [br, bg, bb, ba] = b.components;
    Color::new([
        ar + (br - ar) * t,
        ag + (bg - ag) * t,
        ab + (bb - ab) * t,
        aa + (ba - aa) * t,
    ])
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

fn rgba(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

// ───────────────────────── la piel de una vista ─────────────────────────
//
// Cada `Vista` de mirada (mirada-brain/src/vistas.rs) empaqueta tema + chrome.
// Acá la resolvemos a una `Skin` con TODOS los valores reales: la paleta sale
// de `Theme::by_name` (llimphi-theme) y el chrome (gap, border_width,
// titlebar_height, border_focus/normal) sale de la `Vista`. No se inventa nada.

#[derive(Clone)]
struct Skin {
    label: &'static str,
    theme: Theme,
    // Chrome real de la vista.
    gap: f64,
    border_width: f64,
    titlebar_height: f64,
    border_focus: Color,
    border_normal: Color,
    // Color de fondo del escritorio (wallpaper-ish). Derivado del tema.
    desktop: Color,
}

/// Interpola dos pieles, campo a campo, para el morph entre vistas.
fn lerp_skin(a: &Skin, b: &Skin, t: f32) -> Skin {
    let tt = t as f64;
    Skin {
        // El label salta a la mitad (cross-fade lo maneja el rótulo aparte).
        label: if t < 0.5 { a.label } else { b.label },
        theme: lerp_theme(&a.theme, &b.theme, t),
        gap: lerp(a.gap, b.gap, tt),
        border_width: lerp(a.border_width, b.border_width, tt),
        titlebar_height: lerp(a.titlebar_height, b.titlebar_height, tt),
        border_focus: lerp_color(a.border_focus, b.border_focus, t),
        border_normal: lerp_color(a.border_normal, b.border_normal, t),
        desktop: lerp_color(a.desktop, b.desktop, t),
    }
}

/// Interpola los slots del tema que usamos para pintar el chrome.
fn lerp_theme(a: &Theme, b: &Theme, t: f32) -> Theme {
    // Partimos de `a` y pisamos los slots que tocamos (no necesitamos todos).
    let mut o = *a;
    o.bg_app = lerp_color(a.bg_app, b.bg_app, t);
    o.bg_panel = lerp_color(a.bg_panel, b.bg_panel, t);
    o.bg_panel_alt = lerp_color(a.bg_panel_alt, b.bg_panel_alt, t);
    o.bg_button = lerp_color(a.bg_button, b.bg_button, t);
    o.bg_selected = lerp_color(a.bg_selected, b.bg_selected, t);
    o.fg_text = lerp_color(a.fg_text, b.fg_text, t);
    o.fg_muted = lerp_color(a.fg_muted, b.fg_muted, t);
    o.border = lerp_color(a.border, b.border, t);
    o.accent = lerp_color(a.accent, b.accent, t);
    o
}

/// Fondo de escritorio derivado del tema: un gradiente tibio basado en
/// `bg_app`, levemente teñido por el acento — wallpaper-ish sin imagen.
fn desktop_for(theme: &Theme) -> Color {
    // El propio bg_app del tema es buena base de escritorio.
    theme.bg_app
}

/// Resuelve una vista de fábrica a su `Skin`. Valores REALES de vistas.rs.
/// (No dependemos de mirada-brain en tiempo de ejecución para mantener el
/// example liviano y autónomo, pero los números son copia literal de
/// `mirada-brain/src/vistas.rs` — un cambio allá debe reflejarse acá.)
fn skin(name: &str) -> Skin {
    // (theme_name, gap, border_width, titlebar_height, border_focus, border_normal)
    let (label, theme_name, gap, bw, tbh, bf, bn): (
        &'static str,
        &str,
        f64,
        f64,
        f64,
        [u8; 4],
        [u8; 4],
    ) = match name {
        // mirada nativa = Config::default(): Dark, gap 8, bw 2, tbh 24.
        "mirada" => (
            "mirada",
            "Dark",
            8.0,
            2.0,
            24.0,
            [92, 143, 235, 255],
            [56, 56, 69, 255],
        ),
        "windows-xp" => (
            "Windows XP",
            "WinXP",
            4.0,
            3.0,
            28.0,
            [36, 94, 220, 255],
            [122, 152, 206, 255],
        ),
        "mac" => (
            "macOS",
            "macOS",
            8.0,
            1.0,
            24.0,
            [10, 132, 255, 255],
            [208, 208, 215, 255],
        ),
        "kde" => (
            "KDE Plasma",
            "Breeze",
            6.0,
            2.0,
            26.0,
            [61, 174, 233, 255],
            [188, 192, 196, 255],
        ),
        "solaris" => (
            "Solaris CDE",
            "CDE",
            4.0,
            2.0,
            22.0,
            [64, 132, 132, 255],
            [108, 116, 134, 255],
        ),
        "hyprland" => (
            "Hyprland",
            "Dark",
            10.0,
            2.0,
            0.0,
            [110, 140, 220, 255],
            [46, 54, 70, 255],
        ),
        "dwm" => (
            "dwm",
            "Dark",
            0.0,
            1.0,
            0.0,
            [110, 140, 220, 255],
            [46, 54, 70, 255],
        ),
        other => panic!("vista desconocida: {other}"),
    };
    let theme = Theme::by_name(theme_name)
        .unwrap_or_else(|| panic!("tema {theme_name} no existe en llimphi-theme"));
    Skin {
        label,
        gap,
        border_width: bw,
        titlebar_height: tbh,
        border_focus: rgba(bf),
        border_normal: rgba(bn),
        desktop: desktop_for(&theme),
        theme,
    }
}

// ───────────────────────── geometría del escritorio ─────────────────────────
//
// Master-stack: una ventana maestra a la izquierda (master_ratio 0.6) + una
// pila de 2 ventanas a la derecha. Es el layout de fábrica de mirada. Las
// posiciones NO cambian entre vistas — sólo la piel. Ese es el punto.

#[derive(Clone, Copy)]
struct WinRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// true = la ventana enfocada (borde de acento).
    focused: bool,
    title: &'static str,
}

/// Las 3 ventanas del escritorio en coordenadas de área de trabajo.
/// `gap` real de la vista se aplica como margen entre/alrededor de ventanas.
fn windows(work: KRect, gap: f64) -> [WinRect; 3] {
    let g = gap;
    let x0 = work.x0 + g;
    let y0 = work.y0 + g;
    let total_w = work.width() - g * 2.0;
    let total_h = work.height() - g * 2.0;

    let master_ratio = 0.6; // master_ratio de fábrica
    let master_w = total_w * master_ratio - g / 2.0;
    let stack_w = total_w * (1.0 - master_ratio) - g / 2.0;
    let stack_x = x0 + master_w + g;
    let stack_h = (total_h - g) / 2.0;

    [
        WinRect {
            x: x0,
            y: y0,
            w: master_w,
            h: total_h,
            focused: true,
            title: "pluma — multilienzo",
        },
        WinRect {
            x: stack_x,
            y: y0,
            w: stack_w,
            h: stack_h,
            focused: false,
            title: "shuma",
        },
        WinRect {
            x: stack_x,
            y: y0 + stack_h + g,
            w: stack_w,
            h: stack_h,
            focused: false,
            title: "nahual",
        },
    ]
}

// ───────────────────────── pintura del escritorio ─────────────────────────

/// Dibuja el panel/barra superior (la "barra de pata" depictada).
fn paint_panel(scene: &mut vello::Scene, ts: &mut Typesetter, s: &Skin, cw: f64, panel_h: f64) {
    // La barra usa bg_panel_alt cuando es una franja con color propio (XP,
    // Breeze tienen panel oscuro/azul); si no, bg_panel.
    let bar = s.theme.bg_panel_alt;
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        bar,
        None,
        &KRect::new(0.0, 0.0, cw, panel_h),
    );
    // Línea inferior sutil.
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        with_alpha(Color::BLACK, 0.18),
        None,
        &KRect::new(0.0, panel_h - 1.0, cw, panel_h),
    );

    // ¿El texto de la barra va claro u oscuro? Decidir por luminancia del bar.
    let lum = {
        let k = bar.components;
        0.2126 * k[0] + 0.7152 * k[1] + 0.0722 * k[2]
    };
    let on_bar = if lum < 0.5 {
        Color::from_rgba8(235, 238, 244, 255)
    } else {
        Color::from_rgba8(30, 34, 42, 255)
    };

    // Botón de inicio / launcher: chip de acento a la izquierda.
    let bh = panel_h - 10.0;
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        s.theme.accent,
        None,
        &RoundedRect::new(8.0, 5.0, 8.0 + 96.0, 5.0 + bh, 5.0),
    );
    {
        let layout = ts.layout(
            "menu",
            13.0,
            None,
            Alignment::Start,
            1.0,
            false,
            None,
            600.0,
            false,
            false,
        );
        // Texto contrastante sobre el acento.
        let on_accent = {
            let k = s.theme.accent.components;
            let l = 0.2126 * k[0] + 0.7152 * k[1] + 0.0722 * k[2];
            if l < 0.55 {
                Color::WHITE
            } else {
                Color::from_rgba8(20, 24, 30, 255)
            }
        };
        draw_text(scene, &layout, on_accent, 26.0, (panel_h - 16.0) / 2.0 + 1.0);
    }

    // Reloj a la derecha.
    {
        let layout = ts.layout(
            "14:32",
            13.0,
            None,
            Alignment::Start,
            1.0,
            false,
            None,
            600.0,
            false,
            false,
        );
        let m = measurement(&layout);
        draw_text(
            scene,
            &layout,
            on_bar,
            cw - m.width as f64 - 18.0,
            (panel_h - 16.0) / 2.0 + 1.0,
        );
    }

    // Algunos "items" del panel a la izquierda (taskbar-ish).
    let mut tx = 120.0;
    for name in ["pluma", "shuma", "nahual"] {
        let layout = ts.layout(
            name,
            12.5,
            None,
            Alignment::Start,
            1.0,
            false,
            None,
            600.0,
            false,
            false,
        );
        let m = measurement(&layout);
        let chip_w = m.width as f64 + 22.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(on_bar, 0.10),
            None,
            &RoundedRect::new(tx, 6.0, tx + chip_w, panel_h - 6.0, 4.0),
        );
        draw_text(scene, &layout, on_bar, tx + 11.0, (panel_h - 16.0) / 2.0 + 1.0);
        tx += chip_w + 8.0;
    }
}

/// Helper: pinta un layout de texto en (x,y) con un color sólido.
fn draw_text(
    scene: &mut vello::Scene,
    layout: &llimphi_ui::llimphi_text::parley::Layout<()>,
    color: Color,
    x: f64,
    y: f64,
) {
    use llimphi_ui::llimphi_text::draw_layout_brush_xf;
    let brush = peniko::Brush::Solid(color);
    draw_layout_brush_xf(scene, layout, &brush, Affine::translate((x, y)));
}

/// Dibuja una ventana: marco (border_width/color), barra de título
/// (titlebar_height), botones de control, y contenido simple.
fn paint_window(scene: &mut vello::Scene, ts: &mut Typesetter, w: &WinRect, s: &Skin, alpha: f32) {
    let bw = s.border_width;
    let tbh = s.titlebar_height;
    let border_col = with_alpha(if w.focused { s.border_focus } else { s.border_normal }, alpha);

    let outer = RoundedRect::new(w.x, w.y, w.x + w.w, w.y + w.h, 6.0);

    // Sombra suave bajo la ventana (sólo depicción, da profundidad).
    if alpha > 0.01 {
        let sh = KRect::new(w.x + 4.0, w.y + 8.0, w.x + w.w + 4.0, w.y + w.h + 10.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(Color::BLACK, 0.22 * alpha),
            None,
            &RoundedRect::from_rect(sh, 8.0),
        );
    }

    // Marco: se pinta como relleno del rect externo (el "borde" de mirada).
    if bw > 0.3 {
        scene.fill(peniko::Fill::NonZero, Affine::IDENTITY, border_col, None, &outer);
    }

    // Cuerpo de la ventana, recortado del marco por `bw`.
    let bx0 = w.x + bw;
    let by0 = w.y + bw;
    let bx1 = w.x + w.w - bw;
    let by1 = w.y + w.h - bw;
    let body = RoundedRect::new(bx0, by0, bx1, by1, 4.0);

    // Barra de título (si la vista la tiene): franja con el color de marco
    // enfocado degradado al panel; texto del título.
    if tbh > 0.5 {
        let tb = KRect::new(bx0, by0, bx1, by0 + tbh);
        let tb_col = if w.focused {
            // Barra activa: tono del border_focus, levemente más claro.
            with_alpha(s.border_focus, alpha)
        } else {
            with_alpha(s.theme.bg_button, alpha)
        };
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            tb_col,
            None,
            &RoundedRect::new(bx0, by0, bx1, by0 + tbh + 6.0, 4.0),
        );
        // Recortar la parte de abajo de la barra para que no sobresalga.
        // (Pintamos el cuerpo encima a continuación.)

        // Botones de control (semáforo a la derecha; círculos).
        let r = (tbh * 0.18).clamp(3.0, 7.0);
        let cy = by0 + tbh / 2.0;
        let cols = [
            Color::from_rgba8(0xE7, 0x4C, 0x3C, 255),
            Color::from_rgba8(0xF1, 0xC4, 0x0F, 255),
            Color::from_rgba8(0x2E, 0xCC, 0x71, 255),
        ];
        for (i, c) in cols.iter().enumerate() {
            let cx = bx1 - 16.0 - i as f64 * (r * 2.0 + 8.0);
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(*c, alpha),
                None,
                &vello::kurbo::Circle::new(Point::new(cx, cy), r),
            );
        }

        // Título.
        let on_tb = {
            let k = tb_col.components;
            let l = 0.2126 * k[0] + 0.7152 * k[1] + 0.0722 * k[2];
            if l < 0.55 { Color::WHITE } else { Color::from_rgba8(24, 28, 34, 255) }
        };
        let layout = ts.layout(
            w.title,
            (tbh * 0.5).clamp(11.0, 14.0) as f32,
            None,
            Alignment::Start,
            1.0,
            false,
            None,
            600.0,
            false,
            false,
        );
        draw_text(scene, &layout, with_alpha(on_tb, alpha), bx0 + 12.0, cy - 8.0);
    }

    // Cuerpo / contenido bajo la barra.
    let content_top = by0 + tbh;
    let content = RoundedRect::new(bx0, content_top, bx1, by1, if tbh > 0.5 { 0.0 } else { 4.0 });
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        with_alpha(s.theme.bg_panel, alpha),
        None,
        &body, // pinta todo el cuerpo (incluida zona detrás de la barra superior, queda tapada)
    );
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        with_alpha(s.theme.bg_app, alpha),
        None,
        &content,
    );

    // Contenido depictado: una "sidebar" + filas de texto/acento, simple.
    let inner_x = bx0 + 1.0;
    let inner_w = bx1 - bx0;
    let inner_h = by1 - content_top;
    if inner_w > 80.0 && inner_h > 40.0 {
        // Sidebar fina.
        let sb_w = (inner_w * 0.26).clamp(0.0, 130.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.theme.bg_panel, alpha),
            None,
            &KRect::new(inner_x, content_top, inner_x + sb_w, by1),
        );
        // Item activo en la sidebar (acento).
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.theme.bg_selected, alpha),
            None,
            &RoundedRect::new(
                inner_x + 8.0,
                content_top + 12.0,
                inner_x + sb_w - 8.0,
                content_top + 12.0 + 22.0,
                4.0,
            ),
        );
        // Filas de sidebar (muted).
        for i in 1..5 {
            let ry = content_top + 12.0 + i as f64 * 30.0;
            if ry + 10.0 > by1 {
                break;
            }
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.theme.fg_muted, 0.45 * alpha),
                None,
                &RoundedRect::new(inner_x + 14.0, ry, inner_x + sb_w - 14.0, ry + 8.0, 3.0),
            );
        }

        // Área principal: líneas de "texto" + un chip de acento (título).
        let mx = inner_x + sb_w + 16.0;
        let mw = (bx1 - mx - 16.0).max(0.0);
        if mw > 30.0 {
            // Encabezado (acento).
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.theme.accent, alpha),
                None,
                &RoundedRect::new(mx, content_top + 16.0, mx + (mw * 0.42), content_top + 16.0 + 12.0, 4.0),
            );
            let widths = [0.95, 0.8, 0.88, 0.6, 0.92, 0.7, 0.84];
            for (i, fw) in widths.iter().enumerate() {
                let ry = content_top + 44.0 + i as f64 * 22.0;
                if ry + 8.0 > by1 - 8.0 {
                    break;
                }
                scene.fill(
                    peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    with_alpha(s.theme.fg_text, 0.30 * alpha),
                    None,
                    &RoundedRect::new(mx, ry, mx + mw * fw, ry + 7.0, 3.0),
                );
            }
        }
    }

    // Re-trazar el borde por encima para una línea limpia (stroke).
    if bw > 0.3 {
        scene.stroke(
            &Stroke::new((bw * 0.6).max(0.8)),
            Affine::IDENTITY,
            with_alpha(border_col, alpha),
            None,
            &outer,
        );
    }
}

// ───────────────────────── rótulo de vista + wordmark ─────────────────────────

/// Rótulo sutil del nombre de la vista, abajo-centro, con fade del tramo.
fn paint_view_label(scene: &mut vello::Scene, ts: &mut Typesetter, label: &str, a: f32, cw: f64, ch: f64, _fg: Color) {
    if a <= 0.001 {
        return;
    }
    let size = 30.0_f32;
    let layout = ts.layout(label, size, None, Alignment::Start, 1.0, false, None, 600.0, false, false);
    let m = measurement(&layout);
    let pad_x = 22.0;
    let pad_y = 12.0;
    let bw = m.width as f64 + pad_x * 2.0;
    let bh = m.height as f64 + pad_y * 2.0;
    let bx = (cw - bw) / 2.0;
    let by = ch - bh - 48.0;
    // Pastilla de fondo translúcida.
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        with_alpha(Color::BLACK, 0.58 * a),
        None,
        &RoundedRect::new(bx, by, bx + bw, by + bh, 10.0),
    );
    // Texto SIEMPRE casi-blanco (la pastilla es negra): el fg de la vista es
    // oscuro en temas claros (XP/macOS/KDE) y se lavaba sobre la pastilla.
    draw_text(scene, &layout, with_alpha(Color::WHITE, 0.94 * a), bx + pad_x, by + pad_y - 2.0);
}

/// Wordmark de cierre: "mirada" + subtítulo.
fn paint_wordmark(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, accent: Color, fg: Color, fg_muted: Color) {
    let word_in = seg(t, 0.86, 0.96);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a <= 0.001 {
        return;
    }
    let size = 150.0_f32;
    let layout = ts.layout("mirada", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false);
    let m = measurement(&layout);
    let rise = lerp(28.0, 0.0, word_a as f64);
    let ox = (cw - m.width as f64) / 2.0;
    let oy = (ch - m.height as f64) / 2.0 - 26.0 + rise;
    draw_text(scene, &layout, with_alpha(fg, word_a), ox, oy);

    let sub_a = motion::ease_out_cubic(seg(t, 0.90, 0.995));
    if sub_a > 0.001 {
        let ssz = 28.0_f32;
        let sub = ts.layout(
            "one Wayland compositor, every desktop",
            ssz,
            None,
            Alignment::Start,
            1.0,
            false,
            None,
            400.0,
            false,
            false,
        );
        let sm = measurement(&sub);
        let dot_r = 6.0;
        let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
        let sx = (cw - block_w) / 2.0;
        let sy = oy + m.height as f64 + 16.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(accent, sub_a),
            None,
            &vello::kurbo::Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.5), dot_r),
        );
        draw_text(scene, &sub, with_alpha(fg_muted, sub_a), sx + dot_r * 2.0 + 14.0, sy);
    }
}

// ───────────────────────── timeline del morph ─────────────────────────
//
// La secuencia de vistas. Cada una ocupa un tramo igual; entre tramos hay un
// cross-fade de la piel. mirada encabeza (apertura) y el orden es el pedido:
// mirada → XP → mac → KDE → CDE → Hyprland → dwm.

const SEQ: [&str; 7] = [
    "mirada",
    "windows-xp",
    "mac",
    "kde",
    "solaris",
    "hyprland",
    "dwm",
];

/// Resuelve la piel interpolada para el tiempo `t∈[0,1]` dentro del tramo
/// de morph `[lo,hi]`. Devuelve también el progreso del label (para fade).
fn skin_at(t: f32, lo: f32, hi: f32) -> Skin {
    let local = seg(t, lo, hi); // 0..1 sobre toda la secuencia
    let n = SEQ.len();
    // Cada vista "asienta" un rato y luego transiciona. Dividimos en n tramos;
    // dentro de cada tramo, los últimos 35% son la transición a la siguiente.
    let span = 1.0 / n as f32;
    let idx = ((local / span).floor() as usize).min(n - 1);
    let within = (local - idx as f32 * span) / span; // 0..1 dentro del tramo
    let cur = skin(SEQ[idx]);
    if idx + 1 >= n {
        return cur; // última vista, sin morph saliente
    }
    let hold = 0.55_f32; // fracción del tramo que "se queda" en la vista
    if within < hold {
        cur
    } else {
        let tt = (within - hold) / (1.0 - hold);
        let eased = motion::ease_in_out_cubic(tt);
        let next = skin(SEQ[idx + 1]);
        lerp_skin(&cur, &next, eased)
    }
}

/// Alpha del rótulo de vista: alto cuando la piel "se asienta", fade en las
/// transiciones.
fn label_alpha(t: f32, lo: f32, hi: f32) -> f32 {
    let local = seg(t, lo, hi);
    let n = SEQ.len();
    let span = 1.0 / n as f32;
    let idx = ((local / span).floor() as usize).min(n - 1);
    let within = (local - idx as f32 * span) / span;
    // Aparece (0..0.15), se queda, se va (0.7..0.95) salvo la última que queda.
    let appear = seg(within, 0.02, 0.18);
    let last = idx + 1 >= n;
    let disappear = if last { 1.0 } else { 1.0 - seg(within, 0.72, 0.95) };
    (appear * disappear).clamp(0.0, 1.0)
}

// ───────────────────────── la escena por frame ─────────────────────────

fn build_and_paint(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64) {
    // Fases de la timeline:
    //   0.00–0.10  apertura: las ventanas teselan in (vista mirada/Dark)
    //   0.10–0.84  EL MORPH: atraviesa las 7 vistas
    //   0.84–1.00  cierre: fade-out del escritorio + wordmark "mirada"
    let morph_lo = 0.10;
    let morph_hi = 0.84;

    let skin = skin_at(t, morph_lo, morph_hi);

    // Fade-out del escritorio durante el cierre.
    let desk_fade = 1.0 - seg(t, 0.84, 0.92);

    // Fondo (escritorio). En el cierre desvanece hacia un fondo neutro oscuro.
    let close_bg = Color::from_rgba8(16, 15, 14, 255);
    let bg = lerp_color(skin.desktop, close_bg, 1.0 - desk_fade);
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        bg,
        None,
        &KRect::new(0.0, 0.0, cw, ch),
    );
    // Gradiente sutil sobre el fondo (wallpaper-ish), atenuado en el cierre.
    {
        let top = with_alpha(lerp_color(skin.theme.accent, skin.desktop, 0.82), 0.5 * desk_fade);
        let grad = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(cw * 0.6, ch))
            .with_stops([top, with_alpha(skin.desktop, 0.0)].as_slice());
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            &grad,
            None,
            &KRect::new(0.0, 0.0, cw, ch),
        );
    }

    if desk_fade > 0.001 {
        // Panel superior. dwm/Hyprland igual muestran una barra de status
        // depictada (mirada siempre tiene la barra de pata arriba).
        let panel_h = 34.0;
        paint_panel(scene, ts, &skin, cw, panel_h);

        // Área de trabajo = pantalla menos el panel.
        let work = KRect::new(0.0, panel_h, cw, ch);
        let wins = windows(work, skin.gap);

        // Apertura: stagger de entrada de las 3 ventanas (escala + alpha).
        for (i, w) in wins.iter().enumerate() {
            let delay = i as f32 * 0.04;
            let enter = motion::ease_out_back(seg(t, 0.0 + delay, 0.10 + delay));
            let a = (enter.min(1.0) * desk_fade).clamp(0.0, 1.0);
            if a <= 0.001 {
                continue;
            }
            // Pop de entrada: escala desde el centro de la ventana.
            let scale = lerp(0.92, 1.0, enter.min(1.0) as f64);
            let cx = w.x + w.w / 2.0;
            let cy = w.y + w.h / 2.0;
            let xf = Affine::translate((cx, cy)) * Affine::scale(scale) * Affine::translate((-cx, -cy));
            // Aplicar transform vía push_layer no es trivial; en su lugar
            // dibujamos directo (la escala sólo aporta en los primeros frames;
            // para el morph las ventanas ya están a escala 1).
            let _ = xf; // depicción: el pop se nota por el alpha del stagger.
            paint_window(scene, ts, w, &skin, a);
        }

        // Rótulo de la vista actual.
        let la = label_alpha(t, morph_lo, morph_hi) * desk_fade;
        let label_fg = Color::from_rgba8(238, 240, 244, 255);
        paint_view_label(scene, ts, skin.label, la, cw, ch, label_fg);
    }

    // Wordmark de cierre.
    let close_accent = Color::from_rgba8(0x6E, 0x8C, 0xDC, 255); // azul mirada
    let close_fg = Color::from_rgba8(236, 238, 244, 255);
    let close_muted = Color::from_rgba8(150, 156, 170, 255);
    paint_wordmark(scene, ts, t, cw, ch, close_accent, close_fg, close_muted);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args
        .next()
        .unwrap_or_else(|| "showreel_frames_mirada".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(360);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mirada-showreel"),
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
    let base = Color::from_rgba8(16, 15, 14, 255);

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };

        // Toda la pintura va por `paint_with` sobre un nodo full-screen — la
        // depicción es vector custom, no widgets de árbol (a diferencia del
        // showreel del toolkit). Mantiene el pipeline mount→layout→paint real.
        let root: View<()> = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .paint_with(move |scene, ts, _rect: PaintRect| {
            build_and_paint(scene, ts, t, cw, ch);
        });

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
            eprintln!("mirada-showreel: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("mirada-showreel: {n} frames en {out_dir}/ ({w}x{h})");
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
