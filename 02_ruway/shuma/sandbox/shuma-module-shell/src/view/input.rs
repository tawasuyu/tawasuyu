use super::*;

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
    // byte offset del cursor. El alto se CAPEA a `MAX_INPUT_LINES`
    // visibles: un paste largo no aplasta el output ni empuja el cursor
    // fuera de la ventana — el contenido scrollea adentro, anclado a la
    // línea del cursor (siempre visible).
    const MAX_INPUT_LINES: usize = 12;
    let line_count = text.matches('\n').count() + 1;
    let visible_lines = line_count.min(MAX_INPUT_LINES);
    let zoom = state.font_zoom.clamp(0.5, 3.0) as f64;
    let font_px = 13.0_f64 * zoom;
    let line_h: f64 = 18.0_f64 * zoom;
    let border_inner_h: f64 = 16.0_f64 * zoom; // padding visual sumado al alto
    let container_h = border_inner_h + line_h * visible_lines as f64;
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

        // Scroll interno cuando el texto excede las líneas visibles: la
        // ventana sigue al cursor (queda en la última fila visible al
        // tipear al final; al subir con flechas, la ventana sube con él).
        let scroll_lines = cursor_line_idx.saturating_sub(visible_lines - 1);

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
            // Fuera de la ventana visible: ni se mide ni se pinta.
            if line_idx < scroll_lines || line_idx >= scroll_lines + visible_lines {
                line_byte_start += line_str.len() + 1;
                continue;
            }
            let line_y = baseline_y + (line_idx - scroll_lines) as f64 * line_h;
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

        // Indicador de overflow: con más líneas que las visibles, un
        // contador discreto arriba a la derecha ubica al usuario
        // ("línea 18/40") — sin él, el cap parecería texto perdido.
        if line_count > visible_lines {
            let label = format!("línea {}/{}", cursor_line_idx + 1, line_count);
            let block = TextBlock {
                text: &label,
                size_px: (font_px * 0.8) as f32,
                color: theme_clone.fg_muted,
                origin: (0.0, 0.0),
                max_width: None,
                alignment: TAlign::Start,
                line_height: 1.0,
                italic: false,
                font_family: Some(llimphi_ui::llimphi_text::MONOSPACE.to_string()),
            };
            let layout = layout_block(ts, &block);
            let w = measurement(&layout).width as f64;
            let origin = (rect.x as f64 + rect.w as f64 - w - 10.0, rect.y as f64 + 4.0);
            draw_layout(scene, &layout, theme_clone.fg_muted, origin);
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
    // El cap de líneas visibles scrollea el contenido adentro; sin clip,
    // una línea parcial se filtraría fuera del marco.
    .clip(true)
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
    // `hover_fill` necesario para que el hit-test de hover elija este nodo y
    // dispare el `on_pointer_enter` (Llimphi sólo hoverea nodos con hover_fill).
    // Tinte mínimo sobre el marco; el `inner` lo tapa casi entero.
    .hover_fill(border)
    .on_click(lift(Msg::FocusInput))
    // Pasar el mouse sobre la línea la re-foca: el Enter vuelve a arrancar
    // comandos (deja de alimentar el stdin de un job). Es la otra mitad del
    // "alternar foco con mousemove": mouse sobre un job vivo → le doy input;
    // mouse sobre la línea → arranco otro comando en paralelo.
    .on_pointer_enter(lift(Msg::FocusInput))
    .children(vec![inner])
}

/// Dialect por defecto para el painter — el `LineState` lo guarda
/// internamente pero no lo expone; mientras todos los usos sean bash
/// alcanza con este getter.
pub(crate) fn state_dialect_default() -> shuma_line::Dialect {
    shuma_line::Dialect::default()
}

/// Popup de completado: lista de candidatos con el actual resaltado. Se
/// pinta sobre el input (en la columna, justo antes). Acota a `MAX_ROWS`
/// filas visibles centradas en el índice. `None` si no hay popup abierto.
pub(crate) fn completion_popup<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
) -> Option<View<HostMsg>> {
    let comp = state.completion.as_ref()?;
    // Lista unificada en capas: candidatos de token (tier 1) + sugerencias de
    // línea/grupo (tiers 2/3). Cada fila lleva su texto ya pintado y si es un
    // tier "alto" (línea/grupo) para distinguirlo visualmente.
    use crate::types::SugKind;
    let mut entries: Vec<(String, Option<SugKind>)> = Vec::new();
    for c in &comp.candidates {
        entries.push((c.clone(), None));
    }
    for sug in &state.completion_extra {
        entries.push((sug.display.clone(), Some(sug.kind)));
    }
    if entries.is_empty() {
        return None;
    }
    const MAX_ROWS: usize = 8;
    const ROW: f32 = 18.0;
    let n = entries.len();
    let sel = state.completion_index.min(n - 1);
    // Ventana deslizante centrada en la selección.
    let start = sel.saturating_sub(MAX_ROWS / 2).min(n.saturating_sub(MAX_ROWS));
    let end = (start + MAX_ROWS).min(n);

    let mut rows: Vec<View<HostMsg>> = Vec::new();
    for (i, (cand, tier)) in entries[start..end].iter().enumerate() {
        let idx = start + i;
        let selected = idx == sel;
        // Los tiers altos (línea/grupo) se pintan en el color de acento tenue
        // cuando no están resaltados, para leerse como "otra capa".
        let (fill, fg) = if selected {
            (theme.accent, theme.bg_panel)
        } else {
            match tier {
                Some(_) => (theme.bg_input, theme.fg_muted),
                None => (theme.bg_input, theme.fg_text),
            }
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
