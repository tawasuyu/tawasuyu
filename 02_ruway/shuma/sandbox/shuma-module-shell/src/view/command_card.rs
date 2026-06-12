use super::*;

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

/// Bytes a etiqueta compacta: `840`, `1.2K`, `3.4M`. Sin espacio para que
/// quepa en el chip.
pub(crate) fn humanize_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}K", n as f32 / 1024.0)
    } else {
        format!("{:.1}M", n as f32 / (1024.0 * 1024.0))
    }
}

/// Color estable por índice de etapa — para que cada etapa del pipe lea
/// distinto de un vistazo (chip + sus líneas + su barra-guía).
pub(crate) fn stage_color(i: usize) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (r, g, b) = STAGE_PALETTE[i % STAGE_PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

/// Misma tinta, atenuada (alfa 80%) — para el texto de las líneas
/// capturadas: menos peso visual que el chip que las titula.
pub(crate) fn stage_color_dim(i: usize) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (r, g, b) = STAGE_PALETTE[i % STAGE_PALETTE.len()];
    Color::from_rgba8(r, g, b, 204)
}
