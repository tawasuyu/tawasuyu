//! Rail derecho de herramientas, panel de cada herramienta (historial,
//! monitor, explorador, matilda) e iconos vectoriales.

use super::chrome::RAIL_W;
use super::super::*;
use super::monitors::monitor_stack;
use super::widgets::*;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, FlexDirection, JustifyContent, Rect, Size};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::Theme;

/// Ancho de la columna de historial a la izquierda (px).
const HISTORY_W: f32 = 220.0;

// ─── Rail de herramientas ───────────────────────────────────────────

/// El rail DERECHO de herramientas de la sesión activa.
pub(super) fn tool_rail(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};

    let items: Vec<DockRailItem> = Tool::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| DockRailItem { id: i as u64, active: model.active_tool == Some(*t) })
        .collect();

    dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            let t = Tool::ALL.get(id as usize).copied().unwrap_or(Tool::History);
            tool_icon(t, size, color)
        },
        move |id| Msg::SelectTool(Tool::ALL.get(id as usize).copied().unwrap_or(Tool::History)),
        |_| None,
    )
}

/// Icono vectorial de una herramienta del rail derecho (`paint_with` + kurbo).
pub(super) fn tool_icon(tool: Tool, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(size), height: length(size) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{
            Affine, BezPath, Circle, Point, RoundedRect, Stroke,
        };
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.34).max(2.0);
        let stroke = Stroke::new((r * 0.22).max(1.2));
        match tool {
            // Historial: reloj.
            Tool::History => {
                scene.stroke(
                    &stroke,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new((cx, cy), r),
                );
                let mut h = BezPath::new();
                h.move_to(Point::new(cx, cy));
                h.line_to(Point::new(cx, cy - r * 0.55));
                h.move_to(Point::new(cx, cy));
                h.line_to(Point::new(cx + r * 0.45, cy));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &h);
            }
            // Monitor: tres barras verticales.
            Tool::Monitor => {
                let heights = [0.55_f64, 0.95, 0.7];
                let bw = r * 0.45;
                let gap = r * 0.32;
                let total = 3.0 * bw + 2.0 * gap;
                let x0 = cx - total / 2.0;
                for (i, h) in heights.iter().enumerate() {
                    let x = x0 + i as f64 * (bw + gap);
                    let top = (cy + r) - 2.0 * r * h;
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        color,
                        None,
                        &RoundedRect::new(x, top, x + bw, cy + r, 1.0),
                    );
                }
            }
            // Explorer: carpeta.
            Tool::Explorer => {
                let body = RoundedRect::new(cx - r, cy - r * 0.5, cx + r, cy + r * 0.75, 2.0);
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &body);
                let mut tab = BezPath::new();
                tab.move_to(Point::new(cx - r, cy - r * 0.5));
                tab.line_to(Point::new(cx - r * 0.4, cy - r * 0.5));
                tab.line_to(Point::new(cx - r * 0.2, cy - r * 0.85));
                tab.line_to(Point::new(cx - r, cy - r * 0.85));
                tab.close_path();
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &tab);
            }
            // Matilda: tres racks apilados.
            Tool::Matilda => {
                for i in 0..3 {
                    let y = cy - r + i as f64 * (r * 0.78);
                    scene.stroke(
                        &stroke,
                        Affine::IDENTITY,
                        color,
                        None,
                        &RoundedRect::new(cx - r, y, cx + r, y + r * 0.5, 1.5),
                    );
                }
            }
        }
    })
}

// ─── Panel de herramienta ───────────────────────────────────────────

/// El panel de la herramienta activa (entre el canvas y el rail derecho).
pub(super) fn tool_panel(model: &Model, tool: Tool, theme: &Theme) -> View<Msg> {
    let inner = match tool {
        Tool::History => history_column(model, theme),
        Tool::Monitor => monitor_stack(model, theme),
        Tool::Explorer => explorer_panel(model, theme),
        Tool::Matilda => matilda_panel(model, theme),
    };
    panel_frame(vec![inner], theme)
}

// ─── Historial ──────────────────────────────────────────────────────

/// La columna de historial del rail derecho.
pub(super) fn history_column(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let mut comandos: Vec<String> = Vec::new();
    if let Some(s) = model.active() {
        if let ModuleState::Shell(sh) = &s.shell().state {
            comandos = sh
                .output
                .iter()
                .filter(|l| l.kind == shuma_module_shell::OutputKind::Prompt)
                .map(|l| l.text.trim_start_matches("$ ").to_string())
                .collect();
        }
    }
    comandos.reverse();
    let mut grupos: Vec<(String, usize)> = Vec::new();
    for c in comandos {
        if let Some(g) = grupos.iter_mut().find(|(t, _)| *t == c) {
            g.1 += 1;
        } else {
            grupos.push((c, 1));
        }
    }
    grupos.truncate(60);

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned("Historial".to_string(), 11.0, theme.fg_muted, Alignment::Start);

    let mut children = vec![header];
    if grupos.is_empty() {
        children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                "(sin comandos aún)".to_string(),
                11.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
    } else {
        for (cmd, count) in grupos {
            children.push(history_row(&cmd, count, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(HISTORY_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

/// Una fila del historial: `[ comando…  ×N  ▶ ]`.
pub(super) fn history_row(cmd: &str, count: usize, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let mut cuerpo_hijos: Vec<View<Msg>> = vec![View::new(Style {
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(cmd.to_string(), 12.0, theme.fg_text, Alignment::Start)];
    if count > 1 {
        cuerpo_hijos.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                margin: Rect {
                    left: length(6.0_f32),
                    right: length(0.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(format!("×{count}"), 10.0, theme.fg_muted, Alignment::End),
        );
    }
    let cuerpo = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(cuerpo_hijos)
    .on_click(Msg::RunFromHistory(cmd.to_string()));

    let run = View::new(Style {
        size: Size { width: length(22.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_button_hover)
    .text_aligned("▶".to_string(), 10.0, theme.accent, Alignment::Center)
    .on_click(Msg::RunFromHistoryNow(cmd.to_string()));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .children(vec![cuerpo, run])
}

// ─── Explorer ───────────────────────────────────────────────────────

/// Panel Explorer: lista los archivos del cwd de la sesión. Para sesiones
/// remotas (Remote / RemoteContainer) el listado viene del cache
/// `model.explorer` (lo trae `reconcile_explorer` off-thread por SSH);
/// para locales lee el filesystem directamente con `read_dir`.
pub(super) fn explorer_panel(model: &Model, theme: &Theme) -> View<Msg> {
    // Una búsqueda de archivos activa (`:buscar-archivos`) de la sesión actual
    // reemplaza el listado del cwd por sus resultados rankeados.
    if let Some(fs) = &model.file_search {
        if fs.session == model.active_session {
            return explorer_search_panel(fs, theme);
        }
    }
    let remoto = model.active().and_then(|s| match &s.shell().state {
        ModuleState::Shell(sh)
            if matches!(sh.source, Source::Remote { .. } | Source::RemoteContainer { .. }) =>
        {
            Some(sh.cwd.display().to_string())
        }
        _ => None,
    });
    match remoto {
        Some(cwd) => explorer_panel_remote(model, &cwd, theme),
        None => explorer_panel_local(model, theme),
    }
}

/// Explorer de una sesión local: `read_dir` directo del cwd.
fn explorer_panel_local(model: &Model, theme: &Theme) -> View<Msg> {
    let cwd = model
        .active()
        .and_then(|s| match &s.shell().state {
            ModuleState::Shell(sh) => Some(sh.cwd.display().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| ".".to_string());

    let mut filas: Vec<View<Msg>> = vec![tool_header(&format!("Explorer · {cwd}"), theme)];
    match std::fs::read_dir(&cwd) {
        Ok(rd) => {
            let mut entradas: Vec<(bool, String)> = rd
                .flatten()
                .map(|e| {
                    let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    (dir, e.file_name().to_string_lossy().to_string())
                })
                .collect();
            entradas.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            entradas.truncate(200);
            for (dir, name) in entradas {
                filas.push(explorer_row(dir, &name, theme));
            }
        }
        Err(_) => filas.push(explorer_note("(cwd inaccesible)", theme)),
    }
    explorer_column(filas)
}

/// Explorer de una sesión remota: renderiza el cache `model.explorer`.
fn explorer_panel_remote(model: &Model, cwd: &str, theme: &Theme) -> View<Msg> {
    let mut filas: Vec<View<Msg>> = vec![explorer_remote_header(cwd, theme)];
    // Sin conexión no listamos nada (reconcile_explorer tampoco spawnea).
    let conectada = model.active().map(|s| s.conn == ConnState::Connected).unwrap_or(false);
    if !conectada {
        filas.push(explorer_note("(sesión no conectada)", theme));
        return explorer_column(filas);
    }
    // El cache vale sólo si su clave coincide con la sesión + cwd de ahora.
    let vigente = model
        .explorer
        .key
        .as_ref()
        .is_some_and(|(s, p)| *s == model.active_session && p == cwd);
    if vigente {
        match &model.explorer.state {
            ExplorerState::Loaded(entries) if entries.is_empty() => {
                filas.push(explorer_note("(vacío)", theme));
            }
            ExplorerState::Loaded(entries) => {
                for e in entries {
                    filas.push(explorer_row(e.is_dir, &e.name, theme));
                }
            }
            ExplorerState::Error(err) => filas.push(explorer_note(&format!("✘ {err}"), theme)),
            _ => filas.push(explorer_note("cargando…", theme)),
        }
    } else {
        filas.push(explorer_note("cargando…", theme));
    }
    explorer_column(filas)
}

/// Header del Explorer remoto: título + botón ↻ para re-listar.
fn explorer_remote_header(cwd: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let titulo = View::new(Style {
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(format!("Explorer · {cwd}"), 11.0, theme.fg_muted, Alignment::Start);
    let refrescar = View::new(Style {
        size: Size { width: length(24.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_button_hover)
    .text_aligned("↻".to_string(), 12.0, theme.accent, Alignment::Center)
    .on_click(Msg::RefreshExplorer);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![titulo, refrescar])
}

/// Una fila del Explorer: click en un dir → `cd`, en un archivo → inserta su
/// nombre en el input. Compartida por el panel local y el remoto.
fn explorer_row(dir: bool, name: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let etiqueta = if dir { format!("{name}/") } else { name.to_string() };
    let cmd = if dir { format!("cd {name}") } else { name.to_string() };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::RunFromHistory(cmd))
    .text_aligned(
        etiqueta,
        12.0,
        if dir { theme.accent } else { theme.fg_text },
        Alignment::Start,
    )
}

/// Panel del Explorer en modo **búsqueda de archivos** (`:buscar-archivos`):
/// encabezado con la consulta + ✕ para volver al cwd, y las rutas rankeadas
/// como filas clickeables (insertan la ruta en el input).
fn explorer_search_panel(fs: &crate::types::FileSearch, theme: &Theme) -> View<Msg> {
    let mut filas: Vec<View<Msg>> = vec![explorer_search_header(&fs.query, theme)];
    if fs.hits.is_empty() {
        filas.push(explorer_note("(sin coincidencias por significado)", theme));
    } else {
        for (path, score) in &fs.hits {
            filas.push(explorer_search_row(path, *score, theme));
        }
    }
    explorer_column(filas)
}

/// Encabezado del modo búsqueda: «🔎 query» + botón ✕ que limpia la búsqueda.
fn explorer_search_header(query: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let titulo = View::new(Style {
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(format!("🔎 {query}"), 11.0, theme.fg_muted, Alignment::Start);
    let limpiar = View::new(Style {
        size: Size { width: length(24.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_button_hover)
    .text_aligned("✕".to_string(), 12.0, theme.accent, Alignment::Center)
    .on_click(Msg::ClearFileSearch);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![titulo, limpiar])
}

/// Una fila de resultado de búsqueda: `NN% ruta`, click inserta la ruta en el
/// input (como una fila de archivo del Explorer).
fn explorer_search_row(path: &str, score: f32, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let pct = View::new(Style {
        size: Size { width: length(34.0_f32), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(format!("{}%", (score * 100.0).round() as i32), 10.0, theme.fg_muted, Alignment::Center);
    let nombre = View::new(Style {
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(path.to_string(), 12.0, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::OpenFile(path.to_string()))
    .children(vec![pct, nombre])
}

/// Una línea de nota tenue del Explorer (vacío / cargando / error).
fn explorer_note(text: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, theme.fg_muted, Alignment::Start)
}

/// El contenedor en columna del panel Explorer.
fn explorer_column(filas: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(filas)
}

// ─── Matilda ────────────────────────────────────────────────────────

/// Panel Matilda: hosts + vhosts del inventario de la sesión activa.
pub(super) fn matilda_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(session) = model.active() else {
        return tool_header("Matilda", theme);
    };
    let st = match &session.matilda.state {
        ModuleState::Matilda(st) => st.as_ref(),
        _ => return tool_header("Matilda", theme),
    };
    let slot = Slot::Session(model.active_session, Which::Matilda);

    let acciones = shuma_module_matilda::contributions(st)
        .shortcuts
        .into_iter()
        .map(|spec| {
            action_button(
                &spec.label,
                Msg::ShortcutClicked(slot.clone(), spec.action),
                theme,
            )
        })
        .collect::<Vec<_>>();
    let barra = chip_row(acciones);

    let hosts_v = hosts_view(&st.desired, theme);
    let vhosts_v = vhosts_view(&st.desired, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .children(vec![barra, hosts_v, vhosts_v])
}
