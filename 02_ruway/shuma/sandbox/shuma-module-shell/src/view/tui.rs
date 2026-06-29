use super::*;

/// Geometría del card de vim — compartida entre el painter (resaltado)
/// y `copy_vim_selection` (px → celda) para que las celdas coincidan.
/// `VIM_PAD` es fijo (margen del panel); el avance horizontal y el alto
/// de línea son *fallbacks* — los reales los mide el painter sobre el
/// layout de parley y los publica en `State::vim_metrics`.
pub(crate) const VIM_PAD: f64 = 10.0;
pub(crate) const VIM_LINE_H: f64 = 16.0;
pub(crate) const VIM_CHAR_W: f64 = 7.8;
pub(crate) const VIM_FONT_PX: f32 = 13.0;

/// Coordenadas locales (px, relativas al rect del panel) → celda (fila,
/// col), con las métricas reales del monospace (`char_w`, `line_h`).
pub(crate) fn vim_px_to_cell(x: f64, y: f64, char_w: f64, line_h: f64) -> (usize, usize) {
    let col = (((x - VIM_PAD) / char_w).floor()).max(0.0) as usize;
    let row = (((y - VIM_PAD) / line_h).floor()).max(0.0) as usize;
    (row, col)
}

/// Snapshot copiable del Screen para enviar a una closure `paint_with`.
pub(crate) struct TuiSnapshot {
    pub(crate) cells: Vec<Vec<TuiCell>>,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) cursor_r: u16,
    pub(crate) cursor_c: u16,
    pub(crate) hide_cursor: bool,
    /// Imágenes (kitty/sixel) vivas, ancladas a su celda. Las pinta el painter
    /// por encima del grid de texto.
    pub(crate) images: Vec<crate::types::TermImage>,
}

#[derive(Clone)]
pub(crate) struct TuiCell {
    pub(crate) ch: String,
    pub(crate) fg: vt100::Color,
    pub(crate) bg: vt100::Color,
}

/// Copia el screen actual de un `ActiveRun` PTY a un snapshot
/// `Send`-able. Devuelve `None` si el run no es TUI.
pub(crate) fn capture_tui(active: &std::sync::MutexGuard<'_, ActiveRun>) -> Option<TuiSnapshot> {
    let tui = active.tui.as_ref()?;
    let screen = tui.parser.screen();
    let (rows, cols) = screen.size();
    let mut cells: Vec<Vec<TuiCell>> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut row: Vec<TuiCell> = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            let (ch, fg, bg) = match screen.cell(r, c) {
                Some(cell) => (
                    if cell.has_contents() {
                        cell.contents().to_string()
                    } else {
                        " ".to_string()
                    },
                    cell.fgcolor(),
                    cell.bgcolor(),
                ),
                None => (" ".into(), vt100::Color::Default, vt100::Color::Default),
            };
            row.push(TuiCell { ch, fg, bg });
        }
        cells.push(row);
    }
    let (cursor_r, cursor_c) = screen.cursor_position();
    Some(TuiSnapshot {
        cells,
        rows,
        cols,
        cursor_r,
        cursor_c,
        hide_cursor: screen.hide_cursor(),
        images: tui.images.clone(),
    })
}

/// Panel de TUI app-aware: según el programa bajo el PTY elige un skin.
/// `is_tui_fullscreen(state)` ya garantiza que hay un PTY en alt-screen.
/// vim se pinta como un card themeable; el resto cae al grid vt100 crudo.
pub(crate) fn tui_panel<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    // Snapshot + skin en un solo lock; la closure de paint debe ser
    // `Send + Sync`, así que no captura el Mutex.
    // try_lock por la misma razón que `is_tui_active`: si el lector del PTY
    // está dentro del mutex en este instante, devolvemos snapshot vacío y el
    // panel cae al frame anterior — preferible a pasmar la pantalla.
    let (snapshot, skin) = match state.running.as_ref().and_then(|arc| arc.try_lock().ok()) {
        Some(g) => {
            let skin = g.tui.as_ref().map(|t| t.skin).unwrap_or(AppSkin::Generic);
            (capture_tui(&g), skin)
        }
        None => (None, AppSkin::Generic),
    };
    let rect_slot = Arc::clone(&state.last_tui_rect);
    if let AppSkin::Vim = skin {
        let metrics_slot = Arc::clone(&state.vim_metrics);
        return vim_panel::<HostMsg, _>(
            snapshot,
            theme,
            rect_slot,
            metrics_slot,
            state.vim_sel,
            lift,
        );
    }
    generic_grid_panel::<HostMsg>(
        snapshot,
        theme,
        rect_slot,
        Arc::clone(&state.gpu_grid),
        lift,
    )
}

/// Render de grilla vt100 cruda — el camino histórico para htop/less/man.
///
/// El panel acepta clicks y rueda para programas que habilitaron mouse
/// (htop, btop, less, fzf, …): los handlers emiten `TuiMouseClick` /
/// `TuiMouseWheel` que el `update` convierte en bytes xterm-mouse contra
/// el `mouse_protocol_mode` actual del `vt100::Screen` (no-op si el
/// programa no lo pidió).
pub(crate) fn generic_grid_panel<HostMsg: Clone + 'static>(
    snapshot: Option<TuiSnapshot>,
    theme: &Theme,
    rect_slot: Arc<Mutex<(f32, f32)>>,
    gpu_grid: Arc<Mutex<Option<crate::GpuGridResources>>>,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let theme_clone = *theme;
    // Lectura única del env: si `SHUMA_GPU_GRID=1`, el render del texto va
    // por el `CellPipeline` (atlas + quads instanciados) en vez del path
    // vello. El vello sigue dibujando el fondo + el cursor para mantener
    // los handlers de mouse y la geometría del rect publish.
    let use_gpu = std::env::var("SHUMA_GPU_GRID")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // El snapshot lo comparten paint_with (rect/cursor/bg) y gpu_paint_with
    // (cells). Arc para que cada closure capture su propia handle.
    let snapshot = Arc::new(snapshot);

    let snapshot_paint = Arc::clone(&snapshot);
    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
        use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment as TAlign, TextBlock};
        // Publica el rect al state — el próximo Tick disparará resize
        // si las dims cambiaron.
        if let Ok(mut g) = rect_slot.lock() {
            *g = (rect.w, rect.h);
        }
        let Some(snap) = snapshot_paint.as_ref() else { return };
        // Tamaño de la celda derivado del rect disponible. Monoespacio,
        // ancho/alto fijos por celda. Si el panel es chico el grid
        // se recorta abajo/derecha (no scrolleamos por ahora).
        let pad = 6.0_f64;
        let avail_w = (rect.w as f64 - pad * 2.0).max(0.0);
        let avail_h = (rect.h as f64 - pad * 2.0).max(0.0);
        let cell_w = (avail_w / snap.cols as f64).max(1.0);
        let cell_h = (avail_h / snap.rows as f64).max(1.0);
        let font_size = (cell_h * 0.75).clamp(8.0, 18.0) as f32;
        let origin_x = rect.x as f64 + pad;
        let origin_y = rect.y as f64 + pad;

        // Modo GPU: las celdas (bg + glifos) las dibuja el `gpu_paint_with`
        // de abajo via `CellPipeline`. El vello sigue acá sólo por el cursor
        // (el shader del cell pipeline no lo pinta) y por publicar el rect.
        if use_gpu {
            // Skip bg + text — los pinta el pipeline GPU debajo.
            // (Sigo al cursor más abajo, después del bloque de text/bg que
            // este `if` salta con un `return` del closure ... no, el cursor
            // viene en el mismo closure, así que sólo skipeo bg+text.)
        } else {
            // Backgrounds primero (en bloques rect), texto encima.
            for (r, row) in snap.cells.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    let bg = vt_color(cell.bg, theme_clone, true);
                    if bg.components[3] > 0.0 {
                        let x0 = origin_x + c as f64 * cell_w;
                        let y0 = origin_y + r as f64 * cell_h;
                        let rect = KurboRect::new(x0, y0, x0 + cell_w, y0 + cell_h);
                        scene.fill(
                            Fill::NonZero,
                            vello::kurbo::Affine::IDENTITY,
                            bg,
                            None,
                            &rect,
                        );
                    }
                }
            }
        }
        if !use_gpu {
        // Texto por celda. Para reducir shaping, agrupamos runs con
        // mismo color contiguo en la misma fila.
        for (r, row) in snap.cells.iter().enumerate() {
            let mut c = 0usize;
            while c < row.len() {
                let fg = vt_color(row[c].fg, theme_clone, false);
                let mut end = c + 1;
                let mut buf = String::new();
                buf.push_str(&row[c].ch);
                while end < row.len() && row[end].fg == row[c].fg {
                    buf.push_str(&row[end].ch);
                    end += 1;
                }
                if !buf.trim().is_empty() {
                    let x0 = origin_x + c as f64 * cell_w;
                    let y0 = origin_y + r as f64 * cell_h;
                    let block = TextBlock {
                        text: &buf,
                        size_px: font_size,
                        color: fg,
                        origin: (x0, y0),
                        max_width: None,
                        alignment: TAlign::Start,
                        line_height: 1.0,
                        italic: false,
                        font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
                    };
                    let layout = layout_block(ts, &block);
                    draw_layout(scene, &layout, fg, (x0, y0));
                }
                c = end;
            }
        }
        }
        // Imágenes (kitty/sixel) ancladas a su celda, por encima del texto.
        // Las pinta el path vello en ambos modos (GPU y no-GPU).
        for pi in &snap.images {
            let iw = pi.px_w.max(1) as f64;
            let ih = pi.px_h.max(1) as f64;
            let (tw, th) = if pi.cols > 0 && pi.rows > 0 {
                (pi.cols as f64 * cell_w, pi.rows as f64 * cell_h)
            } else {
                // Sin celdas pedidas: encajamos los píxeles en el área libre a
                // la derecha/abajo del ancla, sin agrandar más allá del 1:1.
                let maxw = (avail_w - pi.col as f64 * cell_w).max(cell_w);
                let maxh = (avail_h - pi.row as f64 * cell_h).max(cell_h);
                let scale = (maxw / iw).min(maxh / ih).min(1.0).max(0.000_1);
                (iw * scale, ih * scale)
            };
            let x0 = origin_x + pi.col as f64 * cell_w;
            let y0 = origin_y + pi.row as f64 * cell_h;
            let xf = vello::kurbo::Affine::translate((x0, y0))
                * vello::kurbo::Affine::scale_non_uniform(tw / iw, th / ih);
            scene.draw_image(&pi.image, xf);
        }
        // Cursor: barra vertical en (cursor_r, cursor_c). Lo sigue dibujando
        // el path vello en ambos modos — el `CellPipeline` no lo emite.
        if !snap.hide_cursor {
            let x0 = origin_x + snap.cursor_c as f64 * cell_w;
            let y0 = origin_y + snap.cursor_r as f64 * cell_h;
            let rect = KurboRect::new(x0, y0 + 2.0, x0 + 2.0, y0 + cell_h);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                Color::from_rgba8(214, 222, 232, 220),
                None,
                &rect,
            );
        }
    };

    let lift_click = lift.clone();
    let lift_right = lift.clone();
    let lift_wheel = lift.clone();
    // Closure GPU: si `use_gpu`, dibuja todas las celdas con el
    // `CellPipeline`. Lazy-init del pipeline + atlas + textura en el primer
    // frame; los resources persisten en `state.gpu_grid`. No-op si el
    // modo GPU está apagado o no hay snapshot.
    let snapshot_gpu = Arc::clone(&snapshot);
    let gpu_grid_for_paint = Arc::clone(&gpu_grid);
    let theme_for_gpu = theme_clone;
    let gpu_painter = move |device: &llimphi_ui::llimphi_hal::wgpu::Device,
                            queue: &llimphi_ui::llimphi_hal::wgpu::Queue,
                            encoder: &mut llimphi_ui::llimphi_hal::wgpu::CommandEncoder,
                            target_view: &llimphi_ui::llimphi_hal::wgpu::TextureView,
                            rect: llimphi_ui::PaintRect,
                            viewport: (u32, u32)| {
        if !use_gpu {
            return;
        }
        let Some(snap) = snapshot_gpu.as_ref() else { return };
        let mut guard = match gpu_grid_for_paint.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // Lazy-init: la primera vez compilamos el pipeline + armamos atlas
        // 32×8 (256 glifos iniciales, alcanza para ASCII + box-drawing).
        if guard.is_none() {
            let Some(atlas) = llimphi_widget_terminal::GlyphAtlas::new(
                llimphi_ui::llimphi_text::MONO_FONT_BYTES,
                14.0,
                32,
                8,
            ) else {
                return;
            };
            // El color_format del target lo sabemos del `Hal` que arma
            // la intermediate (Rgba8Unorm por defecto). Asumir Rgba8Unorm;
            // si el host cambia, recompilar pipeline una vez al detectar.
            let pipeline = llimphi_widget_terminal::CellPipeline::new(
                device,
                llimphi_ui::llimphi_hal::wgpu::TextureFormat::Rgba8Unorm,
            );
            let atlas_size = atlas.size();
            let (atlas_texture, atlas_view) =
                llimphi_widget_terminal::CellPipeline::create_atlas_texture(
                    device,
                    queue,
                    atlas.pixels(),
                    atlas_size,
                );
            *guard = Some(crate::GpuGridResources {
                pipeline,
                atlas,
                atlas_texture,
                atlas_view,
                atlas_size,
            });
        }
        let res = guard.as_mut().unwrap();
        // Build instances ANTES de chequear dirty (rasteriza glifos nuevos).
        let cells = build_cell_instances(snap, &mut res.atlas, theme_for_gpu, rect);
        // Si el atlas creció, re-crear textura.
        let new_size = res.atlas.size();
        if new_size != res.atlas_size {
            let (tex, view) = llimphi_widget_terminal::CellPipeline::create_atlas_texture(
                device,
                queue,
                res.atlas.pixels(),
                new_size,
            );
            res.atlas_texture = tex;
            res.atlas_view = view;
            res.atlas_size = new_size;
        } else if let Some(dirty) = res.atlas.take_dirty() {
            // Subir sólo el rect que cambió. Stride completo del atlas.
            let pixels = res.atlas.pixels();
            let row_w = res.atlas_size.0 as usize;
            let mut sub = Vec::with_capacity((dirty.w * dirty.h) as usize);
            for y in 0..dirty.h {
                let src_y = (dirty.y + y) as usize;
                let start = src_y * row_w + dirty.x as usize;
                let end = start + dirty.w as usize;
                sub.extend_from_slice(&pixels[start..end]);
            }
            queue.write_texture(
                llimphi_ui::llimphi_hal::wgpu::TexelCopyTextureInfo {
                    texture: &res.atlas_texture,
                    mip_level: 0,
                    origin: llimphi_ui::llimphi_hal::wgpu::Origin3d {
                        x: dirty.x,
                        y: dirty.y,
                        z: 0,
                    },
                    aspect: llimphi_ui::llimphi_hal::wgpu::TextureAspect::All,
                },
                &sub,
                llimphi_ui::llimphi_hal::wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dirty.w),
                    rows_per_image: Some(dirty.h),
                },
                llimphi_ui::llimphi_hal::wgpu::Extent3d {
                    width: dirty.w,
                    height: dirty.h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let (acw, ach) = res.atlas.cell_size();
        let snap_cols = snap.cols.max(1) as f32;
        let snap_rows = snap.rows.max(1) as f32;
        let pad = 6.0_f32;
        let render_cell_w = ((rect.w - pad * 2.0).max(1.0) / snap_cols).max(1.0);
        let render_cell_h = ((rect.h - pad * 2.0).max(1.0) / snap_rows).max(1.0);
        let _ = (acw, ach); // los pasamos como atlas_w/atlas_h
        let uniforms = llimphi_widget_terminal::CellUniforms {
            viewport_w: viewport.0 as f32,
            viewport_h: viewport.1 as f32,
            cell_w: render_cell_w,
            cell_h: render_cell_h,
            atlas_w: res.atlas_size.0 as f32,
            atlas_h: res.atlas_size.1 as f32,
            _pad0: 0.0,
            _pad1: 0.0,
        };
        res.pipeline.draw(
            device,
            queue,
            encoder,
            target_view,
            &res.atlas_view,
            &cells,
            uniforms,
        );
    };

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .paint_with(painter)
    .gpu_paint_with(gpu_painter)
    // Click izquierdo → press+release del botón 0 en la celda (col,row)
    // que cubra (lx,ly). El handler de update lo encodea sólo si el
    // programa habilitó mouse, sino no-op silencioso.
    .on_click_at(move |lx, ly, rect_w, rect_h| {
        Some(lift_click(Msg::TuiMouseClick {
            button: 0,
            lx,
            ly,
            rect_w,
            rect_h,
        }))
    })
    // Click derecho → botón 2. Algunos TUIs (htop) lo usan para abrir
    // menús contextuales propios.
    .on_right_click_at(move |lx, ly, rect_w, rect_h| {
        Some(lift_right(Msg::TuiMouseClick {
            button: 2,
            lx,
            ly,
            rect_w,
            rect_h,
        }))
    })
    // Rueda → botones 4/5 si el programa habilitó mouse. Si no, devolver
    // None deja que el chasis siga procesando la rueda como scroll del
    // output (los TUIs ocupan toda el área del panel, así que sólo cae
    // a global cuando el programa no quiere mouse).
    .on_scroll(move |_dx, dy| {
        if dy.abs() < f32::EPSILON {
            return None;
        }
        Some(lift_wheel(Msg::TuiMouseWheel {
            dy,
            lx: 0.0,
            ly: 0.0,
            // El runtime no nos da las dims del rect en `on_scroll`; el
            // update sólo las usa para clampear las coords al grid, y
            // como acá lx/ly son (0,0) — esquina superior-izquierda —
            // basta con `1x1` (cae a (1,1) tras local_to_cell).
            rect_w: 1.0,
            rect_h: 1.0,
        }))
    })
}

/// Skin de vim: reconstruye cada fila del `Screen` como una línea de
/// texto en la paleta del tema — sin la grilla de celdas ni los `~` de
/// relleno —, con la última fila como barra de estado. El contenido se
/// lee como un output normal, dentro del card del panel; las teclas
/// siguen yendo al PTY (vim sigue siendo interactivo).
///
/// MVP: read-only (la selección/click-derecho-pegar nativos vienen
/// después, sobre el widget de texto). El objetivo de este paso es que
/// vim deje de verse "como por un vidrio".
pub(crate) fn vim_panel<HostMsg, L>(
    snapshot: Option<TuiSnapshot>,
    theme: &Theme,
    rect_slot: Arc<Mutex<(f32, f32)>>,
    metrics_slot: Arc<Mutex<(f32, f32)>>,
    sel: Option<VimSel>,
    lift: L,
) -> View<HostMsg>
where
    HostMsg: Clone + 'static,
    L: Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
{
    let theme_clone = *theme;
    let lift_drag = lift.clone();
    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
        use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment as TAlign, TextBlock};
        // Publica el rect para que el próximo Tick dispare resize si cambió.
        if let Ok(mut g) = rect_slot.lock() {
            *g = (rect.w, rect.h);
        }
        let Some(snap) = &snapshot else { return };
        let pad = VIM_PAD;
        let font = VIM_FONT_PX;
        // Métricas reales del monospace: medimos un bloque-sonda de 40
        // glifos idénticos y dividimos para el avance horizontal; el alto
        // del layout (line_height 1.0) da el alto de línea. Adivinar las
        // constantes desfasa el resaltado al acumularse por columna.
        const PROBE: &str = "0000000000000000000000000000000000000000"; // 40
        let probe = TextBlock {
            text: PROBE,
            size_px: font,
            color: theme_clone.fg_text,
            origin: (0.0, 0.0),
            max_width: None,
            alignment: TAlign::Start,
            line_height: 1.0,
            italic: false,
            font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
        };
        let m = llimphi_ui::llimphi_text::measure(ts, &probe);
        let char_w = if m.width > 1.0 {
            (m.width as f64) / PROBE.len() as f64
        } else {
            VIM_CHAR_W
        };
        let line_h = if m.height > 1.0 {
            m.height as f64
        } else {
            VIM_LINE_H
        };
        // Publica las métricas para que `copy_vim_selection` use las mismas.
        if let Ok(mut g) = metrics_slot.lock() {
            *g = (char_w as f32, line_h as f32);
        }
        let origin_x = rect.x as f64 + pad;
        let origin_y = rect.y as f64 + pad;
        let n = snap.cells.len();
        // Resaltado de la selección (drag): un rect translúcido por fila.
        if let Some(vs) = sel {
            let (r0, c0) = vim_px_to_cell(vs.ax as f64, vs.ay as f64, char_w, line_h);
            let (r1, c1) = vim_px_to_cell(vs.hx as f64, vs.hy as f64, char_w, line_h);
            let (sr, sc, er, ec) = if (r0, c0) <= (r1, c1) {
                (r0, c0, r1, c1)
            } else {
                (r1, c1, r0, c0)
            };
            let ncols = snap.cells.first().map(|row| row.len()).unwrap_or(0);
            let er = er.min(n.saturating_sub(1));
            let bg = theme_clone.bg_selected;
            let sel_color = Color::from_rgba8(
                (bg.components[0] * 255.0) as u8,
                (bg.components[1] * 255.0) as u8,
                (bg.components[2] * 255.0) as u8,
                120,
            );
            for r in sr..=er {
                let lo = if r == sr { sc } else { 0 };
                let hi = if r == er { (ec + 1).min(ncols) } else { ncols };
                if hi <= lo {
                    continue;
                }
                let x0 = origin_x + lo as f64 * char_w;
                let x1 = origin_x + hi as f64 * char_w;
                let y0 = origin_y + r as f64 * line_h;
                let hrect = KurboRect::new(x0, y0, x1, y0 + line_h);
                scene.fill(
                    Fill::NonZero,
                    vello::kurbo::Affine::IDENTITY,
                    sel_color,
                    None,
                    &hrect,
                );
            }
        }
        for (r, row) in snap.cells.iter().enumerate() {
            let raw: String = row.iter().map(|c| c.ch.as_str()).collect();
            let line_str = raw.trim_end();
            // La última fila es la barra de estado / línea de comando de vim.
            let is_status = n > 1 && r + 1 == n;
            // Relleno de vim: una fila cuyo único contenido es `~`.
            if !is_status && line_str.trim_start() == "~" {
                continue;
            }
            let y = origin_y + r as f64 * line_h;
            let color = if is_status {
                theme_clone.accent
            } else {
                theme_clone.fg_text
            };
            if is_status {
                // Fondo sutil para distinguir la barra de estado del buffer.
                let bar =
                    KurboRect::new(rect.x as f64, y - 2.0, (rect.x + rect.w) as f64, y + line_h);
                scene.fill(
                    Fill::NonZero,
                    vello::kurbo::Affine::IDENTITY,
                    theme_clone.bg_input,
                    None,
                    &bar,
                );
            }
            if !line_str.is_empty() {
                let block = TextBlock {
                    text: line_str,
                    size_px: font,
                    color,
                    origin: (origin_x, y),
                    max_width: None,
                    alignment: TAlign::Start,
                    line_height: 1.0,
                    italic: false,
                    font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
                };
                let layout = layout_block(ts, &block);
                draw_layout(scene, &layout, color, (origin_x, y));
            }
        }
        // Cursor: barra vertical en la posición del cursor de vim.
        if !snap.hide_cursor {
            let x0 = origin_x + snap.cursor_c as f64 * char_w;
            let y0 = origin_y + snap.cursor_r as f64 * line_h;
            let cur = KurboRect::new(x0, y0 + 2.0, x0 + 2.0, y0 + line_h);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                Color::from_rgba8(214, 222, 232, 220),
                None,
                &cur,
            );
        }
    };

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .paint_with(painter)
    // Selección estilo terminal: arrastrar con el botón izquierdo
    // selecciona celdas; al soltar se copia al clipboard.
    .draggable_at(move |phase, dx, dy, lx0, ly0| {
        Some(lift_drag(Msg::VimDrag {
            end: matches!(phase, llimphi_ui::DragPhase::End),
            dx,
            dy,
            ax: lx0,
            ay: ly0,
        }))
    })
    // Paste estilo terminal: click derecho y botón del medio pegan el
    // clipboard al PTY (vim sigue recibiendo las teclas aparte).
    .on_right_click(lift(Msg::VimPaste))
    .on_middle_click(lift(Msg::VimPaste))
}
