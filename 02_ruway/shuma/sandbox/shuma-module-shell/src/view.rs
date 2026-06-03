use super::*;

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = shell_header(state, theme);
    let main_panel: View<HostMsg> = if is_tui_active(state) {
        tui_panel::<HostMsg>(state, theme, lift.clone())
    } else {
        output_pane::<HostMsg>(state, theme, &lift)
    };
    // Panel de grupos [RUN] a la izquierda (rescate del shell GPUI): cada
    // grupo guardado (`:save`) es una card clickable que lo ejecuta, con su
    // tecla F. Sólo aparece si hay grupos y no estamos en un TUI fullscreen.
    let body: View<HostMsg> = if !state.groups.is_empty() && !is_tui_active(state) {
        View::new(Style {
            flex_direction: FlexDirection::Row,
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
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .children(vec![groups_panel::<HostMsg>(state, theme, &lift), main_panel])
    } else {
        main_panel
    };
    let input = shell_input_view(state, theme, lift.clone());

    let mut children = vec![header, body];
    // Banner de reprocess: el próximo comando recibe por stdin el stdout
    // del bloque armado. Click → cancela (toggle).
    if let Some(src) = state.reprocess_source {
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(18.0_f32),
                },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::SetReprocess(src)))
            .text_aligned(
                format!("» reprocesando la salida del bloque #{src} — Enter ejecuta · click cancela"),
                10.0,
                theme.accent,
                Alignment::Start,
            ),
        );
    }
    // Popup de completado: justo encima del input, candidatos con el
    // resaltado actual. Tab/flechas navegan, Enter acepta, Esc cierra.
    if let Some(popup) = completion_popup::<HostMsg>(state, theme) {
        children.push(popup);
    }
    children.push(input);
    if state.history_search.is_some() {
        children.push(history_search_panel::<HostMsg>(state, theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

/// Color por `TokenKind` — paleta diseñada para que el comando salte y
/// los flags/strings tengan su propio tono.
pub(crate) fn token_color(
    kind: TokenKind,
    theme: &Theme,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    match kind {
        TokenKind::Command => theme.accent,
        TokenKind::Argument => theme.fg_text,
        TokenKind::Flag => Color::from_rgba8(220, 200, 120, 255), // amarillo
        TokenKind::StringLit => Color::from_rgba8(160, 210, 140, 255), // verde
        TokenKind::Variable => Color::from_rgba8(200, 160, 220, 255), // violeta
        TokenKind::Pipe | TokenKind::Redirect | TokenKind::Operator => theme.accent,
        TokenKind::Comment | TokenKind::Whitespace => theme.fg_muted,
        TokenKind::Unknown => theme.fg_destructive,
    }
}

/// Renderiza la línea de entrada con tokens coloreados, cursor visible
/// y ghost suggestion. El layout es un nodo único con `paint_with` —
/// medimos cada token con el typesetter en el closure para alinear el
/// cursor al carácter exacto.
pub(crate) fn shell_input_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let bg = if state.focused {
        theme.bg_input_focus
    } else {
        theme.bg_input
    };
    let border = if state.focused {
        theme.border_focus
    } else {
        theme.border
    };

    let text = state.input.text().to_string();
    let cursor = state.input.cursor();
    let ghost = current_ghost(state);
    let placeholder = if text.is_empty() && ghost.is_none() {
        Some("tipeá un comando…".to_string())
    } else {
        None
    };
    // Multi-línea: cada `\n` agrega una línea visible y crece el alto
    // del input. El cursor cae en (línea, columna) calculadas desde el
    // byte offset del cursor.
    let line_count = text.matches('\n').count() + 1;
    const LINE_H: f64 = 18.0;
    const BORDER_INNER_H: f64 = 16.0; // padding visual sumado al alto
    let container_h = BORDER_INNER_H + LINE_H * line_count as f64;
    let theme_clone = *theme;
    let focused = state.focused;

    let painter = move |scene: &mut vello::Scene,
                        ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        use llimphi_ui::llimphi_text::{
            draw_layout, layout_block, measurement, Alignment as TAlign, TextBlock,
        };
        let pad_x = 10.0;
        let baseline_y = rect.y as f64 + 8.0;
        let line_x_start = rect.x as f64 + pad_x;

        if let Some(ph) = &placeholder {
            let block = TextBlock {
                text: ph,
                size_px: 13.0,
                color: theme_clone.fg_placeholder,
                origin: (line_x_start, baseline_y),
                max_width: None,
                alignment: TAlign::Start,
                line_height: 1.2,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(
                scene,
                &layout,
                theme_clone.fg_placeholder,
                (line_x_start, baseline_y),
            );
        }

        // Calcular qué línea/columna ocupa el cursor.
        let (cursor_line_idx, cursor_byte_in_line) = {
            let pre = &text[..cursor];
            let line_idx = pre.matches('\n').count();
            let line_start = pre.rfind('\n').map(|i| i + 1).unwrap_or(0);
            (line_idx, cursor - line_start)
        };

        let mut cursor_x: f64 = line_x_start;
        let mut cursor_y: f64 = baseline_y;
        let mut last_line_end_x: f64 = line_x_start;
        let mut last_line_y: f64 = baseline_y;
        let mut line_byte_start = 0usize;
        for (line_idx, line_str) in text.split('\n').enumerate() {
            let line_y = baseline_y + line_idx as f64 * LINE_H;
            let mut x = line_x_start;
            // Pintar tokens sobre el slice de la línea, usando el
            // tokenizer estándar (dialect por defecto = bash).
            let tokens = shuma_line::tokenize(line_str, state_dialect_default());
            for tok in &tokens {
                let color = token_color(tok.kind, &theme_clone);
                let segment = &line_str[tok.start..tok.end];
                let block = TextBlock {
                    text: segment,
                    size_px: 13.0,
                    color,
                    origin: (x, line_y),
                    max_width: None,
                    alignment: TAlign::Start,
                    line_height: 1.2,
                    italic: false,
                    font_family: None,
                };
                let layout = layout_block(ts, &block);
                let m = measurement(&layout);
                draw_layout(scene, &layout, color, (x, line_y));
                if line_idx == cursor_line_idx
                    && tok.start < cursor_byte_in_line
                    && cursor_byte_in_line <= tok.end
                {
                    let prefix = &line_str[tok.start..cursor_byte_in_line];
                    if prefix.is_empty() {
                        cursor_x = x;
                    } else {
                        let pblock = TextBlock {
                            text: prefix,
                            size_px: 13.0,
                            color,
                            origin: (x, line_y),
                            max_width: None,
                            alignment: TAlign::Start,
                            line_height: 1.2,
                            italic: false,
                            font_family: None,
                        };
                        let plat = layout_block(ts, &pblock);
                        cursor_x = x + measurement(&plat).width as f64;
                    }
                    cursor_y = line_y;
                }
                x += m.width as f64;
            }
            // Cursor al final de una línea vacía / sin tokens hasta el cursor.
            if line_idx == cursor_line_idx
                && (cursor_byte_in_line == line_str.len() || tokens.is_empty())
            {
                cursor_x = x;
                cursor_y = line_y;
            }
            last_line_end_x = x;
            last_line_y = line_y;
            line_byte_start += line_str.len() + 1; // +1 por el '\n'
        }
        let _ = line_byte_start; // sólo informativo

        // Ghost suggestion: sólo aplica si el cursor está al final del
        // texto (última línea, columna final). Lo pinta detrás del cursor.
        if let Some(suffix) = &ghost {
            if !suffix.is_empty() && cursor == text.len() {
                let block = TextBlock {
                    text: suffix,
                    size_px: 13.0,
                    color: theme_clone.fg_placeholder,
                    origin: (last_line_end_x, last_line_y),
                    max_width: None,
                    alignment: TAlign::Start,
                    line_height: 1.2,
                    italic: false,
                    font_family: None,
                };
                let layout = layout_block(ts, &block);
                draw_layout(
                    scene,
                    &layout,
                    theme_clone.fg_placeholder,
                    (last_line_end_x, last_line_y),
                );
            }
        }

        // Cursor — barra vertical de 2 px en la línea calculada.
        if focused {
            use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
            use llimphi_ui::llimphi_raster::peniko::Fill;
            let cursor_rect =
                KurboRect::new(cursor_x, cursor_y + 2.0, cursor_x + 2.0, cursor_y + LINE_H);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                Color::from_rgba8(214, 222, 232, 220),
                None,
                &cursor_rect,
            );
        }
    };

    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .paint_with(painter);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(container_h as f32),
        },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(border)
    .radius(4.0)
    .on_click(lift(Msg::FocusInput))
    .children(vec![inner])
}

/// Dialect por defecto para el painter — el `LineState` lo guarda
/// internamente pero no lo expone; mientras todos los usos sean bash
/// alcanza con este getter.
pub(crate) fn state_dialect_default() -> shuma_line::Dialect {
    shuma_line::Dialect::default()
}

/// Panel de TUI app-aware: según el programa bajo el PTY elige un skin.
/// `is_tui_active(state)` ya garantiza que hay un run con PTY. vim se
/// pinta como un card themeable; el resto cae al grid vt100 crudo.
pub(crate) fn tui_panel<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    // Snapshot + skin en un solo lock; la closure de paint debe ser
    // `Send + Sync`, así que no captura el Mutex.
    let (snapshot, skin) = match state.running.as_ref().and_then(|arc| arc.lock().ok()) {
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
    generic_grid_panel::<HostMsg>(snapshot, theme, rect_slot)
}

/// Render de grilla vt100 cruda — el camino histórico para htop/less/man.
pub(crate) fn generic_grid_panel<HostMsg: Clone + 'static>(
    snapshot: Option<TuiSnapshot>,
    theme: &Theme,
    rect_slot: Arc<Mutex<(f32, f32)>>,
) -> View<HostMsg> {
    let theme_clone = *theme;

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
        let Some(snap) = &snapshot else { return };
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
                        font_family: None,
                    };
                    let layout = layout_block(ts, &block);
                    draw_layout(scene, &layout, fg, (x0, y0));
                }
                c = end;
            }
        }
        // Cursor: barra vertical en (cursor_r, cursor_c).
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
            font_family: None,
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
                    font_family: None,
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

/// Snapshot copiable del Screen para enviar a una closure `paint_with`.
pub(crate) struct TuiSnapshot {
    cells: Vec<Vec<TuiCell>>,
    rows: u16,
    cols: u16,
    cursor_r: u16,
    cursor_c: u16,
    hide_cursor: bool,
}

#[derive(Clone)]
pub(crate) struct TuiCell {
    ch: String,
    fg: vt100::Color,
    bg: vt100::Color,
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
    })
}

/// Convierte un `vt100::Color` a un `peniko::Color`, respetando el tema
/// del shell (los 16 índices ANSI se mapean a una paleta consistente).
pub(crate) fn vt_color(
    c: vt100::Color,
    theme: Theme,
    is_bg: bool,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    match c {
        vt100::Color::Default => {
            if is_bg {
                // Transparent — el panel ya tiene su propio fill.
                Color::from_rgba8(0, 0, 0, 0)
            } else {
                theme.fg_text
            }
        }
        vt100::Color::Rgb(r, g, b) => Color::from_rgba8(r, g, b, 255),
        vt100::Color::Idx(i) => ansi_idx_to_color(i),
    }
}

/// Mapeo 256 → RGB usando la paleta xterm estándar. Cubre los 16
/// básicos, el cubo 6×6×6 y la rampa de grises.
pub(crate) fn ansi_idx_to_color(i: u8) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    const BASIC: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 49, 49],
        [13, 188, 121],
        [229, 229, 16],
        [36, 114, 200],
        [188, 63, 188],
        [17, 168, 205],
        [229, 229, 229],
        [102, 102, 102],
        [241, 76, 76],
        [35, 209, 139],
        [245, 245, 67],
        [59, 142, 234],
        [214, 112, 214],
        [41, 184, 219],
        [255, 255, 255],
    ];
    if i < 16 {
        let [r, g, b] = BASIC[i as usize];
        return Color::from_rgba8(r, g, b, 255);
    }
    if i >= 232 {
        let v = 8 + (i - 232) * 10;
        return Color::from_rgba8(v, v, v, 255);
    }
    let i = i - 16;
    let r = i / 36;
    let g = (i / 6) % 6;
    let b = i % 6;
    let to_byte = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
    Color::from_rgba8(to_byte(r), to_byte(g), to_byte(b), 255)
}

/// Overlay de búsqueda Ctrl-R. Vive como hijo extra del root cuando
/// `state.history_search` está activo; un input + lista de matches.
pub(crate) fn history_search_panel<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> View<HostMsg> {
    let search = state
        .history_search
        .as_ref()
        .expect("panel sólo se construye con search activo");
    let matches: Vec<String> = {
        let history = state.history.lock().unwrap();
        history
            .fuzzy_search(&search.query, 50)
            .into_iter()
            .map(|e| e.line.clone())
            .collect()
    };
    let label = format!("Ctrl-R › {}", search.query);
    let mut children: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label, 12.0, theme.accent, Alignment::Start)];

    for (i, m) in matches.iter().enumerate().take(8) {
        let color = if i == search.selected {
            theme.accent
        } else {
            theme.fg_text
        };
        let bg = if i == search.selected {
            theme.bg_selected
        } else {
            theme.bg_panel
        };
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(18.0_f32),
                },
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(m.clone(), 12.0, color, Alignment::Start),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .children(children)
}

pub(crate) fn shell_header<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> View<HostMsg> {
    let status = if let Some(arc) = state.running.as_ref() {
        let cmd = match arc.lock() {
            Ok(g) => g.command.clone(),
            Err(p) => p.into_inner().command.clone(),
        };
        let queued = state.queue.len();
        if queued > 0 {
            format!(" · ⟳ {cmd} (+{queued} en cola)")
        } else {
            format!(" · ⟳ {cmd}")
        }
    } else {
        String::new()
    };
    // Rama git del cwd, si estamos en un repo (`· (main)`). La fuente del
    // shell no trae el glifo ⎇, así que usamos la convención de paréntesis.
    let branch = match git_branch(&state.cwd) {
        Some(b) => format!(" · ({b})"),
        None => String::new(),
    };
    let label = format!(
        "Shell · {} · cwd: {}{}{}",
        state.source.label(),
        pretty_path(&state.cwd),
        branch,
        status,
    );
    let color = if state.is_running() {
        theme.accent
    } else {
        theme.fg_text
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label, 12.0, color, Alignment::Start)
}

/// Panel de grupos `[RUN]` a la izquierda: una card por grupo guardado
/// (`:save`), clickable para ejecutarlo, con su tecla F. Ancho fijo. El
/// caller ya garantizó que hay ≥1 grupo.
pub(crate) fn groups_panel<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    const PANEL_W: f32 = 176.0;
    let mut children: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("GRUPOS".to_string(), 10.0, theme.fg_muted, Alignment::Start)];

    for (i, g) in state.groups.iter().enumerate() {
        let title = format!("F{}  {}", i + 1, g.name);
        let sub = format!("{} cmds", g.lines.len());
        let card = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: length(38.0_f32),
            },
            padding: Rect {
                left: length(6.0_f32),
                right: length(6.0_f32),
                top: length(3.0_f32),
                bottom: length(3.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(4.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(Msg::RunGroup(i)))
        .children(vec![
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(title, 12.0, theme.accent, Alignment::Start),
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(14.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(sub, 10.0, theme.fg_muted, Alignment::Start),
        ]);
        children.push(card);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(PANEL_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .children(children)
}

/// Popup de completado: lista de candidatos con el actual resaltado. Se
/// pinta sobre el input (en la columna, justo antes). Acota a `MAX_ROWS`
/// filas visibles centradas en el índice. `None` si no hay popup abierto.
pub(crate) fn completion_popup<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> Option<View<HostMsg>> {
    let comp = state.completion.as_ref()?;
    if comp.candidates.is_empty() {
        return None;
    }
    const MAX_ROWS: usize = 8;
    const ROW: f32 = 18.0;
    let n = comp.candidates.len();
    let sel = state.completion_index.min(n - 1);
    // Ventana deslizante centrada en la selección.
    let start = sel.saturating_sub(MAX_ROWS / 2).min(n.saturating_sub(MAX_ROWS));
    let end = (start + MAX_ROWS).min(n);

    let mut rows: Vec<View<HostMsg>> = Vec::new();
    for (i, cand) in comp.candidates[start..end].iter().enumerate() {
        let idx = start + i;
        let selected = idx == sel;
        let (fill, fg) = if selected {
            (theme.accent, theme.bg_panel)
        } else {
            (theme.bg_input, theme.fg_text)
        };
        rows.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(ROW),
                },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(fill)
            .text_aligned(cand.clone(), 12.0, fg, Alignment::Start),
        );
    }
    // Pie con el conteo cuando hay más de lo que entra.
    let mut total_rows = rows.len();
    if n > MAX_ROWS {
        rows.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(ROW),
                },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                format!("{}/{} · Tab/↑↓ navega · Enter acepta · Esc cierra", sel + 1, n),
                10.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
        total_rows += 1;
    }

    Some(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: length(total_rows as f32 * ROW + 4.0),
            },
            padding: Rect {
                left: length(2.0_f32),
                right: length(2.0_f32),
                top: length(2.0_f32),
                bottom: length(2.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(4.0)
        .children(rows),
    )
}

// Geometría fija del panel de output. Debe coincidir EXACTAMENTE con los
// `Style` de `output_pane`/`command_card`: el scroll calcula `content_h`
// con estas constantes (no medimos el árbol; con alturas fijas alcanza).
pub(crate) const PANE_PAD_V: f32 = 12.0; // padding top 6 + bottom 6 del column interno
pub(crate) const PANE_GAP: f32 = 6.0; // gap entre cards / líneas sueltas
pub(crate) const CARD_PAD_V: f32 = 9.0; // card padding top 4 + bottom 5
pub(crate) const CARD_GAP: f32 = 2.0; // gap entre hijos de la card
pub(crate) const HEADER_H: f32 = 20.0; // header de la card
pub(crate) const STAGES_H: f32 = 20.0; // fila de etapas de pipe
pub(crate) const ROW_H: f32 = 16.0; // una línea de output

pub(crate) fn output_pane<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    const MAX_VISIBLE: usize = 400;
    let start = state.output.len().saturating_sub(MAX_VISIBLE);
    let visible = &state.output[start..];

    // Agrupamos por `block` COLECTANDO todas las líneas del bloque aunque
    // se intercalen en el buffer (un job de fondo que escupe entre líneas
    // del foreground ya no fragmenta ni contamina ninguna card). El orden
    // de las cards es el de primera aparición del bloque.
    let mut order: Vec<u64> = Vec::new();
    let mut groups: std::collections::HashMap<u64, Vec<&OutputLine>> =
        std::collections::HashMap::new();
    for line in visible {
        if !groups.contains_key(&line.block) {
            order.push(line.block);
        }
        groups.entry(line.block).or_default().push(line);
    }

    // Cada item lleva su alto exacto → `content_h` para el scroll.
    let mut items: Vec<(View<HostMsg>, f32)> = Vec::new();
    for id in &order {
        let g = &groups[id];
        if g.first()
            .map(|l| l.kind == OutputKind::Prompt)
            .unwrap_or(false)
        {
            items.push(command_card::<HostMsg>(
                g.as_slice(),
                *id,
                state,
                theme,
                lift,
            ));
        } else {
            // Líneas sueltas (tope parcial tras capar, notices iniciales).
            for &line in g.iter() {
                items.push((
                    render_output_line::<HostMsg>(line, &state.cwd, theme, lift),
                    ROW_H,
                ));
            }
        }
    }

    let content_h = if items.is_empty() {
        PANE_PAD_V
    } else {
        PANE_PAD_V
            + items.iter().map(|(_, h)| *h).sum::<f32>()
            + PANE_GAP * (items.len() as f32 - 1.0)
    };
    let children: Vec<View<HostMsg>> = items.into_iter().map(|(v, _)| v).collect();

    // Scroll: el viewport lo midió el painter el frame anterior. Por
    // defecto pegado al fondo (lo último visible, como una terminal);
    // `scroll_px` (rueda) desplaza hacia el historial. Publicamos el
    // overflow para que `Msg::Scroll` clampe sin recomputar geometría.
    let viewport_h = state.out_viewport_h.lock().map(|g| *g).unwrap_or(0.0);
    let overflow = (content_h - viewport_h).max(0.0);
    if let Ok(mut g) = state.out_overflow.lock() {
        *g = overflow;
    }
    let ty: f64 = if viewport_h < 1.0 {
        0.0 // primer frame, todavía sin medir → tope
    } else {
        (state.scroll_px.clamp(0.0, overflow) - overflow) as f64
    };

    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(PANE_GAP),
        },
        align_items: Some(AlignItems::Stretch),
        ..Default::default()
    })
    .transform(vello::kurbo::Affine::translate((0.0, ty)))
    .children(children);

    // El painter publica el alto del viewport; coexiste con los hijos
    // (el compositor pinta painter y luego children).
    let slot = Arc::clone(&state.out_viewport_h);
    let painter = move |_scene: &mut vello::Scene,
                        _ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        if let Ok(mut g) = slot.lock() {
            *g = rect.h;
        }
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        // Región scrolleable en una flex column: `flex_basis: 0` +
        // `min_height: 0` para que tome SÓLO el espacio sobrante (tras el
        // header y el input) y NO el tamaño de su contenido. Sin esto el
        // alto del contenido (un `ls` largo) se filtra al flex-basis y el
        // panel aplasta/expulsa el input. El contenido se clipa adentro.
        flex_basis: length(0.0_f32),
        flex_grow: 1.0,
        min_size: Size {
            width: Dimension::auto(),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .clip(true)
    .paint_with(painter)
    .children(vec![inner])
}

/// Color del badge de estado a partir del texto de la notice de cierre
/// (`✔ exit 0`, `✘ exit N`, `⏹ cancel …`). `None` si la línea no es un
/// estado de cierre — se queda en el cuerpo de la card.
pub(crate) fn status_color(
    text: &str,
    theme: &Theme,
) -> Option<llimphi_ui::llimphi_raster::peniko::Color> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let t = text.trim_start();
    if t.starts_with('✔') {
        Some(Color::from_rgba8(120, 200, 140, 255)) // verde "ok"
    } else if t.starts_with('✘') || t.starts_with('⏹') {
        Some(theme.fg_destructive)
    } else {
        None
    }
}

/// Extrae el comando crudo del texto del header (`$ ls | wc`, o el de un
/// job de fondo `[0] $ sleep 5 &`) — para parsear las etapas del pipe.
pub(crate) fn extract_command(header: &str) -> String {
    let after = header.splitn(2, "$ ").nth(1).unwrap_or(header);
    after.trim().trim_end_matches('&').trim_end().to_string()
}

/// Fila de etapas de un pipe: `⇢ a | b | c`, cada etapa clickable para
/// re-ejecutar la línea truncada hasta ahí (inspeccionar intermedios).
/// `None` si la línea no es un pipe de ≥2 etapas. Recuperada del shuma
/// GPUI viejo (commit 3751aadb), ahora sobre Llimphi.
pub(crate) fn pipe_stages_row<HostMsg: Clone + 'static>(
    header_text: &str,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    let cmd = extract_command(header_text);
    let toks = shuma_line::tokenize(&cmd, state_dialect_default());
    let pipe = shuma_line::split_pipeline(&toks);
    if pipe.stages.len() < 2 {
        return None;
    }
    let raw_parts: Vec<&str> = cmd.split('|').collect();
    let mut row_children: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("⇢".to_string(), 11.0, theme.fg_muted, Alignment::Start)];

    for (i, st) in pipe.stages.iter().enumerate() {
        let label = st
            .command
            .clone()
            .unwrap_or_else(|| format!("etapa {}", i + 1));
        // Prefijo a re-ejecutar: la línea hasta esta etapa, inclusive.
        let prefix = raw_parts
            .get(..=i)
            .map(|p| p.join("|").trim().to_string())
            .unwrap_or_else(|| cmd.clone());
        let l = lift.clone();
        row_children.push(
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(16.0_f32),
                },
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(l(Msg::RunLine(prefix)))
            .text_aligned(label, 11.0, theme.fg_text, Alignment::Start),
        );
    }

    Some(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(STAGES_H),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(5.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(row_children),
    )
}

/// Paleta de etapa — hues desaturados, en la misma familia que la de
/// tokens. Cicla a las 6; un pipe con más etapas reusa colores, sigue
/// siendo legible.
const STAGE_PALETTE: [(u8, u8, u8); 6] = [
    (130, 195, 205), // teal
    (220, 190, 120), // ámbar
    (160, 205, 150), // verde
    (195, 160, 215), // violeta
    (220, 160, 150), // coral
    (150, 180, 225), // azul
];

/// Color estable por índice de etapa — para que cada etapa del pipe lea
/// distinto de un vistazo (chip + sus líneas + su barra-guía).
pub(crate) fn stage_color(i: usize) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (r, g, b) = STAGE_PALETTE[i % STAGE_PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

/// Misma tinta, atenuada (alfa 80%) — para el texto de las líneas
/// capturadas: menos peso visual que el chip que las titula.
fn stage_color_dim(i: usize) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (r, g, b) = STAGE_PALETTE[i % STAGE_PALETTE.len()];
    Color::from_rgba8(r, g, b, 204)
}

/// Bytes a etiqueta compacta: `840`, `1.2K`, `3.4M`. Sin espacio para que
/// quepa en el chip.
fn humanize_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}K", n as f32 / 1024.0)
    } else {
        format!("{:.1}M", n as f32 / (1024.0 * 1024.0))
    }
}

/// Fila de etapas con **captura en vivo** (tee): cada chip despliega las
/// líneas intermedias ya capturadas de su etapa, sin re-ejecutar. Devuelve
/// `(views, alto)` — la fila de chips más, por cada etapa desplegada, sus
/// líneas. `stage_lines` son las `OutputLine` con `stage = Some(_)` del
/// bloque. La última etapa no se captura (su salida es el cuerpo).
pub(crate) fn stage_capture_rows<HostMsg: Clone + 'static>(
    header_text: &str,
    stage_lines: &[&OutputLine],
    block: u64,
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> (Vec<View<HostMsg>>, f32) {
    let cmd = extract_command(header_text);
    let toks = shuma_line::tokenize(&cmd, state_dialect_default());
    let pipe = shuma_line::split_pipeline(&toks);
    if pipe.stages.len() < 2 {
        return (Vec::new(), 0.0);
    }

    // Chips de etapa.
    let mut row_children: Vec<View<HostMsg>> = vec![View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("⇢".to_string(), 11.0, theme.fg_muted, Alignment::Start)];

    for (i, st) in pipe.stages.iter().enumerate() {
        let captured = stage_lines.iter().filter(|l| l.stage == Some(i)).count();
        let bytes: usize = stage_lines
            .iter()
            .filter(|l| l.stage == Some(i))
            .map(|l| l.text.len())
            .sum();
        let expanded = state.expanded_stages.contains(&(block, i));
        let base = st
            .command
            .clone()
            .unwrap_or_else(|| format!("etapa {}", i + 1));
        // Conteo doble (líneas + bytes) sólo cuando hay captura.
        let label = if captured > 0 {
            format!("{base}  {captured}L {}", humanize_bytes(bytes))
        } else {
            base
        };
        // La última etapa no tiene captura (su salida es el cuerpo): chip
        // inerte, en color tenue, para que se vea la estructura del pipe.
        let is_last = i + 1 == pipe.stages.len();
        let fill = if expanded {
            theme.bg_row_hover
        } else {
            theme.bg_input
        };
        // Color estable por etapa para las que capturan; la última, tenue.
        let txt_color = if is_last {
            theme.fg_muted
        } else {
            stage_color(i)
        };
        let mut chip = View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(16.0_f32),
            },
            padding: Rect {
                left: length(5.0_f32),
                right: length(5.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(fill)
        .radius(3.0)
        .text_aligned(label, 11.0, txt_color, Alignment::Start);
        if !is_last {
            chip = chip
                .hover_fill(theme.bg_row_hover)
                .on_click(lift(Msg::ToggleStage { block, stage: i }));
        }
        row_children.push(chip);
    }

    let chips_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(STAGES_H),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(5.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(row_children);

    let mut out: Vec<View<HostMsg>> = vec![chips_row];
    let mut height = STAGES_H;

    // Líneas capturadas de cada etapa desplegada, en orden de etapa. Cada
    // etapa va como un bloque `Row[barra-guía coloreada | columna de
    // líneas]`: la barra ata visualmente las líneas a su chip por color.
    for (i, _st) in pipe.stages.iter().enumerate() {
        if !state.expanded_stages.contains(&(block, i)) {
            continue;
        }
        let lines: Vec<&&OutputLine> =
            stage_lines.iter().filter(|l| l.stage == Some(i)).collect();
        let color = stage_color(i);
        let dim = stage_color_dim(i);

        // Columna de líneas (o el placeholder si la etapa aún no emitió).
        let mut col_children: Vec<View<HostMsg>> = Vec::new();
        let block_h = if lines.is_empty() {
            col_children.push(
                row_text(ROW_H)
                    .text_aligned(
                        "(sin líneas capturadas)".to_string(),
                        11.0,
                        theme.fg_muted,
                        Alignment::Start,
                    ),
            );
            ROW_H
        } else {
            for l in &lines {
                col_children.push(
                    row_text(ROW_H)
                        .text_aligned(l.text.clone(), 12.0, dim, Alignment::Start),
                );
            }
            lines.len() as f32 * ROW_H
        };

        let col = View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(block_h),
            },
            ..Default::default()
        })
        .children(col_children);

        // Barra-guía: 2px de ancho, estira al alto del bloque (align-items
        // stretch por defecto en el Row), con sangría a izquierda.
        let bar = View::new(Style {
            size: Size {
                width: length(2.0_f32),
                height: percent(1.0_f32),
            },
            margin: Rect {
                left: length(8.0_f32),
                right: length(6.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(color)
        .radius(1.0);

        out.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(block_h),
                },
                ..Default::default()
            })
            .children(vec![bar, col]),
        );
        height += block_h;
    }

    (out, height)
}

/// Una fila de texto de alto `h`, ancho completo, sin padding lateral —
/// la sangría la da la barra-guía del bloque de etapa.
fn row_text<HostMsg: Clone + 'static>(h: f32) -> View<HostMsg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        ..Default::default()
    })
}

/// Renderiza un bloque-comando como card desplegable: header (chevron +
/// comando + badge de estado, clickable para plegar), opcional fila de
/// etapas de pipe, y cuerpo (la salida, oculta si está colapsado).
/// `group[0]` es el `Prompt`. Devuelve `(view, alto_exacto)` — el alto
/// alimenta el cálculo de scroll de `output_pane`.
pub(crate) fn command_card<HostMsg: Clone + 'static>(
    group: &[&OutputLine],
    block: u64,
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> (View<HostMsg>, f32) {
    let collapsed = state.collapsed.contains(&block);
    let header_text = group[0].text.clone();

    // Separamos la notice de cierre (se promueve a badge), las líneas de
    // etapas intermedias (tee — van a su desplegable) y el resto (cuerpo).
    // Si hay varias notices de cierre, gana la última.
    let mut body: Vec<&OutputLine> = Vec::new();
    let mut stage_lines: Vec<&OutputLine> = Vec::new();
    let mut badge: Option<(String, llimphi_ui::llimphi_raster::peniko::Color)> = None;
    for &l in &group[1..] {
        if l.stage.is_some() {
            stage_lines.push(l);
        } else if let Some(color) = status_color(&l.text, theme) {
            badge = Some((l.text.clone(), color));
        } else {
            body.push(l);
        }
    }
    // Comando aún vivo (sin notice de cierre todavía): spinner en accent.
    // (Foreground o job de fondo: ambos siguen "vivos" hasta su exit.)
    let still_running = badge.is_none()
        && ((state.current_block == block && state.is_running())
            || state.bg_jobs.iter().any(|j| {
                j.lock()
                    .map(|g| g.block == block && !g.handle.is_finished())
                    .unwrap_or(false)
            }));
    if still_running {
        badge = Some(("⟳".to_string(), theme.accent));
    }

    let chevron = if collapsed { "▸" } else { "▾" };
    let mut header_children: Vec<View<HostMsg>> = vec![
        View::new(Style {
            size: Size {
                width: length(14.0_f32),
                height: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(chevron.to_string(), 11.0, theme.fg_muted, Alignment::Start),
        View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(16.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(header_text.clone(), 12.0, theme.accent, Alignment::Start),
    ];
    // Chip de reprocess: alimenta el stdout de esta card como stdin del
    // próximo comando. Sólo en cards con stdout. Hit-test innermost-wins:
    // el chip gana el click sobre el header (que pliega el bloque).
    let has_stdout = group
        .iter()
        .any(|l| l.kind == OutputKind::Stdout && l.stage.is_none());
    if has_stdout {
        let armed = state.reprocess_source == Some(block);
        let (fill, fg) = if armed {
            (theme.accent, theme.bg_panel)
        } else {
            (theme.bg_input, theme.fg_muted)
        };
        header_children.push(
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(16.0_f32),
                },
                padding: Rect {
                    left: length(5.0_f32),
                    right: length(5.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(fill)
            .radius(3.0)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::SetReprocess(block)))
            .text_aligned("» stdin".to_string(), 10.0, fg, Alignment::Start),
        );
    }
    if let Some((btxt, bcolor)) = badge {
        header_children.push(
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(btxt, 11.0, bcolor, Alignment::End),
        );
    }

    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(8.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::ToggleBlock(block)))
    .children(header_children);

    let mut card_children: Vec<View<HostMsg>> = vec![header];
    let mut child_h_sum = HEADER_H;

    // Fila de etapas de pipe (sólo si NO está colapsado y es un pipe).
    if !collapsed {
        if stage_lines.is_empty() {
            // Sin captura en vivo (pipe vía `sh -c` o comando suelto): los
            // chips re-ejecutan la línea hasta esa etapa.
            if let Some(row) = pipe_stages_row::<HostMsg>(&header_text, theme, lift) {
                card_children.push(row);
                child_h_sum += STAGES_H;
            }
        } else {
            // Con captura (pipe directo + tee): los chips despliegan las
            // líneas intermedias ya capturadas, sin re-ejecutar.
            let (rows, h) = stage_capture_rows::<HostMsg>(
                &header_text,
                &stage_lines,
                block,
                state,
                theme,
                lift,
            );
            for r in rows {
                card_children.push(r);
            }
            child_h_sum += h;
        }
    }

    if collapsed {
        if !body.is_empty() {
            card_children.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(ROW_H),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    format!("⋯ {} líneas", body.len()),
                    11.0,
                    theme.fg_muted,
                    Alignment::Start,
                ),
            );
            child_h_sum += ROW_H;
        }
    } else {
        for &line in &body {
            card_children.push(render_output_line::<HostMsg>(line, &state.cwd, theme, lift));
            child_h_sum += ROW_H;
        }
    }

    let n_children = card_children.len() as f32;
    let card_h = CARD_PAD_V + child_h_sum + CARD_GAP * (n_children - 1.0);

    let view = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(5.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(CARD_GAP),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(5.0)
    .children(card_children);

    (view, card_h)
}

/// Una "pieza" del partición de una línea: el texto, su color y el
/// kind de decoración (`None` = texto base, no clickable). El render
/// la convierte en `View`s; los tests verifican la partición sin
/// pintar.
#[derive(Debug, Clone)]
pub(crate) struct LinePiece {
    pub(crate) text: String,
    pub(crate) color: llimphi_ui::llimphi_raster::peniko::Color,
    pub(crate) deco: Option<shuma_line::DecorationKind>,
}

/// Divide `text` en piezas según `decorations`. Las piezas no decoradas
/// llevan `color = base` y `deco = None`. Las decoradas llevan el
/// color según el kind y `deco = Some(kind.clone())`.
pub(crate) fn partition_line(
    text: &str,
    decorations: &[shuma_line::Decoration],
    base: llimphi_ui::llimphi_raster::peniko::Color,
    theme: &Theme,
) -> Vec<LinePiece> {
    use shuma_line::DecorationKind as Dk;
    let mut out: Vec<LinePiece> = Vec::new();
    let mut cursor = 0usize;
    for d in decorations {
        if d.start < cursor || d.end > text.len() || d.start >= d.end {
            continue;
        }
        if d.start > cursor {
            out.push(LinePiece {
                text: text[cursor..d.start].to_string(),
                color: base,
                deco: None,
            });
        }
        let color = match &d.kind {
            Dk::GitSha(_) => theme.fg_muted,
            // El resto va al accent — paths, urls, grep refs, issue refs,
            // box-drawing. Sin underline (Llimphi aún no lo soporta).
            _ => theme.accent,
        };
        out.push(LinePiece {
            text: text[d.start..d.end].to_string(),
            color,
            deco: Some(d.kind.clone()),
        });
        cursor = d.end;
    }
    if cursor < text.len() {
        out.push(LinePiece {
            text: text[cursor..].to_string(),
            color: base,
            deco: None,
        });
    }
    out
}

/// Pinta una línea del output. Para Stdout/Stderr aplica
/// `shuma_line::decorate_line`: pinta cada span con su color y, si la
/// decoración es accionable (`Path`/`Url`/`GrepRef`/`GitSha`), agrega
/// un `on_click` que dispara `Msg::OpenDecoration`. Para Prompt/Notice
/// usa el atajo `text_aligned` plano.
pub(crate) fn render_output_line<HostMsg: Clone + 'static>(
    line: &OutputLine,
    cwd: &std::path::Path,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let line_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    };

    match line.kind {
        OutputKind::Prompt => View::new(line_style).text_aligned(
            line.text.clone(),
            12.0,
            theme.accent,
            Alignment::Start,
        ),
        OutputKind::Notice => View::new(line_style).text_aligned(
            line.text.clone(),
            12.0,
            theme.fg_muted,
            Alignment::Start,
        ),
        OutputKind::Stdout | OutputKind::Stderr => {
            let base = if matches!(line.kind, OutputKind::Stderr) {
                theme.fg_destructive
            } else {
                theme.fg_text
            };
            let decorations = shuma_line::decorate_line(&line.text, cwd);
            // Atajo: si no hubo decoraciones, una sola text_aligned alcanza.
            if decorations.is_empty() {
                return View::new(line_style).text_aligned(
                    line.text.clone(),
                    12.0,
                    base,
                    Alignment::Start,
                );
            }
            let children =
                build_span_children::<HostMsg>(&line.text, &decorations, base, theme, lift);
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .children(children)
        }
    }
}

/// Convierte las piezas en una lista de `View`s. Las accionables
/// (Path/Url/GrepRef/GitSha) llevan `on_click`.
/// Mapea la categoría semántica de `shuma-line` al icono vectorial del
/// set canónico `llimphi-icons`. Los iconos monocromos son más gruesos
/// que los emoji (un solo `code` para todos los lenguajes, un `file_text`
/// para todos los documentos) — la pérdida de granularidad es el precio
/// de no depender de fuentes de emoji del sistema.
fn kind_icon(kind: shuma_line::FileKind) -> llimphi_icons::Icon {
    use llimphi_icons::Icon;
    use shuma_line::FileKind as K;
    match kind {
        K::Folder => Icon::Folder,
        K::Symlink => Icon::Link,
        K::Image => Icon::Image,
        K::Audio => Icon::Music,
        K::Video => Icon::Film,
        K::Archive => Icon::Archive,
        K::Document => Icon::FileText,
        K::Code => Icon::Code,
        K::Data => Icon::Code,
        K::Font => Icon::Font,
        K::Executable => Icon::Settings,
        K::Generic => Icon::File,
    }
}

pub(crate) fn build_span_children<HostMsg: Clone + 'static>(
    text: &str,
    decorations: &[shuma_line::Decoration],
    base: llimphi_ui::llimphi_raster::peniko::Color,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Vec<View<HostMsg>> {
    use shuma_line::DecorationKind as Dk;
    let pieces = partition_line(text, decorations, base, theme);
    let mut out: Vec<View<HostMsg>> = Vec::with_capacity(pieces.len());
    for p in pieces {
        if p.text.is_empty() {
            continue;
        }
        let actionable = matches!(
            p.deco,
            Some(Dk::Path { .. } | Dk::Url(_) | Dk::GrepRef { .. } | Dk::GitSha(_))
        );
        // Texto del span. Para paths le anteponemos un icono vectorial por
        // tipo (no emoji): así un `ls` se lee como un explorador de
        // archivos (carpeta/imagen/código/…) sin depender de fuentes de
        // emoji del sistema.
        let text_view: View<HostMsg> = View::new(Style {
            ..Default::default()
        })
        .text_aligned(p.text.clone(), 12.0, p.color, Alignment::Start);
        let mut span_view: View<HostMsg> = match &p.deco {
            Some(Dk::Path {
                abs,
                is_dir,
                is_executable,
                is_symlink,
            }) => {
                let kind = shuma_line::file_kind(abs, *is_dir, *is_executable, *is_symlink);
                let icon_box: View<HostMsg> = View::new(Style {
                    size: Size {
                        width: length(13.0_f32),
                        height: length(13.0_f32),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .children(vec![llimphi_icons::icon_view(
                    kind_icon(kind),
                    p.color,
                    1.6,
                )]);
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    align_items: Some(AlignItems::Center),
                    gap: Size {
                        width: length(5.0_f32),
                        height: length(0.0_f32),
                    },
                    ..Default::default()
                })
                .children(vec![icon_box, text_view])
            }
            _ => text_view,
        };
        if let (true, Some(kind)) = (actionable, p.deco) {
            let l = lift.clone();
            // Feedback de hover: el span se resalta al pasar el cursor —
            // un `ls` se siente como un explorador donde cada archivo
            // "responde". (Llimphi no expone cursor-icon del SO; el
            // realce es el afford idiomático, igual que en tree/button.)
            span_view = span_view
                .radius(3.0)
                .hover_fill(theme.bg_row_hover)
                .on_click(l(Msg::OpenDecoration(kind)));
        }
        out.push(span_view);
    }
    out
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
