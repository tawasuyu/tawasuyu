use super::*;

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
