use super::*;
use llimphi_ui::llimphi_layout::taffy::prelude::auto;
use llimphi_ui::llimphi_layout::taffy::style::Position;

/// Vista pública del **input vivo** del shell, aislado del resto del shell. Lo
/// usan los frontends que quieren hospedar la línea de entrada en su propio
/// chasis (p. ej. la barra de pata: el cabezal de la barra ES este input, no un
/// placeholder). Comparte estado con [`body_view`] — los dos pintan distintas
/// partes del mismo `State` y se enrutan los `Msg` por el mismo `lift`.
pub fn input_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    shell_input_view(state, theme, lift)
}

/// Vista pública del **cuerpo** del shell sin el input: header + panel
/// principal (cards/PTY/TUI) + popups internos (completado, búsqueda de
/// historial, menú contextual). La usa pata para el drawer mientras el input
/// real vive en la barra.
pub fn body_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = shell_header(state, theme);
    let main_panel: View<HostMsg> = if is_tui_fullscreen(state) {
        tui_panel::<HostMsg>(state, theme, lift.clone())
    } else if is_tui_active(state) {
        pty_lines_panel::<HostMsg>(state, theme)
    } else if terminal_surface_enabled() {
        output_pane_surface::<HostMsg>(state, theme, &lift)
    } else {
        output_pane::<HostMsg>(state, theme, &lift)
    };
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

    let mut children = vec![header, body];
    if state.history_search.is_some() {
        children.push(history_search_panel::<HostMsg>(state, theme));
    }
    if let Some(menu) = body_context_menu::<HostMsg>(state, theme, &lift) {
        children.push(menu);
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

pub fn view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = shell_header(state, theme);
    // Render según la señal dura de alt-screen: pantalla completa (grid/vim)
    // sólo si el PTY entró a alternate screen; un PTY en modo líneas (p. ej.
    // `watch`) se lee como IDE-text; sin PTY, las cards de comandos.
    let main_panel: View<HostMsg> = if is_tui_fullscreen(state) {
        tui_panel::<HostMsg>(state, theme, lift.clone())
    } else if is_tui_active(state) {
        pty_lines_panel::<HostMsg>(state, theme)
    } else if terminal_surface_enabled() {
        // Experimental, detrás de SHUMA_TERMINAL_SURFACE (A/B con el viejo).
        output_pane_surface::<HostMsg>(state, theme, &lift)
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
                format!("reprocesando la salida del bloque #{src} — Enter ejecuta · click cancela"),
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
    // Menú contextual del output (click derecho): overlay por encima de todo,
    // sin clip — por eso va último en los children del root. Sus coords son del
    // nodo raíz (este mismo), así que el `anchor` cae donde se hizo click.
    if let Some(menu) = body_context_menu::<HostMsg>(state, theme, &lift) {
        children.push(menu);
    }

    let lift_menu = lift.clone();
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
    // Click derecho en cualquier parte del output → menú contextual en `(x, y)`
    // (coords locales a este nodo raíz). El cuerpo IDE ya no captura el right-
    // click (lo delega acá) para que el menú gane.
    .on_right_click_at(move |x, y, _w, _h| Some(lift_menu(Msg::OpenBodyMenu { x, y })))
    .children(children)
}

/// Menú contextual del output (click derecho): Copiar selección · Copiar todo ·
/// Seleccionar todo. `None` si no está abierto. Las acciones operan sobre el
/// bloque guardado en `state.body_menu`. "Copiar" se deshabilita sin selección.
pub(crate) fn body_context_menu<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    use llimphi_widget_context_menu::{
        context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
    };
    let (x, y, block) = state.body_menu?;
    let mut copiar = ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C");
    if !menu_has_selection(state, block) {
        copiar = copiar.disabled();
    }
    let items = vec![
        copiar,
        ContextMenuItem::action("Copiar todo"),
        ContextMenuItem::action("Seleccionar todo"),
    ];
    let lift_pick = lift.clone();
    let menu = context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: (1280.0, 800.0),
        header: None,
        items,
        active: usize::MAX,
        on_pick: std::sync::Arc::new(move |i| lift_pick(Msg::BodyMenuPick(i))),
        on_dismiss: lift(Msg::BodyMenuDismiss),
        palette: ContextMenuPalette::from_theme(theme),
    });
    // El menú (con su scrim full-screen) está hecho para `view_overlay`; acá lo
    // hospedamos en el flujo del shell, así que lo sacamos del layout flex con
    // un contenedor `Position::Absolute` (si no, el scrim aplasta el output).
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0_f32),
                top: length(0.0_f32),
                right: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![menu]),
    )
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
    let zoom = state.font_zoom.clamp(0.5, 3.0) as f64;
    let font_px = 13.0_f64 * zoom;
    let line_h: f64 = 18.0_f64 * zoom;
    let border_inner_h: f64 = 16.0_f64 * zoom; // padding visual sumado al alto
    let container_h = border_inner_h + line_h * line_count as f64;
    let theme_clone = *theme;
    let focused = state.focused;
    // Parpadeo del caret: sólido el primer medio período tras la última tecla,
    // luego on/off cada ~530 ms. Se computa contra el reloj en el painter (el
    // chasis redibuja cada 100 ms, así que titila suave).
    let edit_anchor = state.input_edit_at_ms;
    let caret_on = {
        let now = now_unix_millis();
        let elapsed = now.saturating_sub(edit_anchor);
        (elapsed / 530) % 2 == 0
    };
    // Rango de selección del input (bytes), para pintar el realce.
    let sel_range = state.input.selection();

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
                size_px: font_px as f32,
                color: theme_clone.fg_placeholder,
                origin: (line_x_start, baseline_y),
                max_width: None,
                alignment: TAlign::Start,
                line_height: 1.2,
                italic: false,
                font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
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

        // Ancho de carácter de la fuente mono: medimos un bloque de N
        // caracteres y dividimos. Avanzamos el cursor y los tokens por
        // **conteo de caracteres**, NO por medición de cada token: parley
        // colapsa el ancho de un token que es sólo espacio(s) a 0 (trailing
        // whitespace), por eso antes el espacio "no se veía" y `echo hola`
        // se pintaba `echohola`. Con ancho fijo esto además alinea exacto.
        let char_w: f64 = {
            let probe = TextBlock {
                text: "0000000000",
                size_px: font_px as f32,
                color: theme_clone.fg_text,
                origin: (0.0, 0.0),
                max_width: None,
                alignment: TAlign::Start,
                line_height: 1.2,
                italic: false,
                font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
            };
            (measurement(&layout_block(ts, &probe)).width as f64 / 10.0).max(1.0)
        };

        let mut cursor_x: f64 = line_x_start;
        let mut cursor_y: f64 = baseline_y;
        let mut last_line_end_x: f64 = line_x_start;
        let mut last_line_y: f64 = baseline_y;
        let mut line_byte_start = 0usize;
        for (line_idx, line_str) in text.split('\n').enumerate() {
            let line_y = baseline_y + line_idx as f64 * line_h;
            let mut x = line_x_start;
            // Pintar tokens sobre el slice de la línea, usando el
            // tokenizer estándar (dialect por defecto = bash).
            let tokens = shuma_line::tokenize(line_str, state_dialect_default());
            for tok in &tokens {
                let color = token_color(tok.kind, &theme_clone);
                let segment = &line_str[tok.start..tok.end];
                let block = TextBlock {
                    text: segment,
                    size_px: font_px as f32,
                    color,
                    origin: (x, line_y),
                    max_width: None,
                    alignment: TAlign::Start,
                    line_height: 1.2,
                    italic: false,
                    font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
                };
                let layout = layout_block(ts, &block);
                draw_layout(scene, &layout, color, (x, line_y));
                if line_idx == cursor_line_idx
                    && tok.start < cursor_byte_in_line
                    && cursor_byte_in_line <= tok.end
                {
                    let prefix = &line_str[tok.start..cursor_byte_in_line];
                    cursor_x = x + prefix.chars().count() as f64 * char_w;
                    cursor_y = line_y;
                }
                // Avance mono por conteo de caracteres (incluye espacios).
                x += segment.chars().count() as f64 * char_w;
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
                    size_px: font_px as f32,
                    color: theme_clone.fg_placeholder,
                    origin: (last_line_end_x, last_line_y),
                    max_width: None,
                    alignment: TAlign::Start,
                    line_height: 1.2,
                    italic: false,
                    font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
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

        // Realce de selección (caso single-line, que es el típico del input).
        // La fuente es mono, así que medir prefijos como bloque coincide con
        // el render por-token.
        if let Some((ss, se)) = sel_range {
            if !text.contains('\n') && se <= text.len() {
                use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
                use llimphi_ui::llimphi_raster::peniko::Fill;
                let measure_w = |ts: &mut llimphi_ui::llimphi_text::Typesetter, upto: usize| -> f64 {
                    if upto == 0 {
                        return 0.0;
                    }
                    let blk = TextBlock {
                        text: &text[..upto],
                        size_px: font_px as f32,
                        color: theme_clone.fg_text,
                        origin: (0.0, 0.0),
                        max_width: None,
                        alignment: TAlign::Start,
                        line_height: 1.2,
                        italic: false,
                        font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
                    };
                    measurement(&layout_block(ts, &blk)).width as f64
                };
                let x0 = line_x_start + measure_w(ts, ss);
                let x1 = line_x_start + measure_w(ts, se);
                let rect = KurboRect::new(x0, baseline_y, x1, baseline_y + line_h);
                let a = theme_clone.bg_selected;
                let sel_color = Color::from_rgba8(
                    (a.components[0] * 255.0) as u8,
                    (a.components[1] * 255.0) as u8,
                    (a.components[2] * 255.0) as u8,
                    150,
                );
                scene.fill(
                    Fill::NonZero,
                    vello::kurbo::Affine::IDENTITY,
                    sel_color,
                    None,
                    &rect,
                );
            }
        }

        // Cursor — barra vertical de 2 px en la línea calculada, parpadeante.
        if focused && caret_on {
            use llimphi_ui::llimphi_raster::kurbo::Rect as KurboRect;
            use llimphi_ui::llimphi_raster::peniko::Fill;
            // Caret un poco más ancho (2.5px) y en el acento para que se note.
            let cursor_rect =
                KurboRect::new(cursor_x, cursor_y + 1.0, cursor_x + 2.5, cursor_y + line_h);
            scene.fill(
                Fill::NonZero,
                vello::kurbo::Affine::IDENTITY,
                theme_clone.accent,
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
        // El input nunca se reduce: cuando la ventana se achica, el
        // body (con flex_grow=1) se shrinkea hasta `min_size.height=0`
        // y el input mantiene su `container_h`. Sin esto, taffy podía
        // shrink el input a 0 y se "perdía".
        flex_shrink: 0.0,
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

/// Empaca un `peniko::Color` a un u32 RGBA8 little-endian listo para el
/// `CellInstance` del pipeline GPU (Fase 4 del SDD-TERMINAL). Espeja
/// `llimphi_widget_terminal::pack_rgba` pero parte del color del runtime
/// (componentes f32 0..1).
pub(crate) fn pack_peniko(c: llimphi_ui::llimphi_raster::peniko::Color) -> u32 {
    let r = (c.components[0].clamp(0.0, 1.0) * 255.0) as u8;
    let g = (c.components[1].clamp(0.0, 1.0) * 255.0) as u8;
    let b = (c.components[2].clamp(0.0, 1.0) * 255.0) as u8;
    let a = (c.components[3].clamp(0.0, 1.0) * 255.0) as u8;
    llimphi_widget_terminal::pack_rgba(r, g, b, a)
}

/// Construye las `CellInstance`s a dibujar para un snapshot vt100 sobre el
/// rect del panel del TUI (Fase 4 del SDD-TERMINAL). Itera fila×col, mira
/// el char + colores fg/bg, rasteriza el glifo si todavía no está en el
/// atlas y arma un instance por celda. Las celdas con char vacío o sólo
/// espacio Y bg default se saltan (el fondo del panel cubre).
///
/// `render_cell_w`/`render_cell_h` son el tamaño de celda en el viewport
/// (deriva del rect / cols×rows); pueden diferir del cell size natural del
/// atlas — la diferencia se absorbe en el shader (el sampler lineal estira
/// el glifo al cell de salida).
pub(crate) fn build_cell_instances(
    snap: &TuiSnapshot,
    atlas: &mut llimphi_widget_terminal::GlyphAtlas,
    theme: Theme,
    rect: llimphi_ui::PaintRect,
) -> Vec<llimphi_widget_terminal::CellInstance> {
    use llimphi_widget_terminal::CellInstance;
    if snap.rows == 0 || snap.cols == 0 {
        return Vec::new();
    }
    let pad = 6.0_f32;
    let avail_w = (rect.w - pad * 2.0).max(0.0);
    let avail_h = (rect.h - pad * 2.0).max(0.0);
    let render_cell_w = (avail_w / snap.cols as f32).max(1.0);
    let render_cell_h = (avail_h / snap.rows as f32).max(1.0);
    let origin_x = rect.x + pad;
    let origin_y = rect.y + pad;
    let (atlas_cell_w, atlas_cell_h) = atlas.cell_size();

    let mut out: Vec<CellInstance> = Vec::with_capacity((snap.rows * snap.cols) as usize);
    for (r, row) in snap.cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let bg = vt_color(cell.bg, theme, true);
            let fg = vt_color(cell.fg, theme, false);
            let ch = cell.ch.chars().next().unwrap_or(' ');
            let is_blank = ch == ' ' || ch == '\0';
            // Salta celdas vacías con fondo default — el panel ya pinta su
            // bg, no hay nada que cubrir ni que pintar.
            if is_blank && bg.components[3] <= 0.001 {
                continue;
            }
            // Pide el slot del glifo. Si el atlas está lleno, intenta
            // crecer una vez; si tampoco entra (raro), salta el char.
            let slot = match atlas.glyph_for(ch) {
                Some(s) => s,
                None => {
                    atlas.grow();
                    match atlas.glyph_for(ch) {
                        Some(s) => s,
                        None => continue,
                    }
                }
            };
            out.push(CellInstance {
                cell_x: origin_x + c as f32 * render_cell_w,
                cell_y: origin_y + r as f32 * render_cell_h,
                uv_x: slot.px as f32,
                uv_y: slot.py as f32,
                uv_w: atlas_cell_w as f32,
                uv_h: atlas_cell_h as f32,
                fg_rgba: pack_peniko(fg),
                bg_rgba: pack_peniko(bg),
            });
        }
    }
    out
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
        // try_lock: si el lector del PTY está dentro del mutex (drenando una
        // ráfaga grande de output), no bloqueamos el render — el header pinta
        // un placeholder vivo (`· ⟳ …`) y el comando real reaparece en el
        // siguiente frame. Antes el lock duro pasmaba la pantalla en negro
        // mientras el PTY drenaba.
        let cmd = match arc.try_lock() {
            Ok(g) => g.command.clone(),
            Err(_) => "…".to_string(),
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

/// Duración del fade de colapso/despliegue de los bloques del output.
pub(crate) const COLLAPSE_ANIM: std::time::Duration = std::time::Duration::from_millis(160);

/// Sobre cuántos comandos hacia atrás se difumina el negro de recencia: el
/// más reciente es negro profundo, y al cabo de `RECENCY_FADE` comandos el
/// fondo llega al tono normal de card.
pub(crate) const RECENCY_FADE: f32 = 6.0;

/// Mezcla lineal de dos colores sRGB (`t=0` → `a`, `t=1` → `b`).
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

/// Fondo de una card según su `depth` de recencia (0 = más reciente, negro
/// profundo; 1 = viejo, tono normal `bg_panel_alt`).
pub(crate) fn recency_base(theme: &Theme, depth: f32) -> llimphi_ui::llimphi_raster::peniko::Color {
    // Negro profundo derivado del tema (canal × 0.28) — para temas oscuros
    // queda casi negro; para claros, un gris hundido.
    let alt = theme.bg_panel_alt.components;
    use llimphi_ui::llimphi_raster::peniko::Color;
    let deep = Color::from_rgba8(
        (alt[0] * 0.28 * 255.0).round() as u8,
        (alt[1] * 0.28 * 255.0).round() as u8,
        (alt[2] * 0.28 * 255.0).round() as u8,
        255,
    );
    mix_color(deep, theme.bg_panel_alt, depth)
}

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

    // Bloque-comando más reciente visible → ancla del gradiente de recencia:
    // el último es negro profundo, los de más arriba menos negros.
    let newest_cmd = order
        .iter()
        .copied()
        .filter(|id| {
            groups
                .get(id)
                .and_then(|g| g.first())
                .map(|l| l.kind == OutputKind::Prompt)
                .unwrap_or(false)
        })
        .max()
        .unwrap_or(0);

    // Cada item lleva su alto exacto → `content_h` para el scroll.
    let mut items: Vec<(View<HostMsg>, f32)> = Vec::new();
    for id in &order {
        let g = &groups[id];
        // Un bloque REAL (id != 0) va siempre a `command_card` (cuerpo IDE con
        // select/copy/numeración), aunque su línea Prompt se haya recortado del
        // buffer por el tope (output gigante tipo `ls -alR`): antes caía a
        // `render_output_line` (líneas planas, sin IDE). Sólo `id == 0` (líneas
        // huérfanas sin comando dueño) sigue como líneas sueltas. (El render
        // plano que el usuario NO quiere ver — la app existe para desplanar.)
        if *id != 0 {
            // depth 0 = el más reciente (negro profundo); crece hacia atrás.
            let depth = if newest_cmd > 0 {
                (newest_cmd.saturating_sub(*id) as f32 / RECENCY_FADE).clamp(0.0, 1.0)
            } else {
                0.0
            };
            items.push(command_card::<HostMsg>(
                g.as_slice(),
                *id,
                depth,
                state,
                theme,
                lift,
            ));
        } else {
            // Líneas sueltas (notices iniciales sin bloque dueño).
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

    // Barra de scroll arrastrable, sobre la geometría canónica de
    // `llimphi-widget-scroll` (su `thumb_geometry` es público justo para
    // callers que pintan su propia barra dentro de su layout). Sólo cuando
    // hay overflow y ya medimos el viewport. Da el eje "arrastre" del scroll
    // (la rueda ya entra por `on_wheel` del chasis) + indicador visible.
    let mut pane_children = vec![inner];
    if overflow > 0.5 && viewport_h > 1.0 {
        // `scroll_px` mide px desde el fondo; `thumb_geometry` quiere offset
        // desde el tope. offset_top=0 (thumb arriba) ⇔ scroll_px=overflow.
        let offset_top = overflow - state.scroll_px.clamp(0.0, overflow);
        let (thumb_h, thumb_y, offset_per_px) =
            llimphi_widget_scroll::thumb_geometry(offset_top, content_h, viewport_h);
        let pal = llimphi_widget_scroll::ScrollPalette::from_theme(theme);
        let bar_w = pal.bar_width;
        // Track tenue de fondo, a lo alto del viewport.
        pane_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: auto(),
                    right: length(1.0_f32),
                    top: length(0.0_f32),
                    bottom: auto(),
                },
                size: Size {
                    width: length(bar_w),
                    height: length(viewport_h),
                },
                ..Default::default()
            })
            .fill(pal.track)
            .radius((bar_w / 2.0) as f64),
        );
        // Thumb arrastrable. Arrastrar hacia abajo (dy>0) lleva al fondo:
        // el offset-desde-el-tope sube, así que `scroll_px` (desde el fondo)
        // baja → `Scroll(-dy * offset_per_px)`.
        let lift_drag = (*lift).clone();
        pane_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: auto(),
                    right: length(1.0_f32),
                    top: length(thumb_y),
                    bottom: auto(),
                },
                size: Size {
                    width: length(bar_w),
                    height: length(thumb_h),
                },
                ..Default::default()
            })
            .fill(pal.thumb)
            .hover_fill(pal.thumb_hover)
            .radius((bar_w / 2.0) as f64)
            .draggable(move |_phase, _dx, dy| {
                if dy == 0.0 {
                    None
                } else {
                    Some(lift_drag(Msg::Scroll(-dy * offset_per_px)))
                }
            }),
        );
    }

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
    // Superficie hundida (un escalón más profunda que el chrome): el output
    // se lee recesado y con más contraste, como un panel de terminal. Las
    // cards (`bg_panel_alt`) flotan por encima.
    .fill(theme.sunken())
    .radius(3.0)
    .clip(true)
    .paint_with(painter)
    .children(pane_children)
}

// ── Camino experimental: superficie de terminal virtualizada (Fase 2 del SDD) ──
//
// Detrás del flag `SHUMA_TERMINAL_SURFACE` (A/B con el `output_pane` de arriba,
// que queda intacto para rollback inmediato). Mapea el modelo del shell
// (`OutputLine` + bloques + `collapsed` + `block_command`) al modelo de bloques
// de `llimphi-widget-terminal`: cada comando = un header (chrome) + su cuerpo
// (rango de líneas en un `Scrollback`); colapsar = no emitir el cuerpo. El
// scroll del widget vive en la superficie (no en un `transform`), evitando de
// raíz el bug clip+transform; convertimos `scroll_px` (px desde el fondo, el
// modelo del shell) ↔ `scroll_y` (px desde arriba, el del widget).

/// Alto fijo del header de comando en la superficie (px).
const SURFACE_HEADER_H: f32 = 22.0;

/// Alto del header de una sub-sección (sub-collapsable dentro de un block).
/// Un pelo más bajo que `SURFACE_HEADER_H` para destacar la jerarquía.
const SECTION_HEADER_H: f32 = 20.0;

/// Alto del header de columnas de una tabla de sección (px).
const SECTION_TABLE_HEADER_H: f32 = 22.0;
/// Alto de una fila de tabla de sección (px).
const SECTION_TABLE_ROW_H: f32 = 20.0;

/// Cap de filas renderizadas por sección-tabla. Más allá, agregamos una
/// fila final "+N filas …" en lugar de pintar 5000 Views. Cuando el usuario
/// ordena por una columna, la limitación sigue aplicando (ve los top-N
/// según ese orden) — útil para tablas muy gordas tipo `ls -lR /usr`.
pub(crate) const SECTION_TABLE_MAX_ROWS: usize = 200;

/// Alto total de una tabla con `n_rows` filas (capeado por SECTION_TABLE_MAX_ROWS,
/// +1 fila para el mensaje "+N filas …" cuando aplica).
pub(crate) fn section_table_height(n_rows: usize) -> f32 {
    let visible = n_rows.min(SECTION_TABLE_MAX_ROWS);
    let truncado = if n_rows > SECTION_TABLE_MAX_ROWS { 1.0 } else { 0.0 };
    SECTION_TABLE_HEADER_H + (visible as f32 + truncado) * SECTION_TABLE_ROW_H
}

/// Pinta una sub-sección como tabla con headers clickeables (ordenar
/// asc/desc/sin orden) + filas mono striped. Las filas se ordenan según
/// `sort = (col, ascending)`; si `None`, orden natural del output.
pub(crate) fn section_table_view<HostMsg: Clone + 'static>(
    block: u64,
    section: usize,
    columns: &[String],
    rows: &[Vec<String>],
    sort: Option<(usize, bool)>,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    // Anchos heurísticos por columna — mejor sería medir, pero
    // `ls -l` tiene anchos típicos predecibles.
    fn col_width(idx: usize, n: usize, name: &str) -> f32 {
        match name {
            "permisos" => 100.0,
            "links" => 50.0,
            "owner" | "group" => 80.0,
            "size" => 80.0,
            "fecha" => 100.0,
            _ if idx == n - 1 => 0.0, // última = flex
            _ => 90.0,
        }
    }
    let n = columns.len();
    // Header row.
    let mut header_children: Vec<View<HostMsg>> = Vec::with_capacity(n);
    for (col, name) in columns.iter().enumerate() {
        let arrow = match sort {
            Some((c, true)) if c == col => " ▲",
            Some((c, false)) if c == col => " ▼",
            _ => "",
        };
        let w = col_width(col, n, name);
        let mut style = Style {
            size: Size { width: length(w.max(40.0)), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        };
        if w == 0.0 {
            style.size.width = Dimension::auto();
            style.flex_grow = 1.0;
        }
        header_children.push(
            View::new(style)
                .hover_fill(theme.bg_row_hover)
                .on_click(lift(Msg::SortSectionColumn { block, section, col }))
                .text_aligned(
                    format!("{name}{arrow}"),
                    11.0,
                    theme.fg_placeholder,
                    Alignment::Start,
                )
                .mono(),
        );
    }
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SECTION_TABLE_HEADER_H) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(header_children);

    // Rows. Aplica orden si lo hay (clon para no mutar el state).
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if let Some((col, asc)) = sort {
        order.sort_by(|&a, &b| {
            let ax = rows[a].get(col).map(|s| s.as_str()).unwrap_or("");
            let bx = rows[b].get(col).map(|s| s.as_str()).unwrap_or("");
            // Si parecen números, orden numérico; si no, lexicográfico.
            let cmp = match (ax.parse::<u64>().ok(), bx.parse::<u64>().ok()) {
                (Some(an), Some(bn)) => an.cmp(&bn),
                _ => ax.cmp(bx),
            };
            if asc { cmp } else { cmp.reverse() }
        });
    }
    let total_rows = order.len();
    let visible_rows = total_rows.min(SECTION_TABLE_MAX_ROWS);
    let truncated = total_rows > SECTION_TABLE_MAX_ROWS;
    let mut row_views: Vec<View<HostMsg>> = Vec::with_capacity(visible_rows + 1);
    for (vis_idx, &ri) in order.iter().take(visible_rows).enumerate() {
        let row = &rows[ri];
        let stripe = if vis_idx % 2 == 0 {
            theme.bg_panel
        } else {
            Color::from_rgba8(0, 0, 0, 0) // transparente
        };
        let mut cells: Vec<View<HostMsg>> = Vec::with_capacity(n);
        for col in 0..n {
            let name = columns.get(col).map(|s| s.as_str()).unwrap_or("");
            let w = col_width(col, n, name);
            let mut style = Style {
                size: Size { width: length(w.max(40.0)), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            };
            if w == 0.0 {
                style.size.width = Dimension::auto();
                style.flex_grow = 1.0;
            }
            cells.push(
                View::new(style)
                    .text_aligned(
                        row.get(col).cloned().unwrap_or_default(),
                        11.0,
                        theme.fg_text,
                        Alignment::Start,
                    )
                    .mono()
                    .max_lines(1),
            );
        }
        row_views.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(SECTION_TABLE_ROW_H) },
                ..Default::default()
            })
            .fill(stripe)
            .hover_fill(theme.bg_row_hover)
            .children(cells),
        );
    }
    // Mensaje de truncado: si la tabla tiene más filas que SECTION_TABLE_MAX_ROWS,
    // mostramos una última fila informativa.
    if truncated {
        let extra = total_rows - visible_rows;
        row_views.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(SECTION_TABLE_ROW_H) },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                format!("… +{extra} filas (sort por una columna para acotar)"),
                10.0,
                theme.fg_muted,
                Alignment::Start,
            )
            .mono(),
        );
    }

    let mut all = vec![header];
    all.extend(row_views);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(section_table_height(rows.len())),
        },
        ..Default::default()
    })
    .children(all)
}

/// `true` si la sub-sección `idx` del bloque `block` debe arrancar colapsada
/// por default. Heurística: dirs con profundidad ≥ 2 (al menos un `/`
/// después del primero) — para que `ls -R` en un árbol grande no rinda
/// miles de filas al toque. El usuario togglea con click; el set
/// `section_collapsed` guarda el OVERRIDE del default (no el estado).
pub(crate) fn section_default_collapsed(title: &str) -> bool {
    // `./` y `.` siempre expandidos. `./algo` también (depth 1). `./a/b`
    // ya cierra (depth 2).
    let stripped = title.trim_start_matches("./");
    stripped.matches('/').count() >= 1
}

/// Estado efectivo de plegado de una sub-sección: el default (heurística
/// por profundidad) flippeado por el override del usuario.
pub(crate) fn is_section_collapsed(state: &State, block: u64, idx: usize, title: &str) -> bool {
    let default_col = section_default_collapsed(title);
    let user_toggled = state.section_collapsed.contains(&(block, idx));
    default_col ^ user_toggled
}

/// Header clickeable de una sub-sección. Pinta chevron + título + el conteo
/// de líneas; click emite `Msg::ToggleSection`. `idx` se usa como número
/// visible ("1.", "2.", …) para navegar listas largas.
fn section_header<HostMsg: Clone + 'static>(
    block: u64,
    idx: usize,
    title: &str,
    line_count: usize,
    collapsed: bool,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let chevron = if collapsed {
        llimphi_icons::Icon::ChevronRight
    } else {
        llimphi_icons::Icon::ChevronDown
    };
    let marker = View::new(Style {
        size: Size { width: length(12.0_f32), height: length(12.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![llimphi_icons::icon_view(chevron, theme.fg_muted, 1.6)]);
    let title_v = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(14.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{}. {}", idx + 1, title),
        11.0,
        theme.fg_text,
        Alignment::Start,
    )
    .mono()
    .max_lines(1);
    let count = View::new(Style {
        size: Size { width: length(60.0_f32), height: length(14.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{line_count} líneas"),
        10.0,
        theme.fg_muted,
        Alignment::End,
    )
    .mono();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SECTION_HEADER_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(18.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::ToggleSection { block, idx }))
    .children(vec![marker, title_v, count])
}

/// `true` si el output va por la **superficie nueva** (modo por defecto
/// tras Fase 5.6). Se lee una sola vez por proceso vía `OnceLock`.
///
/// Política:
/// - **Default**: superficie ON. Todas las features de Fase 1-5 (store
///   virtualizado, anclaje estable, selección + copy + find, scroll
///   inercial, GPU grid opt-in, spill a disco opt-in) están activas.
/// - **Opt-OUT explícito**: setear `SHUMA_TERMINAL_LEGACY=1` (o cualquier
///   valor) vuelve al `output_pane` viejo + per-command IDE editor (rama
///   con `BodyPointer`, `BodyDoubleClick`, multi-cursor del text-editor).
///   Es el botón de pánico para usuarios que necesiten esas features
///   específicas hasta cerrar paridad.
/// - **Opt-IN histórico**: `SHUMA_TERMINAL_SURFACE=1` (sin LEGACY) sigue
///   funcionando como antes — no-op si ya es default.
pub(crate) fn terminal_surface_enabled() -> bool {
    use std::sync::OnceLock;
    static EN: OnceLock<bool> = OnceLock::new();
    *EN.get_or_init(|| std::env::var_os("SHUMA_TERMINAL_LEGACY").is_none())
}

/// Header de un comando como **chrome** de la superficie: chevron + `$ comando`
/// + badge de estado (icono + "hace N"). Click → pliega/despliega el bloque.
/// Chrome header del bloque de líneas spilleadas: rotula "Archivado de
/// spill (N visibles · M total)" y avisa al usuario que el resto se ve
/// con `:scrollback open`. Sin click handler (informativo).
fn spilled_archive_header<HostMsg: Clone + 'static>(
    visible: usize,
    total: usize,
    theme: &Theme,
) -> View<HostMsg> {
    let label = if total > visible {
        format!(
            "≡ Archivado de spill ({visible} visibles · {total} total · `:scrollback open` para todo)"
        )
    } else {
        format!("≡ Archivado de spill ({total} líneas)")
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SURFACE_HEADER_H) },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(label, 11.0, theme.fg_muted, Alignment::Start)
    .mono()
}

/// `has_stdout` (param 6) gatea el chip de reprocess (sin stdout, no hay
/// nada que reprocesar).
fn surface_header<HostMsg: Clone + 'static>(
    block: u64,
    header_text: &str,
    status: Option<CmdStatus>,
    expandable: bool,
    collapsed: bool,
    has_stdout: bool,
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let chevron = if collapsed {
        llimphi_icons::Icon::ChevronRight
    } else {
        llimphi_icons::Icon::ChevronDown
    };
    let marker = View::new(Style {
        size: Size { width: length(14.0_f32), height: length(14.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(if expandable {
        vec![llimphi_icons::icon_view(chevron, theme.fg_muted, 1.6)]
    } else {
        Vec::new()
    });

    let cmd_color = if expandable || status == Some(CmdStatus::Running) {
        theme.accent
    } else {
        theme.fg_muted
    };
    let cmd = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(16.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(header_text.to_string(), 12.0, cmd_color, Alignment::Start)
    .mono()
    .max_lines(1);

    let mut children = vec![marker, cmd];
    // Chip de reprocess: alimenta el stdout de este bloque al stdin del
    // próximo comando (paridad con el `command_card` del path viejo). Clic
    // arma/desarma; el hit-test innermost-wins le da prioridad sobre el
    // header (que pliega el bloque).
    if has_stdout {
        let armed = state.reprocess_source == Some(block);
        let (fill, fg) = if armed {
            (theme.accent, theme.bg_panel)
        } else {
            (theme.bg_input, theme.fg_muted)
        };
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
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
            .text_aligned("» stdin".to_string(), 10.0, fg, Alignment::Start)
            .mono(),
        );
    }
    // Chip "copiar": copia el bloque entero (comando + stdout + stderr) al
    // clipboard, sin depender de una selección — paridad con el "copy command
    // + output" de las terminales modernas. Sólo en bloques con cuerpo. Click
    // propio (innermost-wins) para no plegar el bloque.
    if expandable {
        children.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_shrink: 0.0,
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
            .on_click(lift(Msg::CopyCommandBlock(block)))
            .text_aligned("copiar".to_string(), 10.0, theme.fg_muted, Alignment::Start)
            .mono(),
        );
    }
    if let Some(st) = status {
        let (icon, color) = st.icon_color(theme);
        children.push(
            View::new(Style {
                size: Size { width: length(12.0_f32), height: length(12.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![llimphi_icons::icon_view(icon, color, 1.8)]),
        );
        // Mientras corre, mostrar bytes recibidos en vivo en el slot del
        // timestamp — feedback inmediato de que el stream está moviendo
        // datos (más útil que "hace 0 s"). Al terminar, vuelve al "hace…".
        let right_text = if st == CmdStatus::Running && state.current_block == block {
            format_bytes_short(state.current_run_bytes)
        } else {
            relative_time(
                state.block_started.get(&block).copied().unwrap_or(0),
                now_unix_secs(),
            )
        };
        children.push(
            View::new(Style {
                size: Size { width: length(96.0_f32), height: length(16.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .text_aligned(right_text, 10.0, theme.fg_muted, Alignment::End)
            .mono(),
        );
    }

    let mut v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(SURFACE_HEADER_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel);
    if expandable {
        v = v.on_click(lift(Msg::ToggleBlock(block)));
    }
    v.children(children)
}

/// `output_pane` reimplementado sobre `llimphi-widget-terminal::block_surface`.
/// Mismo modelo de datos, virtualización real (sólo se materializa lo visible).
pub(crate) fn output_pane_surface<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    use llimphi_widget_terminal::{
        blocks_height, Item, LineStyle, Scrollback, TermMetrics, TermPalette,
    };

    // Agrupar por bloque preservando el orden de primera aparición (igual que
    // el camino viejo). Con superficie no hace falta capar a 400: el widget
    // virtualiza, así que pasamos todo el buffer vigente.
    let mut order: Vec<u64> = Vec::new();
    let mut groups: std::collections::HashMap<u64, Vec<&OutputLine>> =
        std::collections::HashMap::new();
    for line in &state.output {
        if !groups.contains_key(&line.block) {
            order.push(line.block);
        }
        groups.entry(line.block).or_default().push(line);
    }

    // Zoom multiplica font_size, row_h y char_width. Lo controla
    // Ctrl+rueda y Ctrl+- / Ctrl+= / Ctrl+0. Clampeo [0.5, 3.0].
    let zoom = state.font_zoom.clamp(0.5, 3.0);
    let row_h = ROW_H * zoom;
    let metrics = TermMetrics {
        font_size: 12.0 * zoom,
        line_height: row_h,
        char_width: 12.0 * 0.6 * zoom,
    };
    let mut palette = TermPalette::from_theme(theme);
    // La superficie entera es el panel hundido; los cuerpos se leen sobre él.
    palette.bg = theme.sunken();

    // Refresh del cache de spilled visibles (Fase 5.11): lee desde el spill
    // file sólo si `spilled_count` cambió desde el último frame. El cache
    // es up-to-`MAX_SPILLED_VISIBLE` líneas; el view las prepende al store.
    crate::refresh_surf_spilled_visible(&state.surf_history, &state.surf_spilled_visible);
    let spilled_cache_lines: Vec<String> = state
        .surf_spilled_visible
        .lock()
        .map(|c| c.lines.clone())
        .unwrap_or_default();
    let total_spilled = state
        .surf_history
        .lock()
        .map(|h| h.spilled_count())
        .unwrap_or(0);

    // Store de scrollback + items + estilo por línea (alineado al índice del
    // store, que crece en lockstep con `push_line`).
    let mut store = Scrollback::new(0);
    let mut items: Vec<Item<HostMsg>> = Vec::new();
    let mut styles: Vec<(bool, Vec<(usize, usize, llimphi_ui::llimphi_raster::peniko::Color)>)> =
        Vec::new();

    // Prepend de las líneas spilleadas: arrancan en `store[0..]`. Tinte
    // discreto (fg_muted) para marcarlas visualmente como archive y un
    // chrome header antes con cuántas hay en total. Si el spill tiene más
    // que `MAX_SPILLED_VISIBLE`, el header lo avisa (el usuario abre el
    // resto con `:scrollback open`).
    if !spilled_cache_lines.is_empty() {
        items.push(Item::chrome(
            SURFACE_HEADER_H,
            spilled_archive_header::<HostMsg>(
                spilled_cache_lines.len(),
                total_spilled,
                theme,
            ),
        ));
        let start = store.len();
        for text in &spilled_cache_lines {
            // Las spilled van en `fg_muted` para diferenciarlas del live.
            let muted = theme.fg_muted;
            styles.push((false, vec![(0usize, text.len(), muted)]));
            store.push_line(text);
        }
        items.push(Item::lines(start, store.len()));
    }

    for id in &order {
        let g = &groups[id];
        if *id != 0 {
            // Bloque-comando: header (chrome) + cuerpo (si no está colapsado).
            let collapsed = state.collapsed.contains(id);
            let has_prompt = g
                .first()
                .map(|l| l.kind == OutputKind::Prompt)
                .unwrap_or(false);
            let header_text = if has_prompt {
                g[0].text.clone()
            } else {
                state
                    .block_command
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "$ … (salida recortada)".to_string())
            };
            // Estado: última notice de cierre del bloque, o "corriendo".
            let mut status = g
                .iter()
                .filter(|l| l.stage.is_none())
                .filter_map(|l| CmdStatus::from_notice(&l.text))
                .last();
            let still_running = status.is_none()
                && ((state.current_block == *id && state.is_running())
                    || state.bg_jobs.iter().any(|j| {
                        j.lock()
                            .map(|gg| gg.block == *id && !gg.handle.is_finished())
                            .unwrap_or(false)
                    }));
            if still_running {
                status = Some(CmdStatus::Running);
            }

            let lines = body_lines_for_block(state, *id);
            let kinds = body_kinds_for_block(state, *id);
            let runs = body_color_runs(state, *id, theme);
            let has_stages = g.iter().any(|l| l.stage.is_some());
            let has_stdout = g
                .iter()
                .any(|l| l.kind == OutputKind::Stdout && l.stage.is_none());
            let expandable = !lines.is_empty() || has_stages;

            items.push(Item::chrome(
                SURFACE_HEADER_H,
                surface_header(
                    *id,
                    &header_text,
                    status,
                    expandable,
                    collapsed,
                    has_stdout,
                    state,
                    theme,
                    lift,
                ),
            ));

            // Chrome de etapas (tee): chips clickeables + capturas desplegadas
            // por etapa. Paridad con el `command_card` viejo. Vacío si el
            // bloque no tiene etapas o si está colapsado. Reusa el helper del
            // path viejo (`stage_capture_rows`) y lo envuelve como un chrome
            // de alto medido por el helper, opaco para la virtualización.
            if !collapsed && has_stages {
                let stage_lines: Vec<&OutputLine> =
                    g.iter().filter(|l| l.stage.is_some()).copied().collect();
                let (views, h) =
                    stage_capture_rows(&header_text, &stage_lines, *id, state, theme, lift);
                if !views.is_empty() && h > 0.0 {
                    let chrome_view = View::new(Style {
                        flex_direction: FlexDirection::Column,
                        size: Size { width: percent(1.0_f32), height: length(h) },
                        ..Default::default()
                    })
                    .children(views);
                    items.push(Item::chrome(h, chrome_view));
                }
            }

            if !collapsed && !lines.is_empty() {
                // Detector de sub-secciones por comando: si reconoce el
                // patrón (p. ej. `ls -R`), parte el output en grupos con
                // su propio header colapsable.
                let cmd_for_sections = state
                    .block_command
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| header_text.clone());
                if let Some(sections) =
                    crate::sections::detect_sections(&cmd_for_sections, &lines)
                {
                    for (sidx, sec) in sections.iter().enumerate() {
                        let sec_col = is_section_collapsed(state, *id, sidx, &sec.title);
                        let item_count = sec.kind.count();
                        // Header (oculto si la sección no tiene título — caso
                        // `ls -l` simple sin árbol).
                        if !sec.title.is_empty() {
                            items.push(Item::chrome(
                                SECTION_HEADER_H,
                                section_header(
                                    *id, sidx, &sec.title, item_count, sec_col, theme, lift,
                                ),
                            ));
                        }
                        if !sec_col {
                            match &sec.kind {
                                crate::sections::SectionKind::Lines(secl) => {
                                    let start = store.len();
                                    for line in secl {
                                        styles.push((false, Vec::new()));
                                        store.push_line(line);
                                    }
                                    items.push(Item::lines(start, store.len()));
                                }
                                crate::sections::SectionKind::Table { columns, rows } => {
                                    let sort = state.section_sort.get(&(*id, sidx)).copied();
                                    let h = section_table_height(rows.len());
                                    items.push(Item::chrome(
                                        h,
                                        section_table_view(
                                            *id, sidx, columns, rows, sort, theme, lift,
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    let start = store.len();
                    for (i, line) in lines.iter().enumerate() {
                        let is_err = matches!(kinds.get(i), Some(OutputKind::Stderr));
                        styles.push((is_err, runs.get(i).cloned().unwrap_or_default()));
                        store.push_line(line);
                    }
                    items.push(Item::lines(start, store.len()));
                }
            }
        } else {
            // Líneas sueltas (notices iniciales sin bloque dueño) — cuerpo sin
            // header, coloreadas por su decoración semántica.
            let start = store.len();
            for &line in g.iter() {
                let is_err = line.kind == OutputKind::Stderr;
                let line_runs: Vec<_> = if is_err {
                    vec![(0usize, line.text.len(), theme.fg_destructive)]
                } else {
                    shuma_line::decorate_line(&line.text, &state.cwd)
                        .into_iter()
                        .filter(|d| d.start < d.end && d.end <= line.text.len())
                        .map(|d| (d.start, d.end, decoration_color(&d.kind, theme)))
                        .collect()
                };
                styles.push((is_err, line_runs));
                store.push_line(&line.text);
            }
            if store.len() > start {
                items.push(Item::lines(start, store.len()));
            }
        }
    }

    // Scroll: convertir el modelo del shell (`scroll_px` desde el fondo) al del
    // widget (`scroll_y` desde arriba). El viewport lo midió el painter el frame
    // anterior; publicamos el overflow para que `Msg::Scroll` clampe.
    let measured = state.out_viewport_h.lock().map(|g| *g).unwrap_or(0.0);
    let content_h = blocks_height(&items, row_h);
    let viewport_h = if measured >= 1.0 { measured } else { 600.0 };
    let overflow = (content_h - viewport_h).max(0.0);
    if let Ok(mut g) = state.out_overflow.lock() {
        *g = overflow;
    }
    // Anclaje estable bajo append (Fase 5 del SDD-TERMINAL): si el usuario
    // está scrolled-up (`scroll_px > 0`), su `scroll_y` se interpreta
    // contra el `surf_scroll_anchor` (el overflow al momento de su última
    // entrada de scroll), NO contra el `overflow` vigente. Append → el
    // overflow crece, pero la fila que el usuario tenía a la vista
    // permanece en la misma `y` del viewport.
    let scroll_y = if state.scroll_px <= 0.5 {
        overflow // pinned al fondo
    } else {
        (state.surf_scroll_anchor - state.scroll_px).clamp(0.0, overflow)
    };

    // Estilo por línea: stderr → tinte rojo tenue; runs ya traen el coloreo
    // semántico (paths/urls/stderr-rojo) calculado arriba.
    let err_bg = {
        let c = theme.fg_destructive.to_rgba8();
        llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(c.r, c.g, c.b, 36)
    };
    let line_style = move |idx: usize, _text: &str| match styles.get(idx) {
        Some((is_err, runs)) => LineStyle {
            fg: None,
            runs: runs.clone(),
            bg: if *is_err { Some(err_bg) } else { None },
        },
        None => LineStyle::default(),
    };

    // La rueda y el arrastre de la barra del widget llegan acá como delta a
    // sumar a `scroll_y` (desde arriba); el shell lo guarda como `scroll_px`
    // (desde el fondo), así que invertimos el signo.
    let lift_scroll = (*lift).clone();
    let on_scroll = move |delta: f32| lift_scroll(Msg::Scroll(-delta));

    use llimphi_widget_terminal::{
        block_surface_with_scroll, gutter_width, SelectionConfig,
    };

    // Snapshot del layout para que el `update` resuelva clicks contra la
    // geometría real del frame anterior, sin re-armar los items.
    let items_geo: Vec<llimphi_widget_terminal::ItemGeo> =
        items.iter().map(|it| it.geo()).collect();
    let gw = gutter_width(&store, metrics);
    if let Ok(mut g) = state.surf_layout.lock() {
        *g = Some(crate::SurfLayout {
            items_geo,
            scroll_y,
            viewport_h,
            metrics,
            gutter_w: gw,
            store: std::sync::Arc::new(store.clone()),
        });
    }

    // Handler de drag de selección: forwardea cada `(phase, lx0, ly0, dx, dy)`
    // del viewport al `update` como `Msg::SurfSelectDrag`. El `update` mantiene
    // el acumulador y resuelve la posición a `Point` con `point_at_geo`.
    let lift_drag = (*lift).clone();
    let on_drag = std::sync::Arc::new(
        move |phase, lx0, ly0, dx, dy| -> Option<HostMsg> {
            Some(lift_drag(Msg::SurfSelectDrag {
                phase,
                dx,
                dy,
                ax: lx0,
                ay: ly0,
            }))
        },
    );
    // Doble-click → select-word, paridad con terminales clásicas.
    let lift_dbl = (*lift).clone();
    let on_double_click = std::sync::Arc::new(
        move |lx, ly, rect_w, rect_h| -> Option<HostMsg> {
            Some(lift_dbl(Msg::SurfDoubleClick {
                lx,
                ly,
                rect_w,
                rect_h,
            }))
        },
    );
    let sel_cfg = SelectionConfig {
        range: state.surf_selection.as_ref(),
        on_drag: Some(on_drag),
        on_double_click: Some(on_double_click),
    };

    let surface = block_surface_with_scroll::<HostMsg, _, _>(
        &store,
        items,
        scroll_y,
        state.surf_scroll_x.max(0.0),
        viewport_h,
        metrics,
        &palette,
        line_style,
        on_scroll,
        None,
        sel_cfg,
    );

    // Nodo flex que toma el espacio sobrante (entre header e input) y mide su
    // alto real para el próximo frame (el widget recibe un alto fijo = el
    // medido; el painter de medición vive acá, en el nodo flex-rellenado).
    let slot = Arc::clone(&state.out_viewport_h);
    let painter = move |_scene: &mut vello::Scene,
                        _ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        if let Ok(mut g) = slot.lock() {
            *g = rect.h;
        }
    };
    let lift_menu = (*lift).clone();
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
        ..Default::default()
    })
    .fill(theme.sunken())
    .radius(3.0)
    .clip(true)
    .paint_with(painter)
    // Right-click sobre el contenedor de la surface abre el menú contextual.
    // El hit-test innermost-wins le da prioridad a hijos con sus propios
    // handlers (p. ej. la barra de find).
    .on_right_click_at(move |x, y, _w, _h| Some(lift_menu(Msg::SurfOpenMenu { x, y })))
    .children({
        // Barra de find encima de la superficie, sólo si está abierta. Es
        // focus-grabbing (la dispatch ya rutea las teclas a `handle_find_key`).
        let mut kids: Vec<View<HostMsg>> = Vec::new();
        if let Some(f) = &state.find {
            kids.push(find_bar_view::<HostMsg>(f, theme, lift));
        }
        // Status del spill: chip que muestra "N líneas archivadas" cuando
        // el history persistente ya recortó al disco. Sólo visible si
        // spill está activo y hay contenido archivado.
        if let Some(status) = spill_status_view::<HostMsg>(state, theme) {
            kids.push(status);
        }
        // Cursor I-beam sobre el cuerpo: señala que el texto es seleccionable
        // (drag selecciona, doble-click la palabra, click derecho el menú). Las
        // decoraciones clickeables (paths/URLs) que traen su propio cursor ganan
        // por hit-test innermost-wins.
        kids.push(surface.cursor(llimphi_ui::Cursor::Text));
        // El menú contextual va como overlay arriba de todo.
        if let Some(menu) = surf_context_menu(state, theme, lift) {
            kids.push(menu);
        }
        kids
    })
}

/// Chip de status del spill del scrollback: "≡ N líneas archivadas en
/// <path>". Sólo aparece si `state.surf_history.spilled_count() > 0`
/// (es decir, el archivo de spill tiene contenido — la sesión llenó el
/// cap en memoria y siguió volcando a disco). `None` mientras esté vacío.
fn spill_status_view<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> Option<View<HostMsg>> {
    let count = state.surf_history.lock().ok().map(|h| h.spilled_count()).unwrap_or(0);
    if count == 0 {
        return None;
    }
    Some(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(2.0_f32),
                bottom: length(2.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text_aligned(
            format!("≡ {count} líneas archivadas en spill"),
            10.0,
            theme.fg_muted,
            Alignment::Start,
        )
        .mono(),
    )
}

/// Menú contextual del surface (click derecho): Copiar selección · Copiar
/// todo · Seleccionar todo. `None` si no está abierto.
pub(crate) fn surf_context_menu<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    use llimphi_widget_context_menu::{
        context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
    };
    let (x, y) = state.surf_menu?;
    let mut copiar = ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C");
    if state.surf_selection.as_ref().map_or(true, |s| s.is_empty()) {
        copiar = copiar.disabled();
    }
    let items = vec![
        copiar,
        ContextMenuItem::action("Copiar todo"),
        ContextMenuItem::action("Seleccionar todo"),
    ];
    let lift_pick = lift.clone();
    let menu = context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: (1280.0, 800.0),
        header: None,
        items,
        active: usize::MAX,
        on_pick: std::sync::Arc::new(move |i| lift_pick(Msg::SurfMenuPick(i))),
        on_dismiss: lift(Msg::SurfMenuDismiss),
        palette: ContextMenuPalette::from_theme(theme),
    });
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0_f32),
                top: length(0.0_f32),
                right: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![menu]),
    )
}

/// Barra de búsqueda Ctrl+F: lupa + query (cursor) + contador `M/N` + chip
/// `Aa` (toggle case) + flechas + ✕. Compacta, encima de la superficie de
/// output. Los clics emiten los `Msg::Find*` ya cableados.
fn find_bar_view<HostMsg: Clone + 'static>(
    f: &crate::FindState,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    let lup = View::new(Style {
        size: Size { width: length(14.0_f32), height: length(14.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::Search,
        theme.fg_muted,
        1.6,
    )]);

    // Query con cursor titilante simulado por sufijo "▏" — paridad simple
    // con el cabezal del shell sin meter blink (innecesario en una barra).
    let mut shown = f.query.clone();
    shown.push('▏');
    let query_view = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(shown, 13.0, theme.fg_text, Alignment::Start)
    .mono();

    // Contador `M/N`. Sin matches: "0/0" muted, sin destacar.
    let total = f.matches.len();
    let cur = f.current.map(|i| i + 1).unwrap_or(0);
    let counter_color = if total == 0 && !f.query.is_empty() {
        theme.fg_destructive
    } else {
        theme.fg_muted
    };
    let counter = View::new(Style {
        size: Size { width: length(54.0_f32), height: length(20.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(format!("{cur}/{total}"), 11.0, counter_color, Alignment::End)
    .mono();

    let case_chip = {
        let (fill, fg) = if f.case_insensitive {
            (theme.accent, theme.bg_panel)
        } else {
            (theme.bg_input, theme.fg_muted)
        };
        View::new(Style {
            size: Size { width: length(24.0_f32), height: length(20.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(fill)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(Msg::FindToggleCase))
        .text_aligned("Aa".to_string(), 11.0, fg, Alignment::Center)
        .mono()
    };

    let arrow = |icon, msg: Msg| {
        View::new(Style {
            size: Size { width: length(20.0_f32), height: length(20.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(msg))
        .children(vec![llimphi_icons::icon_view(icon, theme.fg_muted, 1.6)])
    };
    let prev_btn = arrow(llimphi_icons::Icon::ChevronUp, Msg::FindPrev);
    let next_btn = arrow(llimphi_icons::Icon::ChevronDown, Msg::FindNext);
    let close_btn = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(20.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(3.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::FindClose))
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::X,
        theme.fg_muted,
        1.6,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![lup, query_view, counter, case_chip, prev_btn, next_btn, close_btn])
}

/// Color del badge de estado a partir del texto de la notice de cierre
/// (`✔ exit 0`, `✘ exit N`, `⏹ cancel …`). `None` si la línea no es un
/// estado de cierre — se queda en el cuerpo de la card.
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

/// Reconstruye el `EditorState` read-only del cuerpo de `block` desde su
/// texto + el cursor/selección guardado en `state.body_sel` (si es de este
/// bloque). El buffer es la fuente de verdad (las `OutputLine`); sólo el
/// cursor persiste entre frames. Lo comparten `view` (pintar) y `update`
/// (mapear puntero), así la geometría coincide exacta.
pub(crate) fn body_editor_state(
    state: &State,
    block: u64,
) -> llimphi_widget_text_editor::EditorState {
    let text = body_lines_for_block(state, block).join("\n");
    let mut ed = llimphi_widget_text_editor::EditorState::new();
    ed.set_text(&text);
    if let Some((b, cur)) = &state.body_sel {
        if *b == block {
            ed.cursor = cur.clone();
        }
    }
    ed
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
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::ChevronRight,
        theme.fg_muted,
        1.6,
    )])];

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
    .children(vec![llimphi_icons::icon_view(
        llimphi_icons::Icon::ChevronRight,
        theme.fg_muted,
        1.6,
    )])];

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
                        .text_aligned(l.text.clone(), 12.0, dim, Alignment::Start)
                        .mono()
                        // 1 fila: sin esto una línea de etapa larga wrappea y
                        // pisa la de abajo (la fila es de altura fija ROW_H).
                        .max_lines(1),
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
            .children(vec![bar, col])
            // Desplegar/plegar la captura de la etapa con transición. Key en
            // un namespace propio (etapa) para no chocar con cuerpo/resumen.
            .animated_inout(((block << 8) | (i as u64 & 0xff)) ^ (1 << 62), COLLAPSE_ANIM),
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
    depth: f32,
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> (View<HostMsg>, f32) {
    let collapsed = state.collapsed.contains(&block);
    // El Prompt es `group[0]` salvo que se haya recortado del buffer (output
    // gigante). En ese caso recuperamos el comando del mapa por bloque
    // (`block_command`, poblado al abrir el bloque) y el cuerpo arranca en 0.
    let has_prompt = group
        .first()
        .map(|l| l.kind == OutputKind::Prompt)
        .unwrap_or(false);
    let header_text = if has_prompt {
        group[0].text.clone()
    } else {
        state
            .block_command
            .get(&block)
            .cloned()
            .unwrap_or_else(|| "$ … (salida recortada)".to_string())
    };
    let body_slice: &[&OutputLine] = if has_prompt { &group[1..] } else { group };

    // Separamos la notice de cierre (se promueve a badge), las líneas de
    // etapas intermedias (tee — van a su desplegable) y el resto (cuerpo).
    // Si hay varias notices de cierre, gana la última.
    let mut body: Vec<&OutputLine> = Vec::new();
    let mut stage_lines: Vec<&OutputLine> = Vec::new();
    let mut status: Option<CmdStatus> = None;
    for &l in body_slice {
        if l.stage.is_some() {
            stage_lines.push(l);
        } else if let Some(st) = CmdStatus::from_notice(&l.text) {
            status = Some(st);
        } else {
            body.push(l);
        }
    }
    // Comando aún vivo (sin notice de cierre todavía).
    let still_running = status.is_none()
        && ((state.current_block == block && state.is_running())
            || state.bg_jobs.iter().any(|j| {
                j.lock()
                    .map(|g| g.block == block && !g.handle.is_finished())
                    .unwrap_or(false)
            }));
    if still_running {
        status = Some(CmdStatus::Running);
    }

    let has_body = !body.is_empty();
    let expandable = has_body || !stage_lines.is_empty();
    // Comando terminado sin salida: se muestra distinto (atenuado, sin
    // chevron, no expandible) para no tentar a desplegarlo.
    let no_output = !expandable && status != Some(CmdStatus::Running);

    // ── Marcador de despliegue (chevron por icono, no glifo) ──
    let chevron_icon = if collapsed {
        llimphi_icons::Icon::ChevronRight
    } else {
        llimphi_icons::Icon::ChevronDown
    };
    let marker: View<HostMsg> = if expandable {
        View::new(Style {
            size: Size {
                width: length(14.0_f32),
                height: length(14.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![llimphi_icons::icon_view(
            chevron_icon,
            theme.fg_muted,
            1.6,
        )])
    } else {
        // Sin salida: un guion tenue en lugar del chevron (no clickable).
        View::new(Style {
            size: Size {
                width: length(14.0_f32),
                height: length(14.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
    };

    let cmd_color = if no_output {
        theme.fg_muted
    } else {
        theme.accent
    };
    let mut header_children: Vec<View<HostMsg>> = vec![
        marker,
        View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(16.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(header_text.clone(), 12.0, cmd_color, Alignment::Start)
        .mono()
        // Comando largo: una sola fila (el header es de altura fija); si no,
        // wrappea y pisa la fila de etapas / el cuerpo de abajo.
        .max_lines(1),
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
            // `.mono()` para que el `»` salga de la fuente embebida (que sí lo
            // tiene) y no como tofu de la fuente del sistema.
            .text_aligned("» stdin".to_string(), 10.0, fg, Alignment::Start)
            .mono(),
        );
    }
    // Badge: icono de estado (verde ✓ / rojo ✕ / ⏹ / ▶ corriendo) + cuándo
    // corrió ("hace 4 min"), en vez del crudo "exit N".
    if let Some(st) = status {
        let (icon, color) = st.icon_color(theme);
        let when = relative_time(
            state.block_started.get(&block).copied().unwrap_or(0),
            now_unix_secs(),
        );
        let icon_box: View<HostMsg> = View::new(Style {
            size: Size {
                width: length(13.0_f32),
                height: length(13.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![llimphi_icons::icon_view(icon, color, 1.8)]);
        let mut badge_children = vec![icon_box];
        if !when.is_empty() {
            badge_children.push(
                View::new(Style {
                    size: Size {
                        width: Dimension::auto(),
                        height: length(16.0_f32),
                    },
                    ..Default::default()
                })
                .text_aligned(when, 10.0, theme.fg_muted, Alignment::End),
            );
        }
        header_children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: Dimension::auto(),
                    height: length(16.0_f32),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(4.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(badge_children),
        );
    }

    // El header sólo se hunde y es clickable si el bloque es expandible; los
    // sin salida quedan planos (no invitan al click).
    let mut header_view = View::new(Style {
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
    .radius(4.0)
    .children(header_children);
    if expandable {
        header_view = header_view
            .fill(theme.bg_input)
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::ToggleBlock(block)));
    }
    let header = header_view;

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
                    format!("…  {} líneas ocultas · clic para ver", body.len()),
                    11.0,
                    theme.fg_muted,
                    Alignment::Start,
                )
                .mono()
                // Key distinta del cuerpo (mismo bloque) para que el resumen
                // tenga su propia animación de aparición/desaparición.
                .animated_inout(block ^ (1 << 63), COLLAPSE_ANIM),
            );
            child_h_sum += ROW_H;
        }
    } else {
        // Cuerpo como text de IDE read-only: numeración + selección moderna +
        // copiar (click derecho), CON coloreo semántico propio (ls por tipo
        // de archivo, paths/urls/grep/sha, stderr en rojo) vía
        // `text_editor_view_colored`. La fuente de verdad sigue siendo el
        // buffer de output; el editor se reconstruye por frame desde él + el
        // cursor en `state.body_sel`. (Los paths siguen sin ser *clickables*
        // —el editor no expone spans accionables todavía—; se copian con
        // selección/doble-click. Deuda anotada.)
        let body_lines = body_lines_for_block(state, block);
        if !body_lines.is_empty() {
            let n = body_lines.len();
            let mut ed = body_editor_state(state, block);
            // Tinte rojo tenue de fondo en líneas stderr — refuerza la señal.
            let stderr_tint = llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(
                220, 110, 110, 28,
            );
            ed.line_tints = body_kinds_for_block(state, block)
                .into_iter()
                .map(|k| {
                    if matches!(k, OutputKind::Stderr) {
                        Some(stderr_tint)
                    } else {
                        None
                    }
                })
                .collect();
            let color_runs = body_color_runs(state, block, theme);
            let metrics = body_editor_metrics();
            let palette = body_editor_palette(theme);
            let lift_ptr = (*lift).clone();
            let lift_dbl = (*lift).clone();
            let editor = llimphi_widget_text_editor::text_editor_view_colored::<HostMsg>(
                &ed,
                &palette,
                metrics,
                n,
                &color_runs,
                move |ev| Some(lift_ptr(Msg::BodyPointer { block, ev })),
            )
            // El click derecho del cuerpo se delega al nodo raíz (menú
            // contextual con coords de su espacio); no lo capturamos acá.
            // Doble-click = seleccionar palabra. `(lx,ly)` es local al nodo
            // del editor (incluye el gutter); `update` resta `gutter_width`.
            .on_double_tap_at(move |lx, ly, _w, _h| {
                Some(lift_dbl(Msg::BodyDoubleClick {
                    block,
                    x: lx,
                    y: ly,
                }))
            })
            // Colapsar/desplegar con transición (fade in/out), no salto seco.
            // Key estable por bloque para que el runtime reconcilie su anim.
            .animated_inout(block, COLLAPSE_ANIM);
            card_children.push(editor);
            child_h_sum += n as f32 * ROW_H;
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
    .radius(5.0)
    .children(card_children);

    // Fondo por recencia: el más reciente (depth 0) negro profundo, los de
    // atrás menos negros, con un gradiente vertical sutil (un toque de acento
    // abajo, más marcado cuanto más reciente) — "sutil pero interesante".
    use llimphi_ui::llimphi_raster::peniko::Gradient;
    use llimphi_ui::llimphi_raster::kurbo::Point;
    let base = recency_base(theme, depth);
    let top = mix_color(
        base,
        llimphi_ui::llimphi_raster::peniko::Color::WHITE,
        0.04 * (1.0 - depth),
    );
    let bottom = mix_color(base, theme.accent, 0.07 * (1.0 - depth));
    let grad = Gradient::new_linear(Point::new(0.5, 0.0), Point::new(0.5, 1.0))
        .with_stops([top, bottom].as_slice());
    let view = view.fill(base).fill_gradient(grad);

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

    // `max_lines(1)`: el nodo es de altura fija (16px). Sin esto, una línea
    // larga wrappea a 2+ filas y la sobrante se pinta ENCIMA de la línea de
    // abajo (solapamiento). Cortamos a una sola fila — igual que el cuerpo IDE,
    // que no envuelve. El resto se pierde a la derecha (clip), no se apila.
    match line.kind {
        OutputKind::Prompt => View::new(line_style)
            .text_aligned(line.text.clone(), 12.0, theme.accent, Alignment::Start)
            .mono()
            .max_lines(1),
        OutputKind::Notice => View::new(line_style)
            .text_aligned(line.text.clone(), 12.0, theme.fg_muted, Alignment::Start)
            .mono()
            .max_lines(1),
        OutputKind::Stdout | OutputKind::Stderr => {
            let base = if matches!(line.kind, OutputKind::Stderr) {
                theme.fg_destructive
            } else {
                theme.fg_text
            };
            let decorations = shuma_line::decorate_line(&line.text, cwd);
            // Atajo: si no hubo decoraciones, una sola text_aligned alcanza.
            if decorations.is_empty() {
                return View::new(line_style)
                    .text_aligned(line.text.clone(), 12.0, base, Alignment::Start)
                    .mono()
                    .max_lines(1);
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
            // Clip: spans en Row nowrap; si uno desborda no debe pisar la fila
            // de abajo (misma razón que el `max_lines(1)` de las líneas planas).
            .clip(true)
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
        .text_aligned(p.text.clone(), 12.0, p.color, Alignment::Start)
        .mono();
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

#[cfg(test)]
mod gpu_grid_tests {
    use super::*;

    fn snap_of(cells: &[&[(char, vt100::Color, vt100::Color)]]) -> TuiSnapshot {
        let rows = cells.len() as u16;
        let cols = cells.first().map(|r| r.len()).unwrap_or(0) as u16;
        let mut grid: Vec<Vec<TuiCell>> = Vec::with_capacity(rows as usize);
        for row in cells {
            grid.push(
                row.iter()
                    .map(|(ch, fg, bg)| TuiCell {
                        ch: ch.to_string(),
                        fg: *fg,
                        bg: *bg,
                    })
                    .collect(),
            );
        }
        TuiSnapshot {
            cells: grid,
            rows,
            cols,
            cursor_r: 0,
            cursor_c: 0,
            hide_cursor: true,
        }
    }

    fn atlas() -> llimphi_widget_terminal::GlyphAtlas {
        llimphi_widget_terminal::GlyphAtlas::new(
            llimphi_ui::llimphi_text::MONO_FONT_BYTES,
            14.0,
            16,
            4,
        )
        .expect("atlas")
    }

    fn rect_400_200() -> llimphi_ui::PaintRect {
        llimphi_ui::PaintRect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        }
    }

    #[test]
    fn build_skip_blanks_con_bg_default() {
        let snap = snap_of(&[&[
            (' ', vt100::Color::Default, vt100::Color::Default),
            (' ', vt100::Color::Default, vt100::Color::Default),
        ]]);
        let mut a = atlas();
        let theme = llimphi_theme::Theme::dark();
        let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
        assert!(cells.is_empty(), "celdas vacías con bg default no van");
    }

    #[test]
    fn build_emite_un_instance_por_celda_con_contenido() {
        let snap = snap_of(&[
            &[
                ('h', vt100::Color::Default, vt100::Color::Default),
                ('i', vt100::Color::Default, vt100::Color::Default),
            ],
            &[
                (' ', vt100::Color::Default, vt100::Color::Default),
                ('!', vt100::Color::Default, vt100::Color::Default),
            ],
        ]);
        let mut a = atlas();
        let theme = llimphi_theme::Theme::dark();
        let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
        // Tres chars no-blank (h, i, !), el ' ' con bg default se salta.
        assert_eq!(cells.len(), 3);
        // El primer instance debe arrancar en (pad, pad).
        assert_eq!(cells[0].cell_x, 6.0);
        assert_eq!(cells[0].cell_y, 6.0);
    }

    #[test]
    fn build_no_skip_si_bg_explicito() {
        // Una celda con ' ' pero bg explícito (Idx) SÍ se emite (el bg
        // tiene que pintarse aunque el char sea blank).
        let snap = snap_of(&[&[
            (' ', vt100::Color::Default, vt100::Color::Idx(1)),
            (' ', vt100::Color::Default, vt100::Color::Default),
        ]]);
        let mut a = atlas();
        let theme = llimphi_theme::Theme::dark();
        let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
        // Sólo el primero (bg explícito); el segundo (bg default) se salta.
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn build_uv_y_color_son_consistentes() {
        let snap = snap_of(&[&[('A', vt100::Color::Default, vt100::Color::Default)]]);
        let mut a = atlas();
        let theme = llimphi_theme::Theme::dark();
        let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
        assert_eq!(cells.len(), 1);
        let (acw, ach) = a.cell_size();
        // UV apunta al slot 0 (primer glifo rasterizado).
        assert_eq!(cells[0].uv_x, 0.0);
        assert_eq!(cells[0].uv_y, 0.0);
        assert_eq!(cells[0].uv_w, acw as f32);
        assert_eq!(cells[0].uv_h, ach as f32);
        // fg y bg no son 0 (fg = theme.fg_text, bg = default → alpha 0
        // pero los componentes no se chequean — basta con que el instance
        // se haya armado sin pánico).
        assert_ne!(cells[0].fg_rgba, 0);
    }
}
