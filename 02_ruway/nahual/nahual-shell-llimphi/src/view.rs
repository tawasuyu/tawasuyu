//! Vista del shell nahual: `shell_view`/`shell_view_overlay` y todos los
//! helpers de render (sidebar, canvas, dientes, paneles, grilla, listas,
//! detalle). Movido de `main.rs` en el split de 2026-06-12 (puro movimiento
//! de código).

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::modelo::*;
use crate::helpers::*;
use crate::overlays::*;
use crate::state::{Label, ShellState};
use crate::ops::OpStatus;
use llimphi_ui::{DragPhase, View};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::Position,
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_theme::Theme;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette, GridSpec};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_menubar::{menubar_view, menubar_overlay_animated};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};
use llimphi_widget_text_editor::{text_editor_view_full, EditorMetrics, EditorPalette};
use tullpu_module as tullpu;
use media_module as mediamod;
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_context_menu::context_menu_view;
use nahual_source_core::{Navigator, Node, NodeKind};
use nahual_text_viewer_llimphi::{text_viewer_view, PreviewState, TextViewerPalette};
use nahual_image_viewer_llimphi::{image_viewer_view, ImageViewerPalette};
use nahual_video_viewer_llimphi::{video_viewer_view, VideoViewerPalette};
use nahual_audio_viewer_llimphi::{audio_viewer_view, AudioViewerPalette};
use nahual_card_viewer_llimphi::{card_viewer_view, CardViewerPalette};
use nahual_tree_viewer_llimphi::{tree_viewer_view, TreeViewerPalette};
use nahual_hex_viewer_llimphi::{hex_viewer_view, HexViewerPalette};
use nahual_table_viewer_llimphi::{table_viewer_view, TableViewerPalette};
use nahual_markdown_viewer_llimphi::{markdown_viewer_view, MarkdownViewerPalette};
use nahual_archive_viewer_llimphi::{archive_viewer_view, ArchiveViewerPalette};
use nahual_font_viewer_llimphi::{font_viewer_view, FontViewerPalette};
use nahual_map_viewer_llimphi::{map_viewer_view, MapViewerPalette};

/// Cuerpo de `App::view`: arma la composición completa del shell.
pub(crate) fn shell_view(model: &Model) -> View<Msg> {
    let theme = model.theme;
    let splitter_palette = SplitterPalette::from_theme(&theme);
    let text_palette = TextViewerPalette::from_theme(&theme);
    let image_palette = ImageViewerPalette::from_theme(&theme);
    let video_palette = VideoViewerPalette::from_theme(&theme);
    let audio_palette = AudioViewerPalette::from_theme(&theme);
    let card_palette = CardViewerPalette::from_theme(&theme);
    let tree_palette = TreeViewerPalette::from_theme(&theme);
    let hex_palette = HexViewerPalette::from_theme(&theme);
    let table_palette = TableViewerPalette::from_theme(&theme);
    let markdown_palette = MarkdownViewerPalette::from_theme(&theme);
    let archive_palette = ArchiveViewerPalette::from_theme(&theme);
    let font_palette = FontViewerPalette::from_theme(&theme);
    let map_palette = MapViewerPalette::from_theme(&theme);
    let menu = app_menu(model);
    let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
    let viewer_pane = match &model.preview {
        PreviewPane::Empty => text_viewer_view::<Msg>(
            &PreviewState::Empty,
            None,
            &text_palette,
        ),
        PreviewPane::Text(state) => text_viewer_view::<Msg>(
            state,
            model.preview_of.as_deref(),
            &text_palette,
        ),
        PreviewPane::Image(state) => image_viewer_view::<Msg>(
            state,
            model.preview_of.as_deref(),
            &image_palette,
        ),
        PreviewPane::Video(state) => video_viewer_view::<Msg>(state, &video_palette),
        PreviewPane::Audio(state) => audio_viewer_view::<Msg>(state, &audio_palette),
        PreviewPane::Card(state) => {
            card_viewer_view::<Msg>(state, model.preview_of.as_deref(), &card_palette)
        }
        PreviewPane::Tree(state) => {
            tree_viewer_view::<Msg>(state, model.preview_of.as_deref(), &tree_palette)
        }
        PreviewPane::Hex(state) => {
            hex_viewer_view::<Msg>(state, model.preview_of.as_deref(), &hex_palette)
        }
        PreviewPane::Table(state) => {
            table_viewer_view::<Msg>(state, model.preview_of.as_deref(), &table_palette)
        }
        PreviewPane::Markdown(state) => {
            markdown_viewer_view::<Msg>(state, model.preview_of.as_deref(), &markdown_palette)
        }
        PreviewPane::Archive(state) => {
            archive_viewer_view::<Msg>(state, model.preview_of.as_deref(), &archive_palette)
        }
        PreviewPane::Font(state) => {
            font_viewer_view::<Msg>(state, model.preview_of.as_deref(), &font_palette)
        }
        PreviewPane::Map(state) => {
            map_viewer_view::<Msg, _>(
                state,
                model.preview_of.as_deref(),
                &map_palette,
                &model.map_view,
                // Clic → fracción del rect (el update resuelve con hit_test).
                |lx, ly, w, h| {
                    (w > 0.0 && h > 0.0).then(|| Msg::MapClick(lx / w, ly / h))
                },
            )
            // Arrastrar el panel panea la cámara del mapa.
            .draggable(|phase, dx, dy| match phase {
                DragPhase::Move => Some(Msg::MapPan(dx, dy)),
                DragPhase::End => None,
            })
        }
        // El visor de texto muestra el fuente HTML; abrir (Enter) lanza puriy.
        PreviewPane::Web(state) => text_viewer_view::<Msg>(
            state,
            model.preview_of.as_deref(),
            &text_palette,
        ),
    };

    // El CANVAS es la vista de la carpeta (lista/detalle/iconos/galería a
    // ancho completo); en dual, dos columnas de archivos. El visor del
    // archivo abierto vive en un **sidebar derecho** resizable (Esc/⌫ lo
    // cierra) — nunca tapa la vista de carpeta.
    let folder_view = if model.dual {
        splitter_two(
            Direction::Row,
            pane_column(model, 0, model.focus == 0, &theme),
            PaneSize::Fixed(model.list_width),
            pane_column(model, 1, model.focus == 1, &theme),
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeList(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        )
    } else {
        pane_column(model, model.focus, true, &theme)
    };
    // Centro: la app integrada del canvas si hay una abierta (editor /
    // imagen / media); si no, la vista de carpeta.
    let centro = match &model.canvas {
        Some(canvas) => canvas_app_view(canvas, model, &theme),
        None => folder_view,
    };
    // Canal interno: el contenido arranca después del ancho del rail para
    // que los dientes (overlay) no tapen las primeras columnas.
    let canvas_padded = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        min_size: Size { width: length(0.0), height: length(0.0) },
        padding: Rect {
            left: length(SESSION_RAIL_W),
            right: length(0.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![centro]);
    // Dientes en los DOS bordes internos del área central (patrón
    // canónico de cosmos: el rail flota sobre el canvas, el panel del
    // item activo va como pane al costado). El rail derecho vive acá —
    // dentro, pegado al borde interno del panel derecho — no en el borde
    // de la ventana.
    let center_host = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![
        canvas_padded,
        session_teeth_overlay(model, &theme),
        right_teeth_overlay(model, &theme),
    ]);
    // Panel derecho: tools del editor de imágenes (diente tools) o el
    // visor de preview (diente lupa) — comparten el costado.
    let tools_pane: Option<View<Msg>> = match (&model.canvas, model.tools_open) {
        (Some(CanvasApp::Imagen(st)), true) => {
            Some(tullpu::tools_panel(st, &theme, Msg::CanvasTullpu))
        }
        _ => None,
    };
    let canvas_area = if let Some(tools) = tools_pane {
        splitter_two(
            Direction::Row,
            center_host,
            PaneSize::Flex,
            tools,
            PaneSize::Fixed(model.tools_w),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeTools(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        )
    } else if model.viewer_open {
        splitter_two(
            Direction::Row,
            center_host,
            PaneSize::Flex,
            viewer_pane,
            PaneSize::Fixed(model.preview_w),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizePreview(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        )
    } else {
        center_host
    };

    // Sidebar único (árbol de carpetas) | canvas, con splitter.
    let body = splitter_two(
        Direction::Row,
        sidebar_view(model, &theme),
        PaneSize::Fixed(model.tree_w),
        canvas_area,
        PaneSize::Flex,
        |phase, dx| match phase {
            DragPhase::Move => Some(Msg::ResizeTree(dx)),
            DragPhase::End => None,
        },
        &splitter_palette,
    );
    let body_wrap = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![body]);

    let mut col: Vec<View<Msg>> = vec![menubar, shell_toolbar(model, &theme), body_wrap];
    if let Some(panel) = queue_panel(model, &theme) {
        col.push(panel);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    // Right-click en la raíz (origen 0,0 ⇒ local == coords de ventana)
    // abre el menú contextual sobre la entrada seleccionada.
    .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
    .children(col)
}

/// Cuerpo de `App::view_overlay`: modales + menús flotantes.
pub(crate) fn shell_view_overlay(model: &Model) -> Option<View<Msg>> {
    // Los modales de operación (prompt de nombre, confirmación de borrado)
    // van por encima de todo.
    if let Some(p) = &model.prompt {
        return Some(prompt_overlay(p, &model.theme));
    }
    if let Some(targets) = &model.confirm_delete {
        return Some(confirm_overlay(targets, &model.theme));
    }
    if let Some(b) = &model.batch {
        return Some(batch_overlay(b, &model.theme));
    }
    // El menú contextual del nodo seleccionado tiene prioridad.
    if let Some((x, y)) = model.context_menu {
        return Some(context_menu_view(context_menu_spec(model, x, y)));
    }
    // Si no, el dropdown del menú principal.
    let menu = app_menu(model);
    menubar_overlay_animated(
        &menubar_spec(&menu, model, &model.theme),
        model.menu_active,
        model.menu_anim.value(),
    )
}

/// Ícono vectorial (real, no glifo unicode) para una fila del árbol.
pub(crate) fn tree_icon(icon: Icon, selected: bool, theme: &Theme) -> View<Msg> {
    let color = if selected { theme.fg_text } else { theme.fg_muted };
    View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![icon_view(icon, color, 1.7)])
}

/// Acumula recursivamente las filas visibles del árbol bajo `path`.
pub(crate) fn push_tree_node(
    model: &Model,
    path: &std::path::Path,
    depth: usize,
    cur: &std::path::Path,
    icon: Icon,
    theme: &Theme,
    rows: &mut Vec<TreeRow<Msg>>,
) {
    let expanded = model.tree_expanded.contains(path);
    let selected = path == cur;
    rows.push(
        TreeRow::new(
            node_label(path),
            depth,
            true,
            expanded,
            selected,
            Msg::TreeToggle(path.to_path_buf()),
            Msg::TreeSelect(path.to_path_buf()),
        )
        .with_icon(tree_icon(icon, selected, theme)),
    );
    if expanded {
        if let Some(children) = model.tree_children.get(path) {
            for child in children {
                // Carpeta cerrada/abierta según su propio estado.
                let ic = if model.tree_expanded.contains(child) {
                    Icon::FolderOpen
                } else {
                    Icon::Folder
                };
                push_tree_node(model, child, depth + 1, cur, ic, theme, rows);
            }
        }
    }
}

/// Filas aplanadas del árbol lateral, partiendo de las raíces.
pub(crate) fn build_tree_rows(model: &Model, theme: &Theme) -> Vec<TreeRow<Msg>> {
    let cur = cur_dir(model);
    let mut rows: Vec<TreeRow<Msg>> = Vec::new();
    for (root, icon) in tree_roots(&model.state) {
        push_tree_node(model, &root, 0, &cur, icon, theme, &mut rows);
    }
    rows
}

/// Sidebar **único**: el árbol de carpetas navegable (home · raíz · favoritos),
/// con íconos reales. Click en el chevron expande/colapsa (`TreeToggle`); click
/// en la fila navega el panel activo (`TreeSelect`). La rueda lo scrollea por
/// filas (`TreeScroll`) — sin esto el wheel caía al canvas. Ancho fijo. El set
/// de descolapsadas y el scroll se recuerdan **por sesión**.
pub(crate) fn sidebar_view(model: &Model, theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text("CARPETAS", 12.0, theme.fg_muted);

    // Ventaneo: sólo las filas que entran (offset recordado por sesión).
    let all = build_tree_rows(model, theme);
    let vis = tree_visible_rows(model);
    let off = model.tree_scroll.min(all.len().saturating_sub(vis));
    let rows: Vec<TreeRow<Msg>> = all.into_iter().skip(off).take(vis).collect();

    let tree = tree_view(TreeSpec {
        rows,
        row_height: TREE_ROW_H,
        indent_px: 14.0,
        palette: TreePalette::from_theme(theme),
        guides: true,
    });
    let tree_wrap = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        // El ancho lo dicta el splitter del sidebar (pane Fixed(tree_w)).
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    // La rueda sobre el sidebar la rutea `on_wheel` por región (cursor.x <
    // tree_w) — el handler local se perdía entre updates rápidos.
    .children(vec![header, tree_wrap])
}

/// La app integrada abierta en el canvas: editor de texto potente (con
/// header de estado y Ctrl+S), visor de imagen con zoom/pan, o player de
/// media. Esc/⌫ vuelve a la vista de carpeta.
pub(crate) fn canvas_app_view(canvas: &CanvasApp, model: &Model, theme: &Theme) -> View<Msg> {
    match canvas {
        CanvasApp::Texto { path, editor, dirty, saved } => {
            let nombre = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let estado = if *dirty {
                "● sin guardar"
            } else if *saved {
                "✓ guardado"
            } else {
                ""
            };
            let titulo = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("{nombre}  {estado}"), 13.0, theme.fg_text);
            let hint = View::new(Style {
                size: Size { width: auto(), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text("Ctrl+S guarda · Esc cierra", 11.5, theme.fg_muted);
            let header = View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: pad_h(12.0),
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .children(vec![titulo, hint]);

            let cuerpo = text_editor_view_full(
                editor,
                &EditorPalette::from_theme(theme),
                EditorMetrics::for_font_size(13.0),
                canvas_editor_lines(model),
                language_for_path(path),
                &[],
                |ev| Some(Msg::CanvasEditPointer(ev)),
            );
            let cuerpo_wrap = View::new(Style {
                flex_grow: 1.0,
                min_size: Size { width: length(0.0), height: length(0.0) },
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![cuerpo]);

            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![header, cuerpo_wrap])
        }
        // Sólo el lienzo: los tools viven en el diente derecho del shell
        // (`right_teeth_overlay` + `tullpu::tools_panel`).
        CanvasApp::Imagen(st) => tullpu::lienzo_view(st, theme, Msg::CanvasTullpu),
        CanvasApp::Media(st) => mediamod::view(st, theme, Msg::CanvasMedia),
    }
}

/// Rail de dientes del **borde interno derecho** del área central (espejo
/// del rail de sesiones; el panel del diente activo va como pane al costado,
/// patrón canónico de cosmos). Dientes: 🔍 preview (siempre) y ✎ tools del
/// editor de imágenes (sólo con `CanvasApp::Imagen` abierta).
pub(crate) fn right_teeth_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let mut items = vec![DockRailItem { id: 0, active: model.viewer_open }];
    if matches!(model.canvas, Some(CanvasApp::Imagen(_))) {
        items.push(DockRailItem { id: 1, active: model.tools_open });
    }
    let rail = dock_rail_view(
        &items,
        SESSION_RAIL_W,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            let ic = if id == 1 { Icon::Edit } else { Icon::Search };
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(ic, color, 1.7)])
        },
        |id| {
            if id == 1 {
                Msg::ToggleToolsPanel
            } else {
                Msg::TogglePreviewPanel
            }
        },
        |_payload| -> Option<Msg> { None },
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            right: length(0.0_f32),
            left: auto(),
            bottom: auto(),
        },
        size: Size { width: length(SESSION_RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail])
}

/// **Dientes** de sesión como overlay absoluto pegado al borde interno del
/// canvas (el patrón canónico de cosmos: `dock_rail_overlay`): cada diente
/// (`llimphi-widget-dock-rail`) es una sesión de trabajo y sobresale del
/// sidebar sobre el canvas. Click activa esa sesión (su árbol + su vista de
/// carpeta vuelven); debajo, un `+` abre una sesión nueva.
pub(crate) fn session_teeth_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let items: Vec<DockRailItem> = (0..model.sessions.len())
        .map(|i| DockRailItem { id: i as u64, active: i == model.active })
        .collect();
    let rail = dock_rail_view(
        &items,
        SESSION_RAIL_W,
        &DockRailPalette::from_theme(theme),
        |_id, size, color| {
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(Icon::Folder, color, 1.7)])
        },
        |id| Msg::SessionActivate(id as usize),
        |_payload| -> Option<Msg> { None },
    );
    // "+" nueva sesión, colgado debajo de los dientes.
    let plus = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SessionNew)
    .children(vec![View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .children(vec![icon_view(Icon::Plus, theme.fg_muted, 1.8)])]);

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(SESSION_RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail, plus])
}

/// Panel inferior colapsable de la **cola de operaciones**. `None` si no hay
/// jobs. La barra de cabecera (siempre visible) resume y alterna el detalle;
/// cuando está abierto, lista cada job con su estado.
pub(crate) fn queue_panel(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let q = &model.queue;
    if q.ops.is_empty() {
        return None;
    }
    let corriendo = q.running_count();
    let total = q.ops.len();
    let resumen = if corriendo > 0 {
        format!("⚙ Operaciones · {corriendo} en curso / {total}")
    } else {
        format!("✓ Operaciones · {total} terminadas")
    };
    let flecha = if q.open { "▾" } else { "▸" };

    // Cabecera: resumen (toggle) + botón limpiar.
    let titulo = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(Msg::ToggleQueue)
    .text(format!("{flecha} {resumen}"), 13.0, theme.fg_text);

    let limpiar = View::new(Style {
        size: Size { width: length(96.0_f32), height: length(24.0_f32) },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(5.0)
    .on_click(Msg::ClearQueue)
    .text("Limpiar", 12.0, theme.fg_muted);

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: Rect { left: length(12.0), right: length(12.0), top: length(0.0), bottom: length(0.0) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![titulo, limpiar]);

    let mut hijos = vec![header];

    if q.open {
        // Hasta 6 filas de jobs (las más recientes arriba).
        let filas: Vec<View<Msg>> = q
            .ops
            .iter()
            .rev()
            .take(6)
            .map(|op| {
                let (glyph, color) = match &op.status {
                    OpStatus::Running => ("⋯", theme.accent),
                    OpStatus::Done(_) => ("✓", theme.fg_muted),
                    OpStatus::Failed(_) => ("✗", theme.fg_destructive),
                };
                let texto = match &op.status {
                    OpStatus::Failed(e) => format!("{glyph} {} — {e}", op.label),
                    _ => format!("{glyph} {}", op.label),
                };
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                    padding: Rect { left: length(16.0), right: length(12.0), top: length(0.0), bottom: length(0.0) },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .text(texto, 12.0, color)
            })
            .collect();
        let lista = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(140.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .children(filas);
        hijos.push(lista);
    }

    let alto = if q.open { 172.0 } else { 30.0 };
    Some(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(alto) },
            ..Default::default()
        })
        .children(hijos),
    )
}

/// Barra de **breadcrumb clicable** de un panel (Fase 4.2): cada segmento sube
/// a ese nivel (`BreadcrumbIn(pane, depth)`). Sobre una fuente no-POSIX, el
/// primer segmento lleva el prefijo `⊟ <fuente>`. `focused` tiñe la barra
/// cuando el panel está enfocado (sólo se nota en modo dual).
pub(crate) fn pane_breadcrumb(pane_obj: &Pane, pane: usize, focused: bool, theme: &Theme) -> View<Msg> {
    let nav = pane_obj.nav();
    let mut segs: Vec<String> = nav.ancestors().iter().map(|n| n.name.clone()).collect();
    if pane_obj.is_foreign() && !segs.is_empty() {
        segs[0] = format!("⊟ {}", nav.label());
    }
    let seg_refs: Vec<&str> = segs.iter().map(String::as_str).collect();
    let crumbs = breadcrumb_view(
        &seg_refs,
        move |depth| Msg::BreadcrumbIn(pane, depth),
        &BreadcrumbPalette::from_theme(theme),
    );
    let bg = if focused { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![crumbs])
}

/// Una columna de panel: su breadcrumb arriba + su lista/grilla. `focused`
/// resalta el panel activo (relevante en modo dual). Las filas emiten `Msg`s
/// que llevan `pane`, así que el click actúa sobre el panel correcto.
pub(crate) fn pane_column(model: &Model, pane: usize, focused: bool, theme: &Theme) -> View<Msg> {
    let crumb = pane_breadcrumb(&model.panes[pane], pane, focused, theme);
    // El filtro vivo sólo aplica al panel enfocado.
    let filtering = focused && model.nav_filtering;
    // La vista iconos necesita el cache de miniaturas y las dimensiones del
    // panel: se arma aparte (las otras dos sólo dependen del navegador).
    let content = if model.panes[pane].nav().view.is_grid() {
        navigator_icons_view(model, pane, theme)
    } else {
        nav_pane_view(
            model.panes[pane].nav(),
            &model.panes[pane].marked,
            &model.state,
            theme,
            filtering,
            pane,
        )
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, content])
}

/// Pinta el contenido de un panel según su `ViewMode` (lista o detalle). `pane`
/// es el índice del panel (0/1): las filas y encabezados emiten `Msg`s que lo
/// llevan, para que el click actúe sobre el panel correcto en modo dual.
pub(crate) fn nav_pane_view(
    nav: &Navigator,
    marked: &BTreeSet<nahual_source_core::NodeId>,
    state: &ShellState,
    theme: &Theme,
    filtering: bool,
    pane: usize,
) -> View<Msg> {
    match nav.view {
        nahual_source_core::ViewMode::List => {
            navigator_list_view(nav, marked, state, ListPalette::from_theme(theme), filtering, pane)
        }
        nahual_source_core::ViewMode::Details => {
            navigator_detail_view(nav, marked, state, theme, filtering, pane)
        }
        // Icons y Gallery se interceptan en `pane_column` (necesitan el cache
        // de thumbs y las dimensiones del panel); estos brazos no se alcanzan,
        // pero mantienen el match exhaustivo. Fallback honesto: detalle.
        nahual_source_core::ViewMode::Icons | nahual_source_core::ViewMode::Gallery => {
            navigator_detail_view(nav, marked, state, theme, filtering, pane)
        }
    }
}

/// Métricas de la grilla según el modo: galería = tiles grandes (carpetas de
/// imágenes); iconos = la grilla compacta por defecto.
pub(crate) fn grid_metrics_for(view: nahual_source_core::ViewMode) -> GridMetrics {
    if matches!(view, nahual_source_core::ViewMode::Gallery) {
        GridMetrics { tile_w: 220.0, tile_h: 248.0, gap: 14.0, pad: 14.0 }
    } else {
        GridMetrics::default()
    }
}

/// Ancho útil del panel de la grilla: la ventana menos el sidebar (árbol),
/// el canal de los dientes y, si está abierto, el visor derecho; en dual,
/// la mitad de eso.
pub(crate) fn grid_pane_w(model: &Model) -> f32 {
    let (vw, _) = viewport_of(model);
    let mut canvas_w = (vw - model.tree_w - SESSION_RAIL_W - 8.0).max(240.0);
    if model.viewer_open {
        canvas_w = (canvas_w - model.preview_w).max(240.0);
    }
    if model.dual {
        canvas_w / 2.0
    } else {
        canvas_w
    }
}

/// Columnas actuales de la grilla del panel activo (para que el wheel
/// scrollee por filas enteras).
pub(crate) fn grid_cols(model: &Model) -> usize {
    let nav = model.cur();
    let metrics = grid_metrics_for(nav.view);
    let (_, vh) = viewport_of(model);
    let win = ventana_visible(nav.visible_count(), grid_pane_w(model), vh - 120.0, 0, &metrics);
    win.cols.max(1)
}

/// Toolbar del shell: navegación + modos de vista + acciones, sobre el
/// widget `llimphi-widget-toolbar` (los grupos son datos → componibles).
pub(crate) fn shell_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    use nahual_source_core::ViewMode as VM;
    let v = model.cur().view;
    let vista = |ic: Icon, modo: VM, activo: bool| {
        ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), Msg::SetViewMode(modo)).active(activo)
    };
    let pane = model.cur_pane();
    // Con una app de canvas abierta, atrás/adelante pasan de archivo
    // (anterior/siguiente de la carpeta) — integrados al modo lista.
    let en_canvas = model.canvas.is_some();
    let puede_atras = en_canvas || pane.hist_pos > 0;
    let puede_adelante = en_canvas || pane.hist_pos + 1 < pane.hist.len();
    toolbar_view(
        vec![
            // Navegación: atrás / adelante (historial browser; con canvas
            // abierto, archivo anterior/siguiente) / subir.
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronLeft, c, 1.7), Msg::NavBack)
                    .enabled(puede_atras),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronRight, c, 1.7), Msg::NavForward)
                    .enabled(puede_adelante),
                ToolbarItem::new(|_s, c| icon_view(Icon::ChevronUp, c, 1.7), Msg::Parent)
                    .with_label("subir"),
            ]),
            // Modo de la rueda con un archivo abierto en el canvas:
            // zoom (la app la usa) o lista (pasa al siguiente/anterior).
            ToolbarGroup::new(vec![
                ToolbarItem::new(
                    |_s, c| icon_view(Icon::Search, c, 1.7),
                    Msg::SetWheelMode(WheelMode::Zoom),
                )
                .with_label("zoom")
                .active(model.wheel_mode == WheelMode::Zoom),
                ToolbarItem::new(
                    |_s, c| icon_view(Icon::SkipForward, c, 1.7),
                    Msg::SetWheelMode(WheelMode::Lista),
                )
                .with_label("lista")
                .active(model.wheel_mode == WheelMode::Lista),
            ]),
            // Modos de vista (v cicla; acá acceso directo).
            ToolbarGroup::new(vec![
                vista(Icon::Rows, VM::List, matches!(v, VM::List)),
                vista(Icon::Table, VM::Details, matches!(v, VM::Details)),
                vista(Icon::Grid, VM::Icons, matches!(v, VM::Icons)),
                vista(Icon::Image, VM::Gallery, matches!(v, VM::Gallery)),
            ]),
            // Acciones.
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::Columns, c, 1.7), Msg::ToggleDual)
                    .active(model.dual),
                ToolbarItem::new(|_s, c| icon_view(Icon::Plus, c, 1.7), Msg::NewDirPrompt)
                    .with_label("carpeta")
                    .enabled(model.can_edit()),
            ]),
        ],
        34.0,
        &ToolbarPalette::from_theme(theme),
    )
}

/// Pinta los hijos visibles como **grilla de iconos/miniaturas** (Fase 4.8).
pub(crate) fn navigator_icons_view(model: &Model, pane: usize, theme: &Theme) -> View<Msg> {
    let nav = model.panes[pane].nav();
    let marked = &model.panes[pane].marked;
    let gallery = matches!(nav.view, nahual_source_core::ViewMode::Gallery);
    let metrics = grid_metrics_for(nav.view);
    let modo = if gallery { "galería" } else { "iconos" };

    let (_, vh) = viewport_of(model);
    let pane_w = grid_pane_w(model);
    let total = nav.visible_count();
    let win = ventana_visible(total, pane_w, vh - 120.0, 0, &metrics);

    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + MAX_ICON_TILES).min(visibles.len());
    let cells: Vec<GridCell<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let mark = if marked.contains(&n.id) { "✓ " } else { "" };
            let label = format!("{mark}{}", n.name);
            GridCell {
                content: icon_tile_content(model, n, theme, metrics.tile_w - 12.0),
                label: Some(label),
                selected: *idx == nav.selected,
                on_click: Msg::SelectIn(pane, *idx),
            }
        })
        .collect();

    let mostrados = start + cells.len();
    let truncated_hint = (mostrados < total)
        .then(|| format!("… y {} más (rueda para ver más)", total - mostrados));

    grid_view(GridSpec {
        cells,
        cols: win.cols,
        metrics,
        caption: Some(format!(
            "{total} entradas · {modo} · ↑↓ navega · Enter abre · v cambia vista"
        )),
        truncated_hint,
        palette: GridPalette::from_theme(theme),
    })
}

/// Cuerpo de una celda de la grilla iconos/galería: la miniatura si está lista;
/// si no, un **ícono vectorial real** por tipo (carpeta, imagen pendiente,
/// archivo) o un aviso si la miniatura falló.
pub(crate) fn icon_tile_content(model: &Model, node: &Node, theme: &Theme, lado: f32) -> View<Msg> {
    let base = || Style {
        size: Size { width: length(lado), height: length(lado) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    // Ícono vectorial centrado, dimensionado a la mitad del tile.
    let big = (lado * 0.5).clamp(28.0, 96.0);
    let centered = |icon: Icon, color: Color| {
        View::new(base()).fill(theme.bg_panel_alt).children(vec![View::new(Style {
            size: Size { width: length(big), height: length(big) },
            ..Default::default()
        })
        .children(vec![icon_view(icon, color, 1.6)])])
    };
    if node.is_container {
        let icon = match node.kind {
            NodeKind::Archive => Icon::Archive,
            _ => Icon::Folder,
        };
        return centered(icon, theme.fg_text);
    }
    let path = PathBuf::from(&node.id);
    if let Some(img) = model.thumbs.get(&path) {
        return View::new(base()).image(img.clone());
    }
    // Falló la miniatura: para media el glifo de su naturaleza es más útil
    // que un ⚠ (un webm sin track AV1 sigue siendo un video).
    if model.thumbs_failed.contains(&path) {
        let icon = if es_video(&path) {
            Icon::Film
        } else if es_audio(&path) {
            Icon::Music
        } else {
            Icon::Warning
        };
        return centered(icon, theme.fg_muted);
    }
    // Aún decodificando (o sin thumb posible) → glifo por naturaleza.
    let icon = if es_imagen(&path) {
        Icon::Image
    } else if es_video(&path) {
        Icon::Film
    } else if es_audio(&path) {
        Icon::Music
    } else {
        Icon::File
    };
    centered(icon, theme.fg_muted)
}

/// Color peniko de un label (para el tinte de fila en la vista detalle).
pub(crate) fn label_color(label: Label) -> Color {
    let (r, g, b) = label.rgb();
    Color::from_rgba8(r, g, b, 255)
}

/// Sufijo del caption con el estado del filtro y los atajos.
pub(crate) fn nav_caption(nav: &Navigator, filtering: bool) -> String {
    let f = nav.filter();
    if filtering || !f.is_empty() {
        let cursor = if filtering { "_" } else { "" };
        format!(
            "{} de {} · filtro: {f}{cursor}  (Esc sale · v vista)",
            nav.visible_count(),
            nav.children().len()
        )
    } else {
        format!(
            "{} entradas · ↑↓ navega · Enter abre · ⌫ vuelve · v cambia vista · / filtra",
            nav.children().len()
        )
    }
}

/// Pinta los hijos visibles (filtrados) del contenedor actual como una lista
/// `llimphi-widget-list` — el gemelo genérico de `file_explorer_view`.
pub(crate) fn navigator_list_view(
    nav: &Navigator,
    marked: &BTreeSet<nahual_source_core::NodeId>,
    state: &ShellState,
    palette: ListPalette,
    filtering: bool,
    pane: usize,
) -> View<Msg> {
    use std::cmp::min;
    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = min(visibles.len(), start + nav.visible_rows);
    let rows: Vec<ListRow<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            // Una fila marcada (selección múltiple) lleva un check al frente.
            let mark = if marked.contains(&n.id) { "✓" } else { " " };
            // Punto cuando el nodo tiene label (el color real se ve en detalle;
            // en lista es monocromo — la lista no pinta color por fila).
            let dot = if state.label_of(&n.id).is_some() { "●" } else { " " };
            // Indentación por profundidad (expansión inline) + chevron de
            // estado: ▾ expandida, ▸ colapsada. Click la alterna; doble
            // click la abre en el canvas.
            let sangria = "   ".repeat(nav.depth_of(*idx));
            let icon = if n.is_container {
                if nav.is_expanded(&n.id) { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let label = if n.is_container {
                format!("{mark}{dot}{sangria}{icon}{}/", n.name)
            } else {
                format!("{mark}{dot}{sangria}{icon}{}", n.name)
            };
            ListRow {
                label,
                selected: *idx == nav.selected,
                on_click: Msg::SelectIn(pane, *idx),
            }
        })
        .collect();
    let truncated_hint = if visibles.len() > end {
        Some(format!("… y {} más (rueda o ↓ para ver más)", visibles.len() - end))
    } else {
        None
    };
    list_view(ListSpec {
        rows,
        total: visibles.len(),
        caption: Some(nav_caption(nav, filtering)),
        truncated_hint,
        row_height: 22.0,
        palette,
    })
}

/// Pinta los hijos visibles como grilla detalle con columnas ordenables
/// (nombre · tamaño · modificado · tipo). Click en un encabezado emite
/// `NavSortBy`; click en una fila selecciona.
pub(crate) fn navigator_detail_view(
    nav: &Navigator,
    marked: &BTreeSet<nahual_source_core::NodeId>,
    state: &ShellState,
    theme: &Theme,
    filtering: bool,
    pane: usize,
) -> View<Msg> {
    use llimphi_widget_detail_table::{
        detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
    };
    use nahual_source_core::SortKey;

    let (skey, sdir) = nav.sort();
    let sort_col = match skey {
        SortKey::Name => 0,
        SortKey::Size => 1,
        SortKey::Mtime => 2,
        SortKey::Kind => 3,
    };
    let dt_dir = match sdir {
        nahual_source_core::SortDir::Asc => DtDir::Asc,
        nahual_source_core::SortDir::Desc => DtDir::Desc,
    };

    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + nav.visible_rows).min(visibles.len());
    let rows: Vec<DetailRow<Msg>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let icon = kind_icon(n.kind, n.is_container);
            let is_marked = marked.contains(&n.id);
            let mark = if is_marked { "✓" } else { " " };
            let label = state.label_of(&n.id);
            // El nombre lleva un punto del color del label, si tiene.
            let dot = if label.is_some() { "● " } else { "" };
            // Indentación por profundidad + chevron de expansión inline
            // (▾ expandida / ▸ colapsada). Click alterna; doble click abre.
            let sangria = "   ".repeat(nav.depth_of(*idx));
            let chev = if n.is_container {
                if nav.is_expanded(&n.id) { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            DetailRow {
                cells: vec![
                    format!("{mark}{sangria}{chev}{icon} {dot}{}", n.name),
                    n.size.map(human_size).unwrap_or_default(),
                    n.mtime.map(epoch_ms_to_date).unwrap_or_default(),
                    kind_label(n.kind, &n.name).to_string(),
                ],
                selected: *idx == nav.selected,
                // El acento del nombre lleva el color del label si lo tiene; si
                // no, el acento neutro de las filas marcadas.
                accent: label
                    .map(label_color)
                    .or_else(|| is_marked.then_some(theme.accent)),
                on_click: Msg::SelectIn(pane, *idx),
            }
        })
        .collect();

    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 88.0).right(),
        Column::fixed("Modificado", 140.0),
        Column::fixed("Tipo", 84.0),
    ];
    detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((sort_col, dt_dir)),
            row_height: 22.0,
            caption: Some(nav_caption(nav, filtering)),
            palette: DetailPalette::from_theme(theme),
        },
        move |col| Msg::SortByIn(pane, col),
    )
}

/// Icono de una columna nombre según la naturaleza del nodo.
pub(crate) fn kind_icon(kind: nahual_source_core::NodeKind, is_container: bool) -> &'static str {
    use nahual_source_core::NodeKind::*;
    match kind {
        Dir => "▸",
        Synthetic => "◇",
        Archive => "▤",
        Symlink => "↪",
        File if is_container => "▸",
        File => " ",
    }
}

/// Rótulo de la columna "tipo".
pub(crate) fn kind_label(kind: nahual_source_core::NodeKind, name: &str) -> &'static str {
    use nahual_source_core::NodeKind::*;
    match kind {
        Dir => "carpeta",
        Synthetic => "mónada",
        Archive => "archivo",
        Symlink => "enlace",
        File => match name.rsplit_once('.').map(|(_, e)| e) {
            Some("rs") => "rust",
            Some("md") => "markdown",
            Some("toml") => "toml",
            Some("json") => "json",
            Some("png" | "jpg" | "jpeg" | "webp" | "gif") => "imagen",
            Some("txt") => "texto",
            _ => "archivo",
        },
    }
}

/// Tamaño humano compacto (B/KB/MB/GB/TB), una cifra decimal salvo bytes.
pub(crate) fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut val = b as f64;
    let mut i = 0;
    while val >= 1024.0 && i < U.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{val:.1} {}", U[i])
    }
}

/// Epoch-ms → `YYYY-MM-DD HH:MM` en UTC (civil-from-days de Hinnant). Sin
/// dependencias de fechas — alcanza para la columna "modificado".
pub(crate) fn epoch_ms_to_date(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, min) = (tod / 3600, (tod % 3600) / 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}
