use super::*;

/// `true` si la línea es una notice de cierre (`✔/✘/⏹`) — para que tanto
/// `update` (que no tiene theme) como la `view` calculen el cuerpo igual.
pub(crate) fn is_status_line(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('✔') || t.starts_with('✘') || t.starts_with('⏹')
}

/// Estado de cierre de un comando, para el badge (icono + color en vez del
/// crudo "exit N").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmdStatus {
    Running,
    Ok,
    Fail,
    Cancelled,
}

impl CmdStatus {
    /// Deriva el estado de la notice de cierre (`✔ exit 0`, `✘ exit N`,
    /// `⏹ cancel…`). `None` si no es una notice de estado.
    pub(crate) fn from_notice(text: &str) -> Option<Self> {
        let t = text.trim_start();
        if t.starts_with('✔') {
            Some(Self::Ok)
        } else if t.starts_with('⏹') {
            Some(Self::Cancelled)
        } else if t.starts_with('✘') {
            Some(Self::Fail)
        } else {
            None
        }
    }

    /// Icono vectorial + color del badge.
    pub(crate) fn icon_color(
        self,
        theme: &Theme,
    ) -> (llimphi_icons::Icon, llimphi_ui::llimphi_raster::peniko::Color) {
        use llimphi_icons::Icon;
        use llimphi_ui::llimphi_raster::peniko::Color;
        match self {
            CmdStatus::Ok => (Icon::Check, Color::from_rgba8(120, 200, 140, 255)),
            CmdStatus::Fail => (Icon::X, theme.fg_destructive),
            CmdStatus::Cancelled => (Icon::Stop, theme.fg_destructive),
            CmdStatus::Running => (Icon::Play, theme.accent),
        }
    }
}

/// Formato corto de bytes para el header de un run vivo: `B/KB/MB/GB`
/// sin decimales — entra cómodo en 96 px de slot. "0 B" tras arrancar
/// el run, "12 KB" mientras crece, "2 MB" para outputs gordos.
pub(crate) fn format_bytes_short(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if n < KB {
        format!("{n} B")
    } else if n < MB {
        format!("{} KB", n / KB)
    } else if n < GB {
        format!("{} MB", n / MB)
    } else {
        format!("{} GB", n / GB)
    }
}

/// Tiempo relativo legible ("hace 4 minutos", "hace 2 h", "hace 3 d"…).
/// `then`/`now` en segundos unix. Vacío si `then == 0` (sin timestamp).
/// Cubre del segundo al año; el foco es la lectura rápida del año en curso.
pub(crate) fn relative_time(then: u64, now: u64) -> String {
    if then == 0 {
        return String::new();
    }
    let d = now.saturating_sub(then);
    if d < 5 {
        "recién".to_string()
    } else if d < 60 {
        format!("hace {d} s")
    } else if d < 3600 {
        let m = d / 60;
        format!("hace {m} min")
    } else if d < 86_400 {
        let h = d / 3600;
        format!("hace {h} h")
    } else if d < 7 * 86_400 {
        let days = d / 86_400;
        format!("hace {days} d")
    } else if d < 30 * 86_400 {
        let w = d / (7 * 86_400);
        format!("hace {w} sem")
    } else if d < 365 * 86_400 {
        let mo = d / (30 * 86_400);
        format!("hace {mo} mes{}", if mo == 1 { "" } else { "es" })
    } else {
        let y = d / (365 * 86_400);
        format!("hace {y} año{}", if y == 1 { "" } else { "s" })
    }
}

/// Líneas del **cuerpo** de un bloque, en orden del buffer: stdout/stderr
/// y notices que no son de cierre, excluyendo el Prompt (header) y las
/// líneas de etapa (tee). Es exactamente lo que `command_card` pinta en el
/// cuerpo IDE-text; `update` la usa para mapear el puntero a (línea, col)
/// sobre el mismo texto. El editor las une con `\n`.
pub(crate) fn body_lines_for_block(state: &State, block: u64) -> Vec<String> {
    state
        .output
        .iter()
        .filter(|l| {
            l.block == block
                && l.kind != OutputKind::Prompt
                && l.stage.is_none()
                && !is_status_line(&l.text)
        })
        .map(|l| l.text.clone())
        .collect()
}

/// Kinds de las líneas del cuerpo, alineados 1:1 con
/// [`body_lines_for_block`] — para tintar stderr sin perder el resto.
pub(crate) fn body_kinds_for_block(state: &State, block: u64) -> Vec<OutputKind> {
    state
        .output
        .iter()
        .filter(|l| {
            l.block == block
                && l.kind != OutputKind::Prompt
                && l.stage.is_none()
                && !is_status_line(&l.text)
        })
        .map(|l| l.kind)
        .collect()
}

/// Titular semáforo (A5) de un bloque colapsado: resumen determinista
/// contado desde las decoraciones `Severity` del cuerpo —
/// *«3 errores · 12 avisos · 48 líneas · 4 s»*. El nerdo habitual escanea la
/// columna de headers como un log semáforo sin desplegar nada. `dur_secs` es
/// la duración del bloque (`block_ended − block_started`); se omite si < 1 s.
/// Una línea cuenta como error si contiene alguna palabra/glifo de severidad
/// Error; si no, como aviso si contiene alguno de Warn. El color lo decide el
/// llamador según [`titular_tiene_error`]/[`titular_tiene_aviso`].
pub(crate) fn semaforo_titular(lines: &[String], cwd: &std::path::Path, dur_secs: Option<u64>) -> String {
    let mut errores = 0usize;
    let mut avisos = 0usize;
    for l in lines {
        let mut linea_err = false;
        let mut linea_warn = false;
        for d in shuma_line::decorate::decorate_line(l, cwd) {
            match d.kind {
                shuma_line::decorate::DecorationKind::Severity(
                    shuma_line::decorate::Severity::Error,
                ) => linea_err = true,
                shuma_line::decorate::DecorationKind::Severity(
                    shuma_line::decorate::Severity::Warn,
                ) => linea_warn = true,
                _ => {}
            }
        }
        if linea_err {
            errores += 1;
        } else if linea_warn {
            avisos += 1;
        }
    }
    let plural = |n: usize, uno: &str, varios: &str| {
        if n == 1 {
            format!("{n} {uno}")
        } else {
            format!("{n} {varios}")
        }
    };
    let mut partes: Vec<String> = Vec::new();
    if errores > 0 {
        partes.push(plural(errores, "error", "errores"));
    }
    if avisos > 0 {
        partes.push(plural(avisos, "aviso", "avisos"));
    }
    partes.push(plural(lines.len(), "línea", "líneas"));
    if let Some(secs) = dur_secs {
        if secs >= 1 {
            partes.push(format!("{secs} s"));
        }
    }
    partes.join(" · ")
}

/// `true` si el titular semáforo reporta al menos un error (→ tinte rojo).
pub(crate) fn titular_tiene_error(titular: &str) -> bool {
    titular.contains("error")
}

/// `true` si el titular semáforo reporta avisos (→ tinte ámbar; subordinado
/// al rojo de error en el llamador).
pub(crate) fn titular_tiene_aviso(titular: &str) -> bool {
    titular.contains("aviso")
}

/// Mezcla lineal de dos colores sRGB (`t=0` → `a`, `t=1` → `b`). Vivía en el
/// `output_pane` viejo (borrado en la Fase 5 del SDD-TERMINAL); ahora reside
/// acá, junto a su único consumidor (`body_editor_palette`).
pub(crate) fn mix_color(
    a: llimphi_ui::llimphi_raster::peniko::Color,
    b: llimphi_ui::llimphi_raster::peniko::Color,
    t: f32,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let t = t.clamp(0.0, 1.0);
    let ca = a.components;
    let cb = b.components;
    Color::from_rgba8(
        ((ca[0] + (cb[0] - ca[0]) * t) * 255.0).round() as u8,
        ((ca[1] + (cb[1] - ca[1]) * t) * 255.0).round() as u8,
        ((ca[2] + (cb[2] - ca[2]) * t) * 255.0).round() as u8,
        255,
    )
}

/// Métricas del editor de cuerpo: mono 12px con `line_height` clavado a
/// `ROW_H` para que la contabilidad de alturas del scroll (que asume
/// ROW_H por línea) siga cuadrando.
pub(crate) fn body_editor_metrics() -> llimphi_widget_text_editor::EditorMetrics {
    let mut m = llimphi_widget_text_editor::EditorMetrics::for_font_size(12.0);
    m.line_height = ROW_H;
    m
}

/// Paleta del editor de cuerpo: fondo de la card (`bg_panel_alt`), gutter
/// sutil, resto desde el theme.
pub(crate) fn body_editor_palette(theme: &Theme) -> llimphi_widget_text_editor::EditorPalette {
    let mut p = llimphi_widget_text_editor::EditorPalette::from_theme(theme);
    p.bg = theme.bg_panel_alt;
    // Gutter un escalón más hundido que el cuerpo: la columna de numeración se
    // lee como gutter (look IDE), no flotando sobre el mismo fondo.
    p.bg_gutter = mix_color(theme.bg_panel_alt, theme.sunken(), 0.6);
    p
}

/// Panel de un PTY en **modo líneas** (sin alt-screen): pinta la pantalla
/// del programa como text de IDE read-only (numeración + mono), no como una
/// grilla apretada. Sin selección interactiva por ahora (el contenido viene
/// del screen vt100, no del buffer de OutputLine). Las teclas siguen yendo
/// al PTY (`is_tui_active`).
pub(crate) fn pty_lines_panel<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> View<HostMsg> {
    let lines = pty_line_text(state).unwrap_or_default();
    let n = lines.len().max(1);
    let mut ed = llimphi_widget_text_editor::EditorState::new();
    ed.set_text(&lines.join("\n"));
    let metrics = body_editor_metrics();
    let mut palette = body_editor_palette(theme);
    palette.bg = theme.sunken();
    palette.bg_gutter = theme.sunken();
    let editor = llimphi_widget_text_editor::text_editor_view::<HostMsg>(
        &ed,
        &palette,
        metrics,
        n,
        |_ev| None,
    );
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_basis: length(0.0_f32),
        flex_grow: 1.0,
        min_size: Size {
            width: Dimension::auto(),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.sunken())
    .radius(3.0)
    .clip(true)
    .children(vec![editor])
}

/// Color por tipo de archivo, estilo `ls --color` — para que el `ls` (y
/// cualquier listado con paths) deje de verse plano.
pub(crate) fn kind_color(
    kind: shuma_line::FileKind,
    theme: &Theme,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    use shuma_line::FileKind as K;
    match kind {
        K::Folder => Color::from_rgba8(100, 160, 235, 255),    // azul
        K::Symlink => Color::from_rgba8(90, 200, 205, 255),    // cyan
        K::Image => Color::from_rgba8(200, 140, 210, 255),     // magenta
        K::Audio => Color::from_rgba8(210, 165, 120, 255),     // ámbar
        K::Video => Color::from_rgba8(210, 140, 165, 255),     // rosa
        K::Archive => Color::from_rgba8(210, 120, 110, 255),   // rojo
        K::Document => Color::from_rgba8(205, 200, 140, 255),  // amarillo
        K::Code => Color::from_rgba8(130, 185, 225, 255),      // azul claro
        K::Data => Color::from_rgba8(150, 200, 160, 255),      // verde agua
        K::Font => Color::from_rgba8(190, 170, 220, 255),      // violeta
        K::Executable => Color::from_rgba8(130, 205, 140, 255), // verde
        K::Generic => theme.fg_text,
    }
}

/// Color de una decoración (path/url/grep/sha/issue/box) — el mismo
/// vocabulario semántico que el render por-línea viejo, ahora como runs de
/// color para el editor del cuerpo.
pub(crate) fn decoration_color(
    kind: &shuma_line::DecorationKind,
    theme: &Theme,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    use shuma_line::DecorationKind as Dk;
    match kind {
        Dk::Path {
            abs,
            is_dir,
            is_executable,
            is_symlink,
        } => kind_color(
            shuma_line::file_kind(abs, *is_dir, *is_executable, *is_symlink),
            theme,
        ),
        Dk::Url(_) => Color::from_rgba8(110, 180, 220, 255),
        Dk::GrepRef { .. } => theme.accent,
        Dk::GitSha(_) => Color::from_rgba8(210, 165, 120, 255),
        Dk::IssueRef(_) => Color::from_rgba8(200, 200, 140, 255),
        Dk::BoxDraw => theme.fg_muted,
        // Coloreo semántico de relleno: tonos suaves, claramente por
        // debajo de los accionables (paths/urls) en saturación.
        Dk::Number => Color::from_rgba8(209, 154, 102, 255), // naranja suave
        Dk::DateTime => Color::from_rgba8(126, 166, 180, 255), // teal apagado
        Dk::Severity(shuma_line::Severity::Error) => theme.fg_destructive,
        Dk::Severity(shuma_line::Severity::Warn) => Color::from_rgba8(220, 200, 120, 255),
        Dk::Severity(shuma_line::Severity::Ok) => Color::from_rgba8(130, 205, 140, 255),
        Dk::Version => Color::from_rgba8(187, 160, 220, 255), // violeta
        Dk::Percent => Color::from_rgba8(100, 200, 200, 255), // cian
        Dk::PermMask => Color::from_rgba8(140, 152, 175, 255), // gris azulado
    }
}

/// Runs de color `(byte_start, byte_end, Color)` por cada línea del cuerpo
/// de `block`, alimentando `text_editor_view_colored`: stderr en rojo, y
/// las decoraciones de `shuma-line` (paths por tipo, urls, grep, sha…)
/// coloreadas. Devuelve un vec alineado 1:1 con `body_lines_for_block`.
pub(crate) fn body_color_runs(
    state: &State,
    block: u64,
    theme: &Theme,
) -> Vec<Vec<(usize, usize, llimphi_ui::llimphi_raster::peniko::Color)>> {
    let lines = body_lines_for_block(state, block);
    let kinds = body_kinds_for_block(state, block);
    lines
        .iter()
        .enumerate()
        .map(|(i, text)| {
            // stderr: toda la línea en rojo (señal de error, además del tinte).
            if matches!(kinds.get(i), Some(OutputKind::Stderr)) {
                return vec![(0usize, text.len(), theme.fg_destructive)];
            }
            shuma_line::decorate_line(text, &state.cwd)
                .into_iter()
                .filter(|d| d.start < d.end && d.end <= text.len())
                .map(|d| (d.start, d.end, decoration_color(&d.kind, theme)))
                .collect()
        })
        .collect()
}

pub(crate) fn pretty_path(p: &std::path::Path) -> String {
    let full = p.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if full == home {
            return "~".into();
        }
        if let Some(rest) = full.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    full
}

#[cfg(test)]
mod a5_titular_tests {
    use super::semaforo_titular;

    fn cwd() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp")
    }

    #[test]
    fn cuenta_errores_avisos_y_duracion() {
        let lines = vec![
            "Compiling shuma v0.1.0".to_string(),
            "error[E0308]: mismatched types".to_string(),
            "error: could not compile".to_string(),
            "warning: unused variable `x`".to_string(),
            "Finished".to_string(),
        ];
        let t = semaforo_titular(&lines, &cwd(), Some(4));
        assert_eq!(t, "2 errores · 1 aviso · 5 líneas · 4 s");
    }

    #[test]
    fn limpio_sin_severidad() {
        let lines = vec!["total 248".to_string(), "CLAUDE.md".to_string()];
        // Sin errores/avisos y duración 0 → sólo el conteo de líneas.
        let t = semaforo_titular(&lines, &cwd(), Some(0));
        assert_eq!(t, "2 líneas");
    }

    #[test]
    fn singular_un_error() {
        let lines = vec!["error: boom".to_string()];
        let t = semaforo_titular(&lines, &cwd(), None);
        assert_eq!(t, "1 error · 1 línea");
    }
}
