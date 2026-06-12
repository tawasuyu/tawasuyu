//! `shell_update`: la lógica de `App::update` del shell nahual. Movido de
//! `main.rs` en el split de 2026-06-12 (puro movimiento de código).

use std::time::{Duration, Instant};

use crate::modelo::*;
use crate::helpers::*;
use crate::overlays::{app_menu, handle_menu_command};
use crate::ops::{OpKind, OpStatus};
use crate::view::grid_cols;
use llimphi_ui::Handle;
use llimphi_widget_menubar::{menubar_command_at, menubar_nav};
use llimphi_widget_text_editor::{EditorMetrics, PointerEvent};
use llimphi_ui::llimphi_raster::peniko::{Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat};
use nahual_source_core::{Navigator, NouserSource, MingaSource};
use tullpu_module as tullpu;
use media_module as mediamod;
use wawa_config_llimphi::theme_from_wawa;
use llimphi_motion::{animate, motion, Tween};

pub(crate) fn shell_update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    match msg {
        Msg::Up => {
            if m.cur_mut().up() {
                refresh_preview(&mut m);
            }
        }
        Msg::Down => {
            if m.cur_mut().down() {
                refresh_preview(&mut m);
            }
        }
        Msg::SelectIn(pane, idx) => {
            m.focus = pane;
            // Doble click (mismo pane+fila, < 400 ms) = abrir: carpeta →
            // desciende al canvas + revela en el árbol; archivo → visor.
            let ahora = Instant::now();
            let doble = m.last_click.take().is_some_and(|(p, i, t)| {
                p == pane && i == idx && ahora.duration_since(t) < Duration::from_millis(400)
            });
            if m.cur_mut().select(idx) {
                if doble {
                    do_open_selected(&mut m, handle);
                } else {
                    m.last_click = Some((pane, idx, ahora));
                    // Click simple en una carpeta (lista/detalle) la
                    // expande/colapsa inline; el doble click la abre.
                    let es_dir =
                        m.cur().selected_node().is_some_and(|n| n.is_container);
                    if es_dir && !m.cur().view.is_grid() {
                        let i = m.cur().selected;
                        let _ = m.cur_mut().toggle_expand(i);
                    }
                    refresh_preview(&mut m);
                }
            }
        }
        Msg::ToggleDual => {
            m.dual = !m.dual;
            if !m.dual {
                m.focus = 0; // al volver a simple, el visor vuelve a la derecha
            }
        }
        Msg::SwitchFocus => {
            if m.dual {
                m.focus = 1 - m.focus;
                refresh_preview(&mut m);
            }
        }
        Msg::OpenSelected => {
            do_open_selected(&mut m, handle);
        }
        Msg::Parent => {
            // ⌫/Esc pelan por capas: primero la app del canvas, después
            // el panel de preview, recién entonces sube de directorio.
            if m.canvas.is_some() {
                m.canvas = None;
                m.tools_open = false;
                return m;
            }
            if m.viewer_open {
                m.viewer_open = false;
                return m;
            }
            m.cur_pane_mut().marked.clear();
            match m.cur_mut().parent() {
                Ok(true) => {
                    record_history(&mut m);
                    apply_format(&mut m);
                    refresh_preview(&mut m);
                    if m.cur().view.is_grid() {
                        request_thumbs(&mut m, handle);
                    }
                }
                // Subir desde la raíz de una fuente montada la desmonta
                // (vuelve al nivel de abajo de la pila). En POSIX, la raíz
                // es `/` y no hay a dónde subir.
                Ok(false) => {
                    if m.is_foreign() {
                        m.cur_pane_mut().nav_stack.pop();
                        clear_preview(&mut m);
                        record_history(&mut m);
                    }
                }
                Err(_) => {}
            }
        }
        Msg::SortByIn(pane, col) => {
            m.focus = pane;
            m.cur_mut().set_sort(col_to_sortkey(col as u8));
            // Recordá el orden elegido para esta carpeta (folder format).
            save_format(&mut m);
        }
        Msg::NavToggleView => {
            // Cicla lista → detalle → iconos → lista.
            let nav = m.cur_mut();
            nav.view = nav.view.next();
            // En vista iconos, pedí miniaturas de lo que entró en pantalla.
            if m.cur().view.is_grid() {
                request_thumbs(&mut m, handle);
            }
            // Recordá la vista elegida para esta carpeta (folder format).
            save_format(&mut m);
        }
        Msg::NavFilterStart => {
            m.nav_filtering = true;
        }
        Msg::NavFilterInput(s) => {
            let mut f = m.cur().filter().to_string();
            f.push_str(&s);
            m.cur_mut().set_filter(f);
            refresh_preview(&mut m);
        }
        Msg::NavFilterBackspace => {
            let mut f = m.cur().filter().to_string();
            f.pop();
            m.cur_mut().set_filter(f);
            refresh_preview(&mut m);
        }
        Msg::NavFilterEnd => {
            m.nav_filtering = false;
        }
        Msg::BreadcrumbIn(pane, depth) => {
            m.focus = pane;
            if matches!(m.cur_mut().ascend_to(depth), Ok(true)) {
                m.cur_pane_mut().marked.clear();
                m.canvas = None;
                record_history(&mut m);
                apply_format(&mut m);
                refresh_preview(&mut m);
            }
        }
        Msg::ResizeList(dx) => {
            m.list_width = (m.list_width + dx).clamp(220.0, 900.0);
        }
        Msg::ResizeTree(dx) => {
            m.tree_w = (m.tree_w + dx).clamp(170.0, 420.0);
        }
        Msg::ResizePreview(dx) => {
            // El divisor está a la izquierda del visor: moverlo a la
            // derecha achica el panel.
            m.preview_w = (m.preview_w - dx).clamp(280.0, 860.0);
        }
        Msg::SetViewMode(v) => {
            m.cur_mut().view = v;
            if v.is_grid() {
                request_thumbs(&mut m, handle);
            }
            save_format(&mut m);
        }
        Msg::ExpandSelected => {
            let i = m.cur().selected;
            let ya = m
                .cur()
                .selected_node()
                .is_some_and(|n| m.cur().is_expanded(&n.id));
            if !ya {
                let _ = m.cur_mut().toggle_expand(i);
            }
        }
        Msg::CollapseSelected => {
            let i = m.cur().selected;
            let expandida = m
                .cur()
                .selected_node()
                .is_some_and(|n| n.is_container && m.cur().is_expanded(&n.id));
            if expandida {
                let _ = m.cur_mut().toggle_expand(i);
            } else if let Some(p) = m.cur().parent_of(i) {
                // Colapsada (o archivo): saltá a la fila padre.
                m.cur_mut().select(p);
                refresh_preview(&mut m);
            }
        }
        // Atrás/adelante: con una app de canvas abierta pasan de archivo
        // (anterior/siguiente de la carpeta); si no, historial browser.
        Msg::NavBack => {
            if m.canvas.is_some() {
                canvas_step(&mut m, -1);
            } else {
                nav_history_go(&mut m, handle, -1);
            }
        }
        Msg::NavForward => {
            if m.canvas.is_some() {
                canvas_step(&mut m, 1);
            } else {
                nav_history_go(&mut m, handle, 1);
            }
        }
        Msg::CanvasNav(delta) => canvas_step(&mut m, delta),
        Msg::SetWheelMode(mode) => {
            m.wheel_mode = mode;
        }
        Msg::Resized(w, h) => {
            m.win = (w.max(320.0), h.max(240.0));
        }
        Msg::TogglePreviewPanel => {
            if m.viewer_open {
                m.viewer_open = false;
            } else {
                m.viewer_open = true;
                // Comparten el panel derecho: abrir uno cierra el otro.
                m.tools_open = false;
                refresh_preview(&mut m);
            }
        }
        Msg::ToggleToolsPanel => {
            if m.tools_open {
                m.tools_open = false;
            } else {
                m.tools_open = true;
                m.viewer_open = false;
            }
        }
        Msg::ResizeTools(dx) => {
            // El divisor está a la izquierda del panel: mover a la
            // derecha lo achica (mismo signo que ResizePreview).
            m.tools_w = (m.tools_w - dx).clamp(220.0, 480.0);
        }
        Msg::CanvasClose => {
            m.canvas = None;
            m.tools_open = false;
        }
        Msg::CanvasSave => {
            if let Some(CanvasApp::Texto { path, editor, dirty, saved }) = &mut m.canvas {
                match std::fs::write(&*path, editor.text()) {
                    Ok(()) => {
                        *dirty = false;
                        *saved = true;
                    }
                    Err(e) => eprintln!("[nahual] guardar {}: {e}", path.display()),
                }
            }
        }
        Msg::CanvasEditKey(ev) => {
            let lines = canvas_editor_lines(&m);
            if let Some(CanvasApp::Texto { editor, dirty, saved, .. }) = &mut m.canvas {
                let r = editor.apply_key_with_clipboard(&ev, &mut m.clipboard);
                if r.changed() {
                    *dirty = true;
                    *saved = false;
                }
                if r.touched() {
                    editor.ensure_caret_visible(lines);
                }
            }
        }
        Msg::CanvasEditPointer(ev) => {
            let metrics = EditorMetrics::for_font_size(13.0);
            if let Some(CanvasApp::Texto { editor, .. }) = &mut m.canvas {
                let scroll = editor.scroll_offset;
                match ev {
                    PointerEvent::Click { x, y } => {
                        m.canvas_drag = (0.0, 0.0);
                        let (line, col) = metrics.screen_to_pos(x, y, scroll);
                        editor.set_caret_at(line, col);
                    }
                    PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
                        m.canvas_drag.0 += dx;
                        m.canvas_drag.1 += dy;
                        let cx = initial_x + m.canvas_drag.0;
                        let cy = initial_y + m.canvas_drag.1;
                        let (line, col) = metrics.screen_to_pos(cx, cy, scroll);
                        editor.extend_selection_to(line, col);
                    }
                }
            }
        }
        Msg::CanvasTullpu(tmsg) => {
            if let Some(CanvasApp::Imagen(st)) = m.canvas.take() {
                m.canvas = Some(CanvasApp::Imagen(Box::new(tullpu::update(*st, tmsg))));
            }
        }
        Msg::CanvasMedia(mmsg) => {
            if let Some(CanvasApp::Media(st)) = &mut m.canvas {
                mediamod::update(st, mmsg);
            }
        }
        Msg::Scroll(steps) => {
            // Con una app de canvas abierta, la rueda es suya (el editor
            // scrollea; imagen/media la ignoran — el zoom va por
            // Ctrl+rueda).
            if m.canvas.is_some() {
                match m.canvas.take() {
                    Some(CanvasApp::Texto { path, mut editor, dirty, saved }) => {
                        editor.scroll_by(steps);
                        m.canvas = Some(CanvasApp::Texto { path, editor, dirty, saved });
                    }
                    // Rueda sobre el editor de imágenes = zoom del lienzo.
                    Some(CanvasApp::Imagen(st)) => {
                        let mult = 1.12_f32.powi(-steps);
                        m.canvas = Some(CanvasApp::Imagen(Box::new(tullpu::update(
                            *st,
                            tullpu::Msg::Zoom(mult),
                        ))));
                    }
                    otro => m.canvas = otro,
                }
                return m;
            }
            if m.cur().view.is_grid() {
                // En grilla la unidad de scroll es la FILA entera (cols
                // items), no el item — si no, las celdas se van "halando"
                // de a una y bailan de columna. El offset queda alineado
                // a múltiplo de cols.
                let cols = grid_cols(&m).max(1);
                let nav = m.cur_mut();
                nav.apply_wheel(steps as f32 * cols as f32);
                nav.visible_offset -= nav.visible_offset % cols;
                // Lo que entró en pantalla al scrollear pide su miniatura.
                request_thumbs(&mut m, handle);
            } else {
                // El navegador activo tiene su propio acumulador para
                // touchpads — le pasamos el delta crudo (en líneas).
                m.cur_mut().apply_wheel(steps as f32);
            }
        }
        Msg::MapPan(dx, dy) => {
            m.map_view.pan_by(dx as f64, dy as f64);
            m.basemap_dirty = true;
        }
        Msg::MapZoom(dy, cx, cy) => {
            // Cada "línea" de rueda → ±12% de zoom, anclado al cursor.
            m.map_view.zoom_at(1.12_f64.powf(dy as f64), cx, cy);
            m.basemap_dirty = true;
        }
        Msg::MapReset => {
            m.map_view.reset();
            m.basemap_dirty = true;
        }
        Msg::MapToggleBase => m.map_view.toggle_base(),
        Msg::MapClick(fx, fy) => {
            if let PreviewPane::Map(nahual_map_viewer_llimphi::MapPreview::Map { data, .. }) = &m.preview {
                if m.map_view.routing {
                    // Ruteo: cada clic fija un punto; con dos, calcula la ruta.
                    if let Some(c) =
                        nahual_map_viewer_llimphi::unproject(data, &m.map_view, fx as f64, fy as f64)
                    {
                        if m.map_view.route_pins.len() >= 2 {
                            m.map_view.clear_route();
                        }
                        m.map_view.route_pins.push(c);
                        if m.map_view.route_pins.len() == 2 {
                            let (a, b) = (m.map_view.route_pins[0], m.map_view.route_pins[1]);
                            match nahual_map_viewer_llimphi::route(data, a, b) {
                                Some(res) => {
                                    m.map_view.route_path = res.path;
                                    m.map_view.route_meters = res.meters;
                                }
                                None => {
                                    m.map_view.route_path.clear();
                                    m.map_view.route_meters = 0.0;
                                }
                            }
                        }
                    }
                } else {
                    m.map_view.selected = nahual_map_viewer_llimphi::hit_test(
                        data,
                        &m.map_view,
                        fx as f64,
                        fy as f64,
                    );
                }
            }
        }
        Msg::MapRouteToggle => {
            m.map_view.routing = !m.map_view.routing;
            m.map_view.clear_route();
        }
        Msg::MapCycleColor => {
            if let PreviewPane::Map(nahual_map_viewer_llimphi::MapPreview::Map { data, .. }) = &m.preview {
                let fields = nahual_map_viewer_llimphi::numeric_fields(data);
                m.map_view.color_field = next_in_cycle(&fields, &m.map_view.color_field);
            }
        }
        Msg::MapSearchStart => {
            m.map_view.searching = true;
            m.map_view.query.clear();
        }
        Msg::MapSearchInput(s) => {
            if m.map_view.searching {
                m.map_view.query.push_str(&s);
            }
        }
        Msg::MapSearchBackspace => {
            m.map_view.query.pop();
        }
        Msg::MapSearchCancel => {
            m.map_view.searching = false;
            m.map_view.query.clear();
        }
        Msg::MapSearchSubmit => {
            if let PreviewPane::Map(nahual_map_viewer_llimphi::MapPreview::Map { data, .. }) = &m.preview {
                let hits = nahual_map_viewer_llimphi::search(data, &m.map_view.query, 1);
                if let Some(&fi) = hits.first() {
                    nahual_map_viewer_llimphi::focus_on(data, &mut m.map_view, fi);
                }
            }
            m.map_view.searching = false;
            m.basemap_dirty = true;
        }
        Msg::WawaConfigChanged(cfg) => {
            m.theme = theme_from_wawa(&cfg, &m.theme);
            // nahual-shell no usa rimay_localize hoy; si en el
            // futuro lo hace, agregar el set_locale acá.
        }
        Msg::Tick => {
            match &mut m.preview {
                PreviewPane::Video(state) => {
                    state.tick(FRAME_TICK);
                }
                PreviewPane::Audio(state) => state.tick(FRAME_TICK),
                _ => {}
            }
            // El player del canvas también corre con el reloj.
            if let Some(CanvasApp::Media(st)) = &mut m.canvas {
                mediamod::tick(st, FRAME_TICK);
            }
            // Debounce del streaming del basemap: coalesce los pans/zooms
            // y re-streamea a lo sumo cada `RESTREAM_THROTTLE`.
            if m.basemap_dirty && m.basemap.is_some() {
                let now = Instant::now();
                let ready = m
                    .last_restream
                    .map_or(true, |t| now.duration_since(t) >= RESTREAM_THROTTLE);
                if ready && restream_basemap(&mut m) {
                    m.last_restream = Some(now);
                    m.basemap_dirty = false;
                }
            }
        }
        Msg::TogglePlay => match &mut m.canvas {
            // El player del canvas tiene prioridad sobre el preview.
            Some(CanvasApp::Media(st)) => mediamod::update(st, mediamod::Msg::TogglePlay),
            _ => match &mut m.preview {
                PreviewPane::Video(state) => state.toggle_play(),
                PreviewPane::Audio(state) => state.toggle_play(),
                _ => {}
            },
        },
        Msg::MountNouser => {
            // Sólo montamos desde POSIX (no anidamos fuentes). nouser sólo
            // LEE el dir, así que no hay riesgo de efecto secundario.
            if !m.is_foreign() {
                let dir = target_dir(&m);
                if let Some(nav) = NouserSource::escanear(&dir, 1)
                    .ok()
                    .and_then(|src| Navigator::open(Box::new(src)).ok())
                {
                    m.cur_pane_mut().nav_stack.push(nav);
                    clear_preview(&mut m);
                }
            }
        }
        Msg::MountMinga => {
            // Guard: `PersistentRepo::open` (sled) CREA archivos si el dir
            // no es un repo — sólo montamos si ya parece uno, para no
            // ensuciar directorios ajenos.
            if !m.is_foreign() {
                let dir = target_dir(&m);
                if parece_repo_minga(&dir) {
                    if let Some(nav) = MingaSource::abrir(&dir)
                        .ok()
                        .and_then(|src| Navigator::open(Box::new(src)).ok())
                    {
                        m.cur_pane_mut().nav_stack.push(nav);
                        clear_preview(&mut m);
                    }
                }
            }
        }
        Msg::Unmount => {
            if m.is_foreign() {
                m.cur_pane_mut().nav_stack.pop();
                clear_preview(&mut m);
            }
        }
        Msg::CycleTheme => {
            m.theme = llimphi_theme::Theme::next_after(m.theme.name);
        }
        Msg::MenuOpen(which) => {
            m.menu_open = which;
            // Abrir un menú raíz cierra cualquier contextual.
            m.context_menu = None;
            m.menu_active = usize::MAX;
            if which.is_some() {
                m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
        }
        Msg::MenuNav(dir) => {
            if let Some(mi) = m.menu_open {
                let menu = app_menu(&m);
                m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
            }
        }
        Msg::MenuActivate => {
            if let Some(mi) = m.menu_open {
                let menu = app_menu(&m);
                if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                    m.menu_open = None;
                    return handle_menu_command(m, &cmd, handle);
                }
            }
        }
        Msg::MenuTick => {}
        Msg::CloseMenus => {
            m.menu_open = None;
            m.menu_active = usize::MAX;
            m.context_menu = None;
        }
        Msg::MenuCommand(cmd) => {
            m.menu_open = None;
            m.menu_active = usize::MAX;
            return handle_menu_command(m, &cmd, handle);
        }
        Msg::ContextMenuOpen(x, y) => {
            // Sólo si hay algo seleccionado (POSIX o fuente montada).
            if hay_seleccion(&m) {
                m.menu_open = None;
                // Precomputa las opciones "Abrir con…" del archivo
                // seleccionado (discernir → handlers_for) para que el
                // render no toque el registro ni el disco.
                compute_open_with(&mut m);
                m.context_menu = Some((x, y));
            }
        }
        Msg::OpenWith(id) => {
            if let (Some(app), Some(target)) =
                (m.registry.get(&id), m.ctx_target.as_ref().and_then(|p| p.to_str()))
            {
                if let Err(e) = app.open(target) {
                    eprintln!("[nahual] abrir con {id}: {e}");
                }
            }
            m.context_menu = None;
        }
        Msg::EditSelected => {
            if let Some(target) = m.ctx_target.as_ref().and_then(|p| p.to_str()) {
                let bin = std::env::var("NADA_BIN").unwrap_or_else(|_| "nada".into());
                if let Err(e) = std::process::Command::new(bin).arg(target).spawn() {
                    eprintln!("[nahual] editar en nada: {e}");
                }
            }
            m.context_menu = None;
        }
        Msg::TerminalHere => {
            // El dir POSIX base (la fuente del fondo de la pila), aunque
            // haya una fuente montada encima.
            let dir = std::path::PathBuf::from(m.panes[m.focus].nav_stack[0].current_id());
            let bin = std::env::var("SHUMA_BIN").unwrap_or_else(|_| "shuma-shell-llimphi".into());
            if let Err(e) = std::process::Command::new(bin).current_dir(&dir).spawn() {
                eprintln!("[nahual] terminal shuma: {e}");
            }
            m.context_menu = None;
        }

        // ---- Fase 4.3: operaciones de archivo + cola ----
        Msg::ToggleMark => {
            if let Some(n) = m.cur().selected_node() {
                let id = n.id.clone();
                let pane = m.cur_pane_mut();
                // `insert` devuelve `false` si ya estaba → entonces se quita.
                if !pane.marked.insert(id.clone()) {
                    pane.marked.remove(&id);
                }
            }
            m.cur_mut().down();
            refresh_preview(&mut m);
        }
        Msg::NewDirPrompt => {
            if m.can_edit() {
                let parent = m.cur().current_id().clone();
                m.prompt = Some(Prompt { kind: PromptKind::NewDir { parent }, text: String::new() });
                m.context_menu = None;
            }
        }
        Msg::NewFilePrompt => {
            if m.can_edit() {
                let parent = m.cur().current_id().clone();
                m.prompt = Some(Prompt { kind: PromptKind::NewFile { parent }, text: String::new() });
                m.context_menu = None;
            }
        }
        Msg::RenamePrompt => {
            if m.can_edit() {
                // Con marca múltiple, "Renombrar" abre el batch; si no, el
                // renombrado simple del nodo bajo el cursor.
                if !m.cur_pane().marked.is_empty() {
                    return shell_update(m, Msg::BatchRenameStart, handle);
                }
                if let Some(n) = m.cur().selected_node() {
                    let (id, name) = (n.id.clone(), n.name.clone());
                    m.prompt = Some(Prompt { kind: PromptKind::Rename { id }, text: name });
                    m.context_menu = None;
                }
            }
        }
        Msg::PromptInput(s) => {
            if let Some(p) = m.prompt.as_mut() {
                p.text.push_str(&s);
            }
        }
        Msg::PromptBackspace => {
            if let Some(p) = m.prompt.as_mut() {
                p.text.pop();
            }
        }
        Msg::PromptSubmit => {
            if let Some(p) = m.prompt.take() {
                let texto = p.text.trim().to_string();
                // Selección por patrón no toca el filesystem: marca los hijos
                // visibles que matchean el glob y sale antes del enqueue.
                if matches!(p.kind, PromptKind::SelectPattern) {
                    if !texto.is_empty() {
                        select_by_pattern(&mut m, &texto);
                    }
                } else if !texto.is_empty() {
                    let kind = match p.kind {
                        PromptKind::NewDir { parent } => OpKind::NewDir { parent, name: texto },
                        PromptKind::NewFile { parent } => OpKind::NewFile { parent, name: texto },
                        PromptKind::Rename { id } => OpKind::Rename { id, new_name: texto },
                        PromptKind::SelectPattern => unreachable!("manejado arriba"),
                    };
                    enqueue(&mut m, handle, kind);
                }
            }
        }
        Msg::PromptCancel => {
            m.prompt = None;
        }
        Msg::DeleteSelection => {
            let targets = m.cur_pane().op_targets();
            if !targets.is_empty() {
                m.confirm_delete = Some(targets);
                m.context_menu = None;
            }
        }
        Msg::ConfirmDelete => {
            if let Some(targets) = m.confirm_delete.take() {
                for (id, name) in targets {
                    enqueue(&mut m, handle, OpKind::Delete { id, name });
                }
                m.cur_pane_mut().marked.clear();
            }
        }
        Msg::CancelConfirm => {
            m.confirm_delete = None;
        }
        Msg::CopyToOther => copy_or_move(&mut m, handle, false),
        Msg::MoveToOther => copy_or_move(&mut m, handle, true),
        Msg::RunOp(kind) => {
            m.context_menu = None;
            enqueue(&mut m, handle, kind);
        }
        Msg::OpFinished { id, result } => {
            let status = match &result {
                Ok(r) => OpStatus::Done(r.clone()),
                Err(e) => OpStatus::Failed(e.clone()),
            };
            m.queue.finish(id, status);
            reload_panes(&mut m);
            // Dejá el cursor sobre el resultado (carpeta/archivo nuevo,
            // renombrado) en el panel enfocado.
            if let Ok(Some(new_id)) = &result {
                m.cur_pane_mut().nav_mut().select_id(new_id);
            }
            refresh_preview(&mut m);
        }
        Msg::ToggleQueue => {
            m.queue.open = !m.queue.open;
        }
        Msg::ClearQueue => {
            m.queue.clear_finished();
        }

        // ---- Fase 4.5: renombrado por lote ----
        Msg::BatchRenameStart => {
            if m.can_edit() {
                // Objetivos: la marca, o el cursor si no hay marca.
                let targets = m.cur_pane().op_targets();
                if !targets.is_empty() {
                    m.batch = Some(BatchRename { pattern: "{name}".to_string(), targets });
                    m.context_menu = None;
                }
            }
        }
        Msg::BatchPatternInput(s) => {
            if let Some(b) = m.batch.as_mut() {
                b.pattern.push_str(&s);
            }
        }
        Msg::BatchPatternBackspace => {
            if let Some(b) = m.batch.as_mut() {
                b.pattern.pop();
            }
        }
        Msg::BatchApply => {
            if let Some(b) = m.batch.take() {
                for idx in 0..b.targets.len() {
                    let nuevo = b.nuevo_nombre(idx);
                    let (id, original) = &b.targets[idx];
                    // Sólo encolá los que efectivamente cambian de nombre.
                    if &nuevo != original {
                        enqueue(
                            &mut m,
                            handle,
                            OpKind::Rename { id: id.clone(), new_name: nuevo },
                        );
                    }
                }
                m.cur_pane_mut().marked.clear();
            }
        }
        Msg::BatchCancel => {
            m.batch = None;
        }
        Msg::SetLabel(label) => {
            for (id, _) in m.cur_pane().op_targets() {
                m.state.set_label(&id, label);
            }
            m.state.save();
            m.context_menu = None;
        }
        Msg::ClearLabel => {
            for (id, _) in m.cur_pane().op_targets() {
                m.state.clear_label(&id);
            }
            m.state.save();
            m.context_menu = None;
        }
        Msg::AddPlace => {
            // La carpeta seleccionada si es un dir; si no, la carpeta actual.
            let target = match m.cur().selected_node() {
                Some(n) if n.is_container => n.id.clone(),
                _ => m.cur().current_id().clone(),
            };
            if !m.is_foreign() {
                m.state.add_place(&target);
                m.state.save();
            }
            m.context_menu = None;
        }
        Msg::SessionNew => {
            // Guarda la sesión viva, abre una sesión (diente) nueva en el
            // cwd actual y la activa.
            let cwd = cur_dir(&m);
            let snap = m.snapshot_active();
            m.sessions[m.active].snap = Some(snap);
            m.sessions.push(Session {
                name: session_name(&cwd),
                snap: Some(fresh_snap(&cwd)),
            });
            let nuevo = m.sessions.len() - 1;
            m.active = nuevo;
            if let Some(snap) = m.sessions[nuevo].snap.take() {
                m.restore(snap);
            }
        }
        Msg::SessionActivate(i) => {
            m.switch_to(i);
        }
        Msg::TreeToggle(path) => {
            if m.tree_expanded.contains(&path) {
                m.tree_expanded.remove(&path);
            } else {
                ensure_tree_children(&mut m.tree_children, &path);
                m.tree_expanded.insert(path);
            }
        }
        Msg::TreeSelect(path) => {
            if path.is_dir() {
                ensure_tree_children(&mut m.tree_children, &path);
                m.tree_expanded.insert(path.clone());
                m.cur_pane_mut().nav_stack = vec![posix_nav(&path)];
                m.cur_pane_mut().marked.clear();
                // Seleccionar una carpeta abre su vista en el canvas.
                m.canvas = None;
                record_history(&mut m);
                apply_format(&mut m);
                record_recent(&mut m);
                refresh_preview(&mut m);
                // Si la carpeta hereda vista iconos/galería, pedí thumbs.
                if m.cur().view.is_grid() {
                    request_thumbs(&mut m, handle);
                }
                // Mantené el nombre de la sesión en sync con la carpeta.
                let nombre = session_name(&path);
                let activa = m.active;
                m.sessions[activa].name = nombre;
            }
        }
        Msg::TreeScroll(dy) => {
            // Rueda hacia abajo baja el árbol; ~3 filas por muesca.
            let total = count_tree_rows(&m);
            let max = total.saturating_sub(tree_visible_rows(&m));
            let delta = (dy * 3.0).round() as i32;
            let nuevo = (m.tree_scroll as i32 + delta).clamp(0, max as i32);
            m.tree_scroll = nuevo as usize;
        }
        Msg::ThumbReady(path, thumb) => {
            m.thumbs_pending.remove(&path);
            let img = Image::new(ImageData {
                data: Blob::from(thumb.rgba),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: thumb.w,
                height: thumb.h,
            });
            m.thumbs.insert(path, img);
        }
        Msg::ThumbFailed(path) => {
            m.thumbs_pending.remove(&path);
            m.thumbs_failed.insert(path);
        }
        Msg::Palette(pm) => {
            return crate::palette::apply_palette(m, pm, handle);
        }

        // ---- Selección (parity dOpus) ----
        Msg::SelectAll => {
            let ids: Vec<_> = m.cur().visible().iter().map(|(_, n)| n.id.clone()).collect();
            let pane = m.cur_pane_mut();
            for id in ids {
                pane.marked.insert(id);
            }
        }
        Msg::SelectNone => {
            m.cur_pane_mut().marked.clear();
        }
        Msg::InvertSelection => {
            let ids: Vec<_> = m.cur().visible().iter().map(|(_, n)| n.id.clone()).collect();
            let pane = m.cur_pane_mut();
            for id in ids {
                if !pane.marked.insert(id.clone()) {
                    pane.marked.remove(&id);
                }
            }
        }
        Msg::SelectByPattern => {
            m.prompt = Some(Prompt { kind: PromptKind::SelectPattern, text: String::new() });
            m.context_menu = None;
        }
    }
    m
}

/// Marca los hijos visibles del panel enfocado cuyo nombre matchea el glob
/// `pat` (comodín `*`, case-insensitive). Acumula sobre la marca existente.
fn select_by_pattern(m: &mut Model, pat: &str) {
    let ids: Vec<_> = m
        .cur()
        .visible()
        .iter()
        .filter(|(_, n)| glob_match(pat, &n.name))
        .map(|(_, n)| n.id.clone())
        .collect();
    let pane = m.cur_pane_mut();
    for id in ids {
        pane.marked.insert(id);
    }
}

/// Match de glob simple, case-insensitive: `*` matchea cualquier secuencia
/// (incluida vacía); el resto es literal. Sin patrón (`*` solo o vacío) o sin
/// comodín, cae a "contiene" para que `foto` encuentre `mi_foto.png`.
pub(crate) fn glob_match(pat: &str, name: &str) -> bool {
    let pat = pat.to_lowercase();
    let name = name.to_lowercase();
    if !pat.contains('*') {
        return name.contains(&pat);
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let mut pos = 0usize;
    // Ancla del primer/último fragmento: `*.png` exige terminar en ".png";
    // `foto*` exige empezar con "foto".
    if let Some(first) = parts.first() {
        if !first.is_empty() {
            if !name[pos..].starts_with(first) {
                return false;
            }
            pos += first.len();
        }
    }
    for (i, frag) in parts.iter().enumerate() {
        if frag.is_empty() {
            continue;
        }
        // El primer fragmento ya se ancló arriba.
        if i == 0 {
            continue;
        }
        match name[pos..].find(frag) {
            Some(off) => pos += off + frag.len(),
            None => return false,
        }
    }
    if let Some(last) = parts.last() {
        if !last.is_empty() && parts.len() > 1 {
            return name.ends_with(last);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn glob_extension() {
        assert!(glob_match("*.png", "foto.png"));
        assert!(glob_match("*.PNG", "foto.png")); // case-insensitive
        assert!(!glob_match("*.png", "foto.jpg"));
        assert!(!glob_match("*.png", "png.txt"));
    }

    #[test]
    fn glob_prefix_y_medio() {
        assert!(glob_match("foto*", "foto_001.png"));
        assert!(!glob_match("foto*", "mi_foto.png"));
        assert!(glob_match("img*2024*", "img_enero_2024_final.jpg"));
        assert!(!glob_match("img*2024*", "img_enero.jpg"));
    }

    #[test]
    fn sin_comodin_es_contiene() {
        // Sin `*`, cae a "contiene" (case-insensitive).
        assert!(glob_match("foto", "mi_FOTO_grande.png"));
        assert!(!glob_match("foto", "imagen.png"));
    }
}
