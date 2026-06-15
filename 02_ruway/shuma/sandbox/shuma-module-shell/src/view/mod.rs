use llimphi_ui::llimphi_layout::taffy::style::Position;
use super::*;

mod input;
mod tui;
mod ansi;
mod history_panel;
mod surface_view;
mod command_card;
mod output_line;
#[cfg(test)]
mod gpu_grid_tests;

pub(crate) use input::*;
pub(crate) use tui::*;
pub(crate) use ansi::*;
pub(crate) use history_panel::*;
pub(crate) use surface_view::*;
pub(crate) use command_card::*;
pub(crate) use output_line::*;

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
    } else {
        output_pane_surface::<HostMsg>(state, theme, &lift)
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
    // Popup de completado (Tab / as-you-type): el input vive en la barra del
    // host (pata) y alimenta `state.completion`; el popup se pinta acá, en el
    // cuerpo adyacente. Sin esto el host no tenía autocomplete visible (sólo el
    // ghost inline del input) — parecía "una shuma de cartón". El `view`
    // standalone ya lo incluye; lo espejamos para `body_view`.
    if let Some(popup) = completion_popup::<HostMsg>(state, theme) {
        children.push(popup);
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
    } else {
        // El output va por la superficie de terminal virtualizada (única vía
        // desde la Fase 5 del SDD-TERMINAL: el `output_pane` viejo + las cards
        // per-comando IDE fueron borrados).
        output_pane_surface::<HostMsg>(state, theme, &lift)
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
    if let Some(banner) = input_focus_banner::<HostMsg>(state, theme, &lift) {
        children.push(banner);
    }
    // A1 — oferta de coreografía: discreta, justo sobre el input. A2 — oferta
    // de alias para una línea larga repetida: el gemelo de A1, pero sólo si no
    // hay coreografía pendiente (una sola oferta a la vez, sin apilar chips).
    if let Some(chip) = choreography_chip::<HostMsg>(state, theme, &lift) {
        children.push(chip);
    } else if let Some(chip) = alias_chip::<HostMsg>(state, theme, &lift) {
        children.push(chip);
    }
    children.push(input);
    if state.history_search.is_some() {
        children.push(history_search_panel::<HostMsg>(state, theme));
    }
    // El menú contextual del output (click derecho) lo arma y pinta la propia
    // superficie (`surf_context_menu`, dentro de `output_pane_surface`), sobre
    // su selección del stream — ya no hay un menú legacy a nivel del root.

    let lift_scale = lift.clone();
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
    // Ctrl+rueda (o pinch de trackpad) sobre cualquier parte del shell = zoom
    // del texto. Va por `on_scale` —que el runtime resuelve ANTES que el
    // `on_scroll` de la superficie de output— para que la rueda con Ctrl no se
    // la coma el scroll del cuerpo (era el bug del "zoom con mouse que falta").
    // `factor` es el cambio multiplicativo incremental (>1 agranda); `ZoomBy`
    // lo aplica igual que el pinch.
    .on_scale(move |_phase, factor, _fx, _fy| Some(lift_scale(Msg::ZoomBy(factor))))
    .children(children)
}

/// Banner sobre la línea que avisa a qué comando vivo va el Enter (stdin),
/// cuando el input está dirigido a un job en vez de a la línea. `None` cuando
/// el foco es la línea (arrancar comandos). Click → vuelve a la línea.
pub(crate) fn input_focus_banner<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    let block = state.input_focus?;
    // Sólo si el destino sigue vivo (si murió, el update ya limpió el foco; este
    // chequeo cubre el frame intermedio).
    let arc = state.job_by_block(block)?;
    let cmd = arc.lock().ok().map(|g| g.command.clone())?;
    let label = format!("→ Enter va al stdin de «{cmd}»  ·  click o mouse sobre la línea para volver a tipear comandos");
    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(18.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_input_focus)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(Msg::FocusInput))
        .text_aligned(label, 10.0, theme.accent, Alignment::Start)
        .mono()
        .max_lines(1),
    )
}

/// A1 — chip de coreografía sobre el input: cuando una secuencia repetida
/// supera el umbral, ofrece guardarla como grupo ejecutable. El shell propone,
/// el usuario acepta con un click («guardar» → F-key) o la descarta. `None`
/// si no hay ninguna coreografía que ofrecer. Discreto y descartable: nunca
/// bloquea, nunca ejecuta nada solo.
pub(crate) fn choreography_chip<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    let p = choreography_suggestion(state)?;
    let name = p.suggested_name();
    let preview = p.example.join(" → ");
    let label = format!(
        "↻ lo corriste {} veces · guardar «{name}» como grupo?  ({preview})",
        p.occurrences
    );
    let sig = p.signature.clone();

    // Chip de acción reutilizable (innermost-wins: gana el click sobre el banner).
    let action = |text: &str,
                  fill: llimphi_ui::llimphi_raster::peniko::Color,
                  fg: llimphi_ui::llimphi_raster::peniko::Color,
                  msg: Msg|
     -> View<HostMsg> {
        View::new(Style {
            size: Size { width: Dimension::auto(), height: length(16.0_f32) },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(7.0_f32),
                right: length(7.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(fill)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(msg))
        .text_aligned(text.to_string(), 10.0, fg, Alignment::Start)
        .mono()
    };

    Some(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(20.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(4.0)
        .children(vec![
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_grow: 1.0,
                ..Default::default()
            })
            .text_aligned(label, 10.0, theme.accent, Alignment::Start)
            .mono()
            .max_lines(1),
            action(
                "guardar",
                theme.accent,
                theme.bg_panel,
                Msg::AcceptChoreography(sig.clone()),
            ),
            action(
                "descartar",
                theme.bg_input,
                theme.fg_muted,
                Msg::DismissChoreography(sig),
            ),
        ]),
    )
}

/// A2 — chip de alias sobre el input: cuando una **línea larga** se repitió
/// varias veces idéntica, ofrece bautizarla con un nombre corto (`[aliases]`
/// del shumarc, vía `upsert_key`). Mismo molde que la coreografía (A1), otra
/// fuente: A1 abstrae una *secuencia*, A2 acorta *una* línea. El shell propone,
/// el usuario acepta con un click («aliasar» → aprendido al rc) o la descarta.
/// `None` si no hay ninguna línea que valga acortar.
pub(crate) fn alias_chip<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> Option<View<HostMsg>> {
    let sug = alias_suggestion(state)?;
    let label = format!(
        "⌁ lo tecleaste {} veces · acortar a «{}»?  ({})",
        sug.count, sug.name, sug.line
    );
    let line = sug.line.clone();

    let action = |text: &str,
                  fill: llimphi_ui::llimphi_raster::peniko::Color,
                  fg: llimphi_ui::llimphi_raster::peniko::Color,
                  msg: Msg|
     -> View<HostMsg> {
        View::new(Style {
            size: Size { width: Dimension::auto(), height: length(16.0_f32) },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(7.0_f32),
                right: length(7.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(fill)
        .radius(3.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(lift(msg))
        .text_aligned(text.to_string(), 10.0, fg, Alignment::Start)
        .mono()
    };

    Some(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(20.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(4.0)
        .children(vec![
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(16.0_f32) },
                flex_grow: 1.0,
                ..Default::default()
            })
            .text_aligned(label, 10.0, theme.accent, Alignment::Start)
            .mono()
            .max_lines(1),
            action(
                "aliasar",
                theme.accent,
                theme.bg_panel,
                Msg::AcceptAlias(line.clone()),
            ),
            action(
                "descartar",
                theme.bg_input,
                theme.fg_muted,
                Msg::DismissAlias(line),
            ),
        ]),
    )
}
